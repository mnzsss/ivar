//! Integration tests for `ivar graph`.
//!
//! `tests/graph.rs` is the sole Cargo integration-test entrypoint for the graph
//! target. Test behavior lives in scope modules under `tests/graph/`; this file
//! declares them and the shared infrastructure they consume.
//!
//! Scopes:
//! - [`default_branch`] — base indexing and doctor against a repo's declared default branch
//! - [`feature_layer`] — feature sessions served from their checkout's layer
//! - [`session_readers`] — viewer, affected, hierarchy, viz and explore through session views
//! - [`simulation`] — ignored end-to-end lifecycle over a clone of this repository
//! - [`viewer`] — `ivar graph view` server endpoints and security
//! - [`mcp_registration`] — `ivar graph index` declaring the graph MCP server in `ivar.json`

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

mod support;

use support::common;

#[path = "graph/default_branch.rs"]
mod default_branch;

#[path = "graph/feature_layer.rs"]
mod feature_layer;

#[path = "graph/session_readers.rs"]
mod session_readers;

#[path = "graph/simulation.rs"]
mod simulation;

#[path = "graph/viewer.rs"]
mod viewer;

#[path = "graph/mcp_registration.rs"]
mod mcp_registration;
