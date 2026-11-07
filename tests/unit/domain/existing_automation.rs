#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

const DAYS: Days<'static> = Days {
    state_since: "2026-10-08",
    window_since: "2026-08-09",
};

fn decision(form: Form, updated_at: &str) -> Decision {
    Decision {
        id: 1,
        kind: crate::domain::automation::DecisionKind::Accepted,
        intent: "changelog".to_owned(),
        form: Some(form),
        artifact: Some(".ivar/scripts/changelog.sh".to_owned()),
        until: None,
        created_at: updated_at.to_owned(),
        updated_at: updated_at.to_owned(),
        texts: BTreeSet::new(),
    }
}

fn request<'a>(session_id: &'a str, day: &'a str) -> Request<'a> {
    Request { session_id, day }
}

const OLD: &str = "2026-05-01T10:00:00.000Z";
const RECENT: &str = "2026-10-20T10:00:00.000Z";

#[test]
fn a_use_in_the_last_30_days_is_in_use() {
    let requests = [request("s1", "2026-10-20"), request("s2", "2026-10-25")];

    let found = assess(&decision(Form::Script, OLD), &requests, DAYS, |s| s == "s2");

    assert_eq!(found.state, State::InUse);
    assert_eq!(found.sessions_with_use, ["s2"]);
    assert_eq!(found.sessions_without_use, ["s1"]);
    assert!(!found.retirable);
}

#[test]
fn a_request_without_a_use_is_unused() {
    let requests = [request("s1", "2026-10-20")];

    let found = assess(&decision(Form::Script, OLD), &requests, DAYS, |_| false);

    assert_eq!(found.state, State::Unused);
    assert!(found.sessions_with_use.is_empty());
    assert_eq!(found.sessions_without_use, ["s1"]);
}

#[test]
fn an_unused_script_is_not_retirable() {
    let requests = [request("s1", "2026-10-20")];

    let found = assess(&decision(Form::Script, OLD), &requests, DAYS, |_| false);

    assert_eq!(found.state, State::Unused);
    assert!(!found.retirable);
}

#[test]
fn a_request_only_older_than_30_days_is_no_requests() {
    let requests = [request("s1", "2026-09-01")];

    let found = assess(&decision(Form::Script, OLD), &requests, DAYS, |_| true);

    assert_eq!(found.state, State::NoRequests);
    assert!(found.sessions_with_use.is_empty());
    assert!(
        !found.retirable,
        "a request 40 days ago is inside the 90 days"
    );
}

#[test]
fn a_prompt_outside_a_session_is_a_request_but_never_a_session() {
    let requests = [request("", "2026-10-20")];

    let found = assess(&decision(Form::Script, OLD), &requests, DAYS, |_| true);

    assert_eq!(found.state, State::Unused);
    assert!(found.sessions_with_use.is_empty());
    assert!(found.sessions_without_use.is_empty());
}

#[test]
fn an_old_decision_nothing_asked_for_in_90_days_is_retirable() {
    for form in [Form::Script, Form::Skill, Form::Procedure] {
        let found = assess(&decision(form, OLD), &[], DAYS, |_| false);

        assert_eq!(found.state, State::NoRequests);
        assert!(found.retirable, "{form}");
    }
}

#[test]
fn an_agent_is_never_retirable() {
    let found = assess(&decision(Form::Agent, OLD), &[], DAYS, |_| false);

    assert!(!found.retirable);
}

#[test]
fn a_request_inside_the_90_days_keeps_it_from_being_retirable() {
    let requests = [request("s1", "2026-08-09")];

    let found = assess(&decision(Form::Script, OLD), &requests, DAYS, |_| false);

    assert!(!found.retirable);
}

#[test]
fn a_young_or_renewed_decision_is_not_retirable_yet() {
    let found = assess(&decision(Form::Script, RECENT), &[], DAYS, |_| false);

    assert!(!found.retirable);
}

#[test]
fn a_decision_updated_on_the_first_day_of_the_window_is_retirable() {
    let found = assess(
        &decision(Form::Script, "2026-08-09T00:00:00.000Z"),
        &[],
        DAYS,
        |_| false,
    );

    assert!(found.retirable);
}

#[test]
fn an_unreadable_updated_at_is_never_retirable() {
    for text in ["then", ""] {
        let found = assess(&decision(Form::Script, text), &[], DAYS, |_| false);

        assert!(!found.retirable, "{text:?}");
    }
}

#[test]
fn a_script_or_procedure_is_cited_by_its_hall_relative_path() {
    assert_eq!(
        cited_as(Form::Script, "./.ivar/scripts/changelog.sh").as_deref(),
        Some(".ivar/scripts/changelog.sh")
    );
    assert_eq!(
        cited_as(Form::Procedure, "docs/procedures/release.md").as_deref(),
        Some("docs/procedures/release.md")
    );
    assert_eq!(cited_as(Form::Script, "../outside.sh"), None);
}

