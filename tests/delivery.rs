//! Integration tests for `ivar feature deliver`.
//!
//! `tests/delivery.rs` is the sole Cargo integration-test entrypoint for the
//! delivery target. Test behavior lives in feature-owned scope modules under
//! `tests/delivery/`; this file declares them and the shared infrastructure
//! they consume.
//!
//! Scopes:
//! - [`support`] — delivery-only fixtures and helpers
//! - [`preview`] — preview shape, empty-feature, fingerprint-drift, human rendering
//! - [`apply`] — gates, drift, push, warning, CLI end-to-end cases
//! - [`pull_requests`] — PR creation/update and sibling-link cases
//! - [`metadata`] — `--name`/`--body`, inheritance, file-body, validation, partial-edit, no-op, push-only, defaults, land-conflict

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
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

#[path = "delivery/metadata.rs"]
mod metadata;

#[path = "delivery/draft.rs"]
mod draft;
