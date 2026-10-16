//! `ivar session stop [session] [--all]` — end a session by removing its View Dir.
//!
//! Liveness is a filesystem fact: a session is live while its View Dir exists.
//! Removing the View Dir marks it stopped. Stopping every session takes an
//! explicit `--all`; a session id that matches nothing is an error.

use std::io;

use serde::Serialize;

use crate::action::Ctx;
use crate::action::discovery;
use crate::domain::provider::Provider;
use crate::domain::session::SessionRef;
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::infra::fs;
use crate::providers;
use crate::store::layout::Layout;

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
    let (stopped, warnings) = end(&layout, &session, hall_lists_t3(&layout))?;
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
    let (stopped, warnings) = end_each(layout, lookup::list_all(layout)?);
    Ok(Report::with_warnings(StopOutcome { stopped }, warnings))
}

/// End every session in `sessions`, best-effort: a session whose discovery doc
/// cannot be kept stays live and becomes a warning, never an abort.
pub(crate) fn end_each(
    layout: &Layout,
    sessions: impl IntoIterator<Item = SessionRef>,
) -> (u32, Vec<Warning>) {
    let lists_t3 = hall_lists_t3(layout);
    let mut ended = 0u32;
    let mut warnings = Vec::new();
    for session in sessions {
        match end(layout, &session, lists_t3) {
            Ok((true, found)) => {
                ended += 1;
                warnings.extend(found);
            }
            Ok((false, found)) => warnings.extend(found),
            Err(failure) => warnings.push(Warning::new(
                "session.stop_skipped",
                session.id.as_str(),
                failure.what,
            )),
        }
    }
    (ended, warnings)
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
    let view_dir = &session.view_dir;
    if !fs::exists(view_dir).unwrap_or(false) {
        return Ok((false, Vec::new()));
    }
    let mut warnings = Vec::new();
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

#[cfg(test)]
#[path = "../../../tests/unit/action/session/stop.rs"]
mod tests;
