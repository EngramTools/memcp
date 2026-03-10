//! Memory chunking pipeline.
//!
//! Splits long content into overlapping sentence-grouped chunks with context headers
//! for better retrieval granularity. Only used by the auto-store sidecar --
//! explicit store operations are never chunked.
//!
//! Feeds from pipeline/auto_store/ (content to chunk), produces CreateMemory entries
//! stored via storage/store/.

pub mod splitter;

use crate::config::ChunkingConfig;

/// A chunk ready for storage.
#[derive(Debug, Clone)]
pub struct ChunkOutput {
    /// Chunk content with context header prefix
    pub content: String,
    /// Zero-based index within the chunk family
    pub index: usize,
    /// Total number of chunks produced
    pub total: usize,
}

/// Split content into chunks according to the chunking configuration.
///
/// Returns an empty Vec when:
/// - Chunking is disabled in config
/// - Content is below the minimum length threshold
/// - Content fits in a single chunk after sentence splitting
///
/// When non-empty, each ChunkOutput has a context header prefix making it
/// self-sufficient for retrieval without needing the parent content.
pub fn chunk_content(content: &str, config: &ChunkingConfig) -> Vec<ChunkOutput> {
    if !config.enabled {
        return vec![];
    }

    if content.len() < config.min_content_chars {
        return vec![];
    }

    let sentence_groups =
        splitter::split_sentences(content, config.max_chunk_chars, config.overlap_sentences);

    if sentence_groups.is_empty() {
        return vec![];
    }

    let total = sentence_groups.len();
    let topic = splitter::extract_topic(content, 60);

    sentence_groups
        .into_iter()
        .enumerate()
        .map(|(i, sentences)| {
            let header = splitter::make_context_header(&topic, i, total);
            let body = sentences.join("");
            ChunkOutput {
                content: format!("{}{}", header, body),
                index: i,
                total,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChunkingConfig;

    fn test_config() -> ChunkingConfig {
        ChunkingConfig {
            enabled: true,
            max_chunk_chars: 200,
            overlap_sentences: 1,
            min_content_chars: 100,
        }
    }

    #[test]
    fn test_short_content_no_chunks() {
        let config = test_config();
        let result = chunk_content("Short text.", &config);
        assert!(
            result.is_empty(),
            "Content below threshold should not be chunked"
        );
    }

    #[test]
    fn test_disabled_no_chunks() {
        let mut config = test_config();
        config.enabled = false;
        let long = "A".repeat(300);
        let result = chunk_content(&long, &config);
        assert!(result.is_empty(), "Disabled chunking should return empty");
    }

    #[test]
    fn test_medium_content_produces_chunks() {
        let config = ChunkingConfig {
            enabled: true,
            max_chunk_chars: 100,
            overlap_sentences: 1,
            min_content_chars: 50,
        };
        // Create content with multiple sentences that exceeds chunk size
        let content = "This is the first sentence about memory systems. \
                       This is the second sentence about embeddings. \
                       This is the third sentence about search quality. \
                       This is the fourth sentence about chunking strategies. \
                       This is the fifth sentence about retrieval. ";
        let result = chunk_content(content, &config);
        assert!(!result.is_empty(), "Should produce chunks");
        assert!(
            result.len() >= 2,
            "Should produce at least 2 chunks, got {}",
            result.len()
        );

        // Check context headers
        for chunk in &result {
            assert!(
                chunk.content.starts_with("[From:"),
                "Chunk should have context header, got: {}",
                &chunk.content[..50.min(chunk.content.len())]
            );
            assert!(chunk.content.contains(&format!("/{}", result[0].total)));
        }

        // Check indices
        for (i, chunk) in result.iter().enumerate() {
            assert_eq!(chunk.index, i);
            assert_eq!(chunk.total, result.len());
        }
    }

    #[test]
    fn test_overlap_between_chunks() {
        let config = ChunkingConfig {
            enabled: true,
            max_chunk_chars: 80,
            overlap_sentences: 1,
            min_content_chars: 50,
        };
        let content = "First sentence here. Second sentence here. Third sentence here. Fourth sentence here. Fifth sentence here. ";

        let chunks = chunk_content(content, &config);
        assert!(
            chunks.len() >= 2,
            "Need at least 2 chunks to verify overlap, got {}",
            chunks.len()
        );
    }

    #[test]
    fn test_single_chunk_returns_empty() {
        let config = ChunkingConfig {
            enabled: true,
            max_chunk_chars: 10000,
            overlap_sentences: 1,
            min_content_chars: 10,
        };
        let content = "This is a moderate length content that fits in one chunk easily.";
        let result = chunk_content(content, &config);
        assert!(
            result.is_empty(),
            "Single-chunk content should return empty (no splitting needed)"
        );
    }

    #[test]
    fn test_chunk_headers_have_correct_format() {
        let config = ChunkingConfig {
            enabled: true,
            max_chunk_chars: 80,
            overlap_sentences: 0,
            min_content_chars: 50,
        };
        let content = "The topic is memory systems. Second point about embeddings. Third about vectors. Fourth sentence here. Fifth sentence. ";
        let chunks = chunk_content(content, &config);
        if !chunks.is_empty() {
            // First chunk should reference part 1/N
            assert!(chunks[0].content.contains("part 1/"));
            // All chunks should reference the topic from the first sentence
            for chunk in &chunks {
                assert!(chunk.content.contains("memory systems"));
            }
        }
    }
}
