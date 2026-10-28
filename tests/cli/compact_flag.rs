//! `--compact` is a global flag that only `ivar graph` renders. Every other
//! verb must refuse it (exit 2, nothing on stdout) rather than ignore it and
//! print the human table.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::common::{hall_root, ivar};
use predicates::prelude::*;

const NOT_GRAPH: [&[&str]; 8] = [
    &["feature", "list"],
    &["repo", "list"],
    &["session", "list"],
    &["skill", "list"],
    &["provider", "list"],
    &["plan", "list"],
    &["status"],
    &["doctor"],
];

#[test]
fn verbs_outside_graph_refuse_compact_before_or_after_the_subcommand() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    for verb in NOT_GRAPH {
        let leading = ivar()
            .current_dir(&root)
            .arg("--compact")
            .args(verb)
            .assert()
            .failure()
            .code(2)
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains(
                "`--compact` is only supported by `ivar graph`",
            ))
            .get_output()
            .clone();
        let trailing = ivar()
            .current_dir(&root)
            .args(verb)
            .arg("--compact")
            .assert()
            .failure()
            .code(2)
            .stdout(predicate::str::is_empty())
            .get_output()
            .clone();
        assert_eq!(leading.stderr, trailing.stderr, "{verb:?}");
    }
}

#[test]
fn the_refusal_honours_json() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let out = ivar()
        .current_dir(&root)
        .args(["--compact", "--json", "feature", "list"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).expect("valid json");
    assert_eq!(value["ok"], false);
    assert_eq!(value["code"], "cli.compact_unsupported");
}

#[test]
fn graph_verbs_still_emit_the_compact_format() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let plain = ivar()
        .current_dir(&root)
        .args(["graph", "stats"])
        .output()
        .unwrap();
    for args in [
        ["--compact", "graph", "stats"],
        ["graph", "stats", "--compact"],
    ] {
        let compact = ivar().current_dir(&root).args(args).output().unwrap();
        assert!(compact.status.success(), "{args:?}: {compact:?}");
        assert_ne!(compact.stdout, plain.stdout, "{args:?} must be compact");
        assert!(compact.stderr.is_empty(), "{args:?}");
    }
}
