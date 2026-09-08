//! Shared memory domain models and configuration.

pub mod config;
pub mod context;
pub mod episode;
pub mod handoff;
pub mod sanitizer;
pub mod query;
pub mod topic;
pub mod writeset;
pub use config::{MemoryConfig, MemoryScope, ScopeName};
pub use context::{
    MEMORY_MANAGED_END, MEMORY_MANAGED_START, MemoryBlock, MemoryContext, project_memory_symlink,
    render_memory_context,
};
pub use sanitizer::{Sanitized, sanitize_text};
pub use topic::{MemoryTier, MemoryTopic, TopicMetadata, TopicStatus};
pub use query::{QueryFilter, QueryMatch, ReconcileSummary};
pub use episode::EpisodePayload;
pub use handoff::HandoffPayload;
pub use writeset::MemoryWriteSet;
