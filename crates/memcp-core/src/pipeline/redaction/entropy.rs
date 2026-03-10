//! Shannon entropy calculation for secret detection.
//!
//! Used to filter out placeholder values like "your-key-here" that match
//! secret patterns but have low entropy (< 3.5 bits/char typically).

use std::collections::HashMap;

/// Calculate Shannon entropy (bits per character) of a string.
///
/// Returns 0.0 for empty strings. Higher values indicate more randomness —
/// real API keys typically have entropy > 4.0, while placeholders are < 3.5.
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }

    let len = s.len() as f64;
    let mut freq: HashMap<u8, usize> = HashMap::new();
    for &b in s.as_bytes() {
        *freq.entry(b).or_insert(0) += 1;
    }

    freq.values().fold(0.0, |acc, &count| {
        let p = count as f64 / len;
        acc - p * p.log2()
    })
}
