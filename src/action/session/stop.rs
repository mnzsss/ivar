//! `ivar session stop [session]` — end a session by removing its View Dir.
//!
//! Liveness is a filesystem fact: a session is live while its View Dir exists.
//! Removing the View Dir marks it stopped. Calling `stop` on an already-stopped
//! session (View Dir already gone) is a no-op.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::error::{Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;
use super::lookup;

/// What `ivar session stop` needs.
#[derive(Debug, Clone)]
pub struct StopInput {
    /// The session id, or a unique prefix of one. `None` stops all live
    /// sessions.
    pub session: Option<String>,
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

/// End a session (or all sessions): remove the View Dir(s).
///
/// If no session is named, stops every live session in the hall. An already-
/// stopped session (View Dir gone) is a no-op — never a failure.
pub fn stop(ctx: &Ctx, input: StopInput) -> Outcome<StopOutcome> {
    let layout = discover_hall(ctx)?;

    match &input.session {
        Some(id_prefix) => {
            // Single-session stop: locate it, remove its View Dir. An
            // already-stopped session (View Dir gone) is a no-op — the
            // lookup cannot see it, and that is not a failure.
            let session = match lookup::resolve(&layout, Some(id_prefix), None) {
                Ok(session) => session,
                Err(failure) if failure.code == "session.not_found" => {
                    return Ok(Report::new(StopOutcome { stopped: 0 }));
                }
                Err(failure) => return Err(failure),
            };
            let stopped = remove_view_dir(&session.view_dir);
            Ok(Report::new(StopOutcome {
                stopped: if stopped { 1 } else { 0 },
            }))
        }
        None => {
            // All-sessions stop: enumerate every session, remove each View Dir.
            let mut count = 0u32;

            // Discovery sessions.
            for session in lookup::list_discovery(&layout)? {
                if remove_view_dir(&session.view_dir) {
                    count += 1;
                }
            }

            // Feature sessions.
            if fs::is_dir(&layout.features_dir())? {
                for entry in fs::read_dir(&layout.features_dir())? {
                    let Some(name) = entry.file_name() else {
                        continue;
                    };
                    let Ok(feature_name) = crate::domain::name::FeatureName::new(name) else {
                        continue;
                    };
                    for session in lookup::list_feature(&layout, &feature_name)? {
                        if remove_view_dir(&session.view_dir) {
                            count += 1;
                        }
                    }
                }
            }

            Ok(Report::new(StopOutcome { stopped: count }))
        }
    }
}

/// Remove the View Dir. Returns whether it existed and was removed.
///
/// Idempotent: if the View Dir is already gone, returns `false` — the caller
/// treats this as a no-op, not a failure.
fn remove_view_dir(view_dir: &Utf8PathBuf) -> bool {
    if !fs::exists(view_dir).unwrap_or(false) {
        return false;
    }
    // Use std::fs::remove_dir_all: the View Dir may contain symlinked repos
    // and config dirs; removing it recursively is the right cleanup.
    std::fs::remove_dir_all(view_dir.as_std_path()).is_ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/stop.rs"]
mod tests;
