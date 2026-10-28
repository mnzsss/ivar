#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `--compact` is a graph-only format: every other verb must refuse it
//! instead of silently printing the human table.

use crate::common::{hall_root, ivar};

/// Verbs outside `graph`, list/status shaped and otherwise.
const NON_GRAPH_VERBS: &[&[&str]] = &[
    &["status"],
    &["feature", "list"],
    &["repo", "list"],
    &["session", "list"],
    &["skill", "list"],
    &["doctor"],
];

#[test]
fn non_graph_verbs_refuse_compact_with_a_usage_error() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    for verb in NON_GRAPH_VERBS {
        for before in [true, false] {
            let mut cmd = ivar();
            cmd.current_dir(&root);
            if before {
                cmd.arg("--compact");
                cmd.args(*verb);
            } else {
                cmd.args(*verb);
                cmd.arg("--compact");
            }
            let output = cmd.output().unwrap();
            let label = format!("{verb:?} (compact before verb: {before})");
            assert_eq!(output.status.code(), Some(2), "{label}");
            assert!(output.stdout.is_empty(), "{label}: stdout must be empty");
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                stderr.contains("`--compact` is only supported by `ivar graph`"),
                "{label}: {stderr}"
            );
        }
    }
}

#[test]
fn the_refusal_honours_json() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let output = ivar()
        .current_dir(&root)
        .args(["--json", "--compact", "feature", "list"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["code"], "cli.compact_unsupported");
}

#[test]
fn graph_verbs_still_emit_the_compact_format() {
    let (_guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();

    let run = |args: &[&str]| {
        let output = ivar().current_dir(&root).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    let before = run(&["--compact", "graph", "stats"]);
    let after = run(&["graph", "stats", "--compact"]);
    let human = run(&["graph", "stats"]);

    assert_eq!(before, after);
    assert_ne!(before, human, "compact must differ from the human table");
}
