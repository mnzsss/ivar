//! `ivar feature deliver` — preview, then push + PR, a feature's promoted repos.
//!
//! The valhalla definition this ports: **Delivery Preview** — "a
//! side-effect-free summary of the pending delivery actions generated before
//! any push occurs. For each promoted Repo it includes: local branch, remote,
//! push refspec, existing/new PR action, base branch, dependency ordering, and
//! blockers. Apply is gated on the preview fingerprint and rejected if state
//! drifted."
//!
//! # Preview, then apply, and nothing between
//!
//! `ivar feature deliver <name> --preview` reads the world and prints a
//! [`DeliveryPreview`] — one entry per promoted repo plus a **fingerprint**:
//! SHA-256 of the serialized preview summary. It pushes nothing, so it is
//! side-effect-free by construction — which is a claim about writes, not about
//! the network. The preview reads the remote for PR actions, for the "unpushed
//! commits" blocker, and (in land mode) for remote default tip evidence.
//! Asking git's local config instead would be cheaper and wrong: `deliver`
//! pushes to a URL, and git records no upstream for such a push, so a branch
//! `deliver` itself pushed would read as unpushed forever.
//!
//! `ivar feature deliver <name> --fingerprint <fp>` recomputes the same
//! preview and refuses with [`Failure::blocked`] when the fingerprint differs
//! — the state the human approved has drifted, so nothing is pushed. Only a
//! matching fingerprint opens the push, which then runs **best-effort per
//! repo**: a failed push is a [`Warning`], never an abort of the batch.
//!
//! # What is deliberately not here
//!
//! - **Merge sequencing across repos.** Repos in a hall may depend on each other,
//!   but delivery pushes/lands all promoted repos independently. It does not
//!   topologically order remote pushes or coordinate CI pipelines across repos.
//! - **Local landing without fast-forward.** `--land` fast-forwards local default
//!   branches to feature tips. It refuses diverged defaults and points at
//!   `ivar feature rebase`. Land never resolves merge conflicts.
//!
//! # Pull requests
//!
//! After all repos are pushed, `deliver` creates pull requests for repos whose
//! action is [`DeliveryAction::NewPr`]. A PR that already exists (detected via
//! `gh pr list --head`) is not recreated — the push above already updated it,
//! and `gh pr create` refuses a duplicate — so `deliver` only reads its URL
//! back, and reports it exactly as it reports a freshly created one.
//!
//! Sibling PRs (one per promoted repo) are linked together with a comment on
//! each PR noting the others — always with "part of" language, never "depends
//! on". This linking happens in a second pass, after all PR URLs are known.
//!
//! One repo's failure to create a PR becomes a [`Warning`], not a batch abort.

use std::collections::BTreeMap;
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{
    DeliveryAction, DeliveryMode, DeliveryPreview, DeliveryTreeBlocker, Feature,
    FeatureIntegrationState, GateState, VerificationResult,
};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, Git};
use crate::store::layout::Layout;

use super::super::{discover_hall, read_manifest};
use super::relations;
use super::verification;
use crate::action::Ctx;

mod land;
mod preview;
mod repos;

use super::pull_requests::{create_pull_request, existing_pr_url, link_sibling_prs};
use preview::{fingerprint_for, plan_gate_state, plan_not_approved, preview_required};
use repos::{build_repos, order_by_dependencies, push_repo};

/// What `ivar feature deliver` needs.
#[derive(Debug, Clone)]
pub struct DeliverInput {
    /// The feature to deliver.
    pub feature: String,
    /// Preview only: compute and print the summary, push nothing.
    pub preview: bool,
    /// Land feature branches into default branches locally (fast-forward only).
    pub land: bool,
    /// The fingerprint from the preview the human approved. Required for
    /// apply; the push is refused when the current state does not fingerprint
    /// to it.
    pub fingerprint: Option<String>,
}

/// One repo's push, in apply mode.
#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    /// The repo that was pushed (or not).
    pub repo: RepoName,
    /// Whether the push landed.
    pub ok: bool,
    /// Why it failed, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One repo's land result, in apply mode.
