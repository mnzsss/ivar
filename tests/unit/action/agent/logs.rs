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

fn input(name: &str, run: Option<&str>) -> LogsInput {
    LogsInput {
        name: name.into(),
        run: run.map(Into::into),
    }
}

fn conclude(root: &Utf8PathBuf) {
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.agents_done_dir()).unwrap();
    std::fs::rename(
        layout.agent_file("nightly"),
        layout.agents_done_dir().join("nightly.2026-11-08T0900.md"),
    )
    .unwrap();
}

#[test]
fn a_task_moved_to_done_still_prints_its_latest_and_its_given_run() {
    let (_guard, root) = seeded_hall();
    seed(&root, "20261019T090000Z-aaaaaaaa", "first\n");
    seed(&root, "20261019T100000Z-bbbbbbbb", "second\n");
    conclude(&root);
    let ctx = Ctx::new(root);
    assert_eq!(
        logs(&ctx, input("nightly", None)).unwrap().value.output,
        "second\n"
    );
    assert_eq!(
        logs(&ctx, input("nightly", Some("20261019T090000Z-aaaaaaaa")))
            .unwrap()
            .value
            .output,
        "first\n"
    );
}

#[test]
fn receipts_alone_are_enough_when_the_file_is_gone() {
    let (_guard, root) = seeded_hall();
    seed(&root, "20261019T090000Z-aaaaaaaa", "kept\n");
    std::fs::remove_file(root.join("agents/nightly.md")).unwrap();
    assert_eq!(
        logs(&Ctx::new(root), input("nightly", None))
            .unwrap()
            .value
            .output,
        "kept\n"
    );
}

#[test]
fn a_missing_run_of_a_concluded_task_is_run_not_found() {
    let (_guard, root) = seeded_hall();
    seed(&root, "20261019T090000Z-aaaaaaaa", "first\n");
    conclude(&root);
    let failure = logs(&Ctx::new(root), input("nightly", Some("nope"))).unwrap_err();
    assert_eq!(failure.code, "agent.run_not_found");
}

#[test]
fn a_concluded_task_without_receipts_has_not_run_here() {
    let (_guard, root) = seeded_hall();
    seed(&root, "20261019T090000Z-aaaaaaaa", "first\n");
    conclude(&root);
    std::fs::remove_dir_all(Layout::at(root.clone()).agent_runs_dir("nightly")).unwrap();
    let failure = logs(&Ctx::new(root), input("nightly", None)).unwrap_err();
    assert_eq!(failure.code, "agent.no_runs");
}

#[test]
fn a_name_found_nowhere_is_not_found() {
    let (_guard, root) = seeded_hall();
    let failure = logs(&Ctx::new(root), input("ghost", None)).unwrap_err();
    assert_eq!(failure.code, "agent.not_found");
    assert!(failure.what.contains("agents/done"), "{}", failure.what);
    assert!(failure.what.contains("runs"), "{}", failure.what);
}

#[test]
fn an_invalid_name_is_refused_before_touching_the_disk() {
    let (_guard, root) = seeded_hall();
    for name in ["../x", "a/b", "..", ""] {
        let failure = logs(&Ctx::new(root.clone()), input(name, None)).unwrap_err();
        assert_eq!(failure.code, "agent.invalid", "{name}");
    }
}
