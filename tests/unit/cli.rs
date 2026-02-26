use chrono::Utc;
use memcp::cli::format_relative_time;

#[test]
fn test_format_relative_time() {
    let now = Utc::now();
    assert!(format_relative_time(now).contains("s ago"));
    assert!(format_relative_time(now - chrono::Duration::minutes(5)).contains("5m ago"));
    assert!(format_relative_time(now - chrono::Duration::hours(2)).contains("2h ago"));
    assert!(format_relative_time(now - chrono::Duration::days(3)).contains("3d ago"));
}

#[test]
fn test_format_relative_time_negative_clamps_to_zero() {
    // Future time should clamp to 0s ago
    let future = Utc::now() + chrono::Duration::hours(1);
    assert!(format_relative_time(future).contains("0s ago"));
}
