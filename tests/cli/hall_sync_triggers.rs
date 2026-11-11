//! The three operations that write the hall — `ivar feature cleanup`,
//! `ivar agent run` and `ivar skill sync` — commit and push what they wrote.
//!
//! What `sync_hall` does with the paths is tested in `src/action/hall/sync`;
//! these drive the compiled binary through each trigger against a real hall
//! with a bare remote: which commit holds which files, that a run that fails
//! records nothing, that `auto_commit` and `auto_push` switch it off, and that
//! a sync that cannot finish never changes the exit code.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{FakeGh, declare_repos, git, ivar, record_memory_review, seeded_repo};
use crate::delivery_support::ivar_on_github;
use crate::hall_sync::{branch_tip, hall_with_remote, head};
use camino::{Utf8Path, Utf8PathBuf};

const GENERAL: &str = "chore(hall): Sync pending hall changes";

fn git_out(dir: &Utf8Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed in {dir}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Commit subjects up to `HEAD`, newest first.
fn subjects(dir: &Utf8Path) -> Vec<String> {
    git_out(dir, &["log", "--format=%s"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The paths the commit `rev` touched, sorted.
fn touched(dir: &Utf8Path, rev: &str) -> Vec<String> {
    let mut paths: Vec<String> = git_out(dir, &["show", "--format=", "--name-only", rev])
        .lines()
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths
}

/// What `git status` still lists as pending.
fn pending(dir: &Utf8Path) -> String {
    git_out(dir, &["status", "--porcelain", "--untracked-files=all"])
}

/// Record everything as one pushed commit, so the test starts from a clean hall.
fn baseline(root: &Utf8Path) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "--allow-empty", "-m", "baseline"]);
    git(root, &["push", "origin", "main"]);
}

fn set_manifest(root: &Utf8Path, key: &str, value: &serde_json::Value) {
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest[key] = value.clone();
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
}

/// How a test turns the hall's switches.
#[derive(Clone, Copy)]
enum Switch {
    AllOn,
    NoCommit,
    NoPush,
}

fn apply_switch(root: &Utf8Path, switch: Switch) {
    match switch {
        Switch::AllOn => {}
        Switch::NoCommit => set_manifest(root, "auto_commit", &serde_json::json!(false)),
        Switch::NoPush => set_manifest(root, "auto_push", &serde_json::json!(false)),
    }
}

// -- feature cleanup ----------------------------------------------------------

const PR_URL: &str = "https://github.com/acme/pull/1";
const PAGE: &str = "docs/memory/product/001-checkout.md";
const RECORD: &str = "docs/memory/update/001-checkout.cleanup.json";

struct CleanupHall {
    _guard: tempfile::TempDir,
    root: Utf8PathBuf,
    remote: Utf8PathBuf,
    fake: FakeGh,
}

/// A hall with a remote and a delivered feature `checkout` on repo `api`,
/// reviewed, everything committed and pushed.
fn cleanup_hall(switch: Switch) -> CleanupHall {
    let (guard, root, remote) = hall_with_remote();
    let origin = seeded_repo(&root.parent().unwrap().join("origins/api"), "main");
    declare_repos(&root, &[("api", &origin, "main")]);
    for args in [
        &["sync"][..],
        &["feature", "create", "checkout"][..],
        &["feature", "promote", "checkout", "api"][..],
    ] {
        ivar().current_dir(&root).args(args).assert().success();
    }
    let worktree = root.join(".ivar/repos/api/checkout");
    std::fs::write(worktree.join("work.md"), "work\n").unwrap();
    git(&worktree, &["add", "work.md"]);
    git(&worktree, &["commit", "-m", "work"]);
    record_memory_review(&root, "checkout");
    let fake = FakeGh::install(&root);
    fake.set_existing_pr(
        &root.join(".ivar/repos/api/.bare"),
        "checkout",
        PR_URL,
        "main",
        "MERGED",
    );
    apply_switch(&root, switch);
    baseline(&root);
    CleanupHall {
        _guard: guard,
        root,
        remote,
        fake,
    }
}

impl CleanupHall {
    /// Write the record with `documentation`, then apply the cleanup.
    fn apply(&self, documentation: &serde_json::Value) -> (i32, serde_json::Value) {
        let preview = ivar_on_github(&self.fake, &[])
            .current_dir(&self.root)
            .args(["feature", "cleanup", "checkout", "--preview", "--json"])
            .output()
            .unwrap();
        let preview: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
        let record = serde_json::json!({
            "schema_version": 1,
            "feature": "checkout",
            "branch": "checkout",
            "fingerprint": preview["preview"]["fingerprint"],
            "approvals": {
                "delivery": { "approved": true, "at": "2026-08-28T12:00:00Z" },
                "documentation": documentation,
                "teardown": { "approved": true, "at": "2026-08-28T12:10:00Z" }
            },
            "outcome": null
        });
        std::fs::create_dir_all(self.root.join("docs/memory/update")).unwrap();
        std::fs::write(
            self.root.join(RECORD),
            serde_json::to_string_pretty(&record).unwrap(),
        )
        .unwrap();
        let output = ivar_on_github(&self.fake, &[])
            .current_dir(&self.root)
            .args([
                "feature", "cleanup", "checkout", "--record", RECORD, "--json",
            ])
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            serde_json::from_slice(&output.stdout).unwrap(),
        )
    }

    fn write_page(&self) {
        let page = self.root.join(PAGE);
        std::fs::create_dir_all(page.parent().unwrap()).unwrap();
        std::fs::write(page, "the page\n").unwrap();
        std::fs::write(self.root.join("notes.md"), "stray\n").unwrap();
    }
}

