#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::Ctx;
use crate::action::feature::cleanup::{CleanupInput, cleanup};
use crate::action::feature::close::{CloseInput, close as close_feature};
use crate::action::feature::create::{CreateInput, create as create_feature};
use crate::action::feature::promote::{PromoteInput, promote};
use crate::action::hall::{self, InitInput};
use crate::action::sync::{SyncInput, sync};
use crate::domain::feature::{CleanupPreview, CleanupRecord};
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::git::{self, Git};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git as test_git, hall_root, seeded_repo};
use camino::Utf8PathBuf;
use std::fs::remove_dir_all;

fn hall_with_feature(repos: &[&str], branch: Option<&str>) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let layout = Layout::at(root.clone());
    let manifest_repos = repos
        .iter()
        .map(|repo| {
            let origin = seeded_repo(&root.parent().unwrap().join("origins").join(repo), "main");
            Repo::new(
                RepoName::new(*repo).unwrap(),
                origin.as_str(),
                BranchName::new("main").unwrap(),
            )
        })
        .collect();
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        manifest_repos,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    sync(&ctx, &SyncInput::default()).unwrap();
    create_feature(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: branch.map(str::to_owned),
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    for repo in repos {
        promote(
            &ctx,
            PromoteInput {
                feature: "checkout".to_owned(),
                repo: (*repo).to_owned(),
                base: None,
            },
        )
        .unwrap();
    }
    (guard, root)
}

fn run_preview(root: &Utf8PathBuf) -> CleanupPreview {
    cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
    .value
    .preview
}

fn add_subfeature(root: &Utf8PathBuf, name: &str, parent: &str, promote_api: bool) {
    let ctx = Ctx::new(root.clone());
    create_feature(
        &ctx,
        CreateInput {
            name: name.to_owned(),
            branch: None,
            base: None,
            parent: Some(parent.to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    if promote_api {
        promote(
            &ctx,
            PromoteInput {
                feature: name.to_owned(),
                repo: "api".to_owned(),
                base: None,
            },
        )
        .unwrap();
    }
}

fn preview_from_session(root: &Utf8PathBuf, session: &SessionId) -> CleanupPreview {
    cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: true,
            record: None,
            session_id: Some(session.as_str().to_owned()),
        },
    )
    .unwrap()
    .value
    .preview
}

fn pull_request(state: &str, head_oid: Option<&str>) -> PullRequest {
    PullRequest {
        url: "https://example.test/pr/7".to_owned(),
        number: 7,
        state: state.to_owned(),
        head_oid: head_oid.map(str::to_owned),
        merge_commit: None,
        merged_at: None,
        is_draft: false,
    }
}

fn apply_with_approved_record(root: &Utf8PathBuf) -> crate::domain::feature::CleanupApplyOutcome {
    apply_with_approved_record_in(&Ctx::new(root.clone()), root)
}

fn apply_with_approved_record_in(
    ctx: &Ctx,
    root: &Utf8PathBuf,
) -> crate::domain::feature::CleanupApplyOutcome {
    let preview = run_preview(root);
    let layout = Layout::at(root.clone());
    fs::ensure_dir(&layout.docs_updates_dir()).unwrap();
    let record_rel_path = Utf8PathBuf::from("docs/updates/001-checkout.cleanup.json");
    fs::write_text(
        &root.join(&record_rel_path),
        &format!(
            r#"{{
                "schema_version": 1,
                "feature": "checkout",
                "branch": "checkout",
                "fingerprint": "{}",
                "approvals": {{
                    "delivery": {{ "approved": true, "at": "2026-08-28T12:00:00Z" }},
                    "documentation": {{ "decision": "written", "paths": ["docs/product/001-checkout.md"], "reason": null, "at": "2026-08-28T12:05:00Z" }},
                    "teardown": {{ "approved": true, "at": "2026-08-28T12:10:00Z" }}
                }},
                "outcome": null
            }}"#,
            preview.fingerprint
        ),
    )
    .unwrap();
    cleanup(
        ctx,
        CleanupInput {
            feature: "checkout".to_owned(),
            preview: false,
            record: Some(record_rel_path),
            session_id: None,
        },
    )
    .unwrap()
    .value
    .apply_outcome
    .expect("expected apply outcome")
}

// -- children integrated into a parent that was rebased and delivered -------

fn preview_of(root: &Utf8PathBuf, feature: &str) -> CleanupPreview {
    cleanup(
        &Ctx::new(root.clone()),
        CleanupInput {
            feature: feature.to_owned(),
            preview: true,
            record: None,
            session_id: None,
        },
    )
    .unwrap()
    .value
    .preview
}

