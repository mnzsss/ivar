#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

mod apply;
mod attribution;
mod draft;
mod events;
pub(super) mod fixture;
mod land;
mod memory_review;
mod metadata;
mod ordering;
mod pr_metadata;
mod prefixed;
mod preview;
mod pull_request;
mod push;