fn written() -> serde_json::Value {
    serde_json::json!({
        "decision": "written", "paths": [PAGE], "reason": null, "at": "2026-08-28T12:05:00Z"
    })
}

fn not_required() -> serde_json::Value {
    serde_json::json!({
        "decision": "not_required", "paths": [], "reason": "internal", "at": "2026-08-28T12:05:00Z"
    })
}

#[test]
fn a_cleanup_commits_the_record_pages_then_the_rest_and_pushes() {
    let hall = cleanup_hall(Switch::AllOn);
    hall.write_page();

    let (code, value) = hall.apply(&written());

    assert_eq!(code, 0, "{value}");
    assert_eq!(value["apply_outcome"]["feature_removed"], true);
    let root = &hall.root;
    assert_eq!(
        subjects(root)[..3],
        [
            GENERAL,
            "docs(memory): Record the checkout cleanup",
            "baseline"
        ]
    );
    assert_eq!(touched(root, "HEAD~1"), [PAGE]);
    let general = touched(root, "HEAD");
    assert!(general.contains(&"notes.md".to_owned()), "{general:?}");
    assert!(general.contains(&RECORD.to_owned()), "{general:?}");
    assert_eq!(head(root), branch_tip(&hall.remote));
    assert_eq!(pending(root), "");
}

#[test]
fn a_cleanup_with_no_pages_makes_only_the_general_commit_and_pushes() {
    let hall = cleanup_hall(Switch::AllOn);

    let (code, value) = hall.apply(&not_required());

    assert_eq!(code, 0, "{value}");
    let root = &hall.root;
    assert_eq!(subjects(root)[..2], [GENERAL, "baseline"]);
    assert!(touched(root, "HEAD").contains(&RECORD.to_owned()));
    assert_eq!(head(root), branch_tip(&hall.remote));
}

#[test]
fn a_cleanup_that_removes_only_part_of_the_feature_still_syncs() {
    let hall = cleanup_hall(Switch::AllOn);
    hall.write_page();
    // A locked worktree is one `git worktree remove` will not take away.
    git(
        &hall.root.join(".ivar/repos/api/checkout"),
        &["worktree", "lock", "."],
    );

    let (_, value) = hall.apply(&written());

    assert_eq!(value["apply_outcome"]["feature_removed"], false, "{value}");
    let root = &hall.root;
    assert_eq!(
        subjects(root)[..3],
        [
            GENERAL,
            "docs(memory): Record the checkout cleanup",
            "baseline"
        ]
    );
    assert_eq!(touched(root, "HEAD~1"), [PAGE]);
    assert_eq!(head(root), branch_tip(&hall.remote));
}

#[test]
fn a_cleanup_with_auto_commit_off_commits_nothing() {
    let hall = cleanup_hall(Switch::NoCommit);
    hall.write_page();
    let before = head(&hall.root);

    let (code, value) = hall.apply(&written());

    assert_eq!(code, 0, "{value}");
    assert_eq!(head(&hall.root), before);
    assert!(pending(&hall.root).contains(PAGE));
    assert_eq!(branch_tip(&hall.remote), before);
}

#[test]
fn a_cleanup_with_auto_push_off_commits_but_does_not_push() {
    let hall = cleanup_hall(Switch::NoPush);
    hall.write_page();
    let before = head(&hall.root);

    let (code, value) = hall.apply(&written());

    assert_eq!(code, 0, "{value}");
    assert_eq!(
        subjects(&hall.root)[1],
        "docs(memory): Record the checkout cleanup"
    );
    assert_ne!(head(&hall.root), before);
    assert_eq!(branch_tip(&hall.remote), before);
}

