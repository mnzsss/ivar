#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, HallName};
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::infra::progress::Progress;
use crate::store::manifest::Providers;
use crate::test_support::{git, hall_root, seeded_repo};
fn hall_with(repos: &[(&str, &str)]) -> (tempfile::TempDir, Utf8PathBuf) {
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
        .map(|(name, branch)| {
            let origin = seeded_repo(&origins.join(name), branch);
            Repo::new(
                RepoName::new(*name).unwrap(),
                origin.as_str(),
                BranchName::new(*branch).unwrap(),
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

    (guard, root)
}

/// The seeded origin path of `name`, from the manifest's declared url.
fn origin_path(root: &camino::Utf8Path, name: &str) -> Utf8PathBuf {
    let manifest = Manifest::read(&Layout::at(root.to_path_buf()))
        .unwrap()
        .unwrap();
    let url = manifest
        .repos()
        .iter()
        .find(|repo| repo.name().as_str() == name)
        .unwrap()
        .url();
    Utf8PathBuf::from(url)
}

fn status_of<'a>(report: &'a Report<PullOutcome>, name: &str) -> &'a PullStatus {
    &report
        .value
        .repos
        .iter()
        .find(|repo| repo.repo.as_str() == name)
        .unwrap()
        .status
}

#[test]
fn pull_refreshes_every_declared_repo_and_reports_refreshed() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.repos.len(), 2);
    for repo in &report.value.repos {
        assert_eq!(repo.status, PullStatus::Refreshed);
    }
}

/// The fetch-and-fast-forward is real, not just a report: the default
/// worktree's files catch up to the commit the origin gained after sync.
#[test]
fn pull_advances_the_default_worktree_to_the_origins_new_tip() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let origin = origin_path(&root, "api");
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(status_of(&report, "api"), &PullStatus::Refreshed);
    assert_eq!(
        std::fs::read_to_string(root.join(".ivar/repos/api/main/CHANGELOG.md")).unwrap(),
        "v1\n"
    );
}

#[test]
fn pull_with_no_repos_reports_an_empty_run() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root);

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(report.is_clean());
    assert!(report.value.repos.is_empty());
}

#[test]
fn pull_accepts_a_named_repo() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let report = pull(
        &ctx,
        PullInput {
            repo: Some("api".to_owned()),
            diagnose: false,
        },
    )
    .unwrap();

    assert_eq!(report.value.repos.len(), 1);
    assert_eq!(report.value.repos[0].repo.as_str(), "api");
    assert_eq!(report.value.repos[0].status, PullStatus::Refreshed);
}

#[test]
fn pull_blocks_on_a_repo_that_is_not_in_the_manifest() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root);

    let failure = pull(
        &ctx,
        PullInput {
            repo: Some("ghost".to_owned()),
            diagnose: false,
        },
    )
    .unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "repo.not_found");
}

/// A read-only-guarded default worktree (the state a live session leaves
/// behind) must still refresh: the guard is lifted for the fetch-and-
/// fast-forward and re-applied afterwards. Without the lift, git cannot
/// create the new file and would leave the branch advanced but the file
/// missing — with a zero exit code.
#[test]
fn pull_refreshes_a_read_only_guarded_worktree_and_reapplies_the_guard() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let worktree = root.join(".ivar/repos/api/main");
    let origin = origin_path(&root, "api");
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    // The guard a session's view-dir materialisation would have applied.
    fs::clear_write_bits(&worktree).unwrap();

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert_eq!(status_of(&report, "api"), &PullStatus::Refreshed);
    assert_eq!(
        std::fs::read_to_string(worktree.join("CHANGELOG.md")).unwrap(),
        "v1\n",
        "the guarded worktree must catch up to the new commit"
    );
    assert_eq!(
        fs::unix_mode(&worktree).unwrap().unwrap() & 0o222,
        0,
        "the read-only guard must be re-applied after the refresh"
    );
    // Restore so TempDir can clean up.
    fs::restore_write_bits(&worktree).unwrap();
}

