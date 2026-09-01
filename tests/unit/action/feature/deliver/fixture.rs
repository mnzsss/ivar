use super::*;

pub(super) use crate::action::feature::create::{CreateInput, create as create_action};
pub(super) use crate::action::feature::promote::{self, PromoteInput};
pub(super) use crate::action::hall::{self, InitInput};
pub(super) use crate::domain::feature::{DeliveryAction, DeliveryRepo};
pub(super) use crate::domain::name::{BranchName, HallName, RepoName};
pub(super) use crate::domain::provider::Provider;
pub(super) use crate::error::{Status, WriteHuman};
pub(super) use crate::git::Git;
pub(super) use crate::store::layout::Layout;
pub(super) use crate::store::manifest::{Manifest, Providers, Repo};
pub(super) use crate::test_support::{git, hall_root, seeded_repo};
pub(super) use camino::{Utf8Path, Utf8PathBuf};

pub(super) fn approve_through_plan(root: &Utf8PathBuf) {
    let ctx = Ctx::new(root.clone());
    crate::action::plan::create::create(
        &ctx,
        crate::action::plan::create::CreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    for gate in ["requirements", "analysis", "plan"] {
        crate::action::plan::approve::approve(
            &ctx,
            crate::action::plan::approve::ApproveInput {
                feature: "checkout".to_owned(),
                gate: gate.to_owned(),
            },
        )
        .unwrap();
    }
}

pub(super) fn hall_with_promoted(repos: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
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
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
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
                base: None,
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

pub(super) fn preview_input(feature: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: true,
        land: false,
        fingerprint: None,
        global_metadata: PullRequestMetadata::default(),
        repo_overrides: Vec::new(),
    }
}

pub(super) fn apply_input(feature: &str, fingerprint: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: false,
        land: false,
        fingerprint: Some(fingerprint.to_owned()),
        global_metadata: PullRequestMetadata::default(),
        repo_overrides: Vec::new(),
    }
}

pub(super) fn land_preview_input(feature: &str) -> DeliverInput {
    DeliverInput {
        feature: feature.to_owned(),
        preview: true,
        land: true,
        fingerprint: None,
        global_metadata: PullRequestMetadata::default(),
        repo_overrides: Vec::new(),
    }
}

pub(super) fn remote_ref(origin: &str, branch: &str) -> Option<String> {
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

pub(super) fn origin_of(root: &Utf8Path, repo: &str) -> String {
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

/// Like `test_support::git`, but hands back stdout. The empty `core.hooksPath`
/// is the same scaffolding opt-out and carries the same caveat: this builds the
/// arrangement a test starts from, so it must not be used to assert protection.
pub(super) fn git_stdout(cwd: &Utf8Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(["-c", "core.hooksPath="])
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "git {} failed in {cwd}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

pub(super) fn snapshot_all_worktrees(root: &Utf8Path) -> Vec<(Utf8PathBuf, String)> {
    let layout = Layout::at(root);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let mut snapshots = Vec::new();
    for repo in manifest.repos() {
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        if worktree.exists() {
            let sha = git_stdout(&worktree, &["rev-parse", "HEAD"])
                .trim()
                .to_owned();
            snapshots.push((worktree, sha));
        }
    }
    snapshots.sort_by(|a, b| a.0.cmp(&b.0));
    snapshots
}

pub(super) fn delivery_repo(name: &str, dependencies: Vec<&str>) -> DeliveryRepo {
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
        pr_url: None,
        default_branch: None,
        ff_possible: None,
        remote_default_tip: None,
        pr_title: None,
        pr_body: None,
    }
}

pub(super) fn child_of_checkout(root: &Utf8PathBuf, name: &str) {
    let layout = Layout::at(root.clone());
    let mut child = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    );
    child.parent = Some(crate::domain::name::FeatureName::new("checkout").unwrap());
    child.write(&layout).unwrap();
}
