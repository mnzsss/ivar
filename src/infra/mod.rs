//! Adapters to the outside world.
//!
//! Everything here is a boundary: the filesystem, subprocesses, terminals, the
//! network. `infra` may import [`crate::error`] and nothing else from this crate
//! — it is the bottom of the layering.
//!
//! The rule that makes this worth having: **no other module touches `std::fs`,
//! `std::process`, or `serde_json`'s writers directly.** When a primitive is
//! missing, it gets added here rather than reached around.

pub mod figma;
pub mod frontmatter;
pub mod fs;
pub mod github;
pub mod hash;
pub mod http_callback;
pub mod json;
pub mod oauth;
pub mod proc;
pub mod progress;
pub mod term;
