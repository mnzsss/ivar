#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Ending a discovery session syncs the hall; ending a feature session does not.

use std::process::Command as Git;

use super::*;

fn git(cwd: &Utf8PathBuf, args: &[&str]) -> String {
    let output = Git::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned()
}

/// A git hall on `main` with a pushed first commit, a bare remote it tracks,
/// and one live discovery session.
struct Fixture {
    _guard: tempfile::TempDir,
    root: Utf8PathBuf,
    remote: Utf8PathBuf,
    session: String,
}

fn start_discovery(ctx: &Ctx) -> String {
    session_start::start(
        ctx,
        StartInput {
            feature: None,
            resume: false,
            provider: None,
            detached: true,
            relay: false,
            prompt: None,
        },
    )
    .unwrap()
    .value
    .session_id
}

fn fixture() -> Fixture {
    let (guard, root) = hall_root();
    let parent = root.parent().unwrap().to_path_buf();
    let remote = parent.join("remote.git");
    git(
        &parent,
        &[
            "init",
            "--bare",
            "--initial-branch",
            "main",
            remote.as_str(),
        ],
    );
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
            git: true,
            origin: None,
        },
    )
    .unwrap();
    git(&root, &["config", "user.name", "ivar tests"]);
    git(&root, &["config", "user.email", "tests@ivar.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["config", "core.hooksPath", ".git/hooks"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-m", "first"]);
    git(&root, &["remote", "add", "origin", remote.as_str()]);
    git(&root, &["push", "-u", "origin", "main"]);
    let session = start_discovery(&ctx);
    Fixture {
        _guard: guard,
        root,
        remote,
        session,
    }
}

fn write(root: &Utf8PathBuf, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn stop_session(f: &Fixture) -> Report<StopOutcome> {
    stop(
        &Ctx::new(f.root.clone()),
        &StopInput {
            session: Some(f.session.clone()),
            all: false,
        },
    )
    .unwrap()
}

fn subjects(f: &Fixture) -> Vec<String> {
    git(&f.root, &["log", "--format=%s"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn set_auto_commit(root: &Utf8PathBuf, value: bool) {
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest["auto_commit"] = serde_json::json!(value);
    std::fs::write(path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    git(root, &["commit", "-qam", "manifest"]);
    git(root, &["push", "origin", "main"]);
}

#[test]
fn stopping_a_discovery_session_commits_its_pages_then_the_rest_and_pushes() {
    let f = fixture();
    write(&f.root, "docs/memory/gotcha/a-trap.md", "a trap\n");
    write(&f.root, "notes.md", "unrelated\n");

    let report = stop_session(&f);

    assert_eq!(report.value.stopped, 1);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(
        subjects(&f)[..2],
        [
            "chore(hall): Sync pending hall changes",
            "docs(memory): Record discovery session pages"
        ]
    );
    assert_eq!(
        git(&f.root, &["show", "--format=", "--name-only", "HEAD~1"]),
        "docs/memory/gotcha/a-trap.md"
    );
    assert_eq!(git(&f.root, &["status", "--porcelain"]), "");
    assert_eq!(
        git(&f.remote, &["rev-parse", "main"]),
        git(&f.root, &["rev-parse", "HEAD"]),
        "the commits were pushed"
    );
}

#[test]
fn stopping_a_discovery_session_with_no_pages_still_syncs_the_rest() {
    let f = fixture();
    write(&f.root, "notes.md", "unrelated\n");

    stop_session(&f);

    assert_eq!(subjects(&f)[0], "chore(hall): Sync pending hall changes");
    assert!(!subjects(&f).iter().any(|s| s.starts_with("docs(memory)")));
}

#[test]
fn stop_all_syncs_once_when_a_discovery_session_ends() {
    let f = fixture();
    write(&f.root, "docs/memory/gotcha/a-trap.md", "a trap\n");

    let report = stop(
        &Ctx::new(f.root.clone()),
        &StopInput {
            session: None,
            all: true,
        },
    )
    .unwrap();

    assert_eq!(report.value.stopped, 1);
    assert_eq!(
        subjects(&f)
            .iter()
            .filter(|s| s.starts_with("docs(memory)"))
            .count(),
        1
    );
    assert_eq!(git(&f.root, &["status", "--porcelain"]), "");
}

#[test]
fn auto_commit_off_leaves_a_discovery_stop_with_nothing_committed() {
    let f = fixture();
    set_auto_commit(&f.root, false);
    let before = subjects(&f);
    write(&f.root, "docs/memory/gotcha/a-trap.md", "a trap\n");

    let report = stop_session(&f);

    assert_eq!(report.value.stopped, 1);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(subjects(&f), before);
    assert_eq!(
        git(&f.root, &["status", "--porcelain", "-uall"]),
        "?? docs/memory/gotcha/a-trap.md"
    );
}

#[test]
fn stopping_a_feature_session_syncs_nothing() {
    let (_guard, root) = hall_with_detached_session();
    git(&root, &["init", "--initial-branch", "main"]);
    git(&root, &["config", "user.name", "ivar tests"]);
    git(&root, &["config", "user.email", "tests@ivar.invalid"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    git(&root, &["add", "ivar.json"]);
    git(&root, &["commit", "-m", "first"]);
    write(&root, "docs/memory/gotcha/a-trap.md", "a trap\n");

    let report = stop(
        &Ctx::new(root.clone()),
        &StopInput {
            session: Some(session_id_of(&root)),
            all: false,
        },
    )
    .unwrap();

    assert_eq!(report.value.stopped, 1);
    assert!(report.is_clean(), "{:?}", report.warnings);
    assert_eq!(git(&root, &["log", "--format=%s"]), "first");
    assert!(
        git(&root, &["status", "--porcelain", "-uall"]).contains("docs/memory/gotcha/a-trap.md")
    );
    unguard_worktrees(&root);
}

#[test]
fn stopping_a_discovery_session_in_a_hall_without_git_is_clean() {
    let (_guard, root) = hall_root();
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
    let session = start_discovery(&ctx);

    let report = stop(
        &ctx,
        &StopInput {
            session: Some(session),
            all: false,
        },
    )
    .unwrap();

    assert_eq!(report.value.stopped, 1);
    assert!(report.is_clean(), "{:?}", report.warnings);
}
