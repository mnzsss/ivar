#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! `sync_hall` against real git: a hall with a bare remote and, when a test
//! needs another machine, a second clone.

use std::cell::Cell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

use super::*;
use crate::action::agent::run::HostDeps;
use crate::action::hall::{InitInput, init};
use crate::error::Failure;
use crate::git::{Git, System};
use crate::test_support::hall_root;

/// A hall on `main` with a pushed first commit, and the bare remote it tracks.
struct Fixture {
    _guard: TempDir,
    root: Utf8PathBuf,
    remote: Utf8PathBuf,
    layout: Layout,
}

fn fixture() -> Fixture {
    let (guard, root) = hall_root();
    let remote = root.parent().unwrap().join("remote.git");
    run(
        root.parent().unwrap(),
        &[
            "init",
            "--bare",
            "--initial-branch",
            "main",
            remote.as_str(),
        ],
    );

    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: None,
            git: true,
            origin: None,
        },
    )
    .unwrap();
    configure(&root);
    let ignore = root.join(".gitignore");
    let mut lines = std::fs::read_to_string(&ignore).unwrap();
    lines.push_str(".env\n");
    std::fs::write(&ignore, lines).unwrap();
    run(&root, &["add", "-A"]);
    run(&root, &["commit", "-m", "first"]);
    run(&root, &["remote", "add", "origin", remote.as_str()]);
    run(&root, &["push", "-u", "origin", "main"]);

    let layout = Layout::at(root.clone());
    Fixture {
        _guard: guard,
        root,
        remote,
        layout,
    }
}

fn configure(dir: &Utf8Path) {
    run(dir, &["config", "user.name", "ivar tests"]);
    run(dir, &["config", "user.email", "tests@ivar.invalid"]);
    run(dir, &["config", "commit.gpgsign", "false"]);
    run(dir, &["config", "core.hooksPath", ".git/hooks"]);
}

/// git in `cwd`, returning its stdout and panicking on failure.
fn run(cwd: &Utf8Path, args: &[&str]) -> String {
    let (ok, stdout, stderr) = run_raw(cwd, args);
    assert!(ok, "git {} failed in {cwd}: {stderr}", args.join(" "));
    stdout
}

fn run_raw(cwd: &Utf8Path, args: &[&str]) -> (bool, String, String) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
        String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned(),
    )
}

