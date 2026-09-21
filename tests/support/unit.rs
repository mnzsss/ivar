//! The unit-test adapter: linked from `src/lib.rs` as `crate::test_support`,
//! so library unit tests keep `use crate::test_support::…` unchanged.
//!
//! `#[cfg(test)]` items are not part of the compiled library, which is why
//! integration tests under `tests/` cannot see this module — they link
//! [`integration`](integration) instead. Both adapters pull their
//! implementation from [`shared`](shared), so the two boundaries share one
//! helper set rather than two drifting copies.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use camino::Utf8PathBuf;
use tempfile::TempDir;

use crate::action::Ctx;
use crate::action::hall::{self, InitInput};

#[path = "shared.rs"]
mod shared;

pub(crate) use shared::{
    canonical_temp_dir, empty_repo, git, hall_root, seeded_repo, utf8_temp_dir,
};

/// A canonicalised hall root with a freshly initialised hall named `acme`.
///
/// Shared by every action-layer unit-test module that needs a real hall to
/// operate against rather than a bare directory: initialising once here means
/// the 14 call sites that used to carry their own byte-identical copy stay in
/// sync automatically when `hall::init`'s input shape changes.
pub(crate) fn seeded_hall() -> (TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    (guard, root)
}

/// Creates the hall's graph database and a live discovery session, returning
/// the session's view dir, so app-layer tests can drive graph commands
/// without importing `store`.
pub(crate) fn graph_session_view(
    root: &Utf8PathBuf,
    session_id: &crate::domain::name::SessionId,
) -> Utf8PathBuf {
    let layout = crate::store::layout::Layout::at(root.clone());
    crate::store::graph::db::GraphDb::open(layout.ivar_dir().join("memory.db").as_std_path())
        .unwrap();
    let view_dir = layout.discovery_session(session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::domain::session::SessionState::new(
        crate::domain::provider::Provider::ClaudeCode,
        "2026-08-29T00:00:00Z",
    )
    .write(&view_dir)
    .unwrap();
    view_dir
}

/// The query text of the latest graph call recorded for `session`.
pub(crate) fn last_graph_query(root: &Utf8PathBuf, session: &str) -> Option<Option<String>> {
    let db_path = crate::store::layout::Layout::at(root.clone())
        .ivar_dir()
        .join("memory.db");
    crate::store::graph::db::GraphDb::open(db_path.as_std_path())
        .unwrap()
        .last_graph_call(session)
        .unwrap()
        .map(|(_ts, query)| query)
}
