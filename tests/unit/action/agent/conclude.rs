#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde_json::Value;

use super::*;
use crate::action::Ctx;
use crate::action::agent::run::{RunInput, run};
use crate::action::memory::drain::drain;
use crate::error::Report;
use crate::infra::proc::redirect_on_this_thread;
use crate::store::layout::Layout;
use crate::store::memory::db::MemoryDb;
use crate::test_support::{FakeDocker, fake_docker_command, seeded_hall};

const AT: &str = "2026-11-08T09:00";

fn hall_with_task(extra: &str) -> (tempfile::TempDir, camino::Utf8PathBuf) {
    let (guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/recheck.md"),
        format!("---\nat: {AT}\ntool: claude\n{extra}---\nLook again.\n"),
    )
    .unwrap();
    (guard, root)
}

fn run_task(root: &camino::Utf8PathBuf, trigger: Trigger) -> Report<AgentRunReceipt> {
    run(
        &Ctx::new(root.clone()),
        RunInput {
            name: "recheck".into(),
            trigger,
        },
    )
    .unwrap()
}

fn with_docker(root: &camino::Utf8PathBuf, env: &[(&str, &str)]) -> impl Sized + use<> {
    let fake = FakeDocker::install(root);
    let mut command = fake_docker_command(&fake);
    for &(key, value) in env {
        command = command.env(key, value);
    }
    redirect_on_this_thread("docker", command)
}

fn pending(root: &camino::Utf8Path) -> bool {
    root.join("agents/recheck.md").exists()
}