#[derive(Debug, Clone, Serialize)]
pub struct LandResult {
    /// The repo that landed.
    pub repo: RepoName,
    /// Whether the local default branch fast-forwarded to the feature tip.
    pub merged: bool,
    /// Whether pushing the default branch to remote succeeded.
    pub pushed: bool,
    /// Detail when a step was skipped or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One root repo's ordered checks, run in its worktree before the push.
#[derive(Debug, Clone, Serialize)]
pub struct RepoCheckResult {
    /// The repo whose checks ran.
    pub repo: RepoName,
    /// Whether every check passed.
    pub passed: bool,
    /// The ordered results, in execution order.
    pub results: Vec<VerificationResult>,
}

/// What `ivar feature deliver` produced.
///
/// One value for both modes, so `--json` and the human surface cannot drift:
/// preview mode returns the preview with an empty `pushes`; apply mode returns
/// the same preview (the state that was actually pushed) plus the per-repo
/// results.
#[derive(Debug, Clone, Serialize)]
pub struct DeliverOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The approved (preview) or delivered (apply) state.
    pub preview: DeliveryPreview,
    /// Per-repo push results; present only in apply mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pushes: Vec<PushResult>,
    /// Per-repo land results; present only in apply mode for land deliveries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub land: Vec<LandResult>,
    /// Per-repo ordered check results; present only in apply mode, so the
    /// actual execution is machine-visible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<RepoCheckResult>,
}

impl WriteHuman for DeliverOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.pushes.is_empty() && self.land.is_empty() {
            match self.preview.mode {
                DeliveryMode::Push => {
                    writeln!(
                        w,
                        "Delivery preview for `{}` in {}:",
                        self.preview.feature, self.root
                    )?;
                    if self.preview.repos.is_empty() {
                        writeln!(w, "  no repos promoted")?;
                    }
                    for repo in &self.preview.repos {
                        writeln!(w, "  {}:", repo.repo)?;
                        writeln!(w, "    branch:  {}", repo.local_branch)?;
                        writeln!(w, "    remote:  {}", repo.remote)?;
                        writeln!(w, "    refspec: {}", repo.push_refspec)?;
                        writeln!(w, "    base:    {}", repo.base_branch)?;
                        writeln!(w, "    action:  {}", action_word(repo.action))?;
                        if repo.blockers.is_empty() {
                            writeln!(w, "    blockers: none")?;
                        } else {
                            for blocker in &repo.blockers {
                                writeln!(w, "    blocker: {blocker}")?;
                            }
                        }
                    }
                }
                DeliveryMode::Land => {
                    writeln!(
                        w,
                        "Delivery preview (land on default) for `{}` in {}:",
                        self.preview.feature, self.root
                    )?;
                    if self.preview.repos.is_empty() {
                        writeln!(w, "  no repos promoted")?;
                    }
                    for repo in &self.preview.repos {
                        let target = repo.default_branch.as_ref().map_or("-", |b| b.as_str());
                        let ff_verdict = match repo.ff_possible {
                            Some(true) => "fast-forward",
                            Some(false) => "diverged",
                            None => "unknown",
                        };
                        writeln!(
                            w,
                            "  {}  {} -> {}  {}",
                            repo.repo, repo.local_branch, target, ff_verdict
                        )?;
                        for blocker in &repo.blockers {
                            writeln!(w, "    blocker: {blocker}")?;
                        }
                    }
                }
            }
            writeln!(w, "  plan gate:   {}", self.preview.plan_gate)?;
            writeln!(w, "  fingerprint: {}", self.preview.fingerprint)
        } else if !self.land.is_empty() {
            writeln!(
                w,
                "Landed `{}` in {} (fingerprint {}):",
                self.preview.feature, self.root, self.preview.fingerprint
            )?;
            for res in &self.land {
                if res.merged && res.pushed {
                    writeln!(w, "  {}: merged and pushed", res.repo)?;
                } else if res.merged {
                    if let Some(detail) = &res.detail {
                        writeln!(w, "  {}: merged, not pushed — {detail}", res.repo)?;
                    } else {
                        writeln!(w, "  {}: merged, not pushed", res.repo)?;
                    }
                } else if let Some(detail) = &res.detail {
                    writeln!(w, "  {}: not merged — {detail}", res.repo)?;
                } else {
                    writeln!(w, "  {}: not merged", res.repo)?;
                }
            }
            Ok(())
        } else {
            writeln!(
                w,
                "Delivered `{}` in {} (fingerprint {}):",
                self.preview.feature, self.root, self.preview.fingerprint
            )?;
            for push in &self.pushes {
                if push.ok {
                    writeln!(w, "  {}: pushed", push.repo)?;
                } else if let Some(detail) = &push.detail {
                    writeln!(w, "  {}: not pushed — {detail}", push.repo)?;
                } else {
                    writeln!(w, "  {}: not pushed", push.repo)?;
                }
            }
            Ok(())
        }
    }
}

