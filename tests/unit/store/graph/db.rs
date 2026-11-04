#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{
    MissEvent, MissKind, Span, Symbol, SymbolKind, UsageEvent, UsageSource,
};
use crate::store::graph::db::FORGET_BATCH;
use crate::store::graph::db::usage::MissFilter;
use tempfile::tempdir;

fn event(
    command: &str,
    source: UsageSource,
    ms: u64,
    count: Option<usize>,
    error: bool,
    session: Option<&str>,
    query: Option<&str>,
) -> UsageEvent {
    UsageEvent {
        command: command.to_owned(),
        source,
        duration_ms: ms,
        result_count: count,
        error,
        session: session.map(str::to_owned),
        query: query.map(str::to_owned),
    }
}

#[path = "db/content.rs"]
mod content;
#[path = "db/crud.rs"]
mod crud;
#[path = "db/extraction.rs"]
mod extraction;
#[path = "db/recording.rs"]
mod recording;
