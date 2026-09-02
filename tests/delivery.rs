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
//! - `metadata_*` — scoped values, body files, validation, and existing-PR edits
//! - `draft_*` — creation/scope, conversion/failures, and CLI/fingerprint contracts

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

#[path = "delivery/metadata_scope.rs"]
mod metadata_scope;

#[path = "delivery/metadata_body.rs"]
mod metadata_body;

#[path = "delivery/metadata_validation.rs"]
mod metadata_validation;

#[path = "delivery/metadata_edit.rs"]
mod metadata_edit;

#[path = "delivery/draft_creation.rs"]
mod draft_creation;

#[path = "delivery/draft_conversion.rs"]
mod draft_conversion;

#[path = "delivery/draft_contract.rs"]
mod draft_contract;