fn write(root: &Utf8Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn read(root: &Utf8Path, relative: &str) -> String {
    std::fs::read_to_string(root.join(relative)).unwrap()
}

/// Commit subjects on `branch` in `dir`, newest first.
fn subjects(dir: &Utf8Path, rev: &str) -> Vec<String> {
    run(dir, &["log", "--format=%s", rev])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The paths a commit touched.
fn touched(dir: &Utf8Path, rev: &str) -> Vec<String> {
    let mut paths: Vec<String> = run(dir, &["show", "--format=", "--name-only", rev])
        .lines()
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths
}

/// A second clone of the remote: another machine.
fn other_machine(fixture: &Fixture) -> Utf8PathBuf {
    let dir = fixture.root.parent().unwrap().join("other");
    run(
        fixture.root.parent().unwrap(),
        &["clone", fixture.remote.as_str(), dir.as_str()],
    );
    configure(&dir);
    dir
}

/// The other machine commits and pushes `content` to `relative`.
fn push_from_other(fixture: &Fixture, relative: &str, content: &str) {
    let other = other_machine(fixture);
    write(&other, relative, content);
    run(&other, &["add", "-A"]);
    run(&other, &["commit", "-m", "other machine"]);
    run(&other, &["push", "origin", "main"]);
}

/// What a test's host answers when the conflict-resolving agent must not run.
fn no_agent_host() -> Result<HostDeps, Failure> {
    Err(Failure::failed(
        "test.no_agent",
        "no agent runs in this test",
    ))
}

fn sync_manual(fixture: &Fixture) -> HallSync {
    sync_hall_with(
        &fixture.layout,
        &System,
        &SyncRequest::manual(),
        &resolver::Env {
            host: &no_agent_host,
            timeout: Duration::from_secs(5),
        },
    )
}

fn codes(sync: &HallSync) -> Vec<&'static str> {
    sync.warnings.iter().map(|warning| warning.code).collect()
}

fn operation<'a>(paths: &'a [Utf8PathBuf], message: &'a str) -> SyncRequest<'a> {
    SyncRequest {
        trigger: Trigger::Manual,
        operation: Some(OperationCommit { paths, message }),
    }
}

fn install_hook(root: &Utf8Path, script: &str) {
    let hook = root.join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn set_manifest_flag(layout: &Layout, key: &str, value: bool) {
    let path = layout.root().join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest[key] = serde_json::json!(value);
    std::fs::write(path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
}

// -- the general commit ----------------------------------------------------

#[test]
fn everything_pending_is_committed_and_pushed_except_what_is_ignored() {
    let f = fixture();
    write(&f.root, "docs/memory/page.md", "new page\n");
    write(&f.root, "ivar.json.bak", "modified later\n");
    write(&f.root, ".env", "SECRET=1\n");
    write(&f.root, ".ivar/state.json", "{}\n");
    write(&f.root, ".ivar/skills/state.json", "{}\n");
    write(&f.root, "kept.md", "to be deleted\n");
    run(&f.root, &["add", "kept.md"]);
    run(&f.root, &["commit", "-m", "kept"]);
    run(&f.root, &["push", "origin", "main"]);
    std::fs::remove_file(f.root.join("kept.md")).unwrap();
    write(&f.root, "HALL.md", "edited\n");
    write(&f.root, "staged.md", "staged\n");
    run(&f.root, &["add", "staged.md"]);

    let sync = sync_manual(&f);

    assert!(sync.general_committed && sync.pushed, "{sync:?}");
    assert!(!sync.operation_committed);
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    let subjects = subjects(&f.root, "HEAD");
    assert_eq!(subjects[0], GENERAL_MESSAGE);
    assert_eq!(
        touched(&f.root, "HEAD"),
        [
            ".ivar/skills/state.json",
            "HALL.md",
            "docs/memory/page.md",
            "ivar.json.bak",
            "kept.md",
            "staged.md",
        ]
    );
    assert!(run(&f.root, &["status", "--porcelain"]).is_empty());
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
}

#[test]
fn a_dotenv_is_never_committed_even_when_gitignore_forgot_it() {
    let f = fixture();
    std::fs::write(f.root.join(".gitignore"), ".ivar/*\n").unwrap();
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "drop the .env line"]);
    write(&f.root, ".env", "SECRET=1\n");
    write(&f.root, "sub/.env", "SECRET=2\n");
    write(&f.root, "page.md", "x\n");

    let sync = sync_manual(&f);

    assert!(sync.general_committed);
    assert_eq!(touched(&f.root, "HEAD"), ["page.md"]);
    let tracked = run(&f.root, &["ls-files"]);
    assert!(!tracked.contains(".env"), "{tracked}");
}

#[test]
fn nothing_pending_means_no_commit_no_push_and_no_warning() {
    let f = fixture();
    let before = run(&f.root, &["rev-parse", "HEAD"]);

    let sync = sync_manual(&f);

    assert!(!sync.general_committed && !sync.operation_committed && !sync.pushed);
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert_eq!(run(&f.root, &["rev-parse", "HEAD"]), before);
}

#[test]
fn a_tree_that_only_differs_by_ignored_files_is_nothing_pending() {
    let f = fixture();
    write(&f.root, ".env", "x\n");
    write(&f.root, ".ivar/graph.db", "x\n");

    let sync = sync_manual(&f);

    assert!(!sync.general_committed, "{sync:?}");
    assert!(sync.warnings.is_empty());
}

// -- the operation's commit --------------------------------------------------

#[test]
fn the_operation_commit_holds_only_its_paths_and_the_general_commit_the_rest() {
    let f = fixture();
    write(&f.root, "docs/memory/a.md", "a\n");
    write(&f.root, "docs/memory/b.md", "b\n");
    write(&f.root, "other.md", "other\n");
    write(&f.root, "staged.md", "staged by the user\n");
    run(&f.root, &["add", "staged.md"]);
    let paths = vec![
        Utf8PathBuf::from("docs/memory/a.md"),
        Utf8PathBuf::from("docs/memory/b.md"),
    ];

    let sync = sync_hall(
        &f.layout,
        &System,
        &operation(&paths, "feat(memory): Add two pages"),
    );

    assert!(sync.operation_committed && sync.general_committed && sync.pushed);
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert_eq!(
        subjects(&f.root, "HEAD")[..2],
        [GENERAL_MESSAGE, "feat(memory): Add two pages"]
    );
    assert_eq!(
        touched(&f.root, "HEAD~1"),
        ["docs/memory/a.md", "docs/memory/b.md"]
    );
    // The user's staged file is the general commit's, not the operation's.
    assert_eq!(touched(&f.root, "HEAD"), ["other.md", "staged.md"]);
}

#[test]
fn the_operation_commit_leaves_what_the_user_staged_for_other_paths_staged() {
    let f = fixture();
    write(&f.root, "op.md", "op\n");
    write(&f.root, "staged.md", "staged\n");
    run(&f.root, &["add", "staged.md"]);
    // Refuse the general commit so that what is left can be looked at.
    install_hook(
        &f.root,
        "git diff --cached --name-only | grep -q '^staged' && echo refused >&2 && exit 1; exit 0",
    );
    let paths = vec![Utf8PathBuf::from("op.md")];

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "chore: op"));

    assert!(sync.operation_committed);
    assert!(!sync.general_committed);
    assert_eq!(touched(&f.root, "HEAD"), ["op.md"]);
    assert_eq!(
        run(&f.root, &["diff", "--cached", "--name-only"]),
        "staged.md"
    );
}

#[test]
fn an_operation_deleting_a_tracked_file_commits_the_deletion() {
    let f = fixture();
    write(&f.root, "gone.md", "x\n");
    run(&f.root, &["add", "gone.md"]);
    run(&f.root, &["commit", "-m", "add gone"]);
    run(&f.root, &["push", "origin", "main"]);
    std::fs::remove_file(f.root.join("gone.md")).unwrap();
    let paths = vec![Utf8PathBuf::from("gone.md")];

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "chore: remove"));

    assert!(sync.operation_committed && !sync.general_committed);
    assert_eq!(subjects(&f.root, "HEAD")[0], "chore: remove");
    assert_eq!(touched(&f.root, "HEAD"), ["gone.md"]);
}

#[test]
fn operation_paths_that_are_ignored_outside_the_hall_or_unknown_never_enter_a_commit() {
    let f = fixture();
    write(&f.root, ".env", "SECRET=1\n");
    write(&f.root, ".ivar/state.json", "{}\n");
    let outside = f.root.parent().unwrap().join("outside.md");
    std::fs::write(&outside, "outside\n").unwrap();
    let paths = vec![
        Utf8PathBuf::from(".env"),
        Utf8PathBuf::from(".ivar/state.json"),
        Utf8PathBuf::from("never-existed.md"),
        Utf8PathBuf::from("../outside.md"),
        outside,
        f.root.join(".env"),
    ];
    let before = run(&f.root, &["rev-parse", "HEAD"]);

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "chore: nothing"));

    assert!(
        !sync.operation_committed && !sync.general_committed,
        "{sync:?}"
    );
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert_eq!(run(&f.root, &["rev-parse", "HEAD"]), before);
}

