//! Memory topic model and metadata.

use serde::{Deserialize, Serialize};

use super::config::ScopeName;

/// Storage tier of a memory topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Core memory: always loaded / prioritized.
    Core,
    /// Extended memory: loaded on demand or indexed.
    Extended,
}

/// Lifecycle status of a memory topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "superseded_by")]
pub enum TopicStatus {
    /// Active canonical topic.
    Active,
    /// Deprecated topic.
    Deprecated,
    /// Superseded by another topic slug.
    Superseded(String),
}

/// Metadata header stored in frontmatter of a memory topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicMetadata {
    /// Title of the topic.
    pub title: String,
    /// Scope this topic belongs to.
    pub scope: ScopeName,
    /// Short summary / description of the topic.
    pub description: String,
    /// Core or Extended tier.
    pub tier: MemoryTier,
    /// Current status of the topic.
    #[serde(flatten)]
    pub status: TopicStatus,
    /// ISO 8601 timestamp of the last update.
    pub updated: String,
    /// Tags associated with the topic.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// In-memory representation of a canonical topic document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTopic {
    /// Topic frontmatter metadata.
    pub metadata: TopicMetadata,
    /// Markdown body content.
    pub content: String,
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/topic.rs"]
mod tests;
