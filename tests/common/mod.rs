//! Shared scaffolding for the integration tests.
//!
//! `src/test_support.rs` is `#[cfg(test)]` inside the library, so it is not part
//! of the compiled crate and integration tests under `tests/` genuinely cannot
//! see it. That boundary is real and not worth breaking — but it does not mean
//! every test binary needs its own copy of the same four helpers, which is what
//! `tests/init.rs` and `tests/sync.rs` had.
//!
//! `tests/common/mod.rs` is Cargo's own answer: a subdirectory module is
//! included by the test binaries that `mod common;` it, and is not itself
//! compiled as a test target.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use assert_cmd::Command;
use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

/// The compiled `ivar` binary, ready to be given arguments.
pub(crate) fn ivar() -> Command {
    Command::cargo_bin("ivar").expect("binary builds")
}

/// A canonicalised scratch directory with an empty `hall` subdirectory in it.
///
/// The subdirectory is not incidental: `TempDir` names itself `.tmp*`, and a
/// leading dot is refused by `HallName`. Using the tempdir itself as a hall root
/// would make every test that exercises name *derivation* collide with an
/// unrelated rule.
///
/// Canonicalising matters because `Layout::discover` canonicalises too: on macOS
/// a `TempDir` lives under `/var/...`, whose real name is `/private/var/...`,
/// and the two are not equal as strings.
pub(crate) fn hall_root() -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().expect("create temp dir");
    let raw = Utf8PathBuf::try_from(dir.path().to_path_buf()).expect("temp dir path is utf8");
    let canonical = raw.canonicalize_utf8().expect("canonicalize temp dir");
    let root = canonical.join("hall");
    std::fs::create_dir(&root).expect("create hall subdirectory");
    (dir, root)
}

/// A real git repository at `path`, on `branch`, with one commit adding a
/// `README.md` containing `seed\n`.
///
/// Real, not faked: `ivar` never mocks git (ARCHITECTURE.md, seam 4). The commit
/// matters — a bare clone of an empty repository has an unborn `HEAD`, which no
/// worktree can be added on.
pub(crate) fn seeded_repo(path: &Utf8Path, branch: &str) -> Utf8PathBuf {
    std::fs::create_dir_all(path).expect("create origin dir");
    git(path, &["init", "--initial-branch", branch, "."]);
    std::fs::write(path.join("README.md"), "seed\n").expect("write README");
    git(path, &["add", "README.md"]);
    git(path, &["commit", "-m", "seed"]);
    path.to_path_buf()
}

/// Rewrite a hall's `ivar.json` to declare `repos` as `(name, url, branch)`.
///
/// Written by hand rather than through a verb because `ivar repo add` is a later
/// slice — and because `ivar.json` being hand-editable is the contract, so a
/// test that hand-edits it is testing the real path.
pub(crate) fn declare_repos(root: &Utf8Path, repos: &[(&str, &Utf8Path, &str)]) {
    let entries = repos
        .iter()
        .map(|(name, url, branch)| {
            format!(r#"{{"default_branch":"{branch}","name":"{name}","url":"{url}"}}"#)
        })
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(
        root.join("ivar.json"),
        format!(
            r#"{{"name":"acme","providers":{{"available":["claude-code"],"default":"claude-code"}},"repos":[{entries}],"version":1}}"#
        ),
    )
    .expect("write ivar.json");
}

/// Run git in `cwd` with a fixed identity, panicking with git's own stderr if it
/// refuses. Identity is forced because a machine with no global `user.email`
/// cannot commit at all, and that failure is opaque.
fn git(cwd: &Utf8Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=ivar tests"])
        .args(["-c", "user.email=tests@ivar.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");

    assert!(
        output.status.success(),
        "git {} failed in {cwd}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
