//! Shared memory management actions: `init`, `query`, `reindex`, `validate`.

pub mod init;
pub mod query;
pub mod reindex;
pub mod validate;

pub use init::{MemoryInitInput, MemoryInitOutcome, init};
pub use query::{MemoryQueryInput, MemoryQueryOutcome, query};
pub use reindex::{MemoryReindexInput, MemoryReindexOutcome, reindex};
pub use validate::{MemoryValidateInput, MemoryValidateOutcome, validate};
