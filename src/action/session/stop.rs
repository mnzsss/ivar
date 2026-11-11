//! `ivar session stop [session] [--all]` — end a session by removing its View Dir.
//!
//! Liveness is a filesystem fact: a session is live while its View Dir exists.
//! Removing the View Dir marks it stopped. Stopping every session takes an
//! explicit `--all`; a session id that matches nothing is an error.
//!
//! The end of a discovery session — one bound to no feature — also syncs the
//! hall: pages it wrote under `docs/memory/` have no feature cleanup to commit
//! them, so this is where they are recorded and pushed.

use std::io;

use serde::Serialize;

use camino::Utf8PathBuf;

use crate::action::Ctx;
use crate::action::discovery;
use crate::action::hall::{OperationCommit, SyncRequest, Trigger, sync_hall};
use crate::action::memory::consolidate::consolidate_session;
use crate::action::memory::drain::epoch_secs_now;
use crate::domain::provider::Provider;
use crate::domain::session::SessionRef;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::providers;
use crate::store::layout::Layout;
use crate::store::memory::coordination::owner_has_handoff;
use crate::store::memory::db::open_coordination_read_only;
use crate::store::memory::observations::session_has_activity;

use super::super::discover_hall;
use super::lookup;

/// What `ivar session stop` needs.
#[derive(Debug, Clone)]
pub struct StopInput {
    /// The session id, or a unique prefix of one. The CLI fills it from
    /// `$IVAR_SESSION_ID` when no id is given.
    pub session: Option<String>,
    /// Stop every live session in the hall; `session` is ignored.
    pub all: bool,
}

/// The message of the commit that records the pages a discovery session wrote.
const DISCOVERY_MESSAGE: &str = "docs(memory): Record discovery session pages";

/// What `ivar session stop` did.
#[derive(Debug, Clone, Serialize)]
pub struct StopOutcome {
    /// How many sessions were stopped.
    pub stopped: u32,
}

impl WriteHuman for StopOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Stopped {} session(s).", self.stopped)
    }
}

/// End one session, or every session with `all`: remove the View Dir(s).
///
/// # Errors
///
/// Blocked when neither a session nor `all` is given, and when the session
/// matches no live session or more than one.
pub fn stop(ctx: &Ctx, input: &StopInput) -> Outcome<StopOutcome> {
    let layout = discover_hall(ctx)?;

    if input.all {
        return stop_all(&layout);
    }
    let Some(id_prefix) = &input.session else {
        return Err(
            Failure::blocked("session.stop_target_missing", "no session to stop")
                .expected("a session id, `$IVAR_SESSION_ID`, or `--all`")
                .actual("none given")
                .fix(FixAction::safe(
                    "session.stop_name_target",
                    "Pass the session id, or `--all` to stop every session in the hall.",
                )),
        );
    };
    let session = lookup::resolve(&layout, Some(id_prefix), None)?;
    let (stopped, mut warnings) = end(&layout, &session, hall_lists_t3(&layout))?;
    if stopped && session.feature.is_none() {
        warnings.extend(sync_discovery(&layout, &git::System));
    }
    Ok(Report::with_warnings(
        StopOutcome {
            stopped: u32::from(stopped),
        },
        warnings,
    ))
}

fn hall_lists_t3(layout: &Layout) -> bool {
    crate::action::read_manifest(layout)
        .is_ok_and(|manifest| manifest.providers().available().contains(&Provider::T3))
}

fn stop_all(layout: &Layout) -> Outcome<StopOutcome> {
    let ended = end_all(layout, lookup::list_all(layout)?);
    let mut warnings = ended.warnings;
    if ended.discovery {
        warnings.extend(sync_discovery(layout, &git::System));
    }
    Ok(Report::with_warnings(
        StopOutcome {
            stopped: ended.stopped,
        },
        warnings,
    ))
}

/// What ending a batch of sessions did.
struct Ended {
    /// How many sessions were ended.
    stopped: u32,
    /// Whether one of them was a discovery session.
    discovery: bool,
    warnings: Vec<Warning>,
}

/// End every session in `sessions`, best-effort: a session whose discovery doc
/// cannot be kept stays live and becomes a warning, never an abort.
pub(crate) fn end_each(
    layout: &Layout,
    sessions: impl IntoIterator<Item = SessionRef>,
) -> (u32, Vec<Warning>) {
    let ended = end_all(layout, sessions);
    (ended.stopped, ended.warnings)
}

