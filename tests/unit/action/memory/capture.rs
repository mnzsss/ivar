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

// -- the end of a session without a feature starts a hall sync ----------------

const SESSION_END: &str =
    r#"{"hook_event_name":"SessionEnd","session_id":"native-1","reason":"other"}"#;

fn discovery_env(root: &camino::Utf8Path) -> CaptureEnv {
    CaptureEnv {
        feature: None,
        session_path: Some(root.join(".ivar/sessions/sess-1")),
        ..env_in(root)
    }
}

/// Make `ivar` a script that records its arguments and its directory in
/// `marker`, after `delay_secs`, and returns the guard that undoes it.
fn fake_ivar(marker: &camino::Utf8Path, delay_secs: u32) -> crate::infra::proc::Redirect {
    let script = format!("sleep {delay_secs}; echo \"$@ in $(pwd)\" > '{marker}'; echo logged");
    crate::infra::proc::redirect_on_this_thread(
        TEST_IVAR_PROGRAM,
        crate::infra::proc::Command::new("sh").args(["-c", &script, "ivar"]),
    )
}

fn wait_for(path: &camino::Utf8Path) -> String {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("{path} never appeared");
}

#[test]
fn a_session_end_without_a_feature_starts_a_detached_hall_sync_and_returns() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let marker = root.join("marker.txt");
    let _ivar = fake_ivar(&marker, 2);

    let started = std::time::Instant::now();
    capture(Provider::ClaudeCode, SESSION_END, &discovery_env(&root));

    assert!(
        !marker.exists(),
        "capture must not wait for the sync (took {:?})",
        started.elapsed()
    );
    let ran = wait_for(&marker);
    assert!(ran.starts_with("hall sync in "), "was: {ran}");
    assert_eq!(
        std::fs::canonicalize(ran.trim_start_matches("hall sync in ").trim()).unwrap(),
        std::fs::canonicalize(&root).unwrap(),
        "the sync runs in the hall"
    );
    let log = layout.hall_sync_log();
    for _ in 0..200 {
        if std::fs::read_to_string(&log).is_ok_and(|text| text.contains("logged")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "logged");
    assert_eq!(
        spooled(&layout, "sess-1")[0]["source_event"],
        "session.ended",
        "the end is still captured"
    );
}

#[test]
fn a_session_end_with_a_feature_starts_nothing() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let marker = root.join("marker.txt");
    let _ivar = fake_ivar(&marker, 0);

    capture(Provider::ClaudeCode, SESSION_END, &env_in(&root));
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert!(!marker.exists());
    assert!(!Layout::at(root.clone()).hall_sync_log().exists());
}

#[test]
fn other_events_without_a_feature_start_nothing() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let marker = root.join("marker.txt");
    let _ivar = fake_ivar(&marker, 0);

    capture(
        Provider::ClaudeCode,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"native-1","prompt":"hi"}"#,
        &discovery_env(&root),
    );
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert!(!marker.exists());
}

#[test]
fn auto_commit_off_starts_no_sync() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let manifest_path = root.join("ivar.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["auto_commit"] = serde_json::json!(false);
    std::fs::write(&manifest_path, manifest.to_string()).unwrap();
    let marker = root.join("marker.txt");
    let _ivar = fake_ivar(&marker, 0);

    capture(Provider::ClaudeCode, SESSION_END, &discovery_env(&root));
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert!(!marker.exists());
    assert_eq!(
        spooled(&Layout::at(root), "sess-1").len(),
        1,
        "the end is still captured"
    );
}

#[test]
fn a_sync_that_cannot_be_started_never_fails_the_capture() {
    let (_guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    // No fake: the program is not there to run.

    capture(Provider::ClaudeCode, SESSION_END, &discovery_env(&root));

    assert_eq!(spooled(&layout, "sess-1").len(), 1);
}