#[test]
fn an_absolute_path_inside_the_hall_is_taken_as_relative() {
    let f = fixture();
    write(&f.root, "docs/a.md", "a\n");
    let paths = vec![f.root.join("docs/a.md")];

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "docs: a"));

    assert!(sync.operation_committed);
    assert_eq!(touched(&f.root, "HEAD"), ["docs/a.md"]);
}

#[test]
fn an_operation_without_paths_makes_no_operation_commit() {
    let f = fixture();
    write(&f.root, "x.md", "x\n");

    let sync = sync_hall(&f.layout, &System, &operation(&[], "chore: nothing"));

    assert!(!sync.operation_committed && sync.general_committed);
}

// -- hooks ------------------------------------------------------------------

#[test]
fn a_refused_operation_commit_skips_the_general_one_and_still_pushes_what_exists() {
    let f = fixture();
    // An earlier, failed attempt left a commit behind.
    write(&f.root, "earlier.md", "e\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "earlier"]);
    write(&f.root, "op.md", "op\n");
    write(&f.root, "rest.md", "rest\n");
    install_hook(&f.root, "echo 'hook: file too large' >&2; exit 1");
    let paths = vec![Utf8PathBuf::from("op.md")];

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "chore: op"));

    assert_eq!(codes(&sync), ["hall.commit_failed"]);
    assert!(
        sync.warnings[0].what.contains("hook: file too large"),
        "{}",
        sync.warnings[0].what
    );
    assert!(!sync.operation_committed && !sync.general_committed);
    assert!(sync.pushed, "the existing commit goes up");
    assert_eq!(subjects(&f.root, "HEAD")[0], "earlier");
    // A refusal leaves the new file as it was: untracked, not half-staged.
    assert!(run(&f.root, &["status", "--porcelain"]).contains("?? op.md"));
    assert_eq!(run(&f.root, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn a_refused_general_commit_still_lets_the_operation_commit_and_the_push_through() {
    let f = fixture();
    write(&f.root, "op.md", "op\n");
    write(&f.root, "big.md", "big\n");
    install_hook(
        &f.root,
        "git diff --cached --name-only | grep -q '^big' && echo 'big.md is over 5000 tokens' >&2 && exit 1; exit 0",
    );
    let paths = vec![Utf8PathBuf::from("op.md")];

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "chore: op"));

    assert_eq!(codes(&sync), ["hall.commit_failed"]);
    assert!(sync.warnings[0].what.contains("over 5000 tokens"));
    assert!(sync.operation_committed && !sync.general_committed && sync.pushed);
    assert_eq!(touched(&f.root, "HEAD"), ["op.md"]);
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
}

// -- when nothing may be committed -----------------------------------------

fn assert_skipped_once(sync: &HallSync, why: &str) {
    assert_eq!(codes(sync), ["hall.commit_skipped"], "{:?}", sync.warnings);
    assert!(
        sync.warnings[0].what.contains(why),
        "{}",
        sync.warnings[0].what
    );
    assert!(!sync.operation_committed && !sync.general_committed && !sync.pushed);
}

#[test]
fn a_hall_that_is_not_a_git_repository_is_skipped_once() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: None,
            provider: None,
            git: false,
            origin: None,
        },
    )
    .unwrap();
    let paths = vec![Utf8PathBuf::from("ivar.json")];

    let sync = sync_hall(
        &Layout::at(root.clone()),
        &System,
        &operation(&paths, "chore: op"),
    );

    assert_skipped_once(&sync, "not a git repository");
    assert!(!root.join(".git").exists());
}

#[test]
fn a_detached_head_is_skipped_once() {
    let f = fixture();
    run(&f.root, &["checkout", "--detach"]);
    write(&f.root, "x.md", "x\n");
    let paths = vec![Utf8PathBuf::from("x.md")];

    let sync = sync_hall(&f.layout, &System, &operation(&paths, "chore: op"));

    assert_skipped_once(&sync, "detached");
    assert_eq!(run(&f.root, &["status", "--porcelain"]), "?? x.md");
}

