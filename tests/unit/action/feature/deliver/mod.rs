#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

mod apply;
pub(super) mod fixture;
mod land;
mod metadata;
mod ordering;
mod preview;
mod pull_request;
mod push;
