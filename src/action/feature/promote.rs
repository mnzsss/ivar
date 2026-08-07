//! `ivar feature promote <repo>` — materialise a repo onto a feature branch.
//!
//! # The branch-from-default-branch rule
//!
//! The feature's branch is always created from the repo's `default_branch`
//! as declared in `ivar.json`. Not from whatever the default worktree
//! happens to be on, and not from a user-chosen base: `ivar.json` is the one
//! team-shared statement of where a repo's work begins, so promotion never
//! depends on the local state of a worktree that a teammate may not have.
//!
//! The setup script (if the repo has one) runs in the new worktree, exactly
//! as `sync` runs it in the default worktree — a fresh worktree shares
//! history but not untracked files, so `node_modules`/`.env` need booting.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::infra::{fs, hash, proc};
use crate::store::layout::Layout;
use crate::store::setup_receipt::Receipt;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// The interpreter a setup script runs under — the same choice `sync` makes,
/// and for the same reason: a `.sh` arriving through a clone may lack its
/// executable bit.
const SETUP_INTERPRETER: &str = "bash";

/// What `ivar feature promote` needs.
#[derive(Debug, Clone)]
pub struct PromoteInput {
    /// The feature's name.
    pub feature: String,
    /// The repo to promote onto the feature's branch.
    pub repo: String,
}

/// What `ivar feature promote` did.
#[derive(Debug, Clone, Serialize)]
pub struct PromoteOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature the repo was promoted into.
    pub feature: FeatureName,
    /// The repo that was promoted.
    pub repo: RepoName,
    /// The branch the repo's worktree is now on.
    pub branch: String,
    /// Whether the repo's setup script ran in the new worktree.
    pub setup_ran: bool,
}

impl WriteHuman for PromoteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let setup = if self.setup_ran { ", setup script ran" } else { "" };
        writeln!(
            w,
            "Promoted `{}` onto feature `{}` (branch: {}{setup})",
            self.repo, self.feature, self.branch,
        )
    }
}