/// Both sides change `clash.md`: the other machine's version is on the remote,
/// the hall's is a local commit.
fn diverge(f: &Fixture) {
    write(&f.root, "clash.md", "base\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "base"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(f, "clash.md", "remote\n");
    write(&f.root, "clash.md", "local\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "local clash"]);
}

#[test]
fn a_merge_in_progress_is_skipped_once() {
    let f = fixture();
    diverge(&f);
    run(&f.root, &["fetch", "origin"]);
    let (ok, ..) = run_raw(&f.root, &["merge", "origin/main"]);
    assert!(!ok);
    write(&f.root, "unrelated.md", "u\n");

    let sync = sync_manual(&f);

    assert_skipped_once(&sync, "merge");
}

#[test]
fn a_rebase_in_progress_is_skipped_once() {
    let f = fixture();
    diverge(&f);
    run(&f.root, &["fetch", "origin"]);
    let (ok, ..) = run_raw(&f.root, &["rebase", "origin/main"]);
    assert!(!ok);

    let sync = sync_manual(&f);

    assert_skipped_once(&sync, "rebase");
}

#[test]
fn a_cherry_pick_in_progress_is_skipped_once() {
    let f = fixture();
    diverge(&f);
    run(&f.root, &["fetch", "origin"]);
    run(&f.root, &["reset", "--hard", "origin/main"]);
    let picked = {
        run(&f.root, &["checkout", "-q", "-b", "side", "HEAD~1"]);
        write(&f.root, "clash.md", "side\n");
        run(&f.root, &["commit", "-qam", "side clash"]);
        let sha = run(&f.root, &["rev-parse", "HEAD"]);
        run(&f.root, &["checkout", "-q", "main"]);
        sha
    };
    let (ok, ..) = run_raw(&f.root, &["cherry-pick", &picked]);
    assert!(!ok);

    let sync = sync_manual(&f);

    assert_skipped_once(&sync, "cherry-pick");
}

// -- the push -----------------------------------------------------------------

#[test]
fn commits_from_an_earlier_failed_attempt_are_pushed_without_new_changes() {
    let f = fixture();
    write(&f.root, "earlier.md", "e\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "earlier"]);

    let sync = sync_manual(&f);

    assert!(sync.pushed && !sync.general_committed);
    assert!(sync.warnings.is_empty());
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
}

#[test]
fn a_hall_without_an_upstream_commits_and_warns_that_the_push_was_skipped() {
    let f = fixture();
    run(&f.root, &["branch", "--unset-upstream"]);
    write(&f.root, "x.md", "x\n");

    let sync = sync_manual(&f);

    assert!(sync.general_committed && !sync.pushed);
    assert_eq!(codes(&sync), ["hall.push_skipped"]);
}

#[test]
fn an_unreachable_remote_is_a_warning_and_the_commits_stay_local() {
    let f = fixture();
    run(
        &f.root,
        &[
            "remote",
            "set-url",
            "origin",
            f.root.parent().unwrap().join("no-such-remote.git").as_str(),
        ],
    );
    write(&f.root, "x.md", "x\n");

    let sync = sync_manual(&f);

    assert!(sync.general_committed && !sync.pushed);
    assert_eq!(codes(&sync), ["hall.push_failed"]);
    assert_eq!(subjects(&f.root, "HEAD")[0], GENERAL_MESSAGE);
}

#[test]
fn a_remote_that_is_ahead_is_pulled_with_a_rebase_and_pushed_without_force() {
    let f = fixture();
    push_from_other(&f, "from-other.md", "o\n");
    write(&f.root, "mine.md", "m\n");

    let sync = sync_manual(&f);

    assert!(sync.general_committed && sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    let log = subjects(&f.root, "HEAD");
    assert_eq!(log[0], GENERAL_MESSAGE);
    assert_eq!(log[1], "other machine");
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
    // A rebase, not a merge: history stays linear.
    assert!(run(&f.root, &["rev-list", "--merges", "HEAD"]).is_empty());
}

// -- conflicts ----------------------------------------------------------------

#[test]
fn a_conflict_in_the_skill_state_keeps_the_remotes_copy() {
    let f = fixture();
    write(
        &f.root,
        ".ivar/skills/state.json",
        "{\"installations\":{}}\n",
    );
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "state"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(
        &f,
        ".ivar/skills/state.json",
        "{\"installations\":{},\"from\":\"remote\"}\n",
    );
    write(
        &f.root,
        ".ivar/skills/state.json",
        "{\"installations\":{},\"from\":\"local\"}\n",
    );
    write(&f.root, "mine.md", "m\n");

    let sync = sync_manual(&f);

    assert!(sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert!(read(&f.root, ".ivar/skills/state.json").contains("remote"));
    assert!(run(&f.root, &["status", "--porcelain"]).is_empty());
    assert!(!crate::git::read::is_rebase_in_progress(&f.root));
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
    assert!(read(&f.root, "mine.md").contains('m'));
}

#[test]
fn a_dated_run_file_with_the_same_content_keeps_one_copy() {
    let f = fixture();
    push_from_other(&f, "reports/memory-health/2026-11-09.md", "same\n");
    write(&f.root, "reports/memory-health/2026-11-09.md", "same\n");
    write(&f.root, "mine.md", "m\n");

    let sync = sync_manual(&f);

    assert!(sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    let files = run(&f.root, &["ls-files", "reports"]);
    assert_eq!(files, "reports/memory-health/2026-11-09.md");
}

#[test]
fn a_dated_run_file_with_other_content_keeps_the_remotes_and_saves_the_locals_beside_it() {
    let f = fixture();
    write(&f.root, "reports/memory-health/2026-11-09.md", "base\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "base report"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(&f, "reports/memory-health/2026-11-09.md", "remote run\n");
    write(
        &f.root,
        "reports/memory-health/2026-11-09.md",
        "local run\n",
    );
    write(
        &f.root,
        "brand/metricas/2026-11-09.json",
        "{\"local\":true}\n",
    );

    let sync = sync_manual(&f);

    assert!(sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert_eq!(
        read(&f.root, "reports/memory-health/2026-11-09.md"),
        "remote run\n"
    );
    let saved: Vec<String> = run(&f.root, &["ls-files", "reports"])
        .lines()
        .filter(|path| path.contains("2026-11-09-"))
        .map(str::to_owned)
        .collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert!(saved[0].ends_with(".md"));
    assert_eq!(read(&f.root, &saved[0]), "local run\n");
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
    // The other file in the same commit was not a conflict and still went up.
    assert!(
        run(&f.remote, &["ls-tree", "-r", "--name-only", "main"])
            .contains("brand/metricas/2026-11-09.json")
    );
}

#[test]
fn a_conflict_anywhere_else_aborts_the_rebase_and_restores_the_hall() {
    let f = fixture();
    write(&f.root, "notes.md", "base\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "base notes"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(&f, "notes.md", "remote prose\n");
    write(&f.root, "notes.md", "local prose\n");
    write(&f.root, "extra.md", "extra\n");
    let remote_before = run(&f.remote, &["rev-parse", "main"]);

    let sync = sync_manual(&f);

    assert_eq!(codes(&sync), ["hall.push_failed"], "{:?}", sync.warnings);
    assert!(sync.warnings[0].what.contains("notes.md"));
    assert!(!sync.pushed);
    assert!(!crate::git::read::is_rebase_in_progress(&f.root));
    // As before the pull: both commits are local, on the old base, files intact.
    let log = subjects(&f.root, "HEAD");
    assert_eq!(log[0], GENERAL_MESSAGE);
    assert_eq!(log[1], "base notes");
    assert!(!log.contains(&"other machine".to_owned()));
    assert_eq!(read(&f.root, "notes.md"), "local prose\n");
    assert!(run(&f.root, &["status", "--porcelain"]).is_empty());
    assert_eq!(run(&f.remote, &["rev-parse", "main"]), remote_before);
}

// -- conflicts the agent settles -----------------------------------------------

/// What a sync with the agent needs: a `claude` that runs `body`, and the
/// launcher a host run puts in front of it, which here only runs what follows
/// `--` without Landlock. The agent's own prompt is kept in `prompt.txt`.
struct Agent {
    bin: Utf8PathBuf,
    home: Utf8PathBuf,
}

impl Agent {
    fn install(f: &Fixture, body: &str) -> Self {
        let dir = f.root.parent().unwrap();
        let bin = dir.join("agent-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = |name: &str, text: String| {
            let path = bin.join(name);
            std::fs::write(&path, text).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        script(
            "sandbox",
            "#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n"
                .to_owned(),
        );
        script(
            "claude",
            format!(
                "#!/bin/sh\nfor last; do :; done\nprintf '%s' \"$last\" > '{bin}/prompt.txt'\n{body}\necho resolved\n"
            ),
        );
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Self { bin, home }
    }

    fn host(&self) -> Result<HostDeps, Failure> {
        let path = format!("{}:{}", self.bin, std::env::var("PATH").unwrap());
        Ok(HostDeps {
            launcher: self.bin.join("sandbox"),
            home: self.home.clone(),
            parent_env: vec![
                ("PATH".to_owned(), path),
                ("HOME".to_owned(), self.home.to_string()),
            ],
            landlock: true,
        })
    }

    fn sync(&self, f: &Fixture, timeout: Duration) -> HallSync {
        sync_hall_with(
            &f.layout,
            &System,
            &SyncRequest::manual(),
            &resolver::Env {
                host: &|| self.host(),
                timeout,
            },
        )
    }

    fn prompt(&self) -> String {
        std::fs::read_to_string(self.bin.join("prompt.txt")).unwrap()
    }

    fn ran(&self) -> bool {
        self.bin.join("prompt.txt").exists()
    }
}

const PAGE: &str = "docs/memory/gotcha/fts.md";
const PAGE_BASE: &str = "---\ntitle: FTS\ntags: [index]\n---\n# FTS\n\nbase line\n";
const PAGE_REMOTE: &str = "---\ntitle: FTS\ntags: [index]\n---\n# FTS\n\nbase line\nremote note\n";
const PAGE_LOCAL: &str = "---\ntitle: FTS\ntags: [index]\n---\n# FTS\n\nbase line\nlocal note\n";
const PAGE_MERGED: &str =
    "---\ntitle: FTS\ntags: [index]\n---\n# FTS\n\nbase line\nremote note\nlocal note\n";

/// A shell snippet that writes `text` to `relative` in the hall.
fn writes(f: &Fixture, relative: &str, text: &str) -> String {
    format!(
        "printf '%s' '{}' > '{}/{relative}'\n",
        text.replace('\'', "'\\''"),
        f.root
    )
}

/// The remote and this hall both changed the memory page after a shared base,
/// and the hall has a second, unrelated change pending.
fn page_conflict(f: &Fixture) {
    write(&f.root, PAGE, PAGE_BASE);
    write(&f.root, "extra.md", "extra base\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "base page"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(f, PAGE, PAGE_REMOTE);
    write(&f.root, PAGE, PAGE_LOCAL);
}

/// The sync gave up with one `hall.push_failed` mentioning `needle`, and the
/// hall is as it was after its own commit and before the pull: the local
/// commit on the old base, the remote untouched, no rebase left behind.
fn assert_aborted(f: &Fixture, sync: &HallSync, remote_before: &str, needle: &str) {
    assert_eq!(codes(sync), ["hall.push_failed"], "{:?}", sync.warnings);
    assert!(
        sync.warnings[0].what.contains(needle),
        "{needle:?} not in {}",
        sync.warnings[0].what
    );
    assert!(!sync.pushed && sync.resolved.is_empty(), "{sync:?}");
    assert!(!crate::git::read::is_rebase_in_progress(&f.root));
    let log = subjects(&f.root, "HEAD");
    assert_eq!(log[0], GENERAL_MESSAGE);
    assert_eq!(log[1], "base page");
    assert!(!log.contains(&"other machine".to_owned()));
    assert_eq!(read(&f.root, PAGE), PAGE_LOCAL);
    assert_eq!(run(&f.remote, &["rev-parse", "main"]), remote_before);
}

#[test]
fn a_prose_conflict_is_settled_by_the_agent_checked_and_pushed_with_a_trailer() {
    let f = fixture();
    page_conflict(&f);
    let agent = Agent::install(&f, &writes(&f, PAGE, PAGE_MERGED));

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert!(sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert_eq!(sync.resolved, [Utf8PathBuf::from(PAGE)]);
    assert_eq!(read(&f.root, PAGE), PAGE_MERGED);
    assert!(!crate::git::read::is_rebase_in_progress(&f.root));
    assert!(run(&f.root, &["status", "--porcelain"]).is_empty());
    assert_eq!(
        run(&f.remote, &["rev-parse", "main"]),
        run(&f.root, &["rev-parse", "HEAD"])
    );
    assert!(run(&f.root, &["rev-list", "--merges", "HEAD"]).is_empty());
    // The replayed commit keeps its own subject and gains the trailer; the
    // other machine's commit below it is untouched.
    let log = subjects(&f.root, "HEAD");
    assert_eq!(log[0], GENERAL_MESSAGE);
    assert_eq!(log[1], "other machine");
    let trailer = |rev: &str| {
        run(
            &f.root,
            &[
                "log",
                "-1",
                "--format=%(trailers:key=Resolved-by,valueonly)",
                rev,
            ],
        )
    };
    assert_eq!(trailer("HEAD"), "ivar hall-sync-resolver");
    assert_eq!(trailer("HEAD~"), "");
}

#[test]
fn the_agent_is_given_both_sides_and_each_commit_and_only_what_the_rules_left() {
    let f = fixture();
    write(&f.root, ".ivar/skills/state.json", "{\"a\":0}\n");
    write(&f.root, PAGE, "base text\n");
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "base page"]);
    run(&f.root, &["push", "origin", "main"]);
    let other = other_machine(&f);
    write(&other, ".ivar/skills/state.json", "{\"a\":\"remote\"}\n");
    write(&other, PAGE, "remote text\n");
    run(&other, &["add", "-A"]);
    run(&other, &["commit", "-m", "remote edit"]);
    run(&other, &["push", "origin", "main"]);
    write(&f.root, ".ivar/skills/state.json", "{\"a\":\"local\"}\n");
    write(&f.root, PAGE, "local text\n");
    let agent = Agent::install(&f, &writes(&f, PAGE, PAGE_MERGED));

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert!(sync.pushed, "{sync:?}");
    assert_eq!(sync.resolved, [Utf8PathBuf::from(PAGE)]);
    let prompt = agent.prompt();
    assert!(prompt.contains(PAGE), "{prompt}");
    assert!(prompt.contains("remote text") && prompt.contains("local text"));
    assert!(
        prompt.contains("remote edit"),
        "the remote's commit: {prompt}"
    );
    assert!(
        prompt.contains(GENERAL_MESSAGE),
        "the local commit: {prompt}"
    );
    assert!(
        !prompt.contains("state.json"),
        "a file a rule settled is not sent: {prompt}"
    );
    assert!(read(&f.root, ".ivar/skills/state.json").contains("remote"));
}

#[test]
fn the_hall_notify_channel_hears_which_files_were_resolved() {
    let f = fixture();
    page_conflict(&f);
    let (url, received) = one_shot_server();
    write(
        &f.root,
        "agents/nightly.md",
        &format!("---\nschedule: \"@daily\"\ntool: claude\nnotify: {url}\n---\nGo.\n"),
    );
    let agent = Agent::install(&f, &writes(&f, PAGE, PAGE_MERGED));

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert!(sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    let body = received.join().unwrap();
    assert!(body.contains("hall-sync-resolver"), "{body}");
    assert!(body.contains(PAGE), "{body}");
}

#[test]
fn a_notify_channel_that_is_down_is_a_warning_and_the_resolution_stands() {
    let f = fixture();
    page_conflict(&f);
    // Nothing listens here.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", closed.local_addr().unwrap());
    drop(closed);
    write(
        &f.root,
        "agents/nightly.md",
        &format!("---\nschedule: \"@daily\"\ntool: claude\nnotify: {url}\n---\nGo.\n"),
    );
    let agent = Agent::install(&f, &writes(&f, PAGE, PAGE_MERGED));

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert!(sync.pushed, "{sync:?}");
    assert_eq!(codes(&sync), ["hall.notify_failed"], "{:?}", sync.warnings);
    assert_eq!(sync.resolved, [Utf8PathBuf::from(PAGE)]);
}

/// Every way the agent's answer can be refused ends the same: nothing is
/// committed, nothing is pushed, and the hall is as it was.
fn refused_by(body_for: impl FnOnce(&Fixture) -> String, timeout: Duration, needle: &str) {
    let f = fixture();
    page_conflict(&f);
    let remote_before = run(&f.remote, &["rev-parse", "main"]);
    let agent = Agent::install(&f, &body_for(&f));

    let sync = agent.sync(&f, timeout);

    assert!(agent.ran(), "the agent was never run");
    assert_aborted(&f, &sync, &remote_before, needle);
    assert!(run(&f.root, &["status", "--porcelain"]).is_empty());
}

#[test]
fn a_marker_the_agent_leaves_aborts_the_rebase_and_restores_the_hall() {
    // Not touched at all: git's own markers are still in the file.
    refused_by(
        |_| "true".to_owned(),
        Duration::from_secs(30),
        "conflict marker",
    );
    refused_by(
        |f| writes(f, PAGE, "---\ntitle: FTS\n---\nkept\n=======\nboth\n"),
        Duration::from_secs(30),
        "conflict marker",
    );
}

#[test]
fn a_tracked_file_outside_the_conflict_that_the_agent_changes_aborts_the_rebase() {
    refused_by(
        |f| {
            format!(
                "{}{}",
                writes(f, PAGE, PAGE_MERGED),
                writes(f, "extra.md", "rewritten\n")
            )
        },
        Duration::from_secs(30),
        "extra.md",
    );
}

#[test]
fn a_new_file_the_agent_creates_aborts_the_rebase() {
    let f = fixture();
    page_conflict(&f);
    let remote_before = run(&f.remote, &["rev-parse", "main"]);
    let body = format!(
        "{}{}",
        writes(&f, PAGE, PAGE_MERGED),
        writes(&f, "stray.md", "x\n")
    );
    let agent = Agent::install(&f, &body);

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert_aborted(&f, &sync, &remote_before, "stray.md");
}

#[test]
fn a_page_with_broken_frontmatter_aborts_the_rebase() {
    refused_by(
        |f| writes(f, PAGE, "---\ntitle: [unclosed\n---\n# FTS\n\nmerged\n"),
        Duration::from_secs(30),
        "invalid memory page",
    );
}

#[test]
fn an_agent_that_fails_aborts_the_rebase() {
    refused_by(
        |f| {
            format!(
                "{}echo 'cannot decide' >&2\nexit 3\n",
                writes(f, PAGE, PAGE_MERGED)
            )
        },
        Duration::from_secs(30),
        "cannot decide",
    );
}

#[test]
fn an_agent_that_runs_out_of_time_aborts_the_rebase() {
    refused_by(
        |_| "sleep 30".to_owned(),
        Duration::from_secs(1),
        "did not finish",
    );
}

#[test]
fn a_hook_that_refuses_the_resolved_commit_aborts_the_rebase_and_is_never_skipped() {
    let f = fixture();
    page_conflict(&f);
    // Passes the hall's own commits, refuses what the agent wrote.
    install_hook(
        &f.root,
        "git diff --cached | grep -q 'agent was here' && { echo 'hook says no' >&2; exit 1; }\nexit 0",
    );
    let remote_before = run(&f.remote, &["rev-parse", "main"]);
    let agent = Agent::install(
        &f,
        &writes(&f, PAGE, &format!("{PAGE_MERGED}agent was here\n")),
    );

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert!(agent.ran());
    assert_aborted(&f, &sync, &remote_before, "hook says no");
}

#[test]
fn a_conflict_only_in_files_the_rules_cover_never_runs_the_agent() {
    let f = fixture();
    write(
        &f.root,
        ".ivar/skills/state.json",
        "{\"installations\":{}}\n",
    );
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "state"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(&f, ".ivar/skills/state.json", "{\"from\":\"remote\"}\n");
    write(&f.root, ".ivar/skills/state.json", "{\"from\":\"local\"}\n");
    write(&f.root, "mine.md", "m\n");
    let asked = Cell::new(false);
    let host = || {
        asked.set(true);
        no_agent_host()
    };

    let sync = sync_hall_with(
        &f.layout,
        &System,
        &SyncRequest::manual(),
        &resolver::Env {
            host: &host,
            timeout: Duration::from_secs(5),
        },
    );

    assert!(sync.pushed, "{sync:?}");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert!(sync.resolved.is_empty());
    assert!(!asked.get(), "the agent was started for a rule's file");
}

#[test]
fn auto_resolve_off_keeps_the_agent_out_and_the_rules_in() {
    let f = fixture();
    page_conflict(&f);
    let remote_before = run(&f.remote, &["rev-parse", "main"]);
    set_manifest_flag(&f.layout, "auto_resolve", false);
    let agent = Agent::install(&f, &writes(&f, PAGE, PAGE_MERGED));

    let sync = agent.sync(&f, Duration::from_secs(30));

    assert!(!agent.ran(), "the agent ran with auto_resolve off");
    assert_aborted(&f, &sync, &remote_before, "auto_resolve");

    // The fixed rules do not depend on the switch.
    let g = fixture();
    write(
        &g.root,
        ".ivar/skills/state.json",
        "{\"installations\":{}}\n",
    );
    run(&g.root, &["add", "-A"]);
    run(&g.root, &["commit", "-m", "state"]);
    run(&g.root, &["push", "origin", "main"]);
    push_from_other(&g, ".ivar/skills/state.json", "{\"from\":\"remote\"}\n");
    write(&g.root, ".ivar/skills/state.json", "{\"from\":\"local\"}\n");
    write(&g.root, "mine.md", "m\n");
    set_manifest_flag(&g.layout, "auto_resolve", false);
    assert!(sync_manual(&g).pushed);
}

/// A web server that answers one request and returns what it was sent.
fn one_shot_server() -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&request).into_owned();
            let Some(head_end) = text.find("\r\n\r\n") else {
                continue;
            };
            let length = text[..head_end]
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= head_end + 4 + length {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    (url, handle)
}

#[test]
fn a_remote_ahead_over_a_refused_tracked_change_is_not_pulled() {
    let f = fixture();
    write(&f.root, "earlier.md", "e\n");
    run(&f.root, &["add", "earlier.md"]);
    run(&f.root, &["commit", "-m", "earlier"]);
    write(&f.root, "HALL.md", "local edit\n");
    run(&f.root, &["add", "HALL.md"]);
    install_hook(&f.root, "echo 'refused by hook' >&2; exit 1");
    push_from_other(&f, "from-other.md", "o\n");

    let sync = sync_manual(&f);

    assert_eq!(
        codes(&sync),
        ["hall.commit_failed", "hall.push_failed"],
        "{:?}",
        sync.warnings
    );
    assert!(sync.warnings[1].what.contains("nothing was pulled"));
    assert_eq!(read(&f.root, "HALL.md"), "local edit\n");
    assert!(run(&f.root, &["diff", "--cached", "--name-only"]).contains("HALL.md"));
}

// -- options and the lock -----------------------------------------------------

#[test]
fn auto_commit_off_stops_an_automatic_run_but_not_a_manual_one() {
    let f = fixture();
    set_manifest_flag(&f.layout, "auto_commit", false);
    run(&f.root, &["commit", "-qam", "manifest"]);
    run(&f.root, &["push", "origin", "main"]);
    write(&f.root, "x.md", "x\n");
    let paths = vec![Utf8PathBuf::from("x.md")];
    let automatic = SyncRequest {
        trigger: Trigger::Automatic,
        operation: Some(OperationCommit {
            paths: &paths,
            message: "chore: op",
        }),
    };

    let sync = sync_hall(&f.layout, &System, &automatic);

    assert!(!sync.operation_committed && !sync.general_committed && !sync.pushed);
    assert!(sync.warnings.is_empty());
    assert_eq!(run(&f.root, &["status", "--porcelain"]), "?? x.md");

    let manual = sync_manual(&f);
    assert!(manual.general_committed && manual.pushed, "{manual:?}");
}

#[test]
fn auto_push_off_commits_but_does_not_push_an_automatic_run() {
    let f = fixture();
    set_manifest_flag(&f.layout, "auto_push", false);
    write(&f.root, "x.md", "x\n");
    let before = run(&f.remote, &["rev-parse", "main"]);
    let automatic = SyncRequest {
        trigger: Trigger::Automatic,
        operation: None,
    };

    let sync = sync_hall(&f.layout, &System, &automatic);

    assert!(sync.general_committed && !sync.pushed);
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert_eq!(run(&f.remote, &["rev-parse", "main"]), before);

    let manual = sync_manual(&f);
    assert!(manual.pushed, "a manual sync ignores auto_push");
}

#[test]
fn an_automatic_run_with_the_switches_absent_commits_and_pushes() {
    let f = fixture();
    write(&f.root, "x.md", "x\n");
    let automatic = SyncRequest {
        trigger: Trigger::Automatic,
        operation: None,
    };

    let sync = sync_hall(&f.layout, &System, &automatic);

    assert!(sync.general_committed && sync.pushed);
}

#[test]
fn a_second_run_that_cannot_take_the_lock_leaves_quietly() {
    let f = fixture();
    write(&f.root, "x.md", "x\n");
    let held = crate::infra::fs::try_lock_exclusive(&f.layout.hall_sync_lock())
        .unwrap()
        .expect("the lock is free");

    let sync = sync_manual(&f);

    assert!(sync.busy);
    assert!(sync.warnings.is_empty());
    assert!(!sync.general_committed);
    assert_eq!(run(&f.root, &["status", "--porcelain"]), "?? x.md");

    drop(held);
    assert!(sync_manual(&f).general_committed);
}

// -- state ---------------------------------------------------------------------

#[test]
fn the_state_reports_ahead_behind_and_a_stopped_operation() {
    let f = fixture();
    let state = sync_state(&f.layout, &System).unwrap();
    assert_eq!((state.ahead, state.behind), (0, 0));
    assert_eq!(state.branch.as_deref(), Some("main"));
    assert_eq!(state.upstream.as_ref().unwrap().name, "origin/main");

    diverge(&f);
    run(&f.root, &["fetch", "origin"]);
    let state = sync_state(&f.layout, &System).unwrap();
    assert_eq!((state.ahead, state.behind), (1, 1));
    assert!(state.diverged());
    assert_eq!(state.operation, None);

    let (ok, ..) = run_raw(&f.root, &["rebase", "origin/main"]);
    assert!(!ok);
    let state = sync_state(&f.layout, &System).unwrap();
    assert_eq!(state.operation, Some(crate::git::Operation::Rebase));
    assert!(state.skip_reason().is_some());
}

#[test]
fn the_state_of_a_directory_that_is_not_a_repository_is_an_answer_not_an_error() {
    let (_guard, root) = hall_root();

    let state = System.repo_state(&root).unwrap();

    assert!(!state.is_repository);
    assert_eq!(
        state.skip_reason(),
        Some(crate::git::SkipReason::NotARepository)
    );
}

#[test]
fn a_commit_whose_only_change_loses_to_the_remote_is_dropped_not_pushed_empty() {
    let f = fixture();
    write(
        &f.root,
        ".ivar/skills/state.json",
        "{\"installations\":{}}\n",
    );
    run(&f.root, &["add", "-A"]);
    run(&f.root, &["commit", "-m", "state"]);
    run(&f.root, &["push", "origin", "main"]);
    push_from_other(
        &f,
        ".ivar/skills/state.json",
        "{\"installations\":{},\"from\":\"remote\"}\n",
    );
    write(
        &f.root,
        ".ivar/skills/state.json",
        "{\"installations\":{},\"from\":\"local\"}\n",
    );

    let sync = sync_manual(&f);

    assert!(sync.general_committed);
    assert!(!sync.pushed, "nothing of the local commit survives");
    assert!(sync.warnings.is_empty(), "{:?}", sync.warnings);
    assert!(read(&f.root, ".ivar/skills/state.json").contains("remote"));
    assert_eq!(
        run(&f.root, &["rev-parse", "HEAD"]),
        run(&f.remote, &["rev-parse", "main"])
    );
    assert!(!crate::git::read::is_rebase_in_progress(&f.root));
}
