//! Shared memory domain models and configuration.

pub mod config;
pub mod sanitizer;
pub mod topic;

pub use config::{MemoryConfig, MemoryScope, ScopeName};
pub use sanitizer::{Sanitized, sanitize_text};
pub use topic::{MemoryTier, MemoryTopic, TopicMetadata, TopicStatus};