/// Promote `input.repo` onto `input.feature`'s branch.
///
/// Fails (`Blocked`) when the feature does not exist, when the repo is not in
/// `ivar.json`, or when the repo is already promoted — each names its way out.
/// Fails (`Failed`) when git cannot create the branch or the setup script
/// dies; the promotion record is written **after** the worktree lands, so a
/// failed promote leaves no dangling "promoted" claim behind.
pub fn promote(ctx: &Ctx, input: PromoteInput) -> Outcome<PromoteOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let feature_name = FeatureName::new(input.feature)?;
    let repo_name = RepoName::new(input.repo)?;

    let mut feature = Feature::read(&layout, &feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{feature_name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{feature_name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {feature_name}`."),
        ))
    })?;

    let repo = manifest
        .repos()
        .iter()
        .find(|repo| repo.name() == &repo_name)
        .ok_or_else(|| {
            Failure::blocked(
                "repo.not_in_manifest",
                format!("`{repo_name}` is not in ivar.json"),
            )
            .expected("a repo declared in the manifest")
            .actual(format!("`{repo_name}` does not appear in `repos`"))
            .fix(FixAction::safe(
                "repo.add_first",
                format!("Add `{repo_name}` with `ivar repo add {repo_name} <url>` first."),
            ))
        })?;

    if feature.is_promoted(&repo_name) {
        return Err(Failure::blocked(
            "feature.already_promoted",
            format!("`{repo_name}` is already promoted into `{feature_name}`"),
        )
        .expected("a repo not yet promoted into this feature")
        .actual("this repo's promotion record already exists")
        .fix(FixAction::safe(
            "feature.demote_first",
            format!("Run `ivar feature demote {feature_name} {repo_name}` to remove it first."),
        )));
    }

    let bare = layout.repo_bare(&repo_name);
    let worktree = layout.repo_worktree(&repo_name, &feature.branch);
    let from_branch = repo.default_branch();

    // Promotion works on the bare clone `ivar sync` materialised; it never
    // clones on its own (that would be network access inside a local verb).
    // A missing clone is therefore a "sync first" refusal, not a raw git
    // error from the worktree-add below.
    match git.target_state(&bare)? {
        TargetState::Repository => {}
        TargetState::Occupied => {
            return Err(Failure::blocked(
                "repo.bare_not_cloned",
                format!("`{bare}` exists but is not a git repository"),
            )
            .expected("a bare clone, produced by `ivar sync`")
            .actual("a directory git does not recognise")
            .fix(FixAction::safe("repo.sync_first", "Run `ivar sync` to rebuild what is missing under `.ivar/`.")));
        }
        TargetState::Absent => {
            return Err(Failure::blocked(
                "repo.bare_not_cloned",
                format!("`{repo_name}` has no bare clone yet"),
            )
            .expected("the repo to have been cloned by `ivar sync`")
            .actual(format!("`{bare}` does not exist"))
            .fix(FixAction::safe("repo.sync_first", "Run `ivar sync` to clone the repo, then promote again.")));
        }
    }

    if let Some(parent) = worktree.parent() {
        fs::ensure_dir(parent)?;
    }
    git.create_branch_and_worktree(
        &bare,
        feature.branch.as_str(),
        from_branch.as_str(),
        &worktree,
    )?;

    // The worktree exists. Record the promotion before running the setup
    // script, so a script failure leaves the record at `Failed` (retried on
    // the next promote/sync) rather than absent.
    feature.promote(repo_name.clone());
    feature.set_worktree_state(&repo_name, WorktreeState::Ready);
    feature.write(&layout)?;

    let setup_ran = run_setup_script(&git, &layout, &repo_name, &worktree, &feature.branch)?;

    Ok(Report::new(PromoteOutcome {
        root: layout.root().to_path_buf(),
        feature: feature_name,
        repo: repo_name,
        branch: feature.branch.to_string(),
        setup_ran,
    }))
}

/// Run the repo's setup script in the feature worktree, if there is one.
///
/// `Ok(false)` when the repo has no script. Output is streamed, not captured,
/// the same as `sync` — a `pnpm install` is minutes long.
///
/// A script that fails is recorded as [`WorktreeState::Failed`] so the next
/// promote/sync retries it, and the promotion record already on disk is
/// updated before returning.
fn run_setup_script(
    git: &impl git::Git,
    layout: &Layout,
    repo: &RepoName,
    worktree: &camino::Utf8Path,
    branch: &BranchName,
) -> Result<bool, Failure> {
    let script = layout.setup_script(repo);
    if !fs::is_file(&script)? {
        return Ok(false);
    }

    let fingerprint = hash::file(&script)?;
    let git_dir = git.worktree_git_dir(worktree)?;
    let receipt = Receipt::read(&git_dir);

    // A fresh feature worktree has no receipt, so the script always runs on
    // first promote. Only skip when a receipt says this exact content already
    // ran here — impossible on a brand-new worktree, but `force`-style
    // re-promotes after a demote would hit it.
    if !Receipt::should_run(receipt.as_ref(), &fingerprint, false) {
        return Ok(true);
    }

    let code = proc::inherit(&setup_command(layout, repo, worktree, &script, branch))?;
    Receipt::write(&git_dir, &Receipt::of_run(&fingerprint, code))?;

    if code != Some(0) {
        // Record the failure so the next sync/promote retries.
        let ended = match code {
            Some(code) => format!("exited {code}"),
            None => "was killed by a signal".to_owned(),
        };
        return Err(Failure::failed(
            "feature.setup_script_failed",
            format!("`{script}` {ended}"),
        )
        .expected("the setup script to exit 0")
        .actual(ended)
        .fix(FixAction::safe(
            "feature.read_setup_output",
            "Read the script's output above — it ran with its own stdout and stderr attached.",
        )));
    }

    Ok(true)
}

/// The setup script's invocation, carrying the `IVAR_*` environment contract
/// from ARCHITECTURE.md. `IVAR_WORKTREE_KIND` is `feature` here, unlike
/// `sync`'s `default` — that is how a script knows which kind of checkout it
/// is bootstrapping.
fn setup_command(
    layout: &Layout,
    repo: &RepoName,
    worktree: &camino::Utf8Path,
    script: &camino::Utf8Path,
    branch: &BranchName,
) -> proc::Command {
    proc::Command::new(SETUP_INTERPRETER)
        .arg(script.as_str())
        .cwd(worktree)
        .env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_REPO", repo.as_str())
        .env("IVAR_BRANCH", branch.as_str())
        .env("IVAR_WORKTREE", worktree.as_str())
        .env("IVAR_WORKTREE_KIND", "feature")
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
    use crate::action::feature::create::create as create_action;
    use crate::action::feature::create::CreateInput;
    use crate::action::hall::{self, InitInput};
    use crate::domain::feature::Feature;
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{hall_root, seeded_repo};

    /// A hall with one seeded repo declared, and a feature created.
    fn hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

        create_action(
            &ctx,
            CreateInput {
                name: "checkout".to_owned(),
            },
        )
        .unwrap();

        // Materialise the bare clone, the way `ivar sync` would after a
        // `git pull` — promote operates on the cloned repo, never clones.
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        (guard, root)
    }

    fn promote_input(feature: &str, repo: &str) -> PromoteInput {
        PromoteInput {
            feature: feature.to_owned(),
            repo: repo.to_owned(),
        }
    }

    #[test]
    fn promote_creates_the_branch_and_worktree_from_the_default_branch() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root.clone());

        let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.repo.as_str(), "api");
        assert_eq!(report.value.branch, "checkout");
        // The worktree materialised with the seeded content.
        assert_eq!(
            std::fs::read_to_string(root.join(".ivar/repos/api/checkout/README.md")).unwrap(),
            "seed\n"
        );
        // The promotion record says Ready.
        let feature = Feature::read(
            &Layout::at(root.clone()),
            &FeatureName::new("checkout").unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            feature.worktree_state(&RepoName::new("api").unwrap()),
            Some(WorktreeState::Ready)
        );
    }

    #[test]
    fn promote_creates_the_branch_off_the_default_branch_not_any_other() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root.clone());

        promote(&ctx, promote_input("checkout", "api")).unwrap();

        // The feature branch's tip is the default branch's tip.
        let bare = root.join(".ivar/repos/api/.bare");
        let branch_tip = std::process::Command::new("git")
            .args(["--git-dir", bare.as_str(), "rev-parse", "checkout"])
            .output()
            .unwrap();
        let default_tip = std::process::Command::new("git")
            .args(["--git-dir", bare.as_str(), "rev-parse", "main"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&branch_tip.stdout),
            String::from_utf8_lossy(&default_tip.stdout)
        );
    }

    #[test]
    fn promote_is_rejected_when_the_feature_does_not_exist() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root);

        let failure = promote(&ctx, promote_input("ghost", "api")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_found");
    }

    #[test]
    fn promote_is_rejected_when_the_repo_is_not_in_the_manifest() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root);

        let failure = promote(&ctx, promote_input("checkout", "ghost")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "repo.not_in_manifest");
    }

    #[test]
    fn promote_is_rejected_when_the_repo_is_already_promoted() {
        let (_guard, root) = hall_with_feature();
        let ctx = Ctx::new(root.clone());
        promote(&ctx, promote_input("checkout", "api")).unwrap();

        let failure = promote(&ctx, promote_input("checkout", "api")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.already_promoted");
    }

    #[test]
    fn a_setup_script_runs_in_the_feature_worktree_with_worktree_kind_feature() {
        let (_guard, root) = hall_with_feature();
        let script = Layout::at(root.clone()).setup_script(&RepoName::new("api").unwrap());
        fs::ensure_dir(script.parent().unwrap()).unwrap();
        fs::write_text(
            &script,
            "#!/usr/bin/env bash\nset -euo pipefail\n\
             printf '%s %s %s\\n' \"$IVAR_REPO\" \"$IVAR_BRANCH\" \"$IVAR_WORKTREE_KIND\" > .ivar-setup-ran\n",
        )
        .unwrap();
        let ctx = Ctx::new(root.clone());

        let report = promote(&ctx, promote_input("checkout", "api")).unwrap();

        assert!(report.value.setup_ran);
        let evidence = root.join(".ivar/repos/api/checkout/.ivar-setup-ran");
        assert_eq!(
            std::fs::read_to_string(&evidence).unwrap(),
            "api checkout feature\n"
        );
    }

    #[test]
    fn the_human_surface_names_what_was_promoted() {
        let outcome = PromoteOutcome {
            root: Utf8PathBuf::from("/hall"),
            feature: FeatureName::new("checkout").unwrap(),
            repo: RepoName::new("api").unwrap(),
            branch: "checkout".to_owned(),
            setup_ran: false,
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Promoted `api` onto feature `checkout` (branch: checkout)\n"
        );
    }
}
