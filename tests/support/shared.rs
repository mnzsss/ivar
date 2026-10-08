//! Shared test scaffolding: UTF-8 temp dirs, canonical hall roots, and real
//! Git repositories with deterministic identity.
//!
//! This module is included through the two adapters that sit at the
//! compilation boundaries:
//!
//! - [`unit`](unit) — linked from `src/lib.rs` as `crate::test_support`, so
//!   library unit tests keep `use crate::test_support::…` unchanged;
//! - [`integration`](integration) — linked from each top-level integration
//!   test as `common`, adding the `assert_cmd` binary and manifest helpers
//!   that only integration tests need.
//!
//! The equivalent helpers used to live in `src/test_support.rs` and
//! `tests/common/mod.rs`, byte-for-byte duplicated where their path types
//! diverged. One implementation, two adapters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

/// A scratch directory and its UTF-8 path.
///
/// The `TempDir` is returned so the caller can bind it — dropping it deletes the
/// directory, so `let (_dir, root) = ...` is the shape that works and
/// `let (_, root) = ...` is the shape that mysteriously does not.
pub(crate) fn utf8_temp_dir() -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
    (dir, path)
}

/// A scratch directory, canonicalised, with an empty `hall` subdirectory in it.
///
/// Canonicalising matters for anything that compares paths against the result of
/// `Layout::discover`, which canonicalises too: on macOS `TempDir` hands back a
/// `/var/...` path whose real name is `/private/var/...`, and the two are not
/// equal as strings.
pub(crate) fn canonical_temp_dir() -> (TempDir, Utf8PathBuf) {
    let (dir, path) = utf8_temp_dir();
    let canonical = path.canonicalize_utf8().unwrap();
    (dir, canonical)
}

/// A canonicalised scratch directory with an empty `hall` subdirectory inside it.
///
/// The subdirectory is not incidental: `TempDir` names itself `.tmpXXXX`, and a
/// leading dot is refused by `HallName`. Using the tempdir itself as a hall root
/// would make every test that exercises name *derivation* collide with an
/// unrelated rule.
pub(crate) fn hall_root() -> (TempDir, Utf8PathBuf) {
    let (dir, canonical) = canonical_temp_dir();
    let root = canonical.join("hall");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root)
}

/// A real git repository at `path`, on `branch`, with no commits.
///
/// Real, not faked: `tempfile::TempDir` plus a real `git init` is fast,
/// hermetic, and exercises what ships. `ivar` never mocks git — see
/// ARCHITECTURE.md, seam 4.
///
/// Identity and branch name are passed with `-c` / `--initial-branch` rather
/// than left to the machine's own git config, so a developer whose
/// `init.defaultBranch` is `master` gets the same result as CI.
///
/// Copied from a template built by that same `git init`; see
/// [`copy_template`].
pub(crate) fn empty_repo(path: &Utf8Path, branch: &str) -> Utf8PathBuf {
    copy_template("empty", branch, path, init_empty)
}

/// A real git repository at `path`, on `branch`, with one commit adding a
/// `README.md` containing `seed\n`.
///
/// The commit matters: `git clone --bare` of an empty repository produces a
/// repository whose branch exists only as an unborn `HEAD`, which no worktree
/// can be added on. Anything testing the clone-then-worktree path needs
/// content.
///
/// Copied from a template built by that same init and commit; see
/// [`copy_template`].
pub(crate) fn seeded_repo(path: &Utf8Path, branch: &str) -> Utf8PathBuf {
    copy_template("seeded", branch, path, init_seeded)
}

fn init_empty(path: &Utf8Path, branch: &str) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "--initial-branch", branch, "."]);
}

fn init_seeded(path: &Utf8Path, branch: &str) {
    init_empty(path, branch);
    std::fs::write(path.join("README.md"), "seed\n").unwrap();
    git(path, &["add", "README.md"]);
    git(path, &["commit", "-m", "seed"]);
}

/// Bump when `init_empty`/`init_seeded` change what a repo holds: a template
/// outlives the run that built it and is never rebuilt while it exists.
const TEMPLATE_VERSION: u32 = 1;

/// `init` run once per target dir, then copied: a plain repo's `.git` holds
/// no absolute path, so a copy is a repo of its own. The template lives
/// under the test binary's target profile dir so it survives a
/// process-per-test runner and `cargo clean` removes it; it is published
/// with a `rename`, so a concurrent process sees all of it or none.
///
/// An existing `.git` at `path` is re-initialised in place instead, which is
/// what `git init` there always did: it keeps `HEAD` and history. A branch
/// name outside `[A-Za-z0-9._/-]` is built in place too: such names are
/// generated per run (hostile-name tests), and templating them would leave
/// one template behind per run.
fn copy_template(
    kind: &str,
    branch: &str,
    path: &Utf8Path,
    init: fn(&Utf8Path, &str),
) -> Utf8PathBuf {
    let plain_branch = branch
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
    if !plain_branch || path.join(".git").exists() {
        init(path, branch);
        return path.to_path_buf();
    }
    let root = templates_dir();
    let template = root.join(format!(
        "{kind}-v{TEMPLATE_VERSION}-{}",
        branch.replace('/', "%2F")
    ));
    if !template.exists() {
        std::fs::create_dir_all(&root).unwrap();
        let staging = TempDir::new_in(&root).unwrap();
        let built = Utf8Path::from_path(staging.path()).unwrap().join("repo");
        init(&built, branch);
        // Losing the race to another process leaves its identical template.
        let _ = std::fs::rename(&built, &template);
    }
    copy_tree(template.as_std_path(), path.as_std_path());
    path.to_path_buf()
}

/// `<target>/<profile>/ivar-test-templates`: every test binary runs from
/// `<target>/<profile>/deps/`.
fn templates_dir() -> Utf8PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().and_then(std::path::Path::parent).unwrap();
    Utf8PathBuf::from_path_buf(profile_dir.join("ivar-test-templates")).unwrap()
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Run git in `cwd`, with a fixed identity, panicking with git's own stderr if
/// it refuses. Committer identity is forced because a machine with no global
/// `user.email` cannot commit at all, and that failure is opaque.
///
/// `core.hooksPath` is emptied for the same class of reason: this helper builds
/// the *arrangement* a test starts from, and much of that arrangement is
/// commits on a default branch, which ivar's protection hook exists to refuse.
/// Scaffolding is not the behaviour under test.
///
/// This opt-out belongs to the scaffolding and nowhere else. A test that
/// asserts protection must invoke git without it — see the `git_unguarded`
/// helper in `tests/unit/git/exec.rs` — or it proves nothing.
pub(crate) fn git(cwd: &Utf8Path, args: &[&str]) {
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
}
