//! Sentence-boundary text splitting with overlap for RAG chunking.
//!
//! Uses unicode-segmentation (UAX#29) for robust sentence detection that handles
//! abbreviations ("Dr. Smith"), decimal numbers ("3.14"), and URLs correctly.

use unicode_segmentation::UnicodeSegmentation;

/// Split text into sentence-grouped chunks with overlap.
///
/// Groups consecutive sentences into chunks up to `max_chars` characters.
/// Adjacent chunks overlap by `overlap_sentences` sentences for context continuity.
///
/// Returns empty Vec if:
/// - Input is empty or has no sentences
/// - All content fits in a single chunk (no splitting needed)
///
/// # Arguments
/// - `content`: The text to split
/// - `max_chars`: Maximum characters per chunk (~4 chars/token)
/// - `overlap_sentences`: Number of sentences to repeat between adjacent chunks
pub fn split_sentences(
    content: &str,
    max_chars: usize,
    overlap_sentences: usize,
) -> Vec<Vec<String>> {
    let sentences: Vec<String> = content
        .unicode_sentences()
        .map(|s| s.to_string())
        .collect();

    if sentences.is_empty() {
        return vec![];
    }

    let mut chunks: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut current_len: usize = 0;

    for sentence in &sentences {
        let s_len = sentence.len();

        // If adding this sentence exceeds max and we have content, start new chunk
        if current_len + s_len > max_chars && !current.is_empty() {
            chunks.push(current.clone());

            // Carry overlap sentences to next chunk
            let overlap_start = current.len().saturating_sub(overlap_sentences);
            current = current[overlap_start..].to_vec();
            current_len = current.iter().map(|s| s.len()).sum();
        }

        current.push(sentence.clone());
        current_len += s_len;
    }

    // Don't forget the last chunk
    if !current.is_empty() {
        chunks.push(current);
    }

    // If everything fits in one chunk, no splitting needed
    if chunks.len() <= 1 {
        return vec![];
    }

    chunks
}

/// Find the last valid char boundary at or before `index` in `s`.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Extract a topic summary from the first sentence of content.
///
/// Truncates to `max_len` characters with "..." suffix if needed.
/// Handles multi-byte characters safely via floor_char_boundary.
pub fn extract_topic(content: &str, max_len: usize) -> String {
    let first = content
        .unicode_sentences()
        .next()
        .unwrap_or("")
        .trim();

    if first.len() > max_len {
        let boundary = floor_char_boundary(first, max_len.saturating_sub(3));
        format!("{}...", &first[..boundary])
    } else {
        first.to_string()
    }
}

/// Build the context header prefix for a chunk.
///
/// Format: `[From: "{topic}", part {index+1}/{total}]\n`
pub fn make_context_header(topic: &str, chunk_index: usize, total_chunks: usize) -> String {
    format!(
        "[From: \"{}\", part {}/{}]\n",
        topic,
        chunk_index + 1,
        total_chunks,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_empty() {
        assert!(split_sentences("", 100, 1).is_empty());
    }

    #[test]
    fn test_split_single_chunk() {
        // All fits in one chunk
        let result = split_sentences("Hello world. This is a test.", 1000, 1);
        assert!(result.is_empty(), "Should return empty when all fits in one chunk");
    }

    #[test]
    fn test_split_multiple_chunks() {
        let content = "First sentence. Second sentence. Third sentence. Fourth sentence. Fifth sentence. ";
        let chunks = split_sentences(content, 40, 0);
        assert!(chunks.len() >= 2, "Should produce multiple chunks");
    }

    #[test]
    fn test_split_with_overlap() {
        let content = "First sentence. Second sentence. Third sentence. Fourth sentence. Fifth sentence. ";
        let chunks = split_sentences(content, 40, 1);
        if chunks.len() >= 2 {
            // Last sentence of chunk 0 should be first sentence of chunk 1
            let last_of_first = chunks[0].last().unwrap().clone();
            let first_of_second = chunks[1].first().unwrap().clone();
            assert_eq!(last_of_first, first_of_second, "Overlap sentences should match");
        }
    }

    #[test]
    fn test_extract_topic_short() {
        assert_eq!(extract_topic("Hello world.", 60), "Hello world.");
    }

    #[test]
    fn test_extract_topic_long() {
        let long = "This is a very long first sentence that exceeds our maximum length for topic extraction by quite a bit. Second sentence.";
        let topic = extract_topic(long, 60);
        assert!(topic.len() <= 63); // 60 + "..."
        assert!(topic.ends_with("..."));
    }

    #[test]
    fn test_make_context_header() {
        let header = make_context_header("My topic", 0, 3);
        assert_eq!(header, "[From: \"My topic\", part 1/3]\n");

        let header2 = make_context_header("Another topic", 2, 5);
        assert_eq!(header2, "[From: \"Another topic\", part 3/5]\n");
    }
}
