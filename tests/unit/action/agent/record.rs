#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! What a finished run commits, against a real hall with a bare remote. The
//! receipt is built by hand: nothing here needs a container or Landlock, only
//! `finish`, which is where a run is recorded.

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::action::Ctx;
use crate::action::agent::run::{RunInput, finish, run};
use crate::action::hall::{InitInput, init};
use crate::domain::agent::receipt::Trigger;
use crate::infra::proc::redirect_on_this_thread;
use crate::store;
use crate::test_support::{FakeDocker, fake_docker_command, hall_root};

struct Fixture {
    _guard: tempfile::TempDir,
    root: Utf8PathBuf,
    remote: Utf8PathBuf,
    layout: Layout,
}

fn git(cwd: &Utf8Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed in {cwd}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A hall on `main` with a pushed `baseline` commit, an agent `nightly` that
/// may write `reports/memory-health`, `<base>/elsewhere` and another hall's
/// `.ivar`, and `frontmatter` extra lines.
fn fixture() -> Fixture {
    let (guard, root) = hall_root();
    let base = root.parent().unwrap().to_owned();
    let remote = base.join("remote.git");
    git(
        &base,
        &[
            "init",
            "--bare",
            "--initial-branch",
            "main",
            remote.as_str(),
        ],
    );
    init(
        &Ctx::new(root.clone()),
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: None,
            git: true,
            origin: None,
        },
    )
    .unwrap();
    for (key, value) in [
        ("user.name", "ivar tests"),
        ("user.email", "tests@ivar.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", ".git/hooks"),
    ] {
        git(&root, &["config", key, value]);
    }
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/nightly.md"),
        format!(
            "---\nschedule: \"@daily\"\nruntime: host\ntool: claude\nwrite:\n  - reports/memory-health\n  - {base}/elsewhere\n  - {base}/other-hall/.ivar/agents\n---\nGo.\n"
        ),
    )
    .unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-m", "baseline"]);
    git(&root, &["remote", "add", "origin", remote.as_str()]);
    git(&root, &["push", "-u", "origin", "main"]);
    let layout = Layout::at(root.clone());
    Fixture {
        _guard: guard,
        root,
        remote,
        layout,
    }
}

impl Fixture {
    fn write(&self, relative: &str, content: &str) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    /// What the agent did: a report in its root, one outside the hall, one in
    /// another hall's `.ivar`, and a file of its own that no root covers.
    fn agent_writes(&self) {
        self.write("reports/memory-health/run.md", "report\n");
        self.write("unrelated.md", "not the agent's\n");
        let base = self.root.parent().unwrap();
        for outside in ["elsewhere/out.md", "other-hall/.ivar/agents/x.json"] {
            let path = base.join(outside);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "out\n").unwrap();
        }
    }

    fn finish(&self, outcome: AgentRunOutcome) -> AgentRunReceipt {
        let spec = store::agent::load(&self.layout, "nightly").unwrap();
        let mut receipt = AgentRunReceipt::skipped(
            "20261111T101500Z-abcd1234".to_owned(),
            "nightly".to_owned(),
            Trigger::Manual,
            "2026-11-11T10:15:00Z".to_owned(),
        );
        receipt.outcome = outcome;
        finish(&self.layout, &spec, receipt).unwrap().value
    }

    fn set_manifest(&self, key: &str, value: bool) {
        let path = self.root.join("ivar.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        manifest[key] = serde_json::json!(value);
        std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "-m", "switch"]);
        git(&self.root, &["push", "origin", "main"]);
    }

    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"])
    }

    fn remote_tip(&self) -> String {
        git(&self.remote, &["rev-parse", "main"])
    }

    fn subjects(&self) -> Vec<String> {
        git(&self.root, &["log", "--format=%s"])
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn touched(&self, rev: &str) -> Vec<String> {
        let mut paths: Vec<String> = git(
            &self.root,
            &["show", "--no-renames", "--format=", "--name-only", rev],
        )
        .lines()
        .map(str::to_owned)
        .collect();
        paths.sort();
        paths
    }
}

