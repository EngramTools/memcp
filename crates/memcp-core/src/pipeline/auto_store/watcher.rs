//! File watcher for auto-store sidecar.
//!
//! Uses the `notify` crate for filesystem events with byte-offset tailing.
//! Falls back to polling if fs events are unreliable.
//! Gracefully handles missing files (log warning, retry on next poll).
//! Supports both specific file paths and directory paths (watches for new `.jsonl` files).

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// A new line read from a watched file.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// The file path this line came from
    pub path: PathBuf,
    /// The line content (without trailing newline)
    pub line: String,
}

/// Tracks byte offset per watched file for incremental reads.
struct FileState {
    offset: u64,
}

/// Expand `~` at the start of a path to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Recursively scan a directory for `.jsonl` files.
/// Returns paths sorted alphabetically. Gracefully returns empty vec if dir doesn't exist.
pub fn scan_directory_jsonl(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    scan_directory_jsonl_inner(dir, &mut files);
    files.sort();
    files
}

fn scan_directory_jsonl_inner(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            scan_directory_jsonl_inner(&path, out);
        } else if path.is_file() && is_jsonl(&path) {
            out.push(path);
        }
    }
}

/// Read new lines from a file starting at the given byte offset.
/// Returns the lines read and the new offset.
/// Read new lines from a file starting at `offset`. Exposed as `pub` for external test access.
pub fn read_new_lines(path: &Path, offset: u64) -> (Vec<String>, u64) {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (Vec::new(), offset),
    };

    let metadata = match file.metadata() {
        Ok(m) => m,
        Err(_) => return (Vec::new(), offset),
    };

    let file_len = metadata.len();

    // File was truncated or rotated — reset to beginning
    if file_len < offset {
        tracing::debug!(path = %path.display(), "File truncated, resetting offset to 0");
        return read_new_lines(path, 0);
    }

    // No new data
    if file_len == offset {
        return (Vec::new(), offset);
    }

    let mut reader = BufReader::new(file);
    if let Err(e) = reader.seek(SeekFrom::Start(offset)) {
        tracing::warn!(path = %path.display(), error = %e, "Failed to seek in file");
        return (Vec::new(), offset);
    }

    let mut lines = Vec::new();
    let mut new_offset = offset;

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(n) => {
                new_offset += n as u64;
                let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Error reading line");
                break;
            }
        }
    }

    (lines, new_offset)
}