#[test]
fn a_sync_warning_is_reported_but_never_changes_the_cleanup_exit_code() {
    let hall = cleanup_hall(Switch::AllOn);
    git(&hall.root, &["branch", "--unset-upstream"]);

    let (code, value) = hall.apply(&not_required());

    assert_eq!(code, 0, "{value}");
    assert_eq!(value["warnings"][0]["code"], "hall.push_skipped");
    assert_eq!(value["apply_outcome"]["feature_removed"], true);
}

// -- skill sync ---------------------------------------------------------------

/// A hall on a remote with one shared skill and one personal skill declared,
/// not yet synced, and everything else committed and pushed.
fn hall_with_skills(switch: Switch) -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (guard, root, remote) = hall_with_remote();
    ivar()
        .current_dir(&root)
        .args(["skill", "create", "shared", "--description", "d", "--hall"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["skill", "create", "mine", "--description", "d"])
        .assert()
        .success();
    apply_switch(&root, switch);
    // The state file the creates may have written is part of the baseline only
    // when it is not a change the next `skill sync` is expected to make.
    git(&root, &["add", "-A"]);
    git(
        &root,
        &["reset", "--quiet", "--", ".ivar/skills/state.json"],
    );
    git(&root, &["commit", "--allow-empty", "-m", "baseline"]);
    git(&root, &["push", "origin", "main"]);
    let _ = std::fs::remove_file(root.join(".ivar/skills/state.json"));
    let _ = std::fs::remove_dir_all(root.join(".claude/skills"));
    (guard, root, remote)
}

fn skill_sync(root: &Utf8Path) -> std::process::Output {
    ivar()
        .current_dir(root)
        .args(["skill", "sync", "--json"])
        .output()
        .unwrap()
}

#[test]
fn a_skill_sync_that_changed_something_commits_the_state_then_the_rest_and_pushes() {
    let (_guard, root, remote) = hall_with_skills(Switch::AllOn);
    std::fs::write(root.join("stray.md"), "stray\n").unwrap();

    let output = skill_sync(&root);

    assert!(output.status.success(), "{output:?}");
    let steps: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(steps["steps"].as_u64().unwrap() > 0, "{steps}");
    assert_eq!(
        subjects(&root)[..3],
        [GENERAL, "chore(skills): Sync hall skills", "baseline"]
    );
    assert_eq!(touched(&root, "HEAD~1"), [".ivar/skills/state.json"]);
    assert!(touched(&root, "HEAD").contains(&"stray.md".to_owned()));
    assert_eq!(head(&root), branch_tip(&remote));
    assert_eq!(
        git_out(&root, &["ls-files", ".ivar/skills-local"]),
        "",
        "the personal root is never committed"
    );
    assert_eq!(pending(&root), "");
}

#[test]
fn a_skill_sync_with_nothing_to_do_does_not_sync_the_hall() {
    let (_guard, root, remote) = hall_with_skills(Switch::AllOn);
    assert!(skill_sync(&root).status.success());
    let synced = head(&root);
    std::fs::write(root.join("stray.md"), "stray\n").unwrap();

    let output = skill_sync(&root);

    assert!(output.status.success(), "{output:?}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["steps"], 0, "{value}");
    assert_eq!(head(&root), synced);
    assert_eq!(head(&root), branch_tip(&remote));
    assert!(pending(&root).contains("stray.md"));
}

#[test]
fn a_skill_sync_with_auto_commit_off_commits_nothing() {
    let (_guard, root, remote) = hall_with_skills(Switch::NoCommit);
    let before = head(&root);

    let output = skill_sync(&root);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(head(&root), before);
    assert_eq!(branch_tip(&remote), before);
    assert!(pending(&root).contains(".ivar/skills/state.json"));
}

#[test]
fn a_skill_sync_with_auto_push_off_commits_but_does_not_push() {
    let (_guard, root, remote) = hall_with_skills(Switch::NoPush);
    let before = head(&root);

    let output = skill_sync(&root);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(subjects(&root)[0], "chore(skills): Sync hall skills");
    assert_ne!(head(&root), before);
    assert_eq!(branch_tip(&remote), before);
}

#[test]
fn a_skill_sync_warning_from_the_hall_sync_leaves_the_exit_code_alone() {
    let (_guard, root, _remote) = hall_with_skills(Switch::AllOn);
    git(&root, &["branch", "--unset-upstream"]);

    let output = skill_sync(&root);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["warnings"][0]["code"], "hall.push_skipped");
}

// -- agent run ----------------------------------------------------------------

#[cfg(target_os = "linux")]
mod agent {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct AgentHall {
        _guard: tempfile::TempDir,
        root: Utf8PathBuf,
        remote: Utf8PathBuf,
        home: Utf8PathBuf,
        bin: Utf8PathBuf,
    }

