//! `ivar feature promote <repo>` — materialise a repo onto a feature branch.
//!
//! # The branch-from-base rule
//!
//! A feature's branch that does not yet exist is always created from the
//! **base** — the feature's declared `base` (or a `--base` override for this
//! one repo), and, absent either, the repo's `default_branch` from
//! `ivar.json`. Not from whatever the default worktree happens to be on:
//! `ivar.json` (or the feature's own declaration) is the one team-shared
//! statement of where a repo's work begins, so promotion never depends on the
//! local state of a worktree that a teammate may not have.
//!
//! The base is recorded on the promotion (`Promotion::base`) as a plain fact
//! about where the branch started, never re-derived by probing ancestry —
//! base is a statement about the future, not a measurement of the past. When
//! the declared base names a branch this repo does not have, promotion still
//! proceeds: it falls back to `default_branch`, records that as the fact, and
//! warns (`feature.base_absent`) naming the repo, the base that was asked
//! for, and the one that was used instead.
//!
//! # Adopting a branch that already exists
//!
//! The rule above says where a branch *starts*. It has nothing to say about a
//! branch that is already there, and promotion used to fail on one: it always
//! ran `git worktree add -b`, which refuses a name git already knows.
//!
//! That made three ordinary situations impossible. A teammate pushes a feature
//! branch and you promote the same feature. You delete a feature and recreate
//! it. You arrive at `ivar` with branches from whatever you used before. In
//! each, the branch is the work, and refusing to check it out is refusing the
//! work.
//!
//! So promotion looks first: a branch git already has is **adopted**, checked
//! out as-is with no rebase and no reset, and a branch it does not have is
//! created off `default_branch` as before. Adoption never moves a ref — the
//! commits on that branch are someone's work, and `promote` is not the verb
//! that rewrites it. `ivar feature rebase` is.
//!
//! The setup script (if the repo has one) runs in the new worktree, exactly
//! as `sync` runs it in the default worktree — a fresh worktree shares
//! history but not untracked files, so `node_modules`/`.env` need booting.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{Feature, WorktreeState, effective_base};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
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
    /// Override the branch this promotion starts from, unvalidated. `None`
    /// falls back to the feature's declared base, then the repo's default
    /// branch — see [`crate::domain::feature::effective_base`].
    pub base: Option<String>,
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
    /// Whether that branch already existed and was checked out as-is, rather
    /// than created off the repo's default branch. Reported because the two
    /// leave the worktree at very different commits, and only the user knows
    /// which one they meant.
    pub adopted_branch: bool,
    /// Whether the repo's setup script ran in the new worktree.
    pub setup_ran: bool,
}

impl WriteHuman for PromoteOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let branch = if self.adopted_branch {
            format!("adopted existing branch: {}", self.branch)
        } else {
            format!("branch: {}", self.branch)
        };
        let setup = if self.setup_ran {
            ", setup script ran"
        } else {
            ""
        };
        writeln!(
            w,
            "Promoted `{}` onto feature `{}` ({branch}{setup})",
            self.repo, self.feature,
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
    let default_branch = repo.default_branch();
    let base_override = input.base.map(BranchName::new).transpose()?;
    let declared_base = base_override.as_ref().or(feature.base.as_ref());
    let candidate_base = effective_base(declared_base, default_branch);

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
            .fix(FixAction::safe(
                "repo.sync_first",
                "Run `ivar sync` to rebuild what is missing under `.ivar/`.",
            )));
        }
        TargetState::Absent => {
            return Err(Failure::blocked(
                "repo.bare_not_cloned",
                format!("`{repo_name}` has no bare clone yet"),
            )
            .expected("the repo to have been cloned by `ivar sync`")
            .actual(format!("`{bare}` does not exist"))
            .fix(FixAction::safe(
                "repo.sync_first",
                "Run `ivar sync` to clone the repo, then promote again.",
            )));
        }
    }

    if let Some(parent) = worktree.parent() {
        fs::ensure_dir(parent)?;
    }

    // Look before creating. `git worktree add -b` refuses a branch git already
    // knows, and that branch is usually the work the user is promoting *for* —
    // pushed by a teammate, left by a deleted-and-recreated feature, or carried
    // in from whatever they used before `ivar`.
    let existing_branches = git.list_branches(&bare)?;
    let adopted_branch = existing_branches
        .iter()
        .any(|existing| existing == feature.branch.as_str());

    // The base is recorded as a fact about where the branch starts, never
    // re-derived by probing ancestry. When it was declared explicitly but
    // names a branch this repo does not have, promotion still proceeds —
    // falling back to `default_branch` and warning, rather than refusing —
    // because the declaration is usually right about *some* repo in the hall
    // and simply does not apply to this one.
    let mut warnings = Vec::new();
    let base = if declared_base.is_some()
        && !existing_branches
            .iter()
            .any(|existing| existing == candidate_base.as_str())
    {
        warnings.push(Warning::new(
            "feature.base_absent",
            repo_name.as_str(),
            format!(
                "declared base `{candidate_base}` does not exist in `{repo_name}`; used `{default_branch}` instead"
            ),
        ));
        default_branch.clone()
    } else {
        candidate_base
    };

    if adopted_branch {
        // Checked out as-is. No rebase, no reset: those commits are someone's
        // work, and `ivar feature rebase` is the verb that moves them.
        git.add_worktree(&bare, &worktree, feature.branch.as_str())?;
    } else {
        git.create_branch_and_worktree(&bare, feature.branch.as_str(), base.as_str(), &worktree)?;
    }

    // The worktree exists. Record the promotion before running the setup
    // script, so a script failure leaves the record at `Failed` (retried on
    // the next promote/sync) rather than absent.
    feature.promote(repo_name.clone());
    if let Some(promotion) = feature.promotions.get_mut(&repo_name) {
        promotion.base = Some(base);
    }
    feature.set_worktree_state(&repo_name, WorktreeState::Ready);
    feature.write(&layout)?;

    let setup_ran = run_setup_script(
        &git,
        &layout,
        &repo_name,
        &worktree,
        &feature.branch,
        &feature.name,
    )?;

    Ok(Report::with_warnings(
        PromoteOutcome {
            root: layout.root().to_path_buf(),
            feature: feature_name,
            repo: repo_name,
            branch: feature.branch.to_string(),
            adopted_branch,
            setup_ran,
        },
        warnings,
    ))
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
    feature: &FeatureName,
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

    let code = proc::inherit(&setup_command(
        layout, repo, worktree, &script, branch, feature,
    ))?;
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
    feature: &FeatureName,
) -> proc::Command {
    proc::Command::new(SETUP_INTERPRETER)
        .arg(script.as_str())
        .cwd(worktree)
        .env("IVAR_HALL", layout.root().as_str())
        .env("IVAR_REPO", repo.as_str())
        .env("IVAR_BRANCH", branch.as_str())
        .env("IVAR_WORKTREE", worktree.as_str())
        .env("IVAR_SECRETS_DIR", layout.secrets_dir().as_str())
        .env("IVAR_WORKTREE_KIND", "feature")
        // Set here and nowhere else in this file's sibling path: `sync` runs
        // the same script against the default worktree, where there is no
        // feature to name. `IVAR_WORKTREE_KIND` is what a script branches on
        // to know whether this is set.
        .env("IVAR_FEATURE", feature.as_str())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/promote.rs"]
mod tests;
