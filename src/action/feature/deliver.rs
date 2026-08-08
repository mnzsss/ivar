//! `ivar feature deliver` — preview, then push, a feature's promoted repos.
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
//! # What is deliberately not here
//!
//! No pull requests. `ivar` is serverless — there is no PR surface — so every
//! repo's action is [`DeliveryAction::PushOnly`] and the push is a plain
//! `git push` of the feature branch to the repo's manifest URL. The
//! `dependencies` ordering machinery exists ([`order_by_dependencies`]) but
//! the ivar feature model declares no cross-repo dependencies, so every list
//! is empty and the order is name order.

use std::collections::BTreeMap;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::feature::{DeliveryAction, DeliveryPreview, DeliveryRepo, Feature};
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::git::{self, TargetState};
use crate::infra::{hash, json};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

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

/// Deliver `input.feature`: preview, or fingerprint-gated push.
///
/// Preview never touches the remote. Apply recomputes the preview from the
/// current state and refuses when its fingerprint differs from
/// `input.fingerprint` (state drifted since the human approved), then pushes
/// each promoted repo best-effort — a failed push is a warning, never a batch
/// abort.
pub fn deliver(ctx: &Ctx, input: DeliverInput) -> Outcome<DeliverOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let feature_name = FeatureName::new(input.feature)?;
    let feature = read_feature(&layout, &feature_name)?;

    let mut repos = build_repos(&git, &layout, &manifest, &feature)?;
    repos.sort_by(|a, b| a.repo.cmp(&b.repo));
    order_by_dependencies(&mut repos);
    let fingerprint = fingerprint_for(&feature_name, &repos)?;

    let preview = DeliveryPreview {
        feature: feature_name.clone(),
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

/// The fingerprint that gates apply: SHA-256 of the canonical serialization of
/// the preview summary (with its own fingerprint field empty, so the digest is
/// never part of itself).
///
/// Every fact the human approved is in the digest — branch, remote, refspec,
/// base, blockers — so any drift between preview and apply changes it.
fn fingerprint_for(feature: &FeatureName, repos: &[DeliveryRepo]) -> Result<String, Failure> {
    let preview = DeliveryPreview {
        feature: feature.clone(),
        repos: repos.to_vec(),
        fingerprint: String::new(),
    };
    let rendered = json::to_canonical_string(&preview)?;
    Ok(hash::text(&rendered))
}

/// Compute one [`DeliveryRepo`] per promoted repo: the facts the preview
/// reports, plus the blockers that stand between the current state and a clean
/// push.
///
/// Local reads only — no network — so a preview stays side-effect-free even on
/// an unreachable remote. A repo whose bare clone is missing is a `Blocked`
/// failure (sync first, exactly as `promote` insists); a repo promoted but no
/// longer in the manifest is a `Blocked` failure naming the fix.
fn build_repos(
    git: &impl git::Git,
    layout: &Layout,
    manifest: &Manifest,
    feature: &Feature,
) -> Result<Vec<DeliveryRepo>, Failure> {
    let mut repos = Vec::new();

    for repo_name in feature.promotions.keys() {
        let declared = manifest
            .repos()
            .iter()
            .find(|repo| repo.name() == repo_name)
            .ok_or_else(|| {
                Failure::blocked(
                    "deliver.repo_not_in_manifest",
                    format!(
                        "`{repo_name}` is promoted into `{}` but is no longer in ivar.json",
                        feature.name
                    ),
                )
                .expected("every promoted repo to still be declared in ivar.json")
                .actual(format!("`{repo_name}` does not appear in `repos`"))
                .fix(FixAction::safe(
                    "deliver.restore_manifest",
                    "Restore the repo to ivar.json (or demote it from the feature) before delivering.",
                ))
            })?;

        let bare = layout.repo_bare(repo_name);
        let worktree = layout.repo_worktree(repo_name, &feature.branch);

        match git.target_state(&bare)? {
            TargetState::Repository => {}
            TargetState::Occupied | TargetState::Absent => {
                return Err(Failure::blocked(
                    "repo.bare_not_cloned",
                    format!("`{repo_name}` has no bare clone yet"),
                )
                .expected("the repo to have been cloned by `ivar sync`")
                .actual(format!("`{bare}` does not exist"))
                .fix(FixAction::safe(
                    "repo.sync_first",
                    "Run `ivar sync` to clone the repo, then deliver again.",
                )));
            }
        }

        let mut blockers = Vec::new();

        let branch_exists = git
            .list_branches(&bare)?
            .iter()
            .any(|branch| branch == feature.branch.as_str());
        if !branch_exists {
            blockers.push("branch not materialised; promote the repo first".to_owned());
        }

        // The "unpushed commits" signal, computed locally: commits beyond the
        // feature's base with no upstream configured. ivar never configures
        // upstreams, so every branch with work carries this blocker — which is
        // the truth about a branch nothing has pushed yet.
        if branch_exists {
            let ahead = git.commits_ahead(
                &bare,
                declared.default_branch().as_str(),
                feature.branch.as_str(),
            )?;
            let upstream = git.has_upstream(&bare, feature.branch.as_str())?;
            if ahead > 0 && !upstream {
                blockers.push(format!("{ahead} commit(s) not pushed (no upstream branch)"));
            }
        }

        let worktree_present = matches!(
            git.target_state(&worktree).unwrap_or(TargetState::Absent),
            TargetState::Repository
        );
        if worktree_present && git.worktree_dirty(&worktree)? {
            blockers.push("worktree has uncommitted changes".to_owned());
        }

        repos.push(DeliveryRepo {
            repo: repo_name.clone(),
            local_branch: feature.branch.clone(),
            remote: declared.url().to_owned(),
            push_refspec: format!("{}:refs/heads/{}", feature.branch, feature.branch),
            action: DeliveryAction::PushOnly,
            base_branch: declared.default_branch().clone(),
            dependencies: Vec::new(),
            blockers,
        });
    }

    Ok(repos)
}

/// Push one repo's feature branch to its remote.
///
/// The branch must exist in the bare clone — a promotion recorded but never
/// materialised has nothing to push, which is a per-repo `Blocked` the caller
/// turns into a warning. `remote` is the URL from the preview, so the push
/// lands exactly where the approved summary said it would.
fn push_repo(git: &impl git::Git, bare: &Utf8Path, repo: &DeliveryRepo) -> Result<(), Failure> {
    let branch_exists = git
        .list_branches(bare)?
        .iter()
        .any(|branch| branch == repo.local_branch.as_str());
    if !branch_exists {
        return Err(Failure::blocked(
            "deliver.branch_not_materialised",
            format!(
                "`{}` has no `{}` branch to push",
                repo.repo, repo.local_branch
            ),
        )
        .expected("the feature branch to exist in the repo's bare clone")
        .actual("the branch is not there")
        .fix(FixAction::safe(
            "deliver.promote_first",
            format!(
                "Promote `{}` into the feature first, then deliver again.",
                repo.repo
            ),
        )));
    }

    git.push(
        bare,
        &repo.remote,
        repo.local_branch.as_str(),
        &format!("refs/heads/{}", repo.local_branch),
    )?;
    Ok(())
}

/// The `Blocked` failure for applying without a preview fingerprint.
fn preview_required(feature: &FeatureName) -> Failure {
    Failure::blocked(
        "deliver.preview_required",
        format!("delivering `{feature}` needs a preview fingerprint"),
    )
    .expected("the fingerprint printed by `ivar feature deliver --preview`")
    .actual("no `--fingerprint` was given")
    .fix(FixAction::safe(
        "deliver.preview_first",
        format!(
            "Run `ivar feature deliver {feature} --preview` and pass its fingerprint with `--fingerprint`."
        ),
    ))
}

/// Order `repos` so every repo's dependencies come before it — a stable
/// topological sort that leaves the existing order intact between unrelated
/// repos.
///
/// `ivar`'s feature model declares no cross-repo dependencies (every
/// [`DeliveryRepo::dependencies`] is empty), so this is a no-op today; it
/// exists so the push order — and therefore the fingerprint that gates apply —
/// is well-defined the day dependencies are not empty. A dependency cycle
/// (impossible to declare today) falls back to the remaining order rather than
/// loop.
fn order_by_dependencies(repos: &mut Vec<DeliveryRepo>) {
    let mut remaining: BTreeMap<RepoName, usize> = repos
        .iter()
        .map(|repo| (repo.repo.clone(), repo.dependencies.len()))
        .collect();
    let mut ordered = Vec::with_capacity(repos.len());

    while !repos.is_empty() {
        let index = repos.iter().position(|repo| {
            repo.dependencies
                .iter()
                .all(|dep| !remaining.contains_key(dep))
        });
        let Some(index) = index else {
            ordered.append(repos);
            break;
        };
        let repo = repos.remove(index);
        remaining.remove(&repo.repo);
        ordered.push(repo);
    }

    *repos = ordered;
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
    use crate::action::feature::create::CreateInput;
    use crate::action::feature::create::create as create_action;
    use crate::action::feature::promote::{self, PromoteInput};
    use crate::action::hall::{self, InitInput};
    use crate::domain::feature::DeliveryAction;
    use crate::domain::name::{BranchName, HallName};
    use crate::domain::provider::Provider;
    use crate::error::Status;
    use crate::store::layout::Layout;
    use crate::store::manifest::{Manifest, Providers, Repo};
    use crate::test_support::{git, hall_root, seeded_repo};

    /// A hall with `repos` declared, a `checkout` feature, every repo promoted,
    /// and one commit on each feature branch so there is something to deliver.
    fn hall_with_promoted(repos: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
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
        let declared: Vec<Repo> = repos
            .iter()
            .map(|name| {
                let origin = seeded_repo(&origins.join(name), "main");
                Repo::new(
                    RepoName::new(*name).unwrap(),
                    origin.as_str(),
                    BranchName::new("main").unwrap(),
                )
            })
            .collect();
        let layout = Layout::at(root.clone());
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            declared,
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
        crate::action::sync::sync(&ctx, Default::default()).unwrap();

        let branch = BranchName::new("checkout").unwrap();
        for name in repos {
            promote::promote(
                &ctx,
                PromoteInput {
                    feature: "checkout".to_owned(),
                    repo: (*name).to_owned(),
                },
            )
            .unwrap();
            let worktree = layout.repo_worktree(&RepoName::new(*name).unwrap(), &branch);
            std::fs::write(worktree.join("work.md"), "work\n").unwrap();
            git(&worktree, &["add", "work.md"]);
            git(&worktree, &["commit", "-m", "work"]);
        }

        (guard, root)
    }

    fn preview_input(feature: &str) -> DeliverInput {
        DeliverInput {
            feature: feature.to_owned(),
            preview: true,
            fingerprint: None,
        }
    }

    fn apply_input(feature: &str, fingerprint: &str) -> DeliverInput {
        DeliverInput {
            feature: feature.to_owned(),
            preview: false,
            fingerprint: Some(fingerprint.to_owned()),
        }
    }

    /// The remote's view of one branch: the `ls-remote` line, or `None` when
    /// the branch is not there.
    fn remote_ref(origin: &str, branch: &str) -> Option<String> {
        let output = std::process::Command::new("git")
            .args(["ls-remote", origin, &format!("refs/heads/{branch}")])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "ls-remote failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            None
        } else {
            Some(stdout.trim().to_owned())
        }
    }

    fn origin_of(root: &Utf8Path, repo: &str) -> String {
        let layout = Layout::at(root.to_path_buf());
        Manifest::read(&layout)
            .unwrap()
            .unwrap()
            .repos()
            .iter()
            .find(|declared| declared.name().as_str() == repo)
            .unwrap()
            .url()
            .to_owned()
    }

    // -- preview --------------------------------------------------------------

    #[test]
    fn preview_lists_every_promoted_repo_with_its_delivery_facts() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root.clone());

        let report = deliver(&ctx, preview_input("checkout")).unwrap();

        assert!(report.is_clean());
        assert!(report.value.pushes.is_empty(), "preview must not push");
        assert_eq!(report.value.preview.repos.len(), 1);
        let repo = &report.value.preview.repos[0];
        assert_eq!(repo.repo.as_str(), "api");
        assert_eq!(repo.local_branch.as_str(), "checkout");
        assert!(repo.remote.contains("origins/api"), "was: {}", repo.remote);
        assert_eq!(repo.push_refspec, "checkout:refs/heads/checkout");
        assert_eq!(repo.action, DeliveryAction::PushOnly);
        assert_eq!(repo.base_branch.as_str(), "main");
        assert!(repo.dependencies.is_empty());
        // One commit beyond main, no upstream: the unpushed blocker.
        assert!(
            repo.blockers
                .iter()
                .any(|blocker| blocker.contains("1 commit(s) not pushed")),
            "was: {:?}",
            repo.blockers
        );
        // Preview is side-effect-free: the remote has no branch yet.
        assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
    }

    #[test]
    fn the_preview_has_a_stable_content_fingerprint() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root.clone());

        let first = deliver(&ctx, preview_input("checkout")).unwrap();
        let second = deliver(&ctx, preview_input("checkout")).unwrap();

        let fingerprint = &first.value.preview.fingerprint;
        assert_eq!(fingerprint.len(), 64, "a sha-256 hex digest");
        assert_eq!(fingerprint, &second.value.preview.fingerprint);
    }

    #[test]
    fn a_feature_with_no_promoted_repos_previews_empty() {
        let (_guard, root) = hall_with_promoted(&[]);
        let ctx = Ctx::new(root.clone());

        let report = deliver(&ctx, preview_input("checkout")).unwrap();

        assert!(report.value.preview.repos.is_empty());
        assert_eq!(report.value.preview.fingerprint.len(), 64);
    }

    #[test]
    fn a_dirty_worktree_is_listed_as_a_blocker() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root.clone());
        let worktree = Layout::at(root.clone()).repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("checkout").unwrap(),
        );
        std::fs::write(worktree.join("notes.md"), "mine\n").unwrap();

        let report = deliver(&ctx, preview_input("checkout")).unwrap();

        let repo = &report.value.preview.repos[0];
        assert!(
            repo.blockers
                .iter()
                .any(|blocker| blocker.contains("uncommitted changes")),
            "was: {:?}",
            repo.blockers
        );
    }

    #[test]
    fn delivering_a_missing_feature_is_blocked() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root);

        let failure = deliver(&ctx, preview_input("ghost")).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "feature.not_found");
    }

    // -- apply: gating --------------------------------------------------------

    #[test]
    fn apply_requires_a_preview_fingerprint() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root.clone());

        let failure = deliver(
            &ctx,
            DeliverInput {
                feature: "checkout".to_owned(),
                preview: false,
                fingerprint: None,
            },
        )
        .unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "deliver.preview_required");
    }

    #[test]
    fn apply_is_rejected_when_the_state_has_drifted_since_the_preview() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root.clone());
        let approved = deliver(&ctx, preview_input("checkout")).unwrap();
        let fingerprint = approved.value.preview.fingerprint.clone();

        // Drift: one more commit lands on the feature branch.
        let worktree = Layout::at(root.clone()).repo_worktree(
            &RepoName::new("api").unwrap(),
            &BranchName::new("checkout").unwrap(),
        );
        std::fs::write(worktree.join("more.md"), "more\n").unwrap();
        git(&worktree, &["add", "more.md"]);
        git(&worktree, &["commit", "-m", "more"]);

        let failure = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap_err();

        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, "deliver.fingerprint_mismatch");
        assert!(
            failure
                .fix_actions
                .iter()
                .any(|fix| fix.code == "deliver.re_preview"),
            "the fix must re-run the preview: {:?}",
            failure.fix_actions
        );
        // Nothing was pushed.
        assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_none());
    }

    // -- apply: pushing -------------------------------------------------------

    #[test]
    fn deliver_pushes_the_feature_branch_to_the_remote() {
        let (_guard, root) = hall_with_promoted(&["api"]);
        let ctx = Ctx::new(root.clone());
        let approved = deliver(&ctx, preview_input("checkout")).unwrap();
        let fingerprint = approved.value.preview.fingerprint.clone();

        let report = deliver(&ctx, apply_input("checkout", &fingerprint)).unwrap();

        assert!(report.is_clean());
        assert_eq!(report.value.pushes.len(), 1);
        assert!(report.value.pushes[0].ok);
        assert_eq!(report.value.pushes[0].repo.as_str(), "api");
        // The remote now holds the branch, at the tip that was previewed.
        assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_some());
    }

    #[test]
    fn a_failed_push_is_a_warning_and_does_not_block_the_others() {
        let (_guard, root) = hall_with_promoted(&["api", "web"]);
        // Break web's remote before previewing, so the approved state says the
        // bogus URL — the fingerprint then matches when apply runs.
        let layout = Layout::at(root.clone());
        let manifest = Manifest::read(&layout).unwrap().unwrap();
        let repos: Vec<Repo> = manifest
            .repos()
            .iter()
            .map(|repo| {
                if repo.name().as_str() == "web" {
                    Repo::new(
                        RepoName::new("web").unwrap(),
                        root.join("no-such-origin").as_str(),
                        BranchName::new("main").unwrap(),
                    )
                } else {
                    repo.clone()
                }
            })
            .collect();
        let manifest = Manifest::new(
            HallName::new("acme").unwrap(),
            Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
            repos,
            None,
        )
        .unwrap();
        Manifest::write(&layout, &manifest).unwrap();
        let ctx = Ctx::new(root.clone());

        let approved = deliver(&ctx, preview_input("checkout")).unwrap();
        let report = deliver(
            &ctx,
            apply_input("checkout", &approved.value.preview.fingerprint),
        )
        .unwrap();

        assert!(!report.is_clean(), "a failed push must not be a clean run");
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].subject, "web");
        assert_eq!(report.warnings[0].code, "deliver.push_failed");
        // Best-effort: api still landed.
        assert!(
            report
                .value
                .pushes
                .iter()
                .any(|push| push.repo.as_str() == "api" && push.ok)
        );
        assert!(
            report
                .value
                .pushes
                .iter()
                .any(|push| push.repo.as_str() == "web" && !push.ok)
        );
        assert!(remote_ref(&origin_of(&root, "api"), "checkout").is_some());
    }

    // -- ordering -------------------------------------------------------------

    fn delivery_repo(name: &str, dependencies: Vec<&str>) -> DeliveryRepo {
        DeliveryRepo {
            repo: RepoName::new(name).unwrap(),
            local_branch: BranchName::new("checkout").unwrap(),
            remote: "git@example.com:acme/api.git".to_owned(),
            push_refspec: "checkout:refs/heads/checkout".to_owned(),
            action: DeliveryAction::PushOnly,
            base_branch: BranchName::new("main").unwrap(),
            dependencies: dependencies
                .into_iter()
                .map(|dep| RepoName::new(dep).unwrap())
                .collect(),
            blockers: Vec::new(),
        }
    }

    #[test]
    fn ordering_puts_a_repos_dependencies_before_it() {
        let mut repos = vec![
            delivery_repo("api", vec!["web"]),
            delivery_repo("web", vec![]),
            delivery_repo("cron", vec![]),
        ];

        order_by_dependencies(&mut repos);

        let order: Vec<&str> = repos.iter().map(|repo| repo.repo.as_str()).collect();
        let web = order.iter().position(|name| *name == "web").unwrap();
        let api = order.iter().position(|name| *name == "api").unwrap();
        assert!(web < api, "a dependency must be pushed first: {order:?}");
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn ordering_preserves_name_order_between_unrelated_repos() {
        let mut repos = vec![
            delivery_repo("b", vec![]),
            delivery_repo("a", vec![]),
            delivery_repo("c", vec![]),
        ];

        order_by_dependencies(&mut repos);

        let order: Vec<&str> = repos.iter().map(|repo| repo.repo.as_str()).collect();
        assert_eq!(order, vec!["b", "a", "c"], "no dependencies, no reordering");
    }

    // -- rendering ------------------------------------------------------------

    #[test]
    fn the_human_preview_surface_lists_each_repo_and_the_fingerprint() {
        let outcome = DeliverOutcome {
            root: Utf8PathBuf::from("/hall"),
            preview: DeliveryPreview {
                feature: FeatureName::new("checkout").unwrap(),
                repos: vec![delivery_repo("api", vec![])],
                fingerprint: "abc123".to_owned(),
            },
            pushes: Vec::new(),
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("Delivery preview for `checkout` in /hall:"));
        assert!(rendered.contains("branch:  checkout"));
        assert!(rendered.contains("refspec: checkout:refs/heads/checkout"));
        assert!(rendered.contains("base:    main"));
        assert!(rendered.contains("action:  push only"));
        assert!(rendered.contains("blockers: none"));
        assert!(rendered.contains("fingerprint: abc123"));
    }

    #[test]
    fn the_human_apply_surface_reports_each_push() {
        let outcome = DeliverOutcome {
            root: Utf8PathBuf::from("/hall"),
            preview: DeliveryPreview {
                feature: FeatureName::new("checkout").unwrap(),
                repos: vec![delivery_repo("api", vec![])],
                fingerprint: "abc123".to_owned(),
            },
            pushes: vec![
                PushResult {
                    repo: RepoName::new("api").unwrap(),
                    ok: true,
                    detail: None,
                },
                PushResult {
                    repo: RepoName::new("web").unwrap(),
                    ok: false,
                    detail: Some("remote did not answer".to_owned()),
                },
            ],
        };

        let mut out = Vec::new();
        outcome.write_human(&mut out).unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("Delivered `checkout` in /hall (fingerprint abc123):"));
        assert!(rendered.contains("  api: pushed"));
        assert!(rendered.contains("  web: not pushed — remote did not answer"));
    }
}
