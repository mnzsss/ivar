#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::agent::receipt::{AgentRunReceipt, Trigger};
use crate::store::layout::Layout;
use crate::test_support::seeded_hall;
use camino::Utf8PathBuf;

fn seed(root: &Utf8PathBuf, id: &str, output: &str) {
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/nightly.md"),
        "---\nschedule: \"@daily\"\ntool: claude\n---\n",
    )
    .unwrap();
    let mut r = AgentRunReceipt::skipped(id.into(), "nightly".into(), Trigger::Manual, "t".into());
    r.output = output.into();
    crate::store::agent::write_receipt(&Layout::at(root.clone()), &r).unwrap();
}

#[test]
fn prints_the_latest_run_by_default_and_a_given_run_on_request() {
    let (_guard, root) = seeded_hall();
    seed(&root, "20261019T090000Z-aaaaaaaa", "first\n");
    seed(&root, "20261019T100000Z-bbbbbbbb", "second\n");
    let ctx = Ctx::new(root);
    assert_eq!(
        logs(
            &ctx,
            LogsInput {
                name: "nightly".into(),
                run: None
            }
        )
        .unwrap()
        .value
        .output,
        "second\n"
    );
    let given = logs(
        &ctx,
        LogsInput {
            name: "nightly".into(),
            run: Some("20261019T090000Z-aaaaaaaa".into()),
        },
    )
    .unwrap()
    .value;
    assert_eq!(given.output, "first\n");
    let mut human = Vec::new();
    given.write_human(&mut human).unwrap();
    assert_eq!(human, b"first\n");
}

#[test]
fn an_agent_that_never_ran_says_so() {
    let (_guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/nightly.md"),
        "---\nschedule: \"@daily\"\ntool: claude\n---\n",
    )
    .unwrap();
    let failure = logs(
        &Ctx::new(root),
        LogsInput {
            name: "nightly".into(),
            run: None,
        },
    )
    .unwrap_err();
    assert_eq!(failure.code, "agent.no_runs");
}