#[test]
fn a_successful_run_commits_only_what_changed_under_its_roots_inside_the_hall() {
    let f = fixture();
    f.agent_writes();

    let receipt = f.finish(AgentRunOutcome::Succeeded);

    assert!(
        receipt.sync_warnings.is_empty(),
        "{:?}",
        receipt.sync_warnings
    );
    assert_eq!(
        f.subjects()[..2],
        [
            "chore(hall): Sync pending hall changes",
            "docs(nightly): Record the 2026-11-11 run"
        ]
    );
    assert_eq!(f.touched("HEAD~1"), ["reports/memory-health/run.md"]);
    assert_eq!(f.touched("HEAD"), ["unrelated.md"]);
    assert_eq!(f.head(), f.remote_tip());
}

#[test]
fn a_run_that_did_not_succeed_syncs_nothing() {
    for outcome in [
        AgentRunOutcome::Failed,
        AgentRunOutcome::Blocked,
        AgentRunOutcome::TimedOut,
        AgentRunOutcome::Skipped,
    ] {
        let f = fixture();
        f.agent_writes();
        let before = f.head();

        let receipt = f.finish(outcome);

        assert_eq!(f.head(), before, "{outcome:?}");
        assert!(receipt.sync_warnings.is_empty());
        assert!(
            git(&f.root, &["status", "--porcelain", "-uall"])
                .contains("reports/memory-health/run.md")
        );
    }
}

#[test]
fn a_root_with_nothing_changed_makes_no_operation_commit() {
    let f = fixture();
    f.write("unrelated.md", "not under a root\n");

    f.finish(AgentRunOutcome::Succeeded);

    assert_eq!(
        f.subjects()[..2],
        ["chore(hall): Sync pending hall changes", "baseline"]
    );
}

#[test]
fn auto_commit_off_turns_the_run_sync_off() {
    let f = fixture();
    f.set_manifest("auto_commit", false);
    f.agent_writes();
    let before = f.head();

    let receipt = f.finish(AgentRunOutcome::Succeeded);

    assert_eq!(f.head(), before);
    assert_eq!(f.remote_tip(), before);
    assert!(receipt.sync_warnings.is_empty());
}

#[test]
fn auto_push_off_commits_but_keeps_the_commits_local() {
    let f = fixture();
    f.set_manifest("auto_push", false);
    f.agent_writes();
    let before = f.head();

    f.finish(AgentRunOutcome::Succeeded);

    assert_eq!(f.subjects()[1], "docs(nightly): Record the 2026-11-11 run");
    assert_ne!(f.head(), before);
    assert_eq!(f.remote_tip(), before);
}

#[test]
fn a_sync_warning_lands_in_the_stored_receipt_and_leaves_the_outcome_alone() {
    let f = fixture();
    git(&f.root, &["branch", "--unset-upstream"]);
    f.agent_writes();

    let receipt = f.finish(AgentRunOutcome::Succeeded);

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert_eq!(
        receipt.sync_warnings.len(),
        1,
        "{:?}",
        receipt.sync_warnings
    );
    assert!(receipt.sync_warnings[0].starts_with("hall.push_skipped: "));
    let stored = store::agent::read_receipt(&f.layout, "nightly", &receipt.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored, receipt);
}

#[test]
fn a_concluded_task_moves_to_done_in_the_same_sync() {
    let f = fixture();
    f.write(
        "agents/recheck.md",
        "---\nat: 2026-11-08T09:00\ntool: claude\n---\nLook again.\n",
    );
    git(&f.root, &["add", "-A"]);
    git(&f.root, &["commit", "-m", "task"]);
    git(&f.root, &["push", "origin", "main"]);
    let fake = FakeDocker::install(&f.root);
    let _docker = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let receipt = run(
        &Ctx::new(f.root.clone()),
        RunInput {
            name: "recheck".into(),
            trigger: Trigger::Schedule,
        },
    )
    .unwrap()
    .value;

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert!(!f.root.join("agents/recheck.md").exists());
    let touched = f.touched("HEAD");
    assert!(
        touched.contains(&"agents/recheck.md".to_owned()),
        "{touched:?}"
    );
    assert!(
        touched.iter().any(|path| path.starts_with("agents/done/")),
        "{touched:?}"
    );
    assert_eq!(f.head(), f.remote_tip());
}
