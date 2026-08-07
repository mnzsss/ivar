//! The types and invariants, with no I/O anywhere.
//!
//! `domain` may import [`crate::error`] and nothing else from this crate. Not
//! `store`, not `git`, not `infra`. That is what makes the rules here testable
//! without a temp directory, and what stops them from scattering into the verbs
//! that use them.
//!
//! What belongs here: what a valid promotion is, how hall health is derived, which
//! branch a repo resolves to for a given feature, what a well-formed name looks
//! like. What does not: reading it, writing it, or shelling out about it.

pub mod feature;
pub mod health;
pub mod name;
pub mod provider;
