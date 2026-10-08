//! Repository instructions carried by an allowed call.

use super::*;

/// A Claude feature session whose view links `api` to a directory holding
/// `CLAUDE.md`. The directory lies outside every writable root, so a write
/// through the link is denied while a read is allowed.
fn session_with_instructions() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-0000000001c1").unwrap();
    let view_dir = layout.feature_session(&feature, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-06T00:00:00Z");
    state.bind(feature, "2026-10-06T00:00:00Z");
    state.write(&view_dir).unwrap();

    let outside = root.parent().unwrap().join("outside-api");
    crate::infra::fs::ensure_dir(&outside.join("src")).unwrap();
    crate::infra::fs::write_text(
        &outside.join("CLAUDE.md"),
        "Run cargo xtask before commit.\n",
    )
    .unwrap();
    crate::infra::fs::create_symlink(&outside, &view_dir.join("api")).unwrap();
    (guard, view_dir)
}

fn claude_call(view_dir: &Utf8Path, tool: &str, file: &str, call: &str) -> String {
    serde_json::json!({
        "session_id": "sess-main",
        "tool_use_id": call,
        "tool_name": tool,
        "tool_input": { "file_path": view_dir.join(file), "content": "x" },
        "cwd": view_dir,
    })
    .to_string()
}

fn hook_output(out: &GuardOutcome) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(&out.body).unwrap()["hookSpecificOutput"].clone()
}

