//! End-to-end tests for `ivar init`, driving the compiled binary.
//!
//! Unit tests for the action itself (name derivation, provider default,
//! nesting/already-initialised failures) live in `src/action/hall.rs`. These
//! tests exist for what only the real binary can prove: the manifest's exact
//! on-disk bytes, the process exit code, and that `--json` and the human
//! surface report the same facts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use assert_cmd::Command;
use camino::Utf8PathBuf;
use predicates::prelude::*;
use tempfile::TempDir;

fn ivar() -> Command {
    Command::cargo_bin("ivar").expect("binary builds")
}

/// A fresh directory to `init` into, with a name that does **not** start
/// with `.` — `tempfile::TempDir` defaults to a `.tmp*` prefix itself, and
/// using its path directly as the hall root would make every test that
/// relies on derived naming collide with `HallName`'s "no leading dot" rule
/// instead of exercising what the test is actually about.
fn utf8_temp_dir() -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().expect("create temp dir");
    let raw = Utf8PathBuf::try_from(dir.path().to_path_buf()).expect("temp dir path is utf8");
    let canonical = raw.canonicalize_utf8().expect("canonicalize temp dir");
    let root = canonical.join("hall");
    std::fs::create_dir(&root).expect("create hall subdirectory");
    (dir, root)
}

#[test]
fn fresh_init_creates_the_expected_on_disk_shape() {
    let (_guard, root) = utf8_temp_dir();

    ivar().current_dir(&root).arg("init").assert().success();

    assert!(root.join("ivar.json").is_file());
    assert!(root.join(".ivar").is_dir());
    assert!(root.join(".gitignore").is_file());
}

#[test]
fn ivar_json_is_written_with_canonical_bytes() {
    let (_guard, root) = utf8_temp_dir();

    ivar()
        .current_dir(&root)
        .args(["init", "--name", "acme", "--provider", "opencode"])
        .assert()
        .success();

    let bytes = std::fs::read_to_string(root.join("ivar.json")).expect("read ivar.json");
    let expected = "{\n  \"name\": \"acme\",\n  \"providers\": {\n    \"available\": [\n      \"opencode\"\n    ],\n    \"default\": \"opencode\"\n  },\n  \"repos\": [],\n  \"version\": 1\n}\n";
    assert_eq!(
        bytes, expected,
        "ivar.json must be sorted keys, two-space indent, one trailing newline"
    );
}

#[test]
fn gitignore_uses_the_star_form_never_the_bare_dotdir() {
    let (_guard, root) = utf8_temp_dir();

    ivar().current_dir(&root).arg("init").assert().success();

    let content = std::fs::read_to_string(root.join(".gitignore")).expect("read .gitignore");
    assert_eq!(content, ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n");
    assert!(content.contains("!.ivar/skills/"));
    assert!(
        !content.lines().any(|line| line == ".ivar/"),
        "must never emit the bare `.ivar/` form, which would silently drop the negations"
    );
}

#[test]
fn init_refuses_an_existing_hall_with_a_nonzero_exit() {
    let (_guard, root) = utf8_temp_dir();

    ivar().current_dir(&root).arg("init").assert().success();

    ivar()
        .current_dir(&root)
        .arg("init")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn init_refuses_an_existing_hall_reported_as_json() {
    let (_guard, root) = utf8_temp_dir();

    ivar().current_dir(&root).arg("init").assert().success();

    let output = ivar()
        .current_dir(&root)
        .args(["init", "--json"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["code"], "hall.already_initialised");
}

#[test]
fn init_refuses_to_nest_inside_an_existing_hall() {
    let (_guard, root) = utf8_temp_dir();
    ivar().current_dir(&root).arg("init").assert().success();

    let nested = root.join("nested");
    std::fs::create_dir(&nested).expect("create nested dir");

    ivar()
        .current_dir(&nested)
        .arg("init")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("existing hall"));
}

#[test]
fn json_and_human_surfaces_carry_the_same_facts() {
    let (_guard_json, root_json) = utf8_temp_dir();
    let json_output = ivar()
        .current_dir(&root_json)
        .args(["init", "--name", "acme", "--provider", "opencode", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&json_output).expect("valid json");
    assert_eq!(value["name"], "acme");
    assert_eq!(value["provider"], "opencode");
    assert_eq!(value["root"], root_json.as_str());

    let (_guard_human, root_human) = utf8_temp_dir();
    let human_output = ivar()
        .current_dir(&root_human)
        .args(["init", "--name", "acme", "--provider", "opencode"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let human = String::from_utf8(human_output).expect("utf8 output");
    assert!(human.contains("acme"));
    assert!(human.contains("opencode"));
    assert!(human.contains(root_human.as_str()));
}

#[test]
fn a_root_verb_not_yet_implemented_fails_clearly_instead_of_pretending() {
    ivar()
        .arg("sync")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("not implemented yet"));
}
