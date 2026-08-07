//! Provider adapters: the code that knows what each harness wants on disk.
//!
//! `domain::provider` holds what a harness *is* — its id, its dotdir, its
//! instruction file. This layer holds what `ivar` *does* about it, which needs
//! I/O and therefore cannot live in `domain`.
//!
//! # What is here now
//!
//! [`config`] — the managed block in each harness's instruction file. That is
//! all of slice 2's harness work: `ivar sync` materialises it, and removes it
//! for a provider the hall no longer lists.
//!
//! # What is not here yet, and why not
//!
//! ARCHITECTURE.md, seam 5, describes a `Harness` trait plus closed-enum
//! dispatch carrying explicit capability flags (`supports_resume`,
//! `supports_review`, …) and per-harness command construction and log
//! normalisation. All of that is about *spawning* a harness, which is slice 5
//! (`ivar session start`).
//!
//! Writing the trait now would mean writing it with one method that has nothing
//! to do with its purpose, and every capability flag would be a guess made
//! before the code that reads it exists. The set of harnesses is closed and
//! known ([`crate::domain::provider::Provider`]), so adding the trait later is
//! a compiler-checked change, not an archaeology exercise. It lands with the
//! slice that needs it.
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error`. Not `store` — so paths
//! arrive here already computed by [`crate::store::layout`], which stays the
//! one place that knows where anything under a hall lives.

pub mod config;
