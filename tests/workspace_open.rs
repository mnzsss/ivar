//! `ivar feature workspace` opens the workspace it just wrote — and only when
//! a human is reading the output.
//!
//! The editor is faked: a `code` script early on `PATH` that records having
//! run. Depending on a real VS Code would make this test a statement about the
//! machine it runs on rather than about ivar.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use camino::{Utf8Path, Utf8PathBuf};
use common::{declare_repos, hall_root, ivar, seeded_repo};

// `camino`, `serde_json` and `tempfile` are all reachable here: the first two
// are crate dependencies and `tempfile` is a dev-dependency
// (`Cargo.toml`, `[dev-dependencies]`). No new dependency is needed.

/// A directory holding a `code` executable that appends its arguments to
/// `<dir>/opened` and exits. Returned so it can be put on `PATH`.
fn fake_editor(dir: &Utf8Path) -> Utf8PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("code");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}/opened\n", dir),
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(script.as_std_path(), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }

    bin
}

/// A hall with `api` promoted into `checkout` and `web` left on its default
/// branch, plus the fake editor's directory.
fn hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
    let (guard, root) = hall_root();

    ivar()
        .current_dir(&root)
        .args(["init", "--name", "acme", "--provider", "claude-code"])
        .assert()
        .success();

    let origins = root.parent().unwrap().join("origins");
    let api = seeded_repo(&origins.join("api"), "main");
    let web = seeded_repo(&origins.join("web"), "main");
    declare_repos(&root, &[("api", &api, "main"), ("web", &web, "main")]);

    ivar().current_dir(&root).arg("sync").assert().success();
    ivar()
        .current_dir(&root)
        .args(["feature", "create", "checkout"])
        .assert()
        .success();
    ivar()
        .current_dir(&root)
        .args(["feature", "promote", "checkout", "api"])
        .assert()
        .success();

    let editor_dir = root.parent().unwrap().join("editor");
    std::fs::create_dir_all(&editor_dir).unwrap();
    let bin = fake_editor(&editor_dir);

    (guard, root, bin.parent().unwrap().to_path_buf())
}

/// `PATH` with `dir/bin` first, so the fake `code` wins over anything real.
fn path_with(dir: &Utf8Path) -> String {
    let inherited = std::env::var("PATH").unwrap_or_default();
    format!("{}/bin:{inherited}", dir)
}

/// The child is detached, so it may still be starting when `ivar` exits.
fn wait_for(marker: &Utf8Path) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if marker.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    false
}

#[test]
fn a_human_run_opens_the_workspace_it_wrote() {
    let (_guard, root, editor) = hall_with_feature();
    let marker = editor.join("opened");

    ivar()
        .current_dir(&root)
        .env("PATH", path_with(&editor))
        .args(["feature", "workspace", "checkout"])
        .assert()
        .success();

    assert!(wait_for(&marker), "the editor was never started");

    let opened = std::fs::read_to_string(marker.as_std_path()).unwrap();
    assert!(
        opened.contains("checkout.code-workspace"),
        "the editor was given the wrong path: {opened}"
    );
}

/// `--json` prints exactly the value the action returned, and a machine-shaped
/// run has no editor to open into.
#[test]
fn a_json_run_writes_the_workspace_and_opens_nothing() {
    let (_guard, root, editor) = hall_with_feature();
    let marker = editor.join("opened");

    let output = ivar()
        .current_dir(&root)
        .env("PATH", path_with(&editor))
        .args(["feature", "workspace", "checkout", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let written = Utf8PathBuf::from(value["path"].as_str().expect("a path"));
    assert!(written.is_file(), "the workspace file was not written");

    // A detached spawn is fast, but not instantaneous; give it a window it
    // would comfortably win before concluding it never happened.
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(!marker.exists(), "--json started an editor");
}

/// The file is the deliverable; the editor is a convenience. A `code` that
/// cannot be run leaves the command successful and says so.
#[test]
fn a_missing_editor_is_a_sentence_not_a_failure() {
    let (_guard, root, _editor) = hall_with_feature();

    let assert = ivar()
        .current_dir(&root)
        .env("PATH", "/nonexistent-dir-for-this-test")
        .args(["feature", "workspace", "checkout"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Wrote workspace for `checkout` to"));
    assert!(stdout.contains("could not open it"));
}
