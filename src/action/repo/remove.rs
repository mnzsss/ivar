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
#[path = "../../../tests/unit/action/repo/remove.rs"]
mod tests;