fn end_all(layout: &Layout, sessions: impl IntoIterator<Item = SessionRef>) -> Ended {
    let lists_t3 = hall_lists_t3(layout);
    let mut ended = Ended {
        stopped: 0,
        discovery: false,
        warnings: Vec::new(),
    };
    for session in sessions {
        match end(layout, &session, lists_t3) {
            Ok((stopped, found)) => {
                if stopped {
                    ended.stopped += 1;
                    ended.discovery |= session.feature.is_none();
                }
                ended.warnings.extend(found);
            }
            Err(failure) => ended.warnings.push(Warning::new(
                "session.stop_skipped",
                session.id.as_str(),
                failure.what,
            )),
        }
    }
    ended
}

/// Record and push what a discovery session left in the hall: the pages under
/// `docs/memory/` in their own commit, then everything else pending, then the
/// push. The warnings are the sync's own; a sync that cannot finish never
/// fails the stop. `auto_commit: false` in the manifest turns it off, and a
/// hall that is not a git repository has nothing to sync, which is not worth a
/// warning on every stop.
fn sync_discovery(layout: &Layout, git: &impl Git) -> Vec<Warning> {
    if !git
        .repo_state(layout.root())
        .is_ok_and(|state| state.is_repository)
    {
        return Vec::new();
    }
    let paths: Vec<Utf8PathBuf> = git
        .changed_paths(layout.root())
        .unwrap_or_default()
        .into_iter()
        .filter(|path| path.starts_with("docs/memory"))
        .collect();
    sync_hall(
        layout,
        git,
        &SyncRequest {
            trigger: Trigger::Automatic,
            operation: Some(OperationCommit {
                paths: &paths,
                message: DISCOVERY_MESSAGE,
            }),
        },
    )
    .warnings
}

/// End `session`: keep its discovery doc, close its T3 project, then remove its
/// View Dir. Returns whether the View Dir existed and was removed, plus warnings.
///
/// A pruned session has no readable record, so its provider is unknown: it is
/// closed by path when the hall lists `t3`.
///
/// # Errors
///
/// When the discovery doc cannot be kept; the View Dir is then left in place.
pub(crate) fn end(
    layout: &Layout,
    session: &SessionRef,
    hall_lists_t3: bool,
) -> Result<(bool, Vec<Warning>), Failure> {
    discovery::rescue_session_doc(layout, session)?;
    let mut warnings = Vec::new();
    match consolidate_session(layout, session.id.as_str(), epoch_secs_now()) {
        Ok(report) => {
            if let Some(error) = report.drain_error {
                warnings.push(Warning::new(
                    "memory.drain_failed",
                    session.id.as_str(),
                    error,
                ));
            }
        }
        Err(failure) => warnings.push(Warning::new(
            "memory.consolidate_failed",
            session.id.as_str(),
            failure.what,
        )),
    }
    if let Some(warning) = handoff_missing(layout, session.id.as_str()) {
        warnings.push(warning);
    }
    let view_dir = &session.view_dir;
    if !fs::exists(view_dir).unwrap_or(false) {
        return Ok((false, warnings));
    }
    let hosted = match session.state.as_ref() {
        Some(state) => state.provider() == Provider::T3,
        None => hall_lists_t3,
    };
    if hosted && let Err(reason) = providers::t3::host::close(view_dir) {
        warnings.push(Warning::new(
            "session.t3_project_left",
            session.id.as_str(),
            format!(
                "the T3 project for {view_dir} remains: {reason}; remove it in T3 Code, or run `t3 project remove {view_dir}` (add `--force` to delete its threads too)"
            ),
        ));
    }
    // The View Dir may hold symlinked repos and config dirs; removing it
    // recursively is the right cleanup.
    let removed = std::fs::remove_dir_all(view_dir.as_std_path()).is_ok();
    Ok((removed, warnings))
}

// A missing or pre-queue memory db means no handoff can exist, and an
// unreadable one is already reported as `memory.consolidate_failed`.
fn handoff_missing(layout: &Layout, session: &str) -> Option<Warning> {
    let conn = open_coordination_read_only(layout.memory_db().as_std_path())
        .ok()
        .flatten()?;
    let active = session_has_activity(&conn, session).ok()?;
    let handed_off = owner_has_handoff(&conn, session).ok()?;
    (active && !handed_off).then(|| {
        Warning::new(
            "session.handoff_missing",
            session,
            format!(
                "the session worked and opened no handoff; call memory_handoff_begin, or run `ivar memory handoff begin --session {session}` before stopping"
            ),
        )
    })
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/stop.rs"]
mod tests;
