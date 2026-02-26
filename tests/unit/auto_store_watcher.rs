use memcp::auto_store::watcher::{
    expand_tilde, is_jsonl, read_new_lines, scan_directory_jsonl, spawn_watcher, WatchEvent,
};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;

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

#[test]
fn test_scan_directory_jsonl() {
    let dir = tempfile::tempdir().unwrap();

    // Create mix of .jsonl and other files
    std::fs::write(dir.path().join("session1.jsonl"), "line1\n").unwrap();
    std::fs::write(dir.path().join("session2.jsonl"), "line2\n").unwrap();
    std::fs::write(dir.path().join("readme.txt"), "not jsonl\n").unwrap();
    std::fs::write(dir.path().join("data.json"), "not jsonl\n").unwrap();

    let files = scan_directory_jsonl(dir.path());
    assert_eq!(files.len(), 2);
    assert!(files[0].extension().unwrap() == "jsonl");
    assert!(files[1].extension().unwrap() == "jsonl");
}

#[test]
fn test_scan_directory_jsonl_recursive() {
    let dir = tempfile::tempdir().unwrap();

    // Simulate ~/.openclaw/agents structure
    let vita_sessions = dir.path().join("vita").join("sessions");
    let seba_sessions = dir.path().join("seba").join("sessions");
    std::fs::create_dir_all(&vita_sessions).unwrap();
    std::fs::create_dir_all(&seba_sessions).unwrap();

    std::fs::write(vita_sessions.join("s1.jsonl"), "line\n").unwrap();
    std::fs::write(vita_sessions.join("s2.jsonl"), "line\n").unwrap();
    std::fs::write(seba_sessions.join("s1.jsonl"), "line\n").unwrap();
    // Non-jsonl should be ignored
    std::fs::write(seba_sessions.join("config.json"), "ignored\n").unwrap();

    let files = scan_directory_jsonl(dir.path());
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| f.extension().unwrap() == "jsonl"));
}

#[test]
fn test_scan_directory_missing() {
    let files = scan_directory_jsonl(Path::new("/nonexistent/dir"));
    assert!(files.is_empty());
}

#[test]
fn test_is_jsonl() {
    assert!(is_jsonl(Path::new("/foo/bar.jsonl")));
    assert!(!is_jsonl(Path::new("/foo/bar.json")));
    assert!(!is_jsonl(Path::new("/foo/bar.txt")));
    assert!(!is_jsonl(Path::new("/foo/bar")));
}

#[tokio::test]
async fn test_directory_watching_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::channel::<WatchEvent>(100);

    // Spawn watcher on the directory
    let _handle = spawn_watcher(
        vec![dir.path().to_string_lossy().to_string()],
        Duration::from_millis(100),
        tx,
    );

    // Give watcher time to initialize
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create a new .jsonl file in the watched directory
    let file_path = dir.path().join("new-session.jsonl");
    std::fs::write(&file_path, "{\"content\":\"hello\"}\n").unwrap();

    // Should receive the line within a poll interval
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for watch event")
        .expect("channel closed");

    assert_eq!(event.line, "{\"content\":\"hello\"}");
    assert_eq!(event.path, file_path);
}
