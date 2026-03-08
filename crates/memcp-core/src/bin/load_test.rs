//! Load test binary stub.
//!
//! The full concurrent client driver is implemented in Plan 02.
//! This stub allows the binary target to compile so the `[[bin]]` entry
//! in Cargo.toml is valid during Plan 01 development.

fn main() {
    eprintln!("load-test binary not yet implemented — see Plan 02 for the concurrent client driver");
    std::process::exit(1);
}