fn done_names(root: &camino::Utf8Path) -> Vec<String> {
    std::fs::read_dir(root.join("agents/done"))
        .map(|dir| {
            let mut names: Vec<_> = dir
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

fn spooled(layout: &Layout) -> Vec<Value> {
    std::fs::read_to_string(layout.memory_spool_dir().join("delivery.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_timer_run_concludes_the_task_whatever_its_outcome_but_skipped() {
    type Case = (
        &'static str,
        &'static [(&'static str, &'static str)],
        AgentRunOutcome,
    );
    let cases: [Case; 4] = [
        (
            "",
            &[("DOCKER_FAKE_OUTPUT", "all clear")],
            AgentRunOutcome::Succeeded,
        ),
        ("", &[("DOCKER_FAKE_EXIT", "1")], AgentRunOutcome::Failed),
        (
            "",
            &[
                ("DOCKER_FAKE_OUTPUT", "looked\nBLOCKED: no token"),
                ("DOCKER_FAKE_EXIT", "0"),
            ],
            AgentRunOutcome::Blocked,
        ),
        (
            "timeout: 1s\n",
            &[("DOCKER_FAKE_SLEEP", "20")],
            AgentRunOutcome::TimedOut,
        ),
    ];
    for (extra, env, expected) in cases {
        let (_guard, root) = hall_with_task(extra);
        let _docker = with_docker(&root, env);

        let report = run_task(&root, Trigger::Schedule);

        let receipt = &report.value;
        assert_eq!(receipt.outcome, expected);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(!pending(&root), "{expected:?} left the task pending");
        assert_eq!(done_names(&root), ["recheck.2026-11-08T0900.md"]);
        let kept =
            std::fs::read_to_string(root.join("agents/done/recheck.2026-11-08T0900.md")).unwrap();
        assert!(kept.contains("Look again."));
        assert_eq!(receipt.scheduled_at.as_deref(), Some(AT));
        let layout = Layout::at(root);
        let lines = spooled(&layout);
        assert_eq!(lines.len(), 1, "{expected:?}");
        assert_eq!(lines[0]["source_event"], "agent_task.concluded");
    }
}

#[test]
fn a_skipped_timer_run_leaves_the_task_pending() {
    let (_guard, root) = hall_with_task("");
    let _docker = with_docker(&root, &[]);
    let _held =
        crate::infra::fs::try_lock_exclusive(&Layout::at(root.clone()).agent_lock("recheck"))
            .unwrap()
            .unwrap();

    let report = run_task(&root, Trigger::Schedule);

    assert_eq!(report.value.outcome, AgentRunOutcome::Skipped);
    assert!(pending(&root));
    assert!(done_names(&root).is_empty());
    assert!(spooled(&Layout::at(root)).is_empty());
}

#[test]
fn a_manual_run_is_a_rehearsal_that_records_but_never_concludes() {
    let (_guard, root) = hall_with_task("");
    let _docker = with_docker(&root, &[("DOCKER_FAKE_OUTPUT", "rehearsed")]);

    let receipt = run_task(&root, Trigger::Manual).value;

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert!(pending(&root));
    assert!(done_names(&root).is_empty());
    assert!(spooled(&Layout::at(root.clone())).is_empty());
    let stored =
        crate::store::agent::read_receipt(&Layout::at(root.clone()), "recheck", &receipt.run_id)
            .unwrap()
            .unwrap();
    assert_eq!(stored.trigger, Trigger::Manual);
    assert_eq!(stored.scheduled_at.as_deref(), Some(AT));
    // And a later timer run still finds it.
    assert_eq!(
        run_task(&root, Trigger::Schedule).value.outcome,
        AgentRunOutcome::Succeeded
    );
    assert!(!pending(&root));
}

#[test]
fn a_recurring_agent_is_never_moved() {
    let (_guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/recheck.md"),
        "---\nschedule: \"@daily\"\ntool: claude\n---\nGo.\n",
    )
    .unwrap();
    let _docker = with_docker(&root, &[]);

    let receipt = run_task(&root, Trigger::Schedule).value;

    assert_eq!(receipt.scheduled_at, None);
    assert!(pending(&root));
    assert!(spooled(&Layout::at(root)).is_empty());
}

#[test]
fn a_name_collision_appends_the_run_id_and_never_overwrites() {
    let (_guard, root) = hall_with_task("");
    std::fs::create_dir_all(root.join("agents/done")).unwrap();
    std::fs::write(
        root.join("agents/done/recheck.2026-11-08T0900.md"),
        "an earlier task",
    )
    .unwrap();
    let _docker = with_docker(&root, &[]);

    let receipt = run_task(&root, Trigger::Schedule).value;

    let moved = format!("recheck.2026-11-08T0900.{}.md", receipt.run_id);
    let mut expected = ["recheck.2026-11-08T0900.md".to_owned(), moved.clone()];
    expected.sort();
    assert_eq!(done_names(&root), expected);
    assert_eq!(
        std::fs::read_to_string(root.join("agents/done/recheck.2026-11-08T0900.md")).unwrap(),
        "an earlier task"
    );
    assert!(
        std::fs::read_to_string(root.join("agents/done").join(moved))
            .unwrap()
            .contains("Look again.")
    );
    assert!(!pending(&root));
}

#[test]
fn an_offset_task_is_filed_under_the_time_as_written() {
    let (_guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(
        root.join("agents/recheck.md"),
        "---\nat: 2026-11-08T09:00-03:00\ntool: claude\n---\nGo.\n",
    )
    .unwrap();
    let _docker = with_docker(&root, &[]);

    let receipt = run_task(&root, Trigger::Schedule).value;

    assert_eq!(done_names(&root), ["recheck.2026-11-08T0900.md"]);
    assert_eq!(
        receipt.scheduled_at.as_deref(),
        Some("2026-11-08T09:00-03:00")
    );
}

#[test]
fn the_observation_reaches_memory_db_after_the_drain_as_a_binary_event() {
    let (_guard, root) = hall_with_task("");
    let _docker = with_docker(
        &root,
        &[
            ("DOCKER_FAKE_OUTPUT", "first\nthe prices dropped"),
            ("DOCKER_FAKE_EXIT", "0"),
        ],
    );
    let layout = Layout::at(root.clone());

    let receipt = run_task(&root, Trigger::Schedule).value;
    drain(&layout).unwrap();

    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let (event, origin, session, attempt, importance, body): (
        String,
        String,
        String,
        String,
        i64,
        String,
    ) = db
        .conn()
        .query_row(
            "SELECT source_event, origin, session_id, attempt_id, importance, body
             FROM observations WHERE source_event = 'agent_task.concluded'",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(event, "agent_task.concluded");
    assert_eq!(origin, "", "never a User prompt");
    assert_eq!(session, "");
    assert_eq!(attempt, receipt.run_id);
    assert_eq!(importance, 4);
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["name"], "recheck");
    assert_eq!(body["at"], AT);
    assert_eq!(body["ran_at"], receipt.started_at.as_str());
    assert_eq!(body["outcome"], "succeeded");
    assert_eq!(body["run_id"], receipt.run_id.as_str());
    assert!(
        body["output_tail"]
            .as_str()
            .unwrap()
            .contains("the prices dropped")
    );
    let prompts: i64 = db
        .conn()
        .query_row(
            "SELECT count(*) FROM observations WHERE source_event = 'session.prompted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(prompts, 0);
}

#[test]
fn a_failed_task_outranks_a_successful_one() {
    let (_guard, root) = hall_with_task("");
    let _docker = with_docker(&root, &[("DOCKER_FAKE_EXIT", "2")]);
    run_task(&root, Trigger::Schedule);
    let lines = spooled(&Layout::at(root));
    assert_eq!(lines[0]["importance"], 7);
}

#[test]
fn the_same_run_is_one_event_id() {
    assert_eq!(
        agent_task_event_id("recheck", "20261108T120000Z-aaaaaaaa"),
        agent_task_event_id("recheck", "20261108T120000Z-aaaaaaaa")
    );
    assert_ne!(
        agent_task_event_id("recheck", "20261108T120000Z-aaaaaaaa"),
        agent_task_event_id("recheck", "20261108T120000Z-bbbbbbbb")
    );
}

#[test]
fn a_long_output_is_cut_from_the_front_so_the_body_stays_valid_json() {
    let (_guard, root) = hall_with_task("");
    let layout = Layout::at(root.clone());
    let spec = crate::store::agent::load(&layout, "recheck").unwrap();
    let mut receipt = AgentRunReceipt::skipped(
        "20261108T120000Z-aaaaaaaa".into(),
        "recheck".into(),
        Trigger::Schedule,
        "2026-11-08T12:00:00.000000000Z".into(),
    );
    receipt.outcome = AgentRunOutcome::Failed;
    receipt.output = format!("{}END", "ä\n\"quoted\" ".repeat(5000));

    let text = body(&spec, &receipt);

    assert!(text.len() <= MAX_BODY_BYTES, "{}", text.len());
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert!(parsed["output_tail"].as_str().unwrap().ends_with("END"));
    record_concluded(&layout, &spec, &receipt).unwrap();
    let lines = spooled(&layout);
    let stored: Value = serde_json::from_str(lines[0]["body"].as_str().unwrap()).unwrap();
    assert!(stored["output_tail"].as_str().unwrap().ends_with("END"));
}

#[test]
fn a_file_that_cannot_move_is_a_warning_and_records_no_event() {
    let (_guard, root) = hall_with_task("");
    std::fs::write(root.join("agents/done"), "in the way").unwrap();
    let _docker = with_docker(&root, &[]);

    let report = run_task(&root, Trigger::Schedule);

    assert_eq!(
        report.warnings.iter().map(|w| w.code).collect::<Vec<_>>(),
        ["agent.task_not_concluded"]
    );
    assert!(pending(&root));
    let layout = Layout::at(root);
    assert!(
        crate::store::agent::read_receipt(&layout, "recheck", &report.value.run_id)
            .unwrap()
            .is_some()
    );
    assert!(spooled(&layout).is_empty());
}