/// Commit `file` in the worktree of `branch` in the `api` repo.
fn commit_file(root: &Utf8PathBuf, branch: &str, file: &str) {
    let worktree = root.join(".ivar/repos/api").join(branch);
    std::fs::write(worktree.join(file), format!("{file}\n")).unwrap();
    git_in(&worktree, &["add", file]);
    git_in(&worktree, &["commit", "-m", file]);
}

fn git_in(dir: &Utf8PathBuf, args: &[&str]) {
    test_git(dir, args);
}

/// A child of `checkout` that rewrites `line` of the parent's `shared.md`,
/// with its plan approved so it can integrate.
fn add_editing_child(root: &Utf8PathBuf, child: &str, line: usize) {
    use crate::action::plan::approve as plan_approve;
    use crate::action::plan::create as plan_create;

    let ctx = Ctx::new(root.clone());
    add_subfeature(root, child, "checkout", true);
    let worktree = root.join(".ivar/repos/api").join(child);
    let shared = worktree.join("shared.md");
    let text = std::fs::read_to_string(&shared).unwrap();
    let edited = text
        .lines()
        .enumerate()
        .map(|(index, text)| {
            if index + 1 == line {
                format!("{child} edit")
            } else {
                text.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&shared, format!("{edited}\n")).unwrap();
    git_in(&worktree, &["add", "shared.md"]);
    git_in(&worktree, &["commit", "-m", child]);
    plan_create::create(
        &ctx,
        plan_create::CreateInput {
            feature: child.to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis", "plan"] {
        plan_approve::approve(
            &ctx,
            plan_approve::ApproveInput {
                feature: child.to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
}

fn integrate_child(root: &Utf8PathBuf, child: &str) {
    use crate::action::feature::integrate::{IntegrateInput, integrate};

    integrate(
        &Ctx::new(root.clone()),
        IntegrateInput {
            feature: child.to_owned(),
            via: None,
            strategy: None,
            name: None,
        },
    )
    .unwrap();
}

/// A parent that owns a ten-line `shared.md`, and two children editing lines
/// far enough apart to merge cleanly, but close enough that each child's diff
/// against its own fork point carries the other's edit as context. Integrated
/// in turn, the second squash no longer has the patch its child branch has, so
/// only the receipt can say it landed.
fn parent_with_integrated_children() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with_feature(&["api"], None);
    let bare = root.join(".ivar/repos/api/.bare");
    test_git(&bare, &["config", "user.name", "ivar tests"]);
    test_git(&bare, &["config", "user.email", "tests@ivar.invalid"]);

    let parent = root.join(".ivar/repos/api/checkout");
    let shared = (1..=10)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    std::fs::write(parent.join("shared.md"), shared).unwrap();
    git_in(&parent, &["add", "shared.md"]);
    git_in(&parent, &["commit", "-m", "shared"]);

    add_editing_child(&root, "child-a", 4);
    add_editing_child(&root, "child-b", 7);
    integrate_child(&root, "child-a");
    integrate_child(&root, "child-b");
    (guard, root)
}

fn rebase_parent_onto_advanced_main(root: &Utf8PathBuf) {
    commit_file(root, "main", "other.md");
    crate::action::feature::rebase::rebase(
        &Ctx::new(root.clone()),
        crate::action::feature::rebase::RebaseInput {
            name: "checkout".to_owned(),
            onto: None,
            repos: Vec::new(),
            offline: true,
        },
    )
    .unwrap();
}

/// The parent squash-merged into `main`, the way a delivered pull request
/// lands.
fn deliver_parent(root: &Utf8PathBuf) {
    let main = root.join(".ivar/repos/api/main");
    test_git(&main, &["merge", "--squash", "checkout"]);
    test_git(&main, &["commit", "-m", "deliver checkout"]);
}

fn delivered_parent_with_integrated_children() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = parent_with_integrated_children();
    rebase_parent_onto_advanced_main(&root);
    deliver_parent(&root);
    (guard, root)
}

#[path = "cleanup/branches.rs"]
mod branches;
#[path = "cleanup/events.rs"]
mod events;
#[path = "cleanup/forge.rs"]
mod forge;
#[path = "cleanup/parents.rs"]
mod parents;
#[path = "cleanup/preview.rs"]
mod preview;
#[path = "cleanup/record.rs"]
mod record;
#[path = "cleanup/subtree.rs"]
mod subtree;
#[path = "cleanup/worktrees.rs"]
mod worktrees;
