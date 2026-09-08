//! Shared memory store operations: canonical topic documents and episodes.

pub mod document;
pub mod episode;
pub mod handoff;
pub mod index;
pub use index::MemoryIndex;
pub use episode::persist_episode;
pub use handoff::{claim_pending_handoffs, persist_handoff};
