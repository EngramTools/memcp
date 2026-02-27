//! Algorithmic curator — default, no LLM required.
//!
//! Uses salience + age to flag stale memories.
//! Merges by concatenation (no synthesis).
//! Strengthens high-access memories.

use async_trait::async_trait;
use chrono::Utc;

use super::{ClusterMember, CurationAction, CurationError, CurationProvider};
use crate::config::CurationConfig;

/// Algorithmic curation provider — works without any LLM.
///
/// Stale detection: low salience + old age + no reinforcement.
/// Merge: concatenation of source content (no synthesis).
/// Strengthen: frequently reinforced memories with room to grow.
pub struct AlgorithmicCurator {
    config: CurationConfig,
}

impl AlgorithmicCurator {
    pub fn new(config: CurationConfig) -> Self {
        Self { config }
    }

    /// Check if a memory should be flagged as stale (low salience + old + unreinforced).
    fn is_stale(&self, member: &ClusterMember) -> bool {
        let age_days = (Utc::now() - member.created_at).num_days();
        let low_salience = member.stability < self.config.stale_salience_threshold;
        let old_enough = age_days > self.config.stale_age_days as i64;
        let unreinforced = member.reinforcement_count == 0
            || member.last_reinforced_at.map_or(true, |t| {
                (Utc::now() - t).num_days() > self.config.stale_age_days as i64
            });
        low_salience && old_enough && unreinforced
    }

    /// Check if a memory should be strengthened (frequently accessed/reinforced).
    fn should_strengthen(&self, member: &ClusterMember) -> bool {
        member.reinforcement_count >= 3 && member.stability < 5.0
    }
}

#[async_trait]
impl CurationProvider for AlgorithmicCurator {
    async fn review_cluster(
        &self,
        cluster: &[ClusterMember],
    ) -> Result<Vec<CurationAction>, CurationError> {
        let mut actions = Vec::new();

        // For multi-member clusters: consider merge if all are similar and low-salience
        if cluster.len() >= 2 && cluster.len() <= self.config.max_merge_group_size {
            // In algorithmic mode, only merge if ALL members are low-salience
            // (high-salience memories are valuable individually)
            let all_low = cluster.iter().all(|m| m.stability < 1.0);
            if all_low {
                let content = self.synthesize_merge(cluster).await?;
                actions.push(CurationAction::Merge {
                    source_ids: cluster.iter().map(|m| m.id.clone()).collect(),
                    synthesized_content: content,
                });
                return Ok(actions);
            }
        }

        // Individual review for each member
        for member in cluster {
            if self.is_stale(member) {
                actions.push(CurationAction::FlagStale {
                    memory_id: member.id.clone(),
                    reason: format!(
                        "Low salience ({:.2}) + {} days old + {} reinforcements",
                        member.stability,
                        (Utc::now() - member.created_at).num_days(),
                        member.reinforcement_count,
                    ),
                });
            } else if self.should_strengthen(member) {
                actions.push(CurationAction::Strengthen {
                    memory_id: member.id.clone(),
                    reason: format!(
                        "{} reinforcements, stability {:.2} — deserves boost",
                        member.reinforcement_count, member.stability,
                    ),
                });
            } else {
                actions.push(CurationAction::Skip {
                    memory_id: member.id.clone(),
                    reason: "No curation action needed".to_string(),
                });
            }
        }

        Ok(actions)
    }

    async fn synthesize_merge(
        &self,
        sources: &[ClusterMember],
    ) -> Result<String, CurationError> {
        // Algorithmic merge: concatenate with separator, newest first
        let mut sorted = sources.to_vec();
        sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let merged = sorted
            .iter()
            .enumerate()
            .map(|(i, m)| {
                if sorted.len() > 1 {
                    format!("[{}/{}] {}", i + 1, sorted.len(), m.content)
                } else {
                    m.content.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(merged)
    }

    fn model_name(&self) -> &str {
        "algorithmic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn make_member(id: &str, stability: f64, age_days: i64, reinforcements: i32) -> ClusterMember {
        ClusterMember {
            id: id.to_string(),
            content: format!("Memory content for {}", id),
            type_hint: Some("fact".to_string()),
            tags: vec!["test".to_string()],
            created_at: Utc::now() - Duration::days(age_days),
            updated_at: Utc::now() - Duration::days(age_days),
            stability,
            reinforcement_count: reinforcements,
            last_reinforced_at: if reinforcements > 0 {
                Some(Utc::now() - Duration::days(age_days / 2))
            } else {
                None
            },
        }
    }

    #[tokio::test]
    async fn test_stale_detection() {
        let config = CurationConfig::default();
        let curator = AlgorithmicCurator::new(config);

        // Low salience + old + unreinforced → stale
        let stale = make_member("stale-1", 0.1, 60, 0);
        assert!(curator.is_stale(&stale));

        // High salience → not stale
        let healthy = make_member("healthy-1", 2.0, 60, 0);
        assert!(!curator.is_stale(&healthy));

        // Young → not stale
        let young = make_member("young-1", 0.1, 5, 0);
        assert!(!curator.is_stale(&young));
    }

    #[tokio::test]
    async fn test_strengthen_detection() {
        let config = CurationConfig::default();
        let curator = AlgorithmicCurator::new(config);

        // Frequently reinforced + moderate stability → strengthen
        assert!(curator.should_strengthen(&make_member("str-1", 2.0, 30, 5)));

        // Low reinforcements → no strengthen
        assert!(!curator.should_strengthen(&make_member("str-2", 2.0, 30, 1)));

        // Already high stability → no strengthen
        assert!(!curator.should_strengthen(&make_member("str-3", 10.0, 30, 5)));
    }

    #[tokio::test]
    async fn test_review_cluster_stale() {
        let config = CurationConfig::default();
        let curator = AlgorithmicCurator::new(config);

        let cluster = vec![make_member("s1", 0.1, 60, 0)];
        let actions = curator.review_cluster(&cluster).await.unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], CurationAction::FlagStale { .. }));
    }

    #[tokio::test]
    async fn test_review_cluster_merge() {
        let config = CurationConfig::default();
        let curator = AlgorithmicCurator::new(config);

        // Two low-salience members → merge
        let cluster = vec![
            make_member("m1", 0.5, 10, 0),
            make_member("m2", 0.3, 15, 0),
        ];
        let actions = curator.review_cluster(&cluster).await.unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], CurationAction::Merge { .. }));
    }

    #[tokio::test]
    async fn test_review_cluster_no_merge_high_salience() {
        let config = CurationConfig::default();
        let curator = AlgorithmicCurator::new(config);

        // One high-salience member prevents merge
        let cluster = vec![
            make_member("m1", 0.5, 10, 0),
            make_member("m2", 2.0, 15, 3),
        ];
        let actions = curator.review_cluster(&cluster).await.unwrap();
        // Should get individual actions, not a merge
        assert!(actions.len() >= 2);
        assert!(!actions.iter().any(|a| matches!(a, CurationAction::Merge { .. })));
    }

    #[tokio::test]
    async fn test_synthesize_merge() {
        let config = CurationConfig::default();
        let curator = AlgorithmicCurator::new(config);

        let sources = vec![
            make_member("m1", 0.5, 10, 0),
            make_member("m2", 0.3, 15, 0),
        ];
        let merged = curator.synthesize_merge(&sources).await.unwrap();
        assert!(merged.contains("[1/2]"));
        assert!(merged.contains("[2/2]"));
    }
}
