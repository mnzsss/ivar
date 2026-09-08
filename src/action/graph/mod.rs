pub mod cross_repo;
pub mod index;

pub use cross_repo::{link_cross_repo_edges, CrossRepoLinkOutcome};
pub use index::{index_repo, IndexOutcome};
