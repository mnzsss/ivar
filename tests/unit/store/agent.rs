#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, Trigger};
use crate::test_support::seeded_hall;

fn write_agent(layout: &Layout, name: &str, text: &str) {
    std::fs::create_dir_all(layout.agents_dir()).unwrap();
    std::fs::write(layout.agent_file(name), text).unwrap();
}

#[test]
fn lists_and_parses_agent_files_in_name_order() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "zeta",
        "---\nschedule: \"@daily\"\ntool: codex\n---\nZ\n",
    );
    write_agent(
        &layout,
        "alpha",
        "---\nschedule: \"0 9 * * *\"\ntool: claude\n---\nA\n",
    );
    std::fs::write(layout.agents_dir().join("README.txt"), "not an agent").unwrap();

    let files = list_files(&layout).unwrap();
    assert_eq!(
        files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(parse(&files[0]).unwrap().prompt, "A\n");
}

#[test]
fn an_invalid_file_names_the_file_and_the_field() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "bad",
        "---\nschedule: \"0 0 1 * 1\"\ntool: claude\n---\n",
    );
    let failure = load(&layout, "bad").unwrap_err();
    assert_eq!(failure.code, "agent.invalid");
    assert!(failure.what.contains("agents/bad.md"), "{}", failure.what);
    assert!(failure.what.contains("`schedule`"), "{}", failure.what);
}

#[test]
fn an_unknown_frontmatter_key_is_invalid() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "typo",
        "---\nschedul: \"@daily\"\ntool: claude\n---\n",
    );
    let failure = load(&layout, "typo").unwrap_err();
    assert_eq!(failure.code, "agent.invalid");
    assert!(failure.what.contains("agents/typo.md"), "{}", failure.what);
    assert!(failure.what.contains("`schedul`"), "{}", failure.what);
}

#[test]
fn receipts_round_trip_and_the_latest_is_the_highest_run_id() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    for id in ["20261019T090000Z-aaaaaaaa", "20261019T100000Z-bbbbbbbb"] {
        let receipt =
            AgentRunReceipt::skipped(id.into(), "nightly".into(), Trigger::Schedule, "t".into());
        write_receipt(&layout, &receipt).unwrap();
    }
    let latest = latest_receipt(&layout, "nightly").unwrap().unwrap();
    assert_eq!(latest.run_id, "20261019T100000Z-bbbbbbbb");
    assert_eq!(latest.outcome, AgentRunOutcome::Skipped);
    assert!(
        find_receipt(&layout, "20261019T090000Z-aaaaaaaa")
            .unwrap()
            .is_some()
    );
    assert!(
        read_receipt(&layout, "nightly", "missing")
            .unwrap()
            .is_none()
    );
}
