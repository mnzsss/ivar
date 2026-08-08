//! `ivar repo remove` — deregister a repo from the hall.
//!
//! The inverse of `ivar repo add` (valhalla's **Deregister**): drops the repo
//! from `ivar.json` and tears down its entire `.ivar/repos/<name>/` tree — the
//! bare clone and every worktree, including the feature-branch worktrees of
//! any feature that promoted the repo.
//!
//! Because that can destroy unpushed feature work, it is **gated**: it refuses
//! while the repo is promoted in any feature or referenced by any live session
//! (a session whose view dir exists), naming the blockers. `--force` lifts both
//! gates and cascades — removing the worktrees, scrubbing the repo from every
//! feature's promotion records, regenerating each provider's config, and
//! repairing the dangling `repos/<name>` symlink in every live view dir.
//!
//! Teardown is best-effort per step: a step that fails becomes a
//! [`Warning`] and the run continues. The manifest write and provider
//! regeneration are the authoritative final steps, so an interrupted run
//! leaves state that a retry — idempotent, because absent targets are
//! skipped — can finish.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::sync::{self, Change, Entry};
use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar repo remove` needs.
#[derive(Debug, Clone)]
pub struct RemoveInput {
    /// The repo's name, unvalidated — [`RepoName`] is this module's job.
    pub name: String,
    /// Lift the promotion and live-session gates and cascade the teardown.
    pub force: bool,
}

/// What `ivar repo remove` did.
#[derive(Debug, Clone, Serialize)]
pub struct RemoveOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The repo, as it was removed from `ivar.json`.
    pub name: RepoName,
    /// Every teardown step, in the order it ran. A step ending in
    /// [`Change::Failed`] also produced a warning — the run continued.
    pub steps: Vec<Entry>,
}

impl WriteHuman for RemoveOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "Removed repo `{}` from {}", self.name, self.root)?;
        if self.steps.is_empty() {
            writeln!(w, "  (nothing to tear down)")?;
        }
        for step in &self.steps {
            match &step.detail {
                Some(detail) => {
                    writeln!(w, "  {} {} — {detail}", step.change.symbol(), step.label)?
                }
                None => writeln!(w, "  {} {}", step.change.symbol(), step.label)?,
            }
        }
        Ok(())
    }
}

/// Remove `input.name` from the hall: gate, then cascade.
///
/// A repo that is not in the manifest is blocked ([`Manifest::with_repo_removed`]
/// refuses it with `repo.not_found`), so a typo cannot silently "succeed". A
/// repo that is promoted or live-session-referenced is blocked naming every
/// blocker, unless `input.force` lifts the gate (S-DEREGISTER-SAFETY).
pub fn remove(ctx: &Ctx, input: RemoveInput) -> Outcome<RemoveOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let name = RepoName::new(input.name)?;
    // `with_repo_removed` both validates presence and produces the manifest we
    // write at the end — the canonical "not in ivar.json" failure comes free,
    // and the updated manifest is held until the teardown completes, so a
    // blocked run never rewrites the file.
    let updated = manifest.with_repo_removed(&name)?;

    // Gate (N-PREFLIGHT): collect every blocker before anything is touched.
    if !input.force {
        let blockers = collect_blockers(&layout, &name)?;
        if !blockers.is_empty() {
            return Err(gate_failure(&name, &blockers));
        }
    }

    let mut steps = Vec::new();
    let mut warnings = Vec::new();

    let features = features_promoting(&layout, &name)?;

    // 1. Feature-branch worktrees, removed through git so the bare's worktree
    //    metadata goes with them. `--force` has been given, so git's dirty-
    //    worktree refusal — the guard the gate exists to override — is lifted.
    let bare = layout.repo_bare(&name);
    for feature in &features {
        let worktree = layout.repo_worktree(&name, &feature.branch);
        let surface = format!("feature {}", feature.name);
        if !fs::is_dir(&worktree)? {
            // Never materialised; the promotion scrub below is the whole step.
            continue;
        }
        match git.remove_worktree(&bare, &worktree) {
            Ok(()) => steps.push(Entry::new(
                &surface,
                format!("worktree {}", feature.branch),
                Change::Removed,
            )),
            Err(error) => record_step(
                &mut steps,
                &mut warnings,
                &surface,
                format!("worktree {}", feature.branch),
                error.into(),
            ),
        }
    }

    // 2. Scrub the repo from every feature's promotion records.
    for mut feature in features {
        let surface = format!("feature {}", feature.name);
        feature.demote(&name);
        match feature.write(&layout) {
            Ok(()) => steps.push(Entry::new(
                &surface,
                format!("promotion of `{name}`"),
                Change::Removed,
            )),
            Err(error) => record_step(
                &mut steps,
                &mut warnings,
                &surface,
                format!("promotion of `{name}`"),
                error,
            ),
        }
    }

    // 3. Repair every live view dir: the `repos/<name>` symlink is now
    //    dangling, and with the repo gone the repair is to unlink it.
    for view_dir in live_view_dirs(&layout)? {
        for candidate in [
            view_dir.join(name.as_str()),
            view_dir.join("repos").join(name.as_str()),
        ] {
            if matches!(fs::read_symlink(&candidate)?, fs::SymlinkTarget::Target(_)) {
                match fs::remove_file(&candidate) {
                    Ok(()) => steps.push(Entry::new(
                        "view dir",
                        candidate.to_string(),
                        Change::Removed,
                    )),
                    Err(error) => record_step(
                        &mut steps,
                        &mut warnings,
                        "view dir",
                        candidate.to_string(),
                        error.into(),
                    ),
                }
            }
        }
    }

    // 4. The repo's whole store dir: the bare clone, the default worktree, and
    //    any worktree step 1 could not remove. Idempotent — absent is fine.
    let repo_dir = layout.repo_dir(&name);
    if fs::exists(&repo_dir)? {
        match fs::remove_path(&repo_dir) {
            Ok(()) => steps.push(Entry::new(
                name.to_string(),
                format!(".ivar/repos/{name}/"),
                Change::Removed,
            )),
            Err(error) => record_step(
                &mut steps,
                &mut warnings,
                name.as_str(),
                format!(".ivar/repos/{name}/"),
                error.into(),
            ),
        }
    } else {
        steps.push(Entry::new(
            name.to_string(),
            format!(".ivar/repos/{name}/"),
            Change::Unchanged,
        ));
    }

    // 5. The authoritative final steps. The manifest write failing aborts the
    //    verb — the repo is still declared, so a retry is safe — while provider
    //    regeneration is best-effort per provider, exactly as `ivar sync` runs
    //    it.
    Manifest::write(&layout, &updated)?;
    steps.push(Entry::new("hall", "ivar.json", Change::Updated));

    sync::sync_providers(&layout, &updated, &mut steps, &mut warnings);

    Ok(Report::with_warnings(
        RemoveOutcome {
            root: layout.root().to_path_buf(),
            name,
            steps,
        },
        warnings,
    ))
}

/// Every reason `name` cannot be removed yet: features promoting it, and live
/// session view dirs referencing it. Collected before any mutation.
fn collect_blockers(layout: &Layout, name: &RepoName) -> Result<Vec<String>, Failure> {
    let mut blockers = Vec::new();

    for feature in features_promoting(layout, name)? {
        blockers.push(format!("promoted into feature `{}`", feature.name));
    }

    for view_dir in live_view_dirs(layout)? {
        if view_dir_references(&view_dir, name)? {
            blockers.push(format!(
                "referenced by live session view dir `{}`",
                view_dir
            ));
        }
    }

    Ok(blockers)
}

/// The gate failure, naming every blocker and pointing at `--force`.
fn gate_failure(name: &RepoName, blockers: &[String]) -> Failure {
    Failure::blocked(
        "repo.in_use",
        format!("`{name}` cannot be removed while it is still referenced"),
    )
    .expected("the repo to be promoted in no feature and referenced by no live session")
    .actual(format!("still referenced: {}", blockers.join("; ")))
    .fix(FixAction::unsafe_(
        "repo.remove_force",
        format!(
            "Run `ivar repo remove --force {name}` to tear it down — worktrees, promotion records, and all — anyway."
        ),
    ))
}

/// Every feature that promotes `name`, read from disk.
fn features_promoting(layout: &Layout, name: &RepoName) -> Result<Vec<Feature>, Failure> {
    let mut features = Vec::new();
    for feature_name in feature_names(layout)? {
        if let Some(feature) = Feature::read(layout, &feature_name)?
            && feature.is_promoted(name)
        {
            features.push(feature);
        }
    }
    Ok(features)
}

/// Every feature directory name in the hall, sorted.
fn feature_names(layout: &Layout) -> Result<Vec<FeatureName>, Failure> {
    let dir = layout.features_dir();
    if !fs::is_dir(&dir)? {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let Some(file_name) = entry.file_name() else {
            continue;
        };
        if let Ok(name) = FeatureName::new(file_name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Every live session view dir: feature sessions under each feature, plus
/// discovery sessions. "Live" means the view dir exists — liveness does not
/// depend on any running process.
fn live_view_dirs(layout: &Layout) -> Result<Vec<Utf8PathBuf>, Failure> {
    let mut dirs = Vec::new();

    for feature in feature_names(layout)? {
        let sessions = layout.feature_dir(&feature).join("sessions");
        if fs::is_dir(&sessions)? {
            dirs.extend(fs::read_dir(&sessions)?);
        }
    }

    let sessions = layout.discovery_sessions_dir();
    if fs::is_dir(&sessions)? {
        dirs.extend(fs::read_dir(&sessions)?);
    }

    dirs.sort();
    Ok(dirs)
}

/// Whether `view_dir` references `name`: a symlink named after the repo — or
/// under a `repos/` subdir, the valhalla view-dir shape — pointing somewhere.
fn view_dir_references(view_dir: &Utf8Path, name: &RepoName) -> Result<bool, Failure> {
    for candidate in [
        view_dir.join(name.as_str()),
        view_dir.join("repos").join(name.as_str()),
    ] {
        if matches!(fs::read_symlink(&candidate)?, fs::SymlinkTarget::Target(_)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Turn a best-effort teardown step's [`Failure`] into a report entry plus a
/// warning, and keep going — the warning is what makes the process exit `1`
/// instead of pretending the teardown was clean.
fn record_step(
    steps: &mut Vec<Entry>,
    warnings: &mut Vec<Warning>,
    surface: &str,
    label: String,
    failure: Failure,
) {
    steps.push(Entry::new(surface, label, Change::Failed).detail(failure.what.clone()));
    warnings.push(Warning::new(
        "repo.remove_step_failed",
        surface,
        failure.what,
    ));
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
    use crate::domain::name::{BranchName, HallName, SessionId};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::manifest::{Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    /// A hall with one seeded repo declared in `ivar.json` — not yet synced.
    fn hall_declared() -> (tempfile::TempDir, Utf8PathBuf) {
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

        let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            vec![Repo::new(
                RepoName::new("api").unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )],
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();

        (guard, root)
    }

    /// [`hall_declared`] with the repo materialised the way `ivar sync` would.
    fn hall_with_repo() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = hall_declared();
        let ctx = Ctx::new(root.clone());
        crate::action::sync::sync(&ctx, Default::default()).unwrap();
        (guard, root)
    }

    fn create_feature(ctx: &Ctx, name: &str) {
        feature_create::create(
            ctx,
            CreateInput {
                name: name.to_owned(),
            },
        )
        .unwrap();
    }

    fn promote(ctx: &Ctx, feature: &str, repo: &str) {
        feature_promote::promote(
            ctx,
            PromoteInput {
                feature: feature.to_owned(),
                repo: repo.to_owned(),
            },
        )
        .unwrap();
    }

    fn input(name: &str, force: bool) -> RemoveInput {
        RemoveInput {
            name: name.to_owned(),
            force,
        }
    }

    fn manifest(root: &Utf8Path) -> Manifest {
        Manifest::read(&Layout::at(root.to_path_buf()))
            .unwrap()
            .unwrap()
    }

    // -- not in the hall / not in the manifest --------------------------------

    #[test]
    fn remove_outside_a_hall_is_blocked() {
        let (_guard, root) = hall_root();
        let ctx = Ctx::new(root);

        let failure = remove(&ctx, input("api", false)).unwrap_err();

        assert_eq!(failure.code, "hall.not_found");
    }

    #[test]
    fn remove_rejects_a_repo_that_is_not_in_the_manifest() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root);

        let failure = remove(&ctx, input("ghost", false)).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "manifest.repo_not_found");
    }

    // -- the gate -------------------------------------------------------------

    #[test]
    fn remove_refuses_while_the_repo_is_promoted_in_a_feature() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        create_feature(&ctx, "checkout");
        promote(&ctx, "checkout", "api");

        let failure = remove(&ctx, input("api", false)).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.in_use");
        // The blocker is named.
        let actual = failure.actual.as_deref().unwrap();
        assert!(
            actual.contains("checkout"),
            "the blocking feature must be named: {actual}"
        );
        // Nothing was touched.
        assert_eq!(manifest(&root).repos().len(), 1);
        assert!(root.join(".ivar/repos/api/checkout/README.md").is_file());
        assert!(
            !failure.fix_actions[0].safe,
            "removing a promoted repo must need a human"
        );
    }

    #[test]
    fn remove_refuses_while_the_repo_is_referenced_by_a_live_session_view_dir() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        // A live discovery-session view dir referencing the default worktree.
        let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
        let view_dir = Layout::at(root.clone()).discovery_session(&session);
        fs::ensure_dir(&view_dir).unwrap();
        fs::create_symlink(&root.join(".ivar/repos/api/main"), &view_dir.join("api")).unwrap();

        let failure = remove(&ctx, input("api", false)).unwrap_err();

        assert_eq!(failure.code, "repo.in_use");
        assert!(
            failure.actual.as_deref().unwrap().contains("view dir"),
            "the blocking view dir must be named: {:?}",
            failure.actual
        );
    }

    // -- the teardown ---------------------------------------------------------

    #[test]
    fn remove_without_force_tears_down_a_clean_repo() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());

        let report = remove(&ctx, input("api", false)).unwrap();

        assert!(report.is_clean());
        assert!(manifest(&root).repos().is_empty());
        assert!(
            !fs::exists(&root.join(".ivar/repos/api")).unwrap(),
            "the whole repo tree must go"
        );
        assert!(report.value.steps.iter().any(|step| {
            step.label.contains(".ivar/repos/api/") && step.change == Change::Removed
        }));
    }

    #[test]
    fn remove_works_for_a_declared_but_never_synced_repo() {
        let (_guard, root) = hall_declared();
        let ctx = Ctx::new(root.clone());

        let report = remove(&ctx, input("api", false)).unwrap();

        assert!(report.is_clean());
        assert!(manifest(&root).repos().is_empty());
    }

    /// The full cascade: two features promoting the repo, one with a live
    /// view dir pointing at its feature worktree.
    #[test]
    fn remove_force_cascades_across_worktrees_promotions_and_view_dirs() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        create_feature(&ctx, "checkout");
        create_feature(&ctx, "billing");
        promote(&ctx, "checkout", "api");
        promote(&ctx, "billing", "api");
        // A live feature-session view dir referencing the checkout worktree.
        let session = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
        let view_dir = Layout::at(root.clone())
            .feature_session(&FeatureName::new("checkout").unwrap(), &session);
        fs::ensure_dir(&view_dir).unwrap();
        fs::create_symlink(
            &root.join(".ivar/repos/api/checkout"),
            &view_dir.join("api"),
        )
        .unwrap();

        let report = remove(&ctx, input("api", true)).unwrap();

        assert!(report.is_clean());
        // Every worktree and the whole repo tree are gone.
        assert!(!fs::exists(&root.join(".ivar/repos/api")).unwrap());
        // Both features' promotion records are scrubbed.
        for feature in ["checkout", "billing"] {
            let feature = Feature::read(
                &Layout::at(root.clone()),
                &FeatureName::new(feature).unwrap(),
            )
            .unwrap()
            .unwrap();
            assert!(!feature.is_promoted(&RepoName::new("api").unwrap()));
        }
        // The dangling view-dir symlink is unlinked.
        assert_eq!(
            fs::read_symlink(&view_dir.join("api")).unwrap(),
            fs::SymlinkTarget::Absent
        );
        // The manifest no longer lists the repo.
        assert!(manifest(&root).repos().is_empty());
        // The provider config is regenerated: the repo is gone from the block.
        let block = fs::read_text(&root.join("CLAUDE.md")).unwrap().unwrap();
        assert!(
            !block.contains("`api`"),
            "provider config must be regenerated: {block}"
        );
    }

    /// Best-effort (N-BEST-EFFORT): a teardown step that fails becomes a
    /// warning, the manifest is still written (the authoritative step), and a
    /// retry can finish whatever is left.
    #[test]
    fn remove_force_survives_a_failed_step_and_still_writes_the_manifest() {
        let (_guard, root) = hall_with_repo();
        let ctx = Ctx::new(root.clone());
        // A feature whose promotion record exists but whose "worktree" is a
        // hand-made directory git does not recognise — `git worktree remove`
        // refuses it, exercising the best-effort path.
        create_feature(&ctx, "checkout");
        let mut feature = Feature::read(
            &Layout::at(root.clone()),
            &FeatureName::new("checkout").unwrap(),
        )
        .unwrap()
        .unwrap();
        feature.promote(RepoName::new("api").unwrap());
        feature.write(&Layout::at(root.clone())).unwrap();
        let stray = root.join(".ivar/repos/api/checkout");
        fs::ensure_dir(&stray).unwrap();
        fs::write_text(&stray.join("mine.txt"), "mine").unwrap();

        let report = remove(&ctx, input("api", true)).unwrap();

        assert!(!report.is_clean(), "a failed step must not be a clean run");
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.code == "repo.remove_step_failed"),
            "the failed step must surface as a warning"
        );
        // The manifest write is authoritative: the repo is gone from ivar.json.
        assert!(manifest(&root).repos().is_empty());
        // And the whole repo tree went anyway.
        assert!(!fs::exists(&root.join(".ivar/repos/api")).unwrap());
    }

    #[test]
    fn the_human_surface_lists_the_teardown_steps() {
        let outcome = RemoveOutcome {
            root: Utf8PathBuf::from("/hall"),
            name: RepoName::new("api").unwrap(),
            steps: vec![
                Entry::new("feature checkout", "worktree checkout", Change::Removed),
                Entry::new("hall", "ivar.json", Change::Updated),
            ],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Removed repo `api` from /hall\n  - worktree checkout\n  ~ ivar.json\n"
        );
    }
}
