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
    use crate::infra::fs;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    fn hall_with_two_sessions() -> (tempfile::TempDir, Utf8PathBuf) {
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

        // Two detached sessions on the same feature.
        session_start::start(
            &ctx,
            StartInput {
                feature: Some("checkout".to_owned()),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap();
        session_start::start(
            &ctx,
            StartInput {
                feature: Some("checkout".to_owned()),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap();

        (guard, root)
    }

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

    #[test]
    fn prune_does_not_touch_live_sessions() {
        let (_guard, root) = hall_with_two_sessions();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let sessions_dir = layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap());

        let before_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));

        let report = prune(&ctx).unwrap();

        assert_eq!(report.value.pruned, 0, "live sessions must not be pruned");
        let after_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));
        assert_eq!(before_count, after_count, "session dirs must remain");
        unguard_worktrees(&root);
    }

    /// A stale orphan: a view dir under the feature's session tree with no
    /// `state.json` — what a session from before session records looked like.
    fn orphan_view_dir(layout: &Layout) -> Utf8PathBuf {
        let session_id =
            crate::domain::name::SessionId::new("9a8b7c6d-5e4f-4a3b-9c2d-1e0f1a2b3c4d".to_owned())
                .unwrap();
        let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);
        fs::ensure_dir(&view_dir).unwrap();
        view_dir
    }

    #[test]
    fn prune_removes_dead_sessions_and_their_view_dirs() {
        let (_guard, root) = hall_with_two_sessions();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());
        let sessions_dir = layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap());

        let before_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));
        let orphan = orphan_view_dir(&layout);
        assert!(fs::is_dir(&orphan).unwrap());

        let report = prune(&ctx).unwrap();

        assert_eq!(report.value.pruned, 1, "the dead orphan must be pruned");
        assert!(
            !fs::is_dir(&orphan).unwrap(),
            "the orphan view dir must be gone"
        );
        let after_count = count_entries(&sessions_dir, |n| !n.starts_with('.'));
        assert_eq!(
            after_count, before_count,
            "the live sessions must remain and the orphan must be gone"
        );
        unguard_worktrees(&root);
    }

    #[test]
    fn prune_refuses_a_view_dir_with_pending_writes() {
        let (_guard, root) = hall_with_two_sessions();
        let ctx = Ctx::new(root.clone());
        let layout = Layout::at(root.clone());

        // A dead view dir with a pending conversion: the `.converting` marker
        // exists under the feature, and the orphan's record was never written.
        let orphan = orphan_view_dir(&layout);
        let feature_dir = layout.feature_dir(&FeatureName::new("checkout").unwrap());
        fs::ensure_dir(&feature_dir).unwrap();
        let converting_path = feature_dir.join(".converting");
        fs::write_text(&converting_path, "{}").unwrap();

        let failure = prune(&ctx).unwrap_err();

        assert_eq!(failure.code, "session.prune_locked");
        assert!(
            failure.what.contains(".converting"),
            "the refusal must name the lock: {}",
            failure.what
        );
        assert!(
            fs::is_dir(&orphan).unwrap(),
            "a refused prune must not remove anything"
        );

        // Cleanup
        let _ = std::fs::remove_file(&converting_path);
        unguard_worktrees(&root);
    }

    #[test]
    fn prune_emits_human_output() {
        let (_guard, root) = hall_with_detached_session();
        let ctx = Ctx::new(root.clone());

        let report = prune(&ctx).unwrap();

        let mut out = Vec::new();
        report.value.write_human(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Pruned 0 dead"), "was: {text}");
        unguard_worktrees(&root);
    }

    /// Count directory entries matching a filter.
    fn count_entries<F>(dir: &camino::Utf8Path, filter: F) -> usize
    where
        F: Fn(&str) -> bool,
    {
        fs::read_dir(dir)
            .unwrap()
            .into_iter()
            .filter(|e| e.file_name().is_some_and(&filter))
            .count()
    }

    /// A hall with one detached session (no second session).
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
                feature: Some("checkout".to_owned()),
                resume: false,
                provider: None,
                detached: true,
                relay: false,
            },
        )
        .unwrap();

        (guard, root)
    }
}