fn action_word(action: DeliveryAction) -> &'static str {
    match action {
        DeliveryAction::NewPr => "new pr",
        DeliveryAction::UpdatePr => "update pr",
        DeliveryAction::PushOnly => "push only",
        DeliveryAction::LandOnDefault => "land on default",
    }
}

pub fn deliver(ctx: &Ctx, input: DeliverInput) -> Outcome<DeliverOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let feature_name = FeatureName::new(input.feature)?;
    let feature = read_feature(&layout, &feature_name)?;

    // Only a root delivers. A child's work belongs to its parent — the exact
    // fix names the verb that moves it there.
    if let Some(parent_name) = &feature.parent {
        return Err(Failure::blocked(
            "deliver.child_requires_integration",
            format!("feature `{feature_name}` is a child; it delivers into its parent, not to the remote"),
        )
        .expected("a root feature (one with no parent) to deliver")
        .actual(format!("`{feature_name}` is a subfeature of `{parent_name}`"))
        .fix(
            FixAction::safe(
                "deliver.integrate_child",
                format!("Integrate the child into its parent: `ivar feature integrate {feature_name}`."),
            )
            .command(format!("ivar feature integrate {feature_name}")),
        ));
    }

    // The tree is read as a whole: a corrupt lineage refuses loudly, and the
    // root's blocking descendants are derived from it.
    relations::read_all(&layout)?;
    let blockers = relations::blocking_descendants(&git, &layout, &manifest, &feature)?;
    let tree_blockers: Vec<DeliveryTreeBlocker> = blockers
        .iter()
        .map(|entry| DeliveryTreeBlocker {
            feature: entry.feature.clone(),
            depth: entry.depth,
            state: entry.state,
            reason: blocker_reason(entry.state),
        })
        .collect();

    let plan_gate = plan_gate_state(&layout, &feature_name)?;

    let mode = if input.land {
        DeliveryMode::Land
    } else {
        DeliveryMode::Push
    };

    let mut repos = build_repos(&git, &layout, &manifest, &feature, mode)?;
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));
    order_by_dependencies(&mut repos);
    let fingerprint = fingerprint_for(&feature_name, mode, plan_gate, &tree_blockers, &repos)?;

    let mut preview = DeliveryPreview {
        feature: feature_name.clone(),
        mode,
        plan_gate,
        repos,
        tree_blockers,
        fingerprint,
    };

    if input.land {
        land::preflight(&git, &layout, &manifest, &feature, &preview)?;
    }

    if input.preview {
        return Ok(Report::new(DeliverOutcome {
            root: layout.root().to_path_buf(),
            preview,
            pushes: Vec::new(),
            land: Vec::new(),
            checks: Vec::new(),
        }));
    }

    // A root with blocking descendants cannot deliver: the tree must be
    // healthy below it first, leaves first. Refused before any push or PR.
    if !preview.tree_blockers.is_empty() {
        let names = preview
            .tree_blockers
            .iter()
            .map(|blocker| format!("{} ({})", blocker.feature, blocker.state))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Failure::blocked(
            "deliver.descendants_block",
            format!(
                "cannot deliver `{feature_name}`: {} descendant(s) still block",
                preview.tree_blockers.len()
            ),
        )
        .expected("every descendant to be integrated, verified, or abandoned")
        .actual(names)
        .fix(FixAction::safe(
            "deliver.integrate_leaves_first",
            "Integrate the blocking descendants first, leaves first.",
        )));
    }

    // The approval gate comes before the drift gate. Both refuse, but only one
    // of them tells a human who never planned the feature what to do next, and
    // a `deliver` with no fingerprint at all is far more often that human than
    // one whose preview went stale.
    if plan_gate != GateState::Approved {
        return Err(plan_not_approved(&feature_name, plan_gate));
    }

    let expected = input
        .fingerprint
        .ok_or_else(|| preview_required(&feature_name))?;
    if expected != preview.fingerprint {
        return Err(Failure::blocked(
            "deliver.fingerprint_mismatch",
            format!(
                "the state of feature `{feature_name}` has drifted since the preview was approved"
            ),
        )
        .expected(format!("the preview fingerprint `{expected}`"))
        .actual(format!(
            "the current state fingerprints as `{}`",
            preview.fingerprint
        ))
        .fix(FixAction::safe(
            "deliver.re_preview",
            format!(
                "Run `ivar feature deliver {feature_name} --preview` again, then apply with the new fingerprint."
            ),
        )));
    }

    let mut warnings = Vec::new();

    if input.land {
        let plans = land::preflight(&git, &layout, &manifest, &feature, &preview)?;
        let land_results = land::execute(&git, &layout, &plans, &mut warnings)?;
        return Ok(Report::with_warnings(
            DeliverOutcome {
                root: layout.root().to_path_buf(),
                preview,
                pushes: Vec::new(),
                land: land_results,
                checks: Vec::new(),
            },
            warnings,
        ));
    }

    let mut pushes = Vec::new();
    let mut checks = Vec::new();
    let mut warnings = Vec::new();

    // -- Phase 1: run each root repo's ordered checks, then push best-effort --
    // A repo whose checks fail is not pushed — its work did not verify — while
    // the rest of the batch continues. The results are machine-visible on the
    // outcome.
    for repo in &preview.repos {
        let worktree = layout.repo_worktree(&repo.repo, &feature.branch);
        let repo_checks = verification::checks_for(&manifest, &repo.repo);
        let run = verification::run(&repo_checks, &worktree)?;
        let passed = run.results.iter().all(|result| result.success);
        checks.push(RepoCheckResult {
            repo: repo.repo.clone(),
            passed,
            results: run.results,
        });
        if !passed {
            warnings.push(Warning::new(
                "deliver.checks_failed",
                repo.repo.as_str(),
                "root checks failed; this repo was not pushed",
            ));
            pushes.push(PushResult {
                repo: repo.repo.clone(),
                ok: false,
                detail: Some("root checks failed".to_owned()),
            });
            continue;
        }

        let bare = layout.repo_bare(&repo.repo);
        match push_repo(&git, &bare, repo) {
            Ok(()) => pushes.push(PushResult {
                repo: repo.repo.clone(),
                ok: true,
                detail: None,
            }),
            Err(failure) => {
                let detail = failure.what.clone();
                warnings.push(Warning::new(
                    "deliver.push_failed",
                    repo.repo.as_str(),
                    detail.clone(),
                ));
                pushes.push(PushResult {
                    repo: repo.repo.clone(),
                    ok: false,
                    detail: Some(detail),
                });
            }
        }
    }

    // -- Phase 2: create PRs for repos that need them -------------------------
    let mut pr_url_map: BTreeMap<RepoName, String> = BTreeMap::new();
    let mut pr_results: Vec<(RepoName, Result<String, Failure>)> = Vec::new();
    for repo in &preview.repos {
        if matches!(
            repo.action,
            DeliveryAction::PushOnly | DeliveryAction::LandOnDefault
        ) {
            continue;
        }

        let bare = layout.repo_bare(&repo.repo);

        // The base must still support delivering onto it before a PR is
        // opened or updated against it: a base gone from the remote, or one
        // this branch has drifted off of, would make the PR's diff wrong.
        // Refused per repo — the rest of the batch is unaffected — and never
        // added to `blockers`, which is informational only.
        let default_branch = manifest
            .repos()
            .iter()
            .find(|manifest_repo| manifest_repo.name() == &repo.repo)
            .map(|manifest_repo| manifest_repo.default_branch().clone());
        if let Some(default_branch) = default_branch {
            let remote_tip = git
                .remote_branch_tip(&bare, &repo.remote, repo.base_branch.as_str())
                .map_err(|_| ());
            let secondary = match &remote_tip {
                // Ignored by `check_base` when the remote did not answer —
                // no point spending a local read on it.
                Err(()) => Ok(false),
                Ok(None) => git
                    .is_ancestor(&bare, repo.base_branch.as_str(), default_branch.as_str())
                    .map_err(|_| ()),
                // Against the remote's own tip, not the local branch name:
                // `ivar sync` never re-fetches a non-default branch, so a
                // local `base_branch` ref can be stale — still an ancestor
                // of the local branch even though the remote has moved on.
                // A tip this bare clone never fetched is itself the answer
                // (`is_ancestor` refuses, `check_base` reads that as moved).
                Ok(Some(tip)) => git
                    .is_ancestor(&bare, tip, repo.local_branch.as_str())
                    .map_err(|_| ()),
            };
            if let Some(failure) = repo.check_base(remote_tip, secondary, &default_branch) {
                warnings.push(Warning::new(
                    failure.code,
                    repo.repo.as_str(),
                    failure.what.clone(),
                ));
                continue;
            }
        }

        // A branch that already has a PR was updated by the push above — `gh pr
        // create` would only refuse it as a duplicate. Its URL is still part of
        // the report, and `gh pr list` is the only place it comes from.
        let result = match repo.action {
            DeliveryAction::UpdatePr => existing_pr_url(&bare, repo.local_branch.as_str())
                .map_or_else(
                    || {
                        create_pull_request(
                            &bare,
                            &repo.local_branch,
                            &repo.base_branch,
                            &feature_name,
                        )
                        .map(|pr| pr.url)
                    },
                    Ok,
                ),
            DeliveryAction::NewPr => {
                create_pull_request(&bare, &repo.local_branch, &repo.base_branch, &feature_name)
                    .map(|pr| pr.url)
            }
            DeliveryAction::PushOnly | DeliveryAction::LandOnDefault => unreachable!(),
        };
        pr_results.push((repo.repo.clone(), result));
    }

    for (repo_name, result) in pr_results {
        match result {
            Ok(url) => {
                pr_url_map.insert(repo_name.clone(), url);
            }
            Err(failure) => {
                let detail = failure.what.clone();
                warnings.push(Warning::new(
                    "deliver.pr_create_failed",
                    repo_name.as_str(),
                    detail.clone(),
                ));
            }
        }
    }

    // Record PR URLs on the preview repos so they round-trip through JSON.
    for repo in &mut preview.repos {
        if let Some(url) = pr_url_map.get(&repo.repo) {
            repo.pr_url = Some(url.clone());
        }
    }

    // -- Phase 3: link sibling PRs (second pass — URLs only known after phase 2)
    let pr_urls: Vec<String> = pr_url_map.into_values().collect();
    if !pr_urls.is_empty() {
        link_sibling_prs(&pr_urls);
    }

    Ok(Report::with_warnings(
        DeliverOutcome {
            root: layout.root().to_path_buf(),
            preview,
            pushes,
            land: Vec::new(),
            checks,
        },
        warnings,
    ))
}

/// One sentence for why a descendant's state blocks delivery.
fn blocker_reason(state: FeatureIntegrationState) -> String {
    match state {
        FeatureIntegrationState::Active => "work is still in progress".to_owned(),
        FeatureIntegrationState::Failed => "carries failed integration evidence".to_owned(),
        FeatureIntegrationState::Stale => "its receipt no longer matches live state".to_owned(),
        // Unreachable: blocking descendants are only ever these three.
        other => format!("its state is `{other}`"),
    }
}

/// Read the feature, or a `Blocked` failure naming the way out.
fn read_feature(layout: &Layout, name: &FeatureName) -> Result<Feature, Failure> {
    Feature::read(layout, name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {name}`."),
        ))
    })
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/feature/deliver.rs"]
mod tests;
