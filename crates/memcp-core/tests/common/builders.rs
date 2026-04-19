use memcp::store::CreateMemory;

/// Builder for `CreateMemory` with realistic test defaults.
///
/// Provides fluent overrides for all fields so tests only specify what matters,
/// keeping construction concise and readable.
pub struct MemoryBuilder {
    content: String,
    type_hint: String,
    source: String,
    tags: Option<Vec<String>>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    actor: Option<String>,
    actor_type: String,
    audience: String,
    idempotency_key: Option<String>,
    project: Option<String>,
    trust_level: Option<f32>,
    write_path: Option<String>,
    knowledge_tier: Option<String>,
    source_ids: Option<Vec<String>>,
    // TODO(24.5-01): thread self.reply_to_id into CreateMemory once the field lands.
    #[allow(dead_code)]
    reply_to_id: Option<String>,
}

impl MemoryBuilder {
    /// Create a new builder with realistic defaults.
    pub fn new() -> Self {
        MemoryBuilder {
            content: "The user prefers dark mode in all editors".to_string(),
            type_hint: "fact".to_string(),
            source: "test-agent".to_string(),
            tags: None,
            created_at: None,
            actor: None,
            actor_type: "agent".to_string(),
            audience: "global".to_string(),
            idempotency_key: None,
            project: None,
            trust_level: None,
            write_path: None,
            knowledge_tier: None,
            source_ids: None,
            reply_to_id: None,
        }
    }

    /// Override the memory content.
    pub fn content(mut self, content: &str) -> Self {
        self.content = content.to_string();
        self
    }

    /// Override the type hint (e.g. "fact", "preference", "instruction").
    pub fn type_hint(mut self, type_hint: &str) -> Self {
        self.type_hint = type_hint.to_string();
        self
    }

    /// Override the source identifier.
    pub fn source(mut self, source: &str) -> Self {
        self.source = source.to_string();
        self
    }

    /// Set tags (converts `&str` references to owned `String` values).
    pub fn tags(mut self, tags: Vec<&str>) -> Self {
        self.tags = Some(tags.into_iter().map(|t| t.to_string()).collect());
        self
    }

    /// Set the created_at timestamp override.
    pub fn created_at(mut self, created_at: chrono::DateTime<chrono::Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Set the actor identifier.
    pub fn actor(mut self, actor: &str) -> Self {
        self.actor = Some(actor.to_string());
        self
    }

    /// Set the idempotency key for at-most-once store semantics.
    pub fn idempotency_key(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    /// Set the project scope.
    pub fn project(mut self, project: &str) -> Self {
        self.project = Some(project.to_string());
        self
    }

    /// Set the trust level (0.0 to 1.0).
    pub fn trust_level(mut self, trust_level: f32) -> Self {
        self.trust_level = Some(trust_level);
        self
    }

    /// Set the write path (e.g., "auto_store", "explicit_store", "import").
    pub fn write_path(mut self, wp: &str) -> Self {
        self.write_path = Some(wp.to_string());
        self
    }

    /// Set the knowledge tier (e.g., "raw", "explicit", "derived", "pattern").
    pub fn knowledge_tier(mut self, tier: &str) -> Self {
        self.knowledge_tier = Some(tier.to_string());
        self
    }

    /// Set the source IDs for provenance tracking.
    pub fn source_ids(mut self, ids: Vec<&str>) -> Self {
        self.source_ids = Some(ids.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Set the `reply_to_id` for conversation threading (Phase 24.5 / D-16).
    ///
    /// Note: until Plan 24.5-01 adds `reply_to_id` to `CreateMemory`, this value is
    /// stored on the builder but NOT threaded into the produced `CreateMemory`.
    /// See TODO(24.5-01) in `build()`.
    #[allow(dead_code)]
    pub fn reply_to_id(mut self, id: &str) -> Self {
        self.reply_to_id = Some(id.to_string());
        self
    }

    /// Consume the builder and produce a `CreateMemory`.
    pub fn build(self) -> CreateMemory {
        CreateMemory {
            content: self.content,
            type_hint: self.type_hint,
            source: self.source,
            tags: self.tags,
            created_at: self.created_at,
            actor: self.actor,
            actor_type: self.actor_type,
            audience: self.audience,
            idempotency_key: self.idempotency_key,
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: self.project,
            trust_level: self.trust_level,
            session_id: None,
            agent_role: None,
            write_path: self.write_path,
            knowledge_tier: self.knowledge_tier,
            source_ids: self.source_ids,
            // TODO(24.5-01): thread self.reply_to_id into CreateMemory once the field lands.
        }
    }

    /// Convenience: build with an explicit project override (deprecated in favor of `.project()`).
    pub fn build_with_project(mut self, project: Option<String>) -> CreateMemory {
        self.project = project;
        self.build()
    }
}

impl Default for MemoryBuilder {
    fn default() -> Self {
        Self::new()
    }
}
