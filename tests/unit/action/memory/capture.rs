#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::provider::Provider;
use crate::store::layout::Layout;

fn env_in(root: &camino::Utf8Path) -> CaptureEnv {
    CaptureEnv {
        hall: Some(root.to_path_buf()),
        session_id: Some("sess-1".to_owned()),
        provider: None,
        feature: Some("login".to_owned()),
        session_path: Some(root.join(".ivar/features/login/sessions/sess-1")),
    }
}

fn spooled(layout: &Layout, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(layout.memory_spool_dir().join(format!("{key}.jsonl")))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_prompt_is_spooled_sanitized_with_attribution_and_never_touches_sqlite() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"native-1","prompt":"use ghp_0123456789abcdefghijklmnopqrstuvwxyzAB please"}"#;

    capture(Provider::ClaudeCode, payload, &env_in(&root));

    let lines = spooled(&layout, "sess-1");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["source_event"], "session.prompted");
    assert_eq!(lines[0]["session_id"], "sess-1");
    assert_eq!(lines[0]["native_session_id"], "native-1");
    assert_eq!(lines[0]["feature"], "login");
    assert_eq!(lines[0]["provider"], "claude-code");
    assert_eq!(lines[0]["origin"], "user");
    assert_eq!(lines[0]["importance"], 5);
    assert!(!lines[0]["body"].as_str().unwrap().contains("ghp_"));
    assert!(
        !layout.memory_db().exists(),
        "capture must never open the database"
    );
}

#[test]
fn a_session_opened_outside_ivar_is_keyed_by_the_native_id_with_empty_feature_and_repo() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let env = CaptureEnv {
        hall: Some(root.clone()),
        session_id: None,
        provider: None,
        feature: None,
        session_path: None,
    };

    capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"SessionStart","session_id":"native-9","cwd":"/anywhere"}"#,
        &env,
    );

    let lines = spooled(&layout, "native-9");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["session_id"], "");
    assert_eq!(lines[0]["feature"], "");
    assert_eq!(lines[0]["repo"], "");
}

#[test]
fn repo_and_parent_feature_come_from_the_view_dir_and_the_feature_state() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let feature_dir = root.join(".ivar/features/login");
    std::fs::create_dir_all(&feature_dir).unwrap();
    std::fs::write(
        feature_dir.join("feature.json"),
        r#"{"name":"login","parent":"auth","version":3}"#,
    )
    .unwrap();
    let view = env_in(&root).session_path.unwrap();
    let payload = format!(
        r#"{{"hook_event_name":"SessionStart","session_id":"n","cwd":"{}"}}"#,
        view.join("ivar/src")
    );

    capture(Provider::ClaudeCode, &payload, &env_in(&root));

    let lines = spooled(&layout, "sess-1");
    assert_eq!(lines[0]["repo"], "ivar");
    assert_eq!(lines[0]["parent_feature"], "auth");
}

#[test]
fn t3_sessions_record_the_provider_the_session_env_names() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let mut env = env_in(&root);
    env.provider = Some("t3".to_owned());

    capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"SessionStart","session_id":"n"}"#,
        &env,
    );

    assert_eq!(spooled(&layout, "sess-1")[0]["provider"], "t3");
}

#[test]
fn values_in_the_hall_env_file_are_redacted_from_the_spool() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::write(root.join(".env"), "DB_PASSWORD=hunter2-hunter2\nSHORT=ab\n").unwrap();
    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"n","prompt":"the password is hunter2-hunter2 and ab"}"#;

    capture(Provider::ClaudeCode, payload, &env_in(&root));

    let body = spooled(&layout, "sess-1")[0]["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!body.contains("hunter2"), "{body}");
    assert!(
        body.contains("ab"),
        "values under 6 characters are not redacted: {body}"
    );
}

#[test]
fn a_missing_hall_or_a_bad_payload_leaves_nothing_behind() {
    let env = CaptureEnv {
        hall: None,
        session_id: None,
        provider: None,
        feature: None,
        session_path: None,
    };
    capture(Provider::ClaudeCode, "not json", &env);
    capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"SessionStart","session_id":"n"}"#,
        &env,
    );
}

#[test]
fn a_bad_payload_creates_no_spool_even_inside_a_hall() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());

    capture(Provider::ClaudeCode, "not json", &env_in(&root));
    capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"Stop","session_id":"n"}"#,
        &env_in(&root),
    );

    assert!(!layout.memory_spool_dir().exists());
}