/// A repo whose default worktree was never materialised cannot be
/// refreshed — it fails (with the way back in line named), and the
/// healthy repos still refresh.
#[test]
fn a_repo_with_no_worktree_fails_and_the_others_still_refresh() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    // A second declared repo that was never synced.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let mut repos = manifest.repos().to_vec();
    repos.push(Repo::new(
        RepoName::new("gone").unwrap(),
        root.join("no-such-origin").as_str(),
        BranchName::new("main").unwrap(),
    ));
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(!report.is_clean());
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.subject == "gone"),
        "the failing repo must surface as a warning"
    );
    assert!(matches!(
        status_of(&report, "gone"),
        PullStatus::Failed { reason } if !reason.is_empty()
    ));
    assert_eq!(status_of(&report, "api"), &PullStatus::Refreshed);
}

/// The "skipped" case: a default branch that diverged locally cannot be
/// fast-forwarded — it is reported and skipped, never failed and never
/// clobbered.
#[test]
fn a_non_fast_forward_branch_is_skipped_not_failed() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    // The default worktree gains a local commit while the origin moves
    // elsewhere — the fetch succeeds, the fast-forward cannot.
    let worktree = root.join(".ivar/repos/api/main");
    git(&worktree, &["commit", "--allow-empty", "-m", "local drift"]);
    let origin = origin_path(&root, "api");
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(!report.is_clean());
    assert!(matches!(
        status_of(&report, "api"),
        PullStatus::Skipped { reason, .. } if !reason.is_empty()
    ));
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.subject == "api" && warning.code == "repo.pull_skipped"),
        "a skipped repo must surface as a warning"
    );
}

/// `--diagnose` reports the local-only and remote-only commits of a skipped
/// branch, read-only — the detail that tells a human whether the local
/// commits are genuinely theirs or already re-landed upstream.
#[test]
fn diagnose_reports_the_local_and_remote_commits_of_a_diverged_branch() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    // Local commits the remote does not have…
    let worktree = root.join(".ivar/repos/api/main");
    git(&worktree, &["commit", "--allow-empty", "-m", "local one"]);
    git(&worktree, &["commit", "--allow-empty", "-m", "local two"]);
    // …and remote commits the local does not have.
    let origin = origin_path(&root, "api");
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "remote one"]);

    let report = pull(
        &ctx,
        PullInput {
            repo: Some("api".to_owned()),
            diagnose: true,
        },
    )
    .unwrap();

    let divergence = match status_of(&report, "api") {
        PullStatus::Skipped {
            divergence: Some(divergence),
            ..
        } => divergence,
        other => panic!("expected a diagnosed skip, got {other:?}"),
    };
    assert_eq!(divergence.ahead(), 2, "two local-only commits");
    assert_eq!(divergence.behind(), 1, "one remote-only commit");
    let local_subjects: Vec<_> = divergence
        .local_only
        .iter()
        .map(|commit| commit.subject.as_str())
        .collect();
    assert_eq!(local_subjects, vec!["local two", "local one"]);
    assert_eq!(divergence.remote_only[0].subject, "remote one");
}

/// Without `--diagnose` the skip carries no divergence detail, keeping the
/// default surface and JSON unchanged.
#[test]
fn a_skip_without_diagnose_carries_no_divergence() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let worktree = root.join(".ivar/repos/api/main");
    git(&worktree, &["commit", "--allow-empty", "-m", "local drift"]);
    let origin = origin_path(&root, "api");
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(matches!(
        status_of(&report, "api"),
        PullStatus::Skipped {
            divergence: None,
            ..
        }
    ));
}

/// The human surface renders a diagnosed skip with its commit lists under the
/// "skipped" line.
#[test]
fn the_human_surface_renders_a_diagnosed_skip() {
    use crate::git::CommitInfo;

    let outcome = PullOutcome {
        root: Utf8PathBuf::from("/hall"),
        repos: vec![RepoPull {
            repo: RepoName::new("api").unwrap(),
            status: PullStatus::Skipped {
                reason: "cannot fast-forward".to_owned(),
                divergence: Some(crate::git::Divergence {
                    local_only: vec![CommitInfo {
                        sha: "abcdef1234567890".to_owned(),
                        subject: "local commit".to_owned(),
                    }],
                    remote_only: vec![CommitInfo {
                        sha: "1234567890abcdef".to_owned(),
                        subject: "remote commit".to_owned(),
                    }],
                }),
            },
        }],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Pulled in /hall:\n  api  skipped — cannot fast-forward\n\
             \x20     api is 1 commit(s) ahead — only here:\n\
             \x20       abcdef12  local commit\n\
             \x20     api is 1 commit(s) behind — only upstream:\n\
             \x20       12345678  remote commit\n\
             refreshed: 0  failed: 0  skipped: 1\n"
    );
}

