//! Shared memory domain models and configuration.

pub mod config;
pub mod conflict;
pub mod context;
pub mod episode;
pub mod handoff;
pub mod query;
pub mod sanitizer;
pub mod topic;
pub mod writeset;
pub use config::{MemoryConfig, MemoryScope, ScopeName};
pub use conflict::{ConflictResolution, list_pending_conflicts, preserve_topic_conflict};
pub use context::{
    MEMORY_MANAGED_END, MEMORY_MANAGED_START, MemoryBlock, MemoryContext, project_memory_symlink,
    render_memory_context,
};
pub use episode::EpisodePayload;
pub use handoff::HandoffPayload;
pub use query::{QueryFilter, QueryMatch, ReconcileSummary};
pub use sanitizer::{Sanitized, sanitize_text};
pub use topic::{MemoryTier, MemoryTopic, TopicMetadata, TopicStatus};
pub use writeset::MemoryWriteSet;