#[test]
fn a_skill_or_agent_is_cited_by_its_name() {
    assert_eq!(
        cited_as(Form::Agent, "agents/reviewer.md").as_deref(),
        Some("reviewer")
    );
    assert_eq!(
        cited_as(Form::Skill, ".ivar/skills/deploy.md").as_deref(),
        Some("deploy")
    );
    assert_eq!(
        cited_as(Form::Skill, ".ivar/skills/deploy/SKILL.md").as_deref(),
        Some("deploy")
    );
}

#[test]
fn a_citation_is_a_whole_word() {
    assert!(cites("bash .ivar/scripts/x.sh --now", ".ivar/scripts/x.sh"));
    assert!(cites("/hall/.ivar/scripts/x.sh", ".ivar/scripts/x.sh"));
    assert!(cites("deploy", "deploy"));
    assert!(cites("run /deploy now", "deploy"));
    assert!(cites("use deploy.", "deploy"));
    assert!(!cites("redeploy", "deploy"));
    assert!(!cites("deploy-all", "deploy"));
    assert!(!cites("deploy_all", "deploy"));
    assert!(!cites("deploy.sh", "deploy"));
    assert!(!cites("other.ivar/scripts/x.sh", ".ivar/scripts/x.sh"));
    assert!(!cites("anything", ""));
}

#[test]
fn only_the_input_of_a_tool_observation_cites() {
    let ran = r#"{"tool":"Bash","input":{"command":"bash .ivar/scripts/x.sh"},"result":""}"#;
    let read = r#"{"tool":"Read","path":".ivar/scripts/x.sh"}"#;
    let in_result = r#"{"tool":"Bash","input":{"command":"ls"},"result":".ivar/scripts/x.sh"}"#;

    assert!(body_cites(ran, Form::Script, ".ivar/scripts/x.sh"));
    assert!(!body_cites(in_result, Form::Script, ".ivar/scripts/x.sh"));
    assert!(!body_cites(
        "not json .ivar/scripts/x.sh",
        Form::Script,
        ".ivar/scripts/x.sh"
    ));
    assert!(
        !body_cites(read, Form::Script, ".ivar/scripts/x.sh"),
        "opening a script is not running it"
    );
}

#[test]
fn only_a_procedure_is_used_by_reading_it() {
    let read = r#"{"tool":"Read","path":"docs/procedures/release.md"}"#;

    assert!(body_cites(
        read,
        Form::Procedure,
        "docs/procedures/release.md"
    ));
    assert!(!body_cites(
        read,
        Form::Script,
        "docs/procedures/release.md"
    ));
    assert!(!body_cites(read, Form::Skill, "release"));
    assert!(!body_cites(read, Form::Agent, "release"));
}

#[test]
fn a_skill_is_cited_through_a_nested_input_value() {
    let body = r#"{"tool":"Skill","input":{"skill":"deploy","args":["now"]},"result":""}"#;

    assert!(body_cites(body, Form::Skill, "deploy"));
    assert!(!body_cites(body, Form::Skill, "now-or-never"));
}

#[test]
fn the_scope_is_personal_only_in_the_hall_personal_folders() {
    assert_eq!(scope_of(".ivar/scripts/x.sh"), ScriptScope::Team);
    assert_eq!(scope_of(".ivar/scripts-local/x.sh"), ScriptScope::Personal);
    assert_eq!(
        scope_of(".ivar/skills-local/deploy/SKILL.md"),
        ScriptScope::Personal
    );
    assert_eq!(
        scope_of("./.ivar/scripts-local/x.sh"),
        ScriptScope::Personal
    );
    assert_eq!(scope_of("agents/reviewer.md"), ScriptScope::Team);
    assert_eq!(scope_of("x-local"), ScriptScope::Team);
    assert_eq!(
        scope_of("tools/repo-local/x.sh"),
        ScriptScope::Team,
        "only the hall's personal folders are personal"
    );
}

#[test]
fn the_state_serializes_as_its_name() {
    for (state, name) in [
        (State::InUse, "in use"),
        (State::Unused, "unused"),
        (State::NoRequests, "no requests"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), name);
        assert_eq!(state.to_string(), name);
    }
}

#[test]
fn the_recent_sessions_are_the_named_sessions_of_the_last_30_days() {
    let requests = [
        request("s1", "2026-10-20"),
        request("s1", "2026-10-21"),
        request("", "2026-10-22"),
        request("s2", "2026-09-01"),
    ];

    let sessions: Vec<&str> = recent_sessions(&requests, DAYS).into_iter().collect();

    assert_eq!(sessions, ["s1"]);
}
