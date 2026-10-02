//! Persona end-to-end scenarios: the compiled `ivar` binary driven through a
//! throwaway hall with an isolated HOME/XDG, local remotes and no network.
//!
//! One scenario file per persona or lifecycle-hardening child under
//! `tests/personas/`; [`support`] holds the isolation helpers they share.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

#[path = "personas/support.rs"]
mod support;

#[path = "personas/single_repo_prototype.rs"]
mod single_repo_prototype;

#[path = "personas/multi_repo.rs"]
mod multi_repo;

#[path = "personas/release_smoke.rs"]
mod release_smoke;

#[path = "personas/data_safety.rs"]
mod data_safety;

#[path = "personas/guard.rs"]
mod guard;

#[path = "personas/promote.rs"]
mod promote;

#[path = "personas/integrate.rs"]
mod integrate;

#[path = "personas/deliver.rs"]
mod deliver;

#[path = "personas/contract.rs"]
mod contract;
