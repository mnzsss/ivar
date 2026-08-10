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
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::action::feature::create::{self as feature_create, CreateInput};
    use crate::action::feature::promote::{self as feature_promote, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::action::session::start::{self as session_start, StartInput};
    use crate::domain::name::{BranchName, FeatureName, HallName, RepoName};
    use crate::domain::provider::Provider;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    /// A hall with `api` promoted into `checkout`, plus a detached session.
    fn hall_with_detached_session() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_root();
        let ctx = Ctx::new(root.clone());
        hall::init(
            &ctx,
            InitInput {
                path: Utf8PathBuf::from("."),
                name: Some("acme".to_owned()),
                provider: None,
            },
        )
        .unwrap();

        let origins = root.parent().unwrap().join("origins");
        let api_origin = seeded_repo(&origins.join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        feature_create::create(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
                branch: None,
            },
        )
        .unwrap();
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        feature_promote::promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: "api".to_owned(),
            },
        )
        .unwrap();

        session_start::start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap();

        (guard, root)
    }

    /// Undo the read-only guards applied, so TempDir can clean up.
    fn unguard_worktrees(root: &camino::Utf8Path) {
        let repos = root.join(".ivar/repos");
        if !fs::is_dir(&repos).unwrap() {
            return;
        }
        for repo in fs::read_dir(&repos).unwrap() {
            for worktree in fs::read_dir(&repo).unwrap() {
                let _ = fs::restore_write_bits(&worktree);
            }
        }
    }

    fn session_id_of(root: &camino::Utf8Path) -> String {
        let layout = Layout::at(root.to_path_buf());
        let dir = layout
            .feature_dir(&FeatureName::new("checkout").unwrap())
            .join("sessions");
        let entry = fs::read_dir(&dir).unwrap();
        let session_dir = &entry[0];
        session_dir.file_name().unwrap().to_owned()
    }

    #[test]
    fn stop_ends_a_live_session_and_removes_the_view_dir() {
        let (_guard, root) = hall_with_detached_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let id = session_id_of(&root);
        let session_id = crate::domain::name::SessionId::new(id.clone()).unwrap();
        let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);

        assert!(fs::is_dir(&view_dir).unwrap());

        let report = stop(&ctx, StopInput { session: Some(id) }).unwrap();

        assert_eq!(report.value.stopped, 1);
        assert!(
            !fs::is_dir(&view_dir).unwrap(),
            "the view dir must be removed"
        );
        unguard_worktrees(&root);
    }

    #[test]
    fn stop_of_an_already_stopped_session_is_a_no_op() {
        let (_guard, root) = hall_with_detached_session();
        let ctx = Ctx::new(root.clone());
        let id = session_id_of(&root);

        // First stop: removes the view dir.
        stop(
            &ctx,
            StopInput {
                session: Some(id.clone()),
            },
        )
        .unwrap();

        // Second stop: the view dir is already gone → no-op.
        let report = stop(&ctx, StopInput { session: Some(id) }).unwrap();

        assert_eq!(report.value.stopped, 0, "already-stopped must be a no-op");
        unguard_worktrees(&root);
    }

    #[test]
    fn stop_all_stops_every_live_session() {
        let (_guard, root) = hall_with_detached_session();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());

        // Add a second session.
        session_start::start(
            &ctx,
            StartInput {
                feature: "checkout".to_owned(),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap();

        let report = stop(&ctx, StopInput { session: None }).unwrap();

        assert_eq!(report.value.stopped, 2);

        // Both view dirs must be gone.
        let sessions_dir = layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap());
        let entries: Vec<_> = fs::read_dir(&sessions_dir)
            .unwrap()
            .into_iter()
            .filter(|e| e.file_name().is_some_and(|n| !n.starts_with('.')))
            .collect();
        assert!(entries.is_empty(), "all sessions must be stopped");
        unguard_worktrees(&root);
    }

    #[test]
    fn stop_emits_human_output() {
        let (_guard, root) = hall_with_detached_session();
        let ctx = Ctx::new(root.clone());
        let id = session_id_of(&root);

        let report = stop(&ctx, StopInput { session: Some(id) }).unwrap();

        let mut out = Vec::new();
        report.value.write_human(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Stopped 1 session"), "was: {text}");
        unguard_worktrees(&root);
    }
}
