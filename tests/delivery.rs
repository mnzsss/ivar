//! Integration tests for `ivar feature deliver`.
//!
//! `tests/delivery.rs` is the sole Cargo integration-test entrypoint for the
//! delivery target. Test behavior lives in feature-owned scope modules under
//! `tests/delivery/`; this file declares them and the shared infrastructure
//! they consume.
//!
//! The CLI end-to-end cases only: everything clap, exit codes and the
//! `--json` envelope add on top of `deliver`. The behaviour matrix runs
//! in-process under `tests/unit/action/feature/deliver/`.
//!
//! Scopes:
//! - [`support`] — delivery-only fixtures and helpers
//! - [`preview`] — the `--json` preview envelope
//! - [`apply`] — CLI-only path, `--only`, partial push failure
//! - [`pull_requests`] — PR creation and update
//! - [`metadata_validation`] — `--land` with metadata, refused
//! - [`draft_creation`] — `--draft` on a new PR

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value,
    clippy::redundant_clone,
    clippy::assigning_clones,
    clippy::implicit_clone,
    clippy::format_push_string,
    clippy::unnecessary_wraps,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

#[path = "support/integration.rs"]
mod common;

#[path = "delivery/support.rs"]
mod support;

#[path = "delivery/preview.rs"]
mod preview;

#[path = "delivery/apply.rs"]
mod apply;

#[path = "delivery/pull_requests.rs"]
mod pull_requests;

#[path = "delivery/metadata_validation.rs"]
mod metadata_validation;

#[path = "delivery/draft_creation.rs"]
mod draft_creation;
