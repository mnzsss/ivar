#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::agent::run::{RunInput, run};
use crate::domain::agent::receipt::{AgentRunOutcome, Trigger};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::infra::proc::redirect_on_this_thread;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{
    FakeDocker, FakeGh, fake_docker_command, fake_gh_on_this_thread, seeded_hall, seeded_repo,
};

fn hall_with_repo_and_agent() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = seeded_hall();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/app"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("app").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&Ctx::new(root.clone()), &Default::default()).unwrap();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/fixer.md"),
        "---\nschedule: \"@daily\"\ntool: codex\naccess: feature\nworkdir: app\n---\nFix it.\n",
    )
    .unwrap();
    (guard, root)
}

#[test]
fn writes_only_into_the_agent_feature_and_records_a_preview_fingerprint() {
    let (_guard, root) = hall_with_repo_and_agent();
    let layout = Layout::at(root.clone());
    let worktree = layout.repo_worktree(
        &RepoName::new("app").unwrap(),
        &BranchName::new("agent-fixer").unwrap(),
    );
    let fake = FakeDocker::install(&root);
    let gh = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&gh);
    let _docker = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_WRITE", worktree.join("FIX.md").as_str()),
    );

    let receipt = run(
        &Ctx::new(root.clone()),
        RunInput {
            name: "fixer".into(),
            trigger: Trigger::Manual,
        },
    )
    .unwrap()
    .value;

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    let docker_log = fake.log();
    let mounts: Vec<&str> = docker_log
        .lines()
        .filter(|l| l.starts_with("run --rm"))
        .collect();
    assert_eq!(mounts.len(), 1);
    assert!(
        mounts[0].contains(&format!("-v {worktree}:/workspace/app ")),
        "{}",
        mounts[0]
    );
    assert!(!mounts[0].contains(":ro"), "{}", mounts[0]);
    assert!(receipt.diff.as_deref().unwrap().contains("+agent edit"));
    assert!(
        receipt
            .delivery_fingerprint
            .as_deref()
            .is_some_and(|f| !f.is_empty())
    );
    let log = std::process::Command::new("git")
        .args(["-C", worktree.as_str(), "log", "-1", "--format=%s"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        format!("agent fixer: run {}", receipt.run_id)
    );
    assert!(!gh.log().contains("pr create"));
}

#[test]
fn a_planted_git_pointer_is_ignored_by_host_git() {
    let (_guard, root) = hall_with_repo_and_agent();
    let layout = Layout::at(root.clone());
    let worktree = layout.repo_worktree(
        &RepoName::new("app").unwrap(),
        &BranchName::new("agent-fixer").unwrap(),
    );
    let evil = root.parent().unwrap().join("evil-git");
    std::fs::create_dir_all(evil.join("hooks")).unwrap();
    std::fs::write(
        evil.join("hooks/pre-commit"),
        "#!/bin/sh\ntouch \"$0.ran\"\n",
    )
    .unwrap();
    let fake = FakeDocker::install(&root);
    let gh = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&gh);
    let _docker = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake)
            .env("DOCKER_FAKE_WRITE", worktree.join("FIX.md").as_str())
            .env("DOCKER_FAKE_PLANT", worktree.join(".git").as_str())
            .env("DOCKER_FAKE_PLANT_TARGET", evil.as_str()),
    );

    let receipt = run(
        &Ctx::new(root.clone()),
        RunInput {
            name: "fixer".into(),
            trigger: Trigger::Manual,
        },
    )
    .unwrap()
    .value;

    assert!(receipt.diff.as_deref().unwrap().contains("+agent edit"));
    assert!(
        !evil.join("hooks/pre-commit.ran").exists(),
        "a hook from the planted git dir ran"
    );
    assert!(
        !evil.join("HEAD").exists() && !evil.join("index").exists(),
        "host git wrote into the planted dir"
    );
    let pointer = std::fs::read_to_string(worktree.join(".git")).unwrap();
    assert!(pointer.contains(".bare/worktrees/"), "{pointer}");
}

#[test]
fn a_run_that_changes_nothing_commits_nothing() {
    let (_guard, root) = hall_with_repo_and_agent();
    let fake = FakeDocker::install(&root);
    let gh = FakeGh::install(&root);
    let _gh = fake_gh_on_this_thread(&gh);
    let _docker = redirect_on_this_thread("docker", fake_docker_command(&fake));
    let receipt = run(
        &Ctx::new(root),
        RunInput {
            name: "fixer".into(),
            trigger: Trigger::Manual,
        },
    )
    .unwrap()
    .value;
    assert_eq!(receipt.diff, None);
    assert_eq!(receipt.delivery_fingerprint, None);
}

#[test]
fn a_delivery_error_is_scrubbed() {
    let d = Delivered {
        diff: None,
        fingerprint: None,
        error: Some("push to https://x:hunter2@h failed".into()),
    };
    assert_eq!(
        crate::domain::agent::receipt::scrub(d.error.as_deref().unwrap(), &["hunter2"]),
        "push to https://x:[redacted]@h failed"
    );
}
