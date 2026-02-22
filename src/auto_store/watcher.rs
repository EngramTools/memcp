/// File watcher for auto-store sidecar.
///
/// Uses the `notify` crate for filesystem events with byte-offset tailing.
/// Falls back to polling if fs events are unreliable.
/// Gracefully handles missing files (log warning, retry on next poll).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

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

/// Read new lines from a file starting at the given byte offset.
/// Returns the lines read and the new offset.
fn read_new_lines(path: &Path, offset: u64) -> (Vec<String>, u64) {
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
pub fn spawn_watcher(
    watch_paths: Vec<String>,
    poll_interval: Duration,
    tx: mpsc::Sender<WatchEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn(async move {
        let expanded_paths: Vec<PathBuf> = watch_paths.iter().map(|p| expand_tilde(p)).collect();

        // Track byte offsets per file
        let mut file_states: HashMap<PathBuf, FileState> = HashMap::new();

        // Initialize offsets — seek to end of existing files so we only get new content
        for path in &expanded_paths {
            if path.exists() {
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                file_states.insert(path.clone(), FileState { offset: len });
                tracing::info!(path = %path.display(), offset = len, "Watching file (starting at end)");
            } else {
                file_states.insert(path.clone(), FileState { offset: 0 });
                tracing::warn!(path = %path.display(), "Watch target does not exist yet — will watch for creation");
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

        // Watch parent directories (to catch file creation)
        if let Some(ref mut w) = watcher {
            for path in &expanded_paths {
                if let Some(parent) = path.parent() {
                    if parent.exists() {
                        if let Err(e) = w.watch(parent, RecursiveMode::NonRecursive) {
                            tracing::warn!(
                                path = %parent.display(),
                                error = %e,
                                "Failed to watch directory"
                            );
                        }
                    }
                }
            }
        }

        let mut poll_interval_timer = tokio::time::interval(poll_interval);

        loop {
            tokio::select! {
                // Triggered by fs events
                Some(changed_path) = notify_rx.recv() => {
                    // Check if this path is one we're watching
                    for path in &expanded_paths {
                        if changed_path == *path || changed_path.starts_with(path.parent().unwrap_or(path)) {
                            process_file(path, &mut file_states, &tx).await;
                        }
                    }
                }

                // Fallback polling
                _ = poll_interval_timer.tick() => {
                    for path in &expanded_paths {
                        process_file(path, &mut file_states, &tx).await;
                    }
                }
            }
        }
    })
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
        if tx.send(WatchEvent {
            path: path.to_path_buf(),
            line,
        }).await.is_err() {
            tracing::warn!("Watch event channel closed");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/some/path");
        assert!(!expanded.to_string_lossy().contains('~'));
        assert!(expanded.to_string_lossy().ends_with("some/path"));

        let no_tilde = expand_tilde("/absolute/path");
        assert_eq!(no_tilde, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_read_new_lines_basic() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "line1").unwrap();
        writeln!(file, "line2").unwrap();

        let (lines, offset) = read_new_lines(file.path(), 0);
        assert_eq!(lines, vec!["line1", "line2"]);
        assert!(offset > 0);

        // Reading again from same offset yields nothing
        let (lines2, offset2) = read_new_lines(file.path(), offset);
        assert!(lines2.is_empty());
        assert_eq!(offset, offset2);

        // Append more
        writeln!(file, "line3").unwrap();
        let (lines3, _) = read_new_lines(file.path(), offset);
        assert_eq!(lines3, vec!["line3"]);
    }

    #[test]
    fn test_read_new_lines_missing_file() {
        let (lines, offset) = read_new_lines(Path::new("/nonexistent/file"), 0);
        assert!(lines.is_empty());
        assert_eq!(offset, 0);
    }

    #[test]
    fn test_read_new_lines_truncated_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "long line of content here").unwrap();
        let (_, offset) = read_new_lines(file.path(), 0);

        // Truncate the file
        file.as_file().set_len(0).unwrap();
        writeln!(file, "new").unwrap();

        // Should detect truncation and read from beginning
        let (lines, _) = read_new_lines(file.path(), offset);
        assert_eq!(lines, vec!["new"]);
    }
}
