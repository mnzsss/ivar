#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

fn seed_tool(layout: &Layout, session: &str) {
    let observation = Observation {
        event_id: format!("{session}-tool"),
        source_event: "session.tool".to_owned(),
        occurred_at: "2026-11-06T00:00:00Z".to_owned(),
        session_id: session.to_owned(),
        native_session_id: "native".to_owned(),
        provider: "claude-code".to_owned(),
        ivar_version: "0".to_owned(),
        feature: "checkout".to_owned(),
        parent_feature: String::new(),
        repo: String::new(),
        origin: String::new(),
        importance: Importance::session_tool(ToolEffect::Reads),
        attempt_id: String::new(),
        body: "{}".to_owned(),
    };
    let ctx = SanitizeContext::new(layout.root().to_path_buf(), Vec::new());
    let mut db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    insert_batch(&mut db, &[Sanitized::sanitize(observation, &ctx).unwrap()]).unwrap();
}

fn seed_handoff(layout: &Layout, owner: &str) {
    use crate::domain::coordination::{NewHandoff, Owner, Queue};
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    crate::store::memory::coordination::insert_handoff(
        db.conn(),
        &NewHandoff {
            id: "h-1".to_owned(),
            queue: Queue::Hall,
            address: None,
            owner: Owner {
                key: owner.to_owned(),
                provider: "claude-code".to_owned(),
            },
            summary: "Resume".to_owned(),
            pending_tasks: Vec::new(),
            open_decisions: Vec::new(),
            modified_files: Vec::new(),
            created_at: 1,
            expires_at: i64::MAX,
        },
    )
    .unwrap();
}

fn codes(warnings: &[Warning]) -> Vec<&'static str> {
    warnings.iter().map(|w| w.code).collect()
}

#[test]
fn stop_warns_when_an_active_session_opened_no_handoff() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let id = session_id_of(&root);
    seed_tool(&layout, &id);

    let report = stop(
        &ctx,
        &StopInput {
            session: Some(id.clone()),
            all: false,
        },
    )
    .unwrap();

    assert_eq!(report.value.stopped, 1);
    let warning = report
        .warnings
        .iter()
        .find(|w| w.code == "session.handoff_missing")
        .unwrap_or_else(|| panic!("no handoff warning in {:?}", report.warnings));
    assert_eq!(warning.subject, id);
    assert!(warning.what.contains("memory_handoff_begin"));
    assert!(
        warning
            .what
            .contains(&format!("ivar memory handoff begin --session {id}"))
    );
    unguard_worktrees(&root);
}

#[test]
fn stop_stays_quiet_once_the_session_opened_a_handoff_or_did_nothing() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let id = session_id_of(&root);
    seed_tool(&layout, &id);
    seed_handoff(&layout, &id);

    let report = stop(
        &ctx,
        &StopInput {
            session: Some(id),
            all: false,
        },
    )
    .unwrap();
    assert!(!codes(&report.warnings).contains(&"session.handoff_missing"));
    unguard_worktrees(&root);

    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let report = stop(
        &ctx,
        &StopInput {
            session: Some(session_id_of(&root)),
            all: false,
        },
    )
    .unwrap();
    assert!(!codes(&report.warnings).contains(&"session.handoff_missing"));
    unguard_worktrees(&root);
}

#[test]
fn stop_all_warns_once_for_each_active_session_without_a_handoff() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    session_start::start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
            prompt: None,
        },
    )
    .unwrap();
    let sessions =
        fs::read_dir(&layout.feature_sessions_dir(&FeatureName::new("checkout").unwrap()))
            .unwrap()
            .into_iter()
            .filter_map(|dir| dir.file_name().map(str::to_owned))
            .filter(|name| !name.starts_with('.'))
            .collect::<Vec<_>>();
    for id in &sessions {
        seed_tool(&layout, id);
    }

    let report = stop(
        &ctx,
        &StopInput {
            session: None,
            all: true,
        },
    )
    .unwrap();

    assert_eq!(report.value.stopped, 2);
    let mut warned: Vec<&str> = report
        .warnings
        .iter()
        .filter(|w| w.code == "session.handoff_missing")
        .map(|w| w.subject.as_str())
        .collect();
    warned.sort_unstable();
    let mut expected: Vec<&str> = sessions.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(warned, expected);
    unguard_worktrees(&root);
}

#[test]
fn stop_without_a_memory_db_neither_warns_nor_creates_one() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let _ = std::fs::remove_file(layout.memory_db().as_std_path());

    let report = stop(
        &ctx,
        &StopInput {
            session: Some(session_id_of(&root)),
            all: false,
        },
    )
    .unwrap();

    assert!(!codes(&report.warnings).contains(&"session.handoff_missing"));
    assert!(!layout.memory_db().as_std_path().exists());
    unguard_worktrees(&root);
}
