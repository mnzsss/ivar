#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::infra::proc::redirect_on_this_thread;
use crate::test_support::{FakeDocker, fake_docker_command, seeded_hall};

fn hall_with_agents(names: &[&str]) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = seeded_hall();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    for name in names {
        std::fs::write(
            root.join(format!("agents/{name}.md")),
            "---\nschedule: \"@daily\"\ntool: claude\n---\nGo.\n",
        )
        .unwrap();
    }
    (guard, root)
}

fn receipt(
    root: &Utf8PathBuf,
    agent: &str,
    run_id: &str,
    outcome: AgentRunOutcome,
    output: &str,
) -> AgentRunReceipt {
    let mut receipt = AgentRunReceipt::skipped(
        run_id.into(),
        agent.into(),
        Trigger::Schedule,
        "2026-11-01T00:00:00Z".into(),
    );
    receipt.outcome = outcome;
    receipt.output = output.into();
    store::agent::write_receipt(&Layout::at(root.clone()), &receipt).unwrap();
    receipt
}

fn resume_all(root: &Utf8PathBuf, name: Option<&str>) -> Outcome<ResumeOutcome> {
    resume_with(
        &Ctx::new(root.clone()),
        ResumeInput {
            name: name.map(str::to_owned),
        },
        &HostDeps::system,
        &|_| true,
    )
}

fn latest(root: &Utf8PathBuf, agent: &str) -> AgentRunReceipt {
    store::agent::latest_receipt(&Layout::at(root.clone()), agent)
        .unwrap()
        .unwrap()
}

