//! On-disk persistence. Owns the file layout, and nothing else.
//!
//! `store` may import [`crate::domain`], [`crate::infra`] and [`crate::error`].
//! It may not import `git`, `harness`, `tui`, or `action`.
//!
//! Every write goes through `infra::json::write_canonical` and every path comes
//! from [`layout`]. No module outside `store` computes a path under a hall.

pub mod feature;
pub mod gitignore;
pub mod layout;
pub mod manifest;
pub mod setup_receipt;
pub mod versioned;
