//! `ivar session prune` — remove dead sessions and their View Dirs.
//!
//! A **dead** session is a stale orphan: its View Dir exists but holds no
//! readable `state.json` — a view dir that predates session records, or whose
//! record was lost. A **live** session (View Dir present with a readable
//! record) is never touched.
//!
//! A dead View Dir under a feature with a pending write lock — the
//! `.converting` marker of an in-flight conversion — is **refused**, naming
//! the lock: pruning would race the conversion. The refusal happens before
//! anything is removed, so a `Blocked` run has mutated nothing.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::session::SessionRef;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;
use crate::store::layout::Layout;

use super::super::discover_hall;
use super::lookup;

/// What `ivar session prune` did.
#[derive(Debug, Clone, Serialize)]
pub struct PruneOutcome {
    /// How many dead sessions were removed.
    pub pruned: u32,
}

impl WriteHuman for PruneOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Pruned {} dead session(s).", self.pruned)
    }
}

/// Remove dead sessions, reconciling state.
///
/// Live sessions are never touched. A dead View Dir with a pending write lock
/// (a `.converting` marker under the owning feature) is refused with a
/// `Blocked` failure naming the lock — and the refusal comes before any
/// removal, so a refused run has mutated nothing.
pub fn prune(ctx: &Ctx) -> Outcome<PruneOutcome> {
    let layout = discover_hall(ctx)?;
    let sessions = enumerate(&layout)?;

    // Refuse before removing anything: a dead view dir with pending writes
    // could be mid-conversion, its state record not yet written.
    for session in &sessions {
        if is_dead(session)
            && let Some(lock) = pending_lock(&layout, session)
        {
            return Err(prune_refused(session, &lock));
        }
    }

    let mut pruned = 0u32;
    for session in sessions {
        if !is_dead(&session) {
            continue; // live: never touched
        }
        if remove_view_dir(&session.view_dir) {
            pruned += 1;
        }
    }

    Ok(Report::new(PruneOutcome { pruned }))
}

/// Every session in the hall — discovery and feature sessions alike.
fn enumerate(layout: &Layout) -> Result<Vec<SessionRef>, Failure> {
    let mut sessions = lookup::list_discovery(layout)?;
    if fs::is_dir(&layout.features_dir())? {
        for entry in fs::read_dir(&layout.features_dir())? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature_name) = crate::domain::name::FeatureName::new(name) else {
                continue;
            };
            sessions.extend(lookup::list_feature(layout, &feature_name)?);
        }
    }
    Ok(sessions)
}

/// Whether a session is dead (should be pruned).
///
/// A session is dead when its View Dir holds no readable `state.json` — a
/// stale orphan. A View Dir that is gone entirely is already stopped; `lookup`
/// never lists those, and removing a gone dir is a no-op anyway.
fn is_dead(session: &SessionRef) -> bool {
    match fs::exists(&session.view_dir) {
        Ok(true) => session.state.is_none(), // present, but no record → orphan
        Ok(false) => true,                   // gone → already stopped
        Err(_) => false,                     // can't check → assume live, don't risk it
    }
}

/// The pending write lock on a session's feature, if any.
///
/// The only current lock is the `.converting` transition marker written by
/// `conversion` during an in-flight session conversion. If this marker exists
/// for the session's feature, pruning must wait.
fn pending_lock(layout: &Layout, session: &SessionRef) -> Option<Utf8PathBuf> {
    let feature = session.feature.as_ref()?;
    let lock = layout.feature_dir(feature).join(".converting");
    fs::exists(&lock).unwrap_or(false).then_some(lock)
}

/// The `Blocked` failure naming the lock that stopped a prune.
fn prune_refused(session: &SessionRef, lock: &Utf8PathBuf) -> Failure {
    Failure::blocked(
        "session.prune_locked",
        format!(
            "session `{}` has pending writes — pruning refused while a `.converting` lock exists",
            session.id
        ),
    )
    .expected("no pending write lock under the session's feature")
    .actual(format!("`{lock}` exists — a conversion may be in flight"))
    .fix(FixAction::safe(
        "session.prune_after_conversion",
        "Wait for the in-flight conversion to finish (or fail), then run `ivar session prune` again.",
    ))
}

/// Remove the View Dir. Returns whether it existed and was removed.
fn remove_view_dir(view_dir: &Utf8PathBuf) -> bool {
    if !fs::exists(view_dir).unwrap_or(false) {
        return false;
    }
    std::fs::remove_dir_all(view_dir.as_std_path()).is_ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/prune.rs"]
mod tests;
