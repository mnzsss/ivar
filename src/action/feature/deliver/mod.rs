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
//! side-effect-free by construction; even the "unpushed commits" blocker is
//! computed locally (the branch's commits beyond its base, with no upstream
//! configured), never by reaching for the remote.
//!
//! `ivar feature deliver <name> --fingerprint <fp>` recomputes the same
//! preview and refuses with [`Failure::blocked`] when the fingerprint differs
//! — the state the human approved has drifted, so nothing is pushed. Only a
//! matching fingerprint opens the push, which then runs **best-effort per
//! repo**: a failed push is a [`Warning`], never an abort of the batch.
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

use crate::domain::feature::{DeliveryAction, DeliveryPreview, Feature, GateState};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self};
use crate::store::layout::Layout;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

mod preview;
mod pull_requests;
mod repos;

use preview::{fingerprint_for, plan_gate_state, plan_not_approved, preview_required};
use pull_requests::{create_pr, existing_pr_url, link_sibling_prs};
use repos::{build_repos, order_by_dependencies, push_repo};

/// What `ivar feature deliver` needs.
#[derive(Debug, Clone)]
pub struct DeliverInput {
    /// The feature to deliver.
    pub feature: String,
    /// Preview only: compute and print the summary, push nothing.
    pub preview: bool,
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
}

impl WriteHuman for DeliverOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.pushes.is_empty() {
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
            writeln!(w, "  plan gate:   {}", self.preview.plan_gate)?;
            writeln!(w, "  fingerprint: {}", self.preview.fingerprint)
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
    }
}

pub fn deliver(ctx: &Ctx, input: DeliverInput) -> Outcome<DeliverOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let feature_name = FeatureName::new(input.feature)?;
    let feature = read_feature(&layout, &feature_name)?;

    let plan_gate = plan_gate_state(&layout, &feature_name)?;

    let mut repos = build_repos(&git, &layout, &manifest, &feature)?;
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));
    order_by_dependencies(&mut repos);
    let fingerprint = fingerprint_for(&feature_name, plan_gate, &repos)?;

    let mut preview = DeliveryPreview {
        feature: feature_name.clone(),
        plan_gate,
        repos,
        fingerprint,
    };

    if input.preview {
        return Ok(Report::new(DeliverOutcome {
            root: layout.root().to_path_buf(),
            preview,
            pushes: Vec::new(),
        }));
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

    let mut pushes = Vec::new();
    let mut warnings = Vec::new();

    // -- Phase 1: push every repo best-effort ---------------------------------
    for repo in &preview.repos {
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
        if repo.action == DeliveryAction::PushOnly {
            continue;
        }

        let bare = layout.repo_bare(&repo.repo);
        // A branch that already has a PR was updated by the push above — `gh pr
        // create` would only refuse it as a duplicate. Its URL is still part of
        // the report, and `gh pr list` is the only place it comes from.
        let result = match repo.action {
            DeliveryAction::UpdatePr => existing_pr_url(&bare, repo.local_branch.as_str())
                .map_or_else(
                    || create_pr(&bare, &repo.local_branch, &repo.base_branch, &feature_name),
                    Ok,
                ),
            _ => create_pr(&bare, &repo.local_branch, &repo.base_branch, &feature_name),
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
        },
        warnings,
    ))
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
