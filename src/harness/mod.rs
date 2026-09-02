//! Provider adapters: the code that knows what each harness wants on disk.
//!
//! `domain::provider` holds what a harness *is* — its id, its dotdir, its
//! instruction file. This layer holds what `ivar` *does* about it, which needs
//! I/O and therefore cannot live in `domain`.
//!
//! # What is here now
//!
//! [`config`] — the managed block in each harness's instruction file. That is
//! slice 2's harness work: `ivar sync` materialises it, and removes it for a
//! provider the hall no longer lists.
//!
//! [`commands`] — the embedded catalog of shipped workflow commands, and the
//! reconciliation that materialises them into each provider's command
//! directory. The catalog and every Markdown source are compiled into the
//! binary; this module owns only the command files `ivar-*.md` and leaves every
//! other file in the command directory to the user.
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error`. Not `store` — so paths
//! arrive here already computed by [`crate::store::layout`], which stays the
//! one place that knows the on-disk tree.

pub mod commands;
pub mod config;

