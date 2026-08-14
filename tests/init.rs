//! End-to-end tests for `ivar init`, driving the compiled binary.
//!
//! Unit tests for the action itself (name derivation, provider default,
//! nesting/already-initialised failures) live in `src/action/hall/init.rs`. These
//! tests exist for what only the real binary can prove: the manifest's exact
//! on-disk bytes, the process exit code, and that `--json` and the human
//! surface report the same facts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/integration.rs"]
mod common;

use common::{hall_root as utf8_temp_dir, ivar};
use predicates::prelude::*;

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
    let expected = "{\n  \"integration\": {\n    \"strategy\": \"squash\",\n    \"via\": \"local\"\n  },\n  \"name\": \"acme\",\n  \"providers\": {\n    \"available\": [\n      \"opencode\"\n    ],\n    \"default\": \"opencode\"\n  },\n  \"repos\": [],\n  \"version\": 2\n}\n";
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
    assert_eq!(
        content,
        ".ivar/*\n!.ivar/skills/\n!.ivar/setups/\n\
         .claude/commands/ivar-*.md\n.opencode/commands/ivar-*.md\n"
    );
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

/// A verb that operates on a hall fails clearly when there is none, rather
/// than exiting zero having done nothing. `status` stands in for the rest;
/// the point is the shape of the refusal, not which verb it is.
///
/// The `current_dir` is load-bearing. `Layout::discover` walks *up*, and
/// without an explicit cwd this inherits the test process's — the crate root,
/// which is inside a hall whenever `ivar` is developed with `ivar`. The test
/// would then assert a refusal against a directory that genuinely has a hall
/// above it, and fail for being right.
#[test]
fn a_hall_verb_outside_a_hall_fails_clearly_instead_of_pretending() {
    let (_guard, outside) = utf8_temp_dir();
    ivar()
        .current_dir(&outside)
        .arg("status")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no hall at"));
}
