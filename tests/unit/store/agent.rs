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

#[test]
fn every_example_agent_validates() {
    let dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/agents");
    let examples: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| Utf8PathBuf::try_from(entry.unwrap().path()).unwrap())
        .filter(|path| path.extension() == Some("md") && path.file_stem() != Some("README"))
        .collect();
    assert!(examples.len() >= 4, "expected the example agents in {dir}");
    for path in examples {
        let file = AgentFile {
            name: path.file_stem().unwrap().to_owned(),
            source: std::fs::read_to_string(&path).unwrap(),
            path,
        };
        parse(&file).unwrap_or_else(|f| panic!("{}", f.what));
    }
}

#[test]
fn set_notify_replaces_the_line_and_keeps_the_body() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "nightly",
        "---\nschedule: \"@daily\"\ntool: claude\nnotify: https://hooks.example.com/x\ntimeout: 5m\n---\nGo.\n\n---\nnot frontmatter\n",
    );
    let file = list_files(&layout).unwrap().remove(0);
    set_notify(&file, "telegram:-100123").unwrap();
    assert_eq!(
        std::fs::read_to_string(layout.agent_file("nightly")).unwrap(),
        "---\nschedule: \"@daily\"\ntool: claude\nnotify: telegram:-100123\ntimeout: 5m\n---\nGo.\n\n---\nnot frontmatter\n"
    );
}

#[test]
fn set_notify_inserts_the_line_when_absent() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "nightly",
        "---\nschedule: \"@daily\"\ntool: claude\n---\nGo.\n",
    );
    let file = list_files(&layout).unwrap().remove(0);
    set_notify(&file, "telegram:@ivar_runs").unwrap();
    assert_eq!(
        std::fs::read_to_string(layout.agent_file("nightly")).unwrap(),
        "---\nschedule: \"@daily\"\ntool: claude\nnotify: telegram:@ivar_runs\n---\nGo.\n"
    );
    assert_eq!(
        load(&layout, "nightly").unwrap().notify,
        Some(crate::domain::agent::spec::NotifyTarget::Telegram {
            chat_id: "@ivar_runs".into()
        })
    );
}

#[test]
fn set_notify_leaves_the_file_alone_when_the_result_does_not_validate() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let original = "---\nschedule: \"@daily\"\ntool: claude\n---\nGo.\n";
    write_agent(&layout, "nightly", original);
    let file = list_files(&layout).unwrap().remove(0);
    let failure = set_notify(&file, "telegram:abc").unwrap_err();
    assert_eq!(failure.code, "agent.invalid");
    assert_eq!(
        std::fs::read_to_string(layout.agent_file("nightly")).unwrap(),
        original
    );
}

#[test]
fn set_notify_keeps_crlf_line_endings_and_the_body() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "nightly",
        "---\r\nschedule: \"@daily\"\r\ntool: claude\r\n---\r\nGo.\r\nNow.\r\n",
    );
    let file = list_files(&layout).unwrap().remove(0);
    set_notify(&file, "telegram:-100123").unwrap();
    assert_eq!(
        std::fs::read_to_string(layout.agent_file("nightly")).unwrap(),
        "---\r\nschedule: \"@daily\"\r\ntool: claude\r\nnotify: telegram:-100123\r\n---\r\nGo.\r\nNow.\r\n"
    );
}

#[cfg(unix)]
#[test]
fn set_notify_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    write_agent(
        &layout,
        "nightly",
        "---\nschedule: \"@daily\"\ntool: claude\n---\nGo.\n",
    );
    let path = layout.agent_file("nightly");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let file = list_files(&layout).unwrap().remove(0);
    set_notify(&file, "telegram:-100123").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}