    /// A hall with a host agent `nightly` that may write `reports/memory-health`
    /// and a directory outside the hall, and a `claude` that writes into both
    /// and exits with `exit`.
    fn agent_hall(exit: i32, switch: Switch) -> AgentHall {
        let (guard, root, remote) = hall_with_remote();
        let base = root.parent().unwrap().to_owned();
        let (home, bin, elsewhere) = (base.join("home"), base.join("bin"), base.join("elsewhere"));
        for dir in [
            home.join(".claude"),
            bin.clone(),
            elsewhere.clone(),
            root.join("reports/memory-health"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let script = bin.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf report > '{root}/reports/memory-health/run.md'\nprintf out > '{elsewhere}/out.md'\nexit {exit}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::create_dir_all(root.join("agents")).unwrap();
        std::fs::write(
            root.join("agents/nightly.md"),
            format!(
                "---\nschedule: \"@daily\"\nruntime: host\ntool: claude\nwrite:\n  - reports/memory-health\n  - {elsewhere}\n---\nGo.\n"
            ),
        )
        .unwrap();
        apply_switch(&root, switch);
        baseline(&root);
        AgentHall {
            _guard: guard,
            root,
            remote,
            home,
            bin,
        }
    }

    impl AgentHall {
        /// The run's receipt, or `None` when this kernel has no Landlock.
        #[expect(clippy::print_stderr, reason = "a Landlock-less kernel skips visibly")]
        fn run(&self) -> Option<std::process::Output> {
            let output = ivar()
                .current_dir(&self.root)
                .env("HOME", self.home.as_str())
                .env(
                    "PATH",
                    format!("{}:{}", self.bin, std::env::var("PATH").unwrap()),
                )
                .args(["--json", "agent", "run", "nightly"])
                .output()
                .unwrap();
            if String::from_utf8_lossy(&output.stdout).contains("Landlock") {
                eprintln!("skipped: this kernel has no Landlock");
                return None;
            }
            Some(output)
        }
    }

    fn receipt(output: &std::process::Output) -> serde_json::Value {
        serde_json::from_slice(&output.stdout).unwrap()
    }

    #[test]
    fn a_successful_run_commits_what_it_wrote_under_its_hall_roots_and_pushes() {
        let hall = agent_hall(0, Switch::AllOn);
        let Some(output) = hall.run() else { return };

        let receipt = receipt(&output);
        assert_eq!(receipt["outcome"], "succeeded", "{receipt}");
        let day = receipt["started_at"].as_str().unwrap()[..10].to_owned();
        let root = &hall.root;
        assert_eq!(
            subjects(root)[..2],
            [
                format!("docs(nightly): Record the {day} run"),
                "baseline".to_owned()
            ]
        );
        assert_eq!(touched(root, "HEAD"), ["reports/memory-health/run.md"]);
        assert_eq!(head(root), branch_tip(&hall.remote));
        assert_eq!(pending(root), "");
    }

    #[test]
    fn a_failed_run_syncs_nothing() {
        let hall = agent_hall(1, Switch::AllOn);
        let before = head(&hall.root);
        let Some(output) = hall.run() else { return };

        assert_eq!(receipt(&output)["outcome"], "failed");
        assert_eq!(head(&hall.root), before);
        assert!(pending(&hall.root).contains("reports/memory-health/run.md"));
        assert_eq!(branch_tip(&hall.remote), before);
    }

    #[test]
    fn a_run_with_auto_commit_off_commits_nothing() {
        let hall = agent_hall(0, Switch::NoCommit);
        let before = head(&hall.root);
        let Some(output) = hall.run() else { return };

        assert_eq!(receipt(&output)["outcome"], "succeeded");
        assert_eq!(head(&hall.root), before);
        assert!(pending(&hall.root).contains("reports/memory-health/run.md"));
    }

    #[test]
    fn a_run_with_auto_push_off_commits_but_does_not_push() {
        let hall = agent_hall(0, Switch::NoPush);
        let before = head(&hall.root);
        let Some(output) = hall.run() else { return };

        assert_eq!(receipt(&output)["outcome"], "succeeded");
        assert_ne!(head(&hall.root), before);
        assert_eq!(branch_tip(&hall.remote), before);
    }

    #[test]
    fn a_sync_warning_goes_to_the_receipt_and_not_to_the_outcome() {
        let hall = agent_hall(0, Switch::AllOn);
        git(&hall.root, &["branch", "--unset-upstream"]);
        let Some(output) = hall.run() else { return };

        let receipt = receipt(&output);
        assert_eq!(receipt["outcome"], "succeeded", "{receipt}");
        assert_eq!(output.status.code(), Some(0));
        assert!(
            receipt["sync_warnings"][0]
                .as_str()
                .unwrap()
                .starts_with("hall.push_skipped"),
            "{receipt}"
        );
    }
}