#[test]
fn pull_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = pull(&ctx, PullInput::default()).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn the_human_surface_reports_per_repo_status_and_the_counts() {
    let outcome = PullOutcome {
        root: Utf8PathBuf::from("/hall"),
        repos: vec![
            RepoPull {
                repo: RepoName::new("api").unwrap(),
                status: PullStatus::Refreshed,
            },
            RepoPull {
                repo: RepoName::new("web").unwrap(),
                status: PullStatus::Failed {
                    reason: "no worktree".to_owned(),
                },
            },
            RepoPull {
                repo: RepoName::new("app").unwrap(),
                status: PullStatus::Skipped {
                    reason: "diverged".to_owned(),
                    divergence: None,
                },
            },
        ],
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Pulled in /hall:\n  api  refreshed\n  web  FAILED — no worktree\n  app  skipped — diverged\n\
             refreshed: 1  failed: 1  skipped: 1\n"
    );
}

#[test]
fn the_json_surface_carries_the_status_per_repo() {
    let outcome = PullOutcome {
        root: Utf8PathBuf::from("/hall"),
        repos: vec![RepoPull {
            repo: RepoName::new("api").unwrap(),
            status: PullStatus::Refreshed,
        }],
    };

    let json = serde_json::to_string(&Report::new(outcome)).unwrap();

    assert_eq!(
        json,
        r#"{"root":"/hall","repos":[{"repo":"api","status":"refreshed"}]}"#
    );
}

/// A progress sink that remembers what it was told, so a test can assert the
/// run says which repo it is waiting on instead of sitting silent through a
/// fetch per repo.
#[derive(Debug, Default)]
struct Recording {
    steps: Mutex<Vec<String>>,
    clears: AtomicUsize,
}

impl Recording {
    fn steps(&self) -> Vec<String> {
        self.steps.lock().unwrap().clone()
    }
}

impl Progress for Recording {
    fn step(&self, message: &str) {
        self.steps.lock().unwrap().push(message.to_owned());
    }

    fn clear(&self) {
        self.clears.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn pull_reports_which_repo_it_is_fetching_before_each_round_trip() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "trunk")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let recording = Arc::new(Recording::default());
    let ctx = ctx.with_progress(recording.clone());

    let report = pull(&ctx, PullInput::default()).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        recording.steps(),
        vec![
            "[1/2] api: fetching main…".to_owned(),
            "[2/2] web: fetching trunk…".to_owned(),
        ],
        "each repo is announced before its fetch, in manifest order"
    );
}

/// The line is transient. Leaving it on screen would collide with the
/// `WriteHuman` summary `bin/ivar.rs` renders next.
#[test]
fn pull_clears_the_progress_line_before_returning() {
    let (_guard, root) = hall_with(&[("api", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let recording = Arc::new(Recording::default());
    let ctx = ctx.with_progress(recording.clone());

    pull(&ctx, PullInput::default()).unwrap();

    assert_eq!(recording.clears.load(Ordering::Relaxed), 1);
}

/// A named repo is one round trip, and the counter says so rather than
/// counting the whole manifest.
#[test]
fn pull_of_a_named_repo_counts_only_that_repo() {
    let (_guard, root) = hall_with(&[("api", "main"), ("web", "main")]);
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    let recording = Arc::new(Recording::default());
    let ctx = ctx.with_progress(recording.clone());

    pull(
        &ctx,
        PullInput {
            repo: Some("web".to_owned()),
            diagnose: false,
        },
    )
    .unwrap();

    assert_eq!(recording.steps(), vec!["[1/1] web: fetching main…"]);
}

#[test]
fn pull_with_no_repos_reports_no_steps() {
    let (_guard, root) = hall_with(&[]);
    let ctx = Ctx::new(root);

    let recording = Arc::new(Recording::default());
    let ctx = ctx.with_progress(recording.clone());

    pull(&ctx, PullInput::default()).unwrap();

    assert!(recording.steps().is_empty());
    assert_eq!(recording.clears.load(Ordering::Relaxed), 1);
}