#[test]
fn an_allowed_claude_call_carries_each_instruction_file_once_per_agent() {
    let (_guard, view_dir) = session_with_instructions();

    let out = guard(
        Provider::ClaudeCode,
        &claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_1"),
        None,
    )
    .unwrap();
    assert!(out.exit_zero);
    let output = hook_output(&out);
    assert_eq!(output["permissionDecision"], "allow");
    let context = output["additionalContext"].as_str().unwrap();
    assert!(
        context.starts_with(&format!(
            "Repository instructions from {view_dir}/api/CLAUDE.md (they apply to work under {view_dir}/api):\n"
        )),
        "{context}"
    );
    assert!(
        context.contains("Run cargo xtask before commit."),
        "{context}"
    );

    // Already delivered to this agent: the next call adds nothing.
    let again = guard(
        Provider::ClaudeCode,
        &claude_call(&view_dir, "Read", "api/src/main.rs", "toolu_2"),
        None,
    )
    .unwrap();
    let output = hook_output(&again);
    assert_eq!(output["permissionDecision"], "allow");
    assert!(output.get("additionalContext").is_none(), "{output}");

    // A subagent keeps its own state (R-SUBAGENTS).
    let mut sub: serde_json::Value =
        serde_json::from_str(&claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_3")).unwrap();
    sub["agent_id"] = serde_json::json!("agent-7");
    let sub_out = guard(Provider::ClaudeCode, &sub.to_string(), None).unwrap();
    assert!(
        hook_output(&sub_out)["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Run cargo xtask before commit."),
        "{}",
        sub_out.body
    );
}

#[test]
fn a_denied_call_carries_no_instructions_and_does_not_mark_them_delivered() {
    let (_guard, view_dir) = session_with_instructions();
    let write = claude_call(&view_dir, "Write", "api/src/new.rs", "toolu_w");

    let out = guard(Provider::ClaudeCode, &write, None).unwrap();
    let output = hook_output(&out);
    assert_eq!(output["permissionDecision"], "deny");
    assert!(output.get("additionalContext").is_none(), "{output}");

    // The slice entries of the denied call run the same decision, emit no
    // permissionDecision and no context, and never reach deliver_slice.
    let slice = guard(Provider::ClaudeCode, &write, Some(1)).unwrap();
    assert!(slice.exit_zero);
    assert_eq!(slice.body, "{}");
    let parsed: serde_json::Value = serde_json::from_str(&slice.body).unwrap();
    assert!(
        parsed
            .pointer("/hookSpecificOutput/additionalContext")
            .is_none()
    );
    assert!(
        parsed
            .pointer("/hookSpecificOutput/permissionDecision")
            .is_none()
    );
    let state = crate::action::session::instructions::state_dir(&view_dir, Provider::ClaudeCode);
    assert!(
        !state.join("toolu_w.ctx").exists(),
        "a denied call must cache nothing"
    );
    assert!(
        !state.join("toolu_w.ctx.lock").exists(),
        "a denied call must lock nothing"
    );

    // Nothing was recorded: the next allowed call still receives the file.
    let read = guard(
        Provider::ClaudeCode,
        &claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_r"),
        None,
    )
    .unwrap();
    assert!(
        hook_output(&read)["additionalContext"].is_string(),
        "{}",
        read.body
    );
}

#[test]
fn omp_and_opencode_allows_carry_the_instructions_as_the_body() {
    for provider in [Provider::Omp, Provider::OpenCode] {
        let (_guard, view_dir) = session_with_instructions();
        let target = view_dir.join("api/src/lib.rs");
        let read = serde_json::json!({
            "tool": "read",
            "args": { "path": target, "filePath": target },
            "cwd": view_dir,
            "agent": "main",
        });
        let out = guard(provider, &read.to_string(), None).unwrap();
        assert!(out.exit_zero, "{provider:?}");
        assert!(
            out.body.starts_with(&format!(
                "Repository instructions from {view_dir}/api/CLAUDE.md"
            )),
            "{provider:?}: {}",
            out.body
        );

        let new_file = view_dir.join("api/src/new.rs");
        let write = serde_json::json!({
            "tool": "write",
            "args": { "path": new_file, "filePath": new_file, "content": "x" },
            "cwd": view_dir,
            "agent": "fresh-agent",
        });
        let denied = guard(provider, &write.to_string(), None).unwrap();
        assert!(!denied.exit_zero, "{provider:?}");
        assert!(
            !denied.body.contains("Repository instructions"),
            "{provider:?}: {}",
            denied.body
        );
    }
}

#[test]
fn a_slice_entry_returns_its_part_and_never_decides() {
    let (_guard, view_dir) = session_with_instructions();
    let slice_chars = crate::action::session::instructions::CLAUDE_SLICE_CHARS;
    let big = "x".repeat(slice_chars + 100);
    crate::infra::fs::write_text(&view_dir.join("api/CLAUDE.md"), &big).unwrap();
    let call = claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_big");

    let entry = guard(Provider::ClaudeCode, &call, None).unwrap();
    let first = hook_output(&entry)["additionalContext"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(first.chars().count(), slice_chars);

    let slice = guard(Provider::ClaudeCode, &call, Some(1)).unwrap();
    assert!(slice.exit_zero);
    let output = hook_output(&slice);
    assert!(output.get("permissionDecision").is_none(), "{output}");
    assert_eq!(output["hookEventName"], "PreToolUse");
    let second = output["additionalContext"].as_str().unwrap();
    assert!(format!("{first}{second}").contains(&big));

    let past_the_end = guard(Provider::ClaudeCode, &call, Some(2)).unwrap();
    assert!(past_the_end.exit_zero);
    assert_eq!(past_the_end.body, "{}");
}

#[test]
fn a_slice_entry_exits_zero_on_unparseable_input() {
    let out = guard(Provider::ClaudeCode, "not json", Some(1)).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "{}");
}

#[test]
fn an_instruction_failure_never_changes_the_decision() {
    // Unreadable instruction file (not UTF-8).
    let (_guard, view_dir) = session_with_instructions();
    crate::infra::fs::write_bytes(&view_dir.join("api/CLAUDE.md"), &[0xff, 0xfe, 0xfd]).unwrap();
    let out = guard(
        Provider::ClaudeCode,
        &claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_u"),
        None,
    )
    .unwrap();
    assert!(out.exit_zero);
    assert_eq!(hook_output(&out)["permissionDecision"], "allow");

    // State directory blocked by a plain file: delivery cannot record anything.
    let (_guard2, view_dir) = session_with_instructions();
    let state = crate::action::session::instructions::state_dir(&view_dir, Provider::ClaudeCode);
    crate::infra::fs::ensure_dir(state.parent().unwrap()).unwrap();
    crate::infra::fs::write_text(&state, "not a directory").unwrap();
    let out = guard(
        Provider::ClaudeCode,
        &claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_b"),
        None,
    )
    .unwrap();
    assert!(out.exit_zero);
    let output = hook_output(&out);
    assert_eq!(output["permissionDecision"], "allow");
    assert!(output.get("additionalContext").is_none(), "{output}");

    // omp keeps its state under its own config dir: block that one too.
    let omp_state = crate::action::session::instructions::state_dir(&view_dir, Provider::Omp);
    crate::infra::fs::ensure_dir(omp_state.parent().unwrap()).unwrap();
    crate::infra::fs::write_text(&omp_state, "not a directory").unwrap();
    let omp = serde_json::json!({
        "tool": "read",
        "args": { "path": view_dir.join("api/src/lib.rs") },
        "cwd": view_dir,
    });
    let out = guard(Provider::Omp, &omp.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn a_call_outside_any_session_carries_no_instructions() {
    let (_guard, view_dir) = session_with_instructions();
    let mut call: serde_json::Value =
        serde_json::from_str(&claude_call(&view_dir, "Read", "api/src/lib.rs", "toolu_n")).unwrap();
    call["cwd"] = serde_json::json!("/");
    let out = guard(Provider::ClaudeCode, &call.to_string(), None).unwrap();
    let output = hook_output(&out);
    assert_eq!(output["permissionDecision"], "allow");
    assert!(output.get("additionalContext").is_none(), "{output}");
}
