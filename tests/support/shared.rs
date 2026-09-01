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
pub(crate) fn empty_repo(path: &Utf8Path, branch: &str) -> Utf8PathBuf {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "--initial-branch", branch, "."]);
    path.to_path_buf()
}

/// A real git repository at `path`, on `branch`, with one commit adding a
/// `README.md` containing `seed\n`.
///
/// The commit matters: `git clone --bare` of an empty repository produces a
/// repository whose branch exists only as an unborn `HEAD`, which no worktree
/// can be added on. Anything testing the clone-then-worktree path needs
/// content.
pub(crate) fn seeded_repo(path: &Utf8Path, branch: &str) -> Utf8PathBuf {
    empty_repo(path, branch);
    std::fs::write(path.join("README.md"), "seed\n").unwrap();
    git(path, &["add", "README.md"]);
    git(path, &["commit", "-m", "seed"]);
    path.to_path_buf()
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