/// Spawn a file watcher that sends new lines to the provided channel.
///
/// Uses `notify` for fs events with a fallback polling interval.
/// Handles missing files gracefully — logs a warning and retries on each poll.
///
/// Supports two kinds of watch_paths:
/// - **File paths**: tails a specific file (existing behavior)
/// - **Directory paths**: watches for `.jsonl` files, picks up new ones as they appear
pub fn spawn_watcher(
    watch_paths: Vec<String>,
    poll_interval: Duration,
    tx: mpsc::Sender<WatchEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn(async move {
        let expanded_paths: Vec<PathBuf> = watch_paths.iter().map(|p| expand_tilde(p)).collect();

        // Separate into file paths and directory paths
        let mut file_paths: Vec<PathBuf> = Vec::new();
        let mut dir_paths: Vec<PathBuf> = Vec::new();

        for path in &expanded_paths {
            if path.is_dir() {
                dir_paths.push(path.clone());
            } else {
                file_paths.push(path.clone());
            }
        }

        // Track byte offsets per file
        let mut file_states: HashMap<PathBuf, FileState> = HashMap::new();

        // Initialize offsets for explicit file paths — seek to end so we only get new content
        for path in &file_paths {
            if path.exists() {
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                file_states.insert(path.clone(), FileState { offset: len });
                tracing::info!(path = %path.display(), offset = len, "Watching file (starting at end)");
            } else {
                file_states.insert(path.clone(), FileState { offset: 0 });
                tracing::warn!(path = %path.display(), "Watch target does not exist yet — will watch for creation");
            }
        }

        // Initialize offsets for files already in watched directories — seek to end
        for dir in &dir_paths {
            let existing_files = scan_directory_jsonl(dir);
            tracing::info!(
                dir = %dir.display(),
                files = existing_files.len(),
                "Watching directory for .jsonl files (starting at end of existing files)"
            );
            for file in existing_files {
                let len = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                file_states.insert(file, FileState { offset: len });
            }
        }

        // Set up notify watcher with a channel
        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<PathBuf>(100);

        let notify_tx_clone = notify_tx.clone();
        let mut watcher: Option<RecommendedWatcher> = match notify::recommended_watcher(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                        for path in event.paths {
                            let _ = notify_tx_clone.try_send(path);
                        }
                    }
                }
            },
        ) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to create fs watcher, falling back to polling only");
                None
            }
        };

        // Watch parent directories of file paths (to catch file creation)
        if let Some(ref mut w) = watcher {
            for path in &file_paths {
                if let Some(parent) = path.parent() {
                    if parent.exists() {
                        if let Err(e) = w.watch(parent, RecursiveMode::NonRecursive) {
                            tracing::warn!(path = %parent.display(), error = %e, "Failed to watch directory");
                        }
                    }
                }
            }
            // Watch directory paths recursively (picks up nested agent/sessions dirs)
            for dir in &dir_paths {
                if dir.exists() {
                    if let Err(e) = w.watch(dir, RecursiveMode::Recursive) {
                        tracing::warn!(path = %dir.display(), error = %e, "Failed to watch directory");
                    }
                } else {
                    tracing::warn!(path = %dir.display(), "Watch directory does not exist yet");
                    // Watch parent to catch directory creation
                    if let Some(parent) = dir.parent() {
                        if parent.exists() {
                            let _ = w.watch(parent, RecursiveMode::NonRecursive);
                        }
                    }
                }
            }
        }

        let dir_set: HashSet<PathBuf> = dir_paths.iter().cloned().collect();
        let mut poll_interval_timer = tokio::time::interval(poll_interval);

        loop {
            tokio::select! {
                // Triggered by fs events
                Some(changed_path) = notify_rx.recv() => {
                    // Check if this event is for a .jsonl file under a watched directory
                    if is_jsonl(&changed_path) {
                        let under_watched_dir = dir_set.iter().any(|d| changed_path.starts_with(d));
                        if under_watched_dir {
                            // New or modified .jsonl under a watched directory tree — track and process
                            if !file_states.contains_key(&changed_path) {
                                tracing::info!(path = %changed_path.display(), "New .jsonl file detected in watched directory");
                                file_states.insert(changed_path.clone(), FileState { offset: 0 });
                            }
                            process_file(&changed_path, &mut file_states, &tx).await;
                            continue;
                        }
                    }

                    // Check if this is a directly watched file path
                    for path in &file_paths {
                        if changed_path == *path || changed_path.starts_with(path.parent().unwrap_or(path)) {
                            process_file(path, &mut file_states, &tx).await;
                        }
                    }
                }

                // Fallback polling — also scans directories for new files
                _ = poll_interval_timer.tick() => {
                    // Process known file paths
                    for path in &file_paths {
                        process_file(path, &mut file_states, &tx).await;
                    }

                    // Scan directories for new .jsonl files
                    for dir in &dir_paths {
                        let current_files = scan_directory_jsonl(dir);
                        for file in current_files {
                            if !file_states.contains_key(&file) {
                                tracing::info!(path = %file.display(), "New .jsonl file detected in watched directory (poll)");
                                file_states.insert(file.clone(), FileState { offset: 0 });
                            }
                            process_file(&file, &mut file_states, &tx).await;
                        }
                    }

                    // Also process all tracked files from directories
                    let tracked: Vec<PathBuf> = file_states.keys().cloned().collect();
                    for path in tracked {
                        if !file_paths.contains(&path) {
                            // Already processed in dir scan above — skip to avoid double processing
                        }
                    }
                }
            }
        }
    })
}

/// Check if a path has a `.jsonl` extension.
/// Check if a path has a `.jsonl` extension. Exposed as `pub` for external test access.
pub fn is_jsonl(path: &Path) -> bool {
    path.extension().map(|ext| ext == "jsonl").unwrap_or(false)
}

/// Read new lines from a file and send them to the channel.
async fn process_file(
    path: &Path,
    file_states: &mut HashMap<PathBuf, FileState>,
    tx: &mpsc::Sender<WatchEvent>,
) {
    let state = file_states
        .entry(path.to_path_buf())
        .or_insert(FileState { offset: 0 });

    let (lines, new_offset) = read_new_lines(path, state.offset);
    if !lines.is_empty() {
        tracing::debug!(
            path = %path.display(),
            lines = lines.len(),
            old_offset = state.offset,
            new_offset = new_offset,
            "Read new lines from watched file"
        );
    }
    state.offset = new_offset;

    for line in lines {
        if tx
            .send(WatchEvent {
                path: path.to_path_buf(),
                line,
            })
            .await
            .is_err()
        {
            tracing::warn!("Watch event channel closed");
            return;
        }
    }
}