#[test]
fn without_a_name_every_blocked_agent_is_run_again_and_the_others_are_left() {
    let (_guard, root) = hall_with_agents(&["alpha", "beta", "gamma", "delta"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Blocked,
        "needs approval",
    );
    receipt(
        &root,
        "beta",
        "20261101T000000Z-bbbb0001",
        AgentRunOutcome::Succeeded,
        "ok",
    );
    receipt(
        &root,
        "gamma",
        "20261101T000000Z-cccc0001",
        AgentRunOutcome::Failed,
        "boom",
    );
    // delta never ran.
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let report = resume_all(&root, None).unwrap();

    assert_eq!(report.value.resumed.len(), 1, "{:?}", report.value);
    let resumed = &report.value.resumed[0];
    assert_eq!(resumed.agent, "alpha");
    assert_eq!(resumed.blocked_run, "20261101T000000Z-aaaa0001");
    assert_eq!(resumed.outcome, AgentRunOutcome::Succeeded);
    assert_ne!(resumed.run_id, resumed.blocked_run);
    assert!(report.is_clean());

    let new = latest(&root, "alpha");
    assert_eq!(new.run_id, resumed.run_id);
    assert_eq!(new.trigger, Trigger::Manual);
    assert_eq!(latest(&root, "beta").run_id, "20261101T000000Z-bbbb0001");
    assert_eq!(latest(&root, "gamma").run_id, "20261101T000000Z-cccc0001");
    assert!(
        store::agent::latest_receipt(&Layout::at(root.clone()), "delta")
            .unwrap()
            .is_none()
    );
    let log = fake.log();
    assert_eq!(log.matches("claude -p").count(), 1, "{log}");
}

#[test]
fn a_run_that_succeeded_since_is_no_longer_blocked() {
    let (_guard, root) = hall_with_agents(&["alpha"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Blocked,
        "x",
    );
    receipt(
        &root,
        "alpha",
        "20261102T000000Z-aaaa0002",
        AgentRunOutcome::Succeeded,
        "ok",
    );

    let report = resume_all(&root, None).unwrap();

    assert!(report.value.resumed.is_empty());
}

#[test]
fn a_name_resumes_only_that_agent() {
    let (_guard, root) = hall_with_agents(&["alpha", "beta"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Blocked,
        "x",
    );
    receipt(
        &root,
        "beta",
        "20261101T000000Z-bbbb0001",
        AgentRunOutcome::Blocked,
        "x",
    );
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let report = resume_all(&root, Some("beta")).unwrap();

    let agents: Vec<&str> = report
        .value
        .resumed
        .iter()
        .map(|r| r.agent.as_str())
        .collect();
    assert_eq!(agents, ["beta"]);
    assert_eq!(latest(&root, "alpha").run_id, "20261101T000000Z-aaaa0001");
}

#[test]
fn a_named_agent_that_is_not_blocked_is_refused_with_its_latest_outcome() {
    let (_guard, root) = hall_with_agents(&["alpha"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Succeeded,
        "ok",
    );

    let failure = resume_all(&root, Some("alpha")).unwrap_err();

    assert_eq!(failure.code, "agent.not_blocked");
    assert!(failure.what.contains("succeeded"), "{}", failure.what);
    assert_eq!(latest(&root, "alpha").run_id, "20261101T000000Z-aaaa0001");
}

#[test]
fn a_named_agent_that_never_ran_or_does_not_exist_is_refused() {
    let (_guard, root) = hall_with_agents(&["alpha"]);
    assert_eq!(
        resume_all(&root, Some("alpha")).unwrap_err().code,
        "agent.no_runs"
    );
    assert_eq!(
        resume_all(&root, Some("ghost")).unwrap_err().code,
        "agent.not_found"
    );
    assert_eq!(
        resume_all(&root, Some("../x")).unwrap_err().code,
        "agent.invalid"
    );
}

#[test]
fn a_legacy_failed_run_with_a_blocked_reason_is_resumed() {
    let (_guard, root) = hall_with_agents(&["alpha", "beta"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Failed,
        "blocked: agent.secrets_mode: file must be owner-only",
    );
    receipt(
        &root,
        "beta",
        "20261101T000000Z-bbbb0001",
        AgentRunOutcome::Failed,
        "the tool crashed",
    );
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let report = resume_all(&root, None).unwrap();

    let agents: Vec<&str> = report
        .value
        .resumed
        .iter()
        .map(|r| r.agent.as_str())
        .collect();
    assert_eq!(agents, ["alpha"]);
}

#[test]
fn a_run_that_is_blocked_again_is_reported_and_stays_resumable() {
    let (_guard, root) = hall_with_agents(&["alpha"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Blocked,
        "x",
    );
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread(
        "docker",
        fake_docker_command(&fake).env("DOCKER_FAKE_OUTPUT", "tried\nBLOCKED: still no token"),
    );

    let report = resume_all(&root, None).unwrap();

    assert_eq!(report.value.resumed[0].outcome, AgentRunOutcome::Blocked);
    assert!(latest(&root, "alpha").is_resumable());
}

#[test]
fn an_invalid_agent_file_warns_and_the_others_still_run() {
    let (_guard, root) = hall_with_agents(&["alpha", "beta"]);
    receipt(
        &root,
        "alpha",
        "20261101T000000Z-aaaa0001",
        AgentRunOutcome::Blocked,
        "x",
    );
    receipt(
        &root,
        "beta",
        "20261101T000000Z-bbbb0001",
        AgentRunOutcome::Blocked,
        "x",
    );
    std::fs::write(
        root.join("agents/alpha.md"),
        "---\nschedule: nonsense\n---\n",
    )
    .unwrap();
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let report = resume_all(&root, None).unwrap();

    let agents: Vec<&str> = report
        .value
        .resumed
        .iter()
        .map(|r| r.agent.as_str())
        .collect();
    assert_eq!(agents, ["beta"]);
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].code, "agent.invalid");
    assert_eq!(report.warnings[0].subject, "alpha");
}

#[test]
fn nothing_blocked_is_an_empty_clean_report() {
    let (_guard, root) = hall_with_agents(&["alpha"]);
    let report = resume_all(&root, None).unwrap();
    assert!(report.value.resumed.is_empty());
    assert!(report.is_clean());
    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "No blocked agent runs to resume.\n"
    );
}

#[test]
fn the_human_output_lists_each_resumed_run_with_its_result() {
    let outcome = ResumeOutcome {
        resumed: vec![Resumed {
            agent: "alpha".into(),
            blocked_run: "r1".into(),
            run_id: "r2".into(),
            outcome: AgentRunOutcome::Succeeded,
        }],
    };
    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "alpha resumed r1: succeeded (r2)\n"
    );
}
