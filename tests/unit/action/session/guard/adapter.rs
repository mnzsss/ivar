//! Provider hook adapters: the output shape of `guard()` per provider.

use super::*;

#[test]
fn claude_adapter_allow_and_deny_outputs_characterization() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    let cwd = view_dir.join("src");
    crate::infra::fs::ensure_dir(&cwd).unwrap();

    // Deny: structured write outside writable set
    let deny_payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "/etc/passwd" },
        "cwd": cwd,
    });
    let deny_out = guard(Provider::ClaudeCode, &deny_payload.to_string(), None).unwrap();
    assert!(deny_out.exit_zero);
    let deny_body: serde_json::Value = serde_json::from_str(&deny_out.body).unwrap();
    assert_eq!(
        deny_body["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    assert!(
        deny_body["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("writable")
    );

    // Allow: non-write tool
    let allow_payload = serde_json::json!({
        "tool_name": "Read",
        "tool_input": { "file_path": "/etc/passwd" },
        "cwd": cwd,
    });
    let allow_out = guard(Provider::ClaudeCode, &allow_payload.to_string(), None).unwrap();
    assert!(allow_out.exit_zero);
    let allow_body: serde_json::Value = serde_json::from_str(&allow_out.body).unwrap();
    assert_eq!(
        allow_body["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert_eq!(
        allow_body["hookSpecificOutput"]["permissionDecisionReason"],
        ""
    );
}

#[test]
fn opencode_adapter_allow_and_deny_outputs_characterization() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::OpenCode, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    let cwd = view_dir.join("src");
    crate::infra::fs::ensure_dir(&cwd).unwrap();

    // Deny: exits non-zero, body contains reason
    let deny_payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": "/etc/passwd" },
        "cwd": cwd,
    });
    let deny_out = guard(Provider::OpenCode, &deny_payload.to_string(), None).unwrap();
    assert!(!deny_out.exit_zero);
    assert!(deny_out.body.contains("writable set:"));

    // Allow: exits zero, empty body
    let allow_payload = serde_json::json!({
        "tool": "read",
        "args": { "filePath": "/etc/passwd" },
        "cwd": cwd,
    });
    let allow_out = guard(Provider::OpenCode, &allow_payload.to_string(), None).unwrap();
    assert!(allow_out.exit_zero);
    assert_eq!(allow_out.body, "");
}

#[test]
fn omp_adapter_allows_read_and_non_write_tools() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    let cwd = view_dir.clone();

    let payload = serde_json::json!({
        "tool": "read",
        "args": { "filePath": "/etc/passwd" },
        "cwd": cwd,
    });
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn omp_adapter_denies_unpromoted_write_by_exiting_non_zero() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": "/etc/passwd" },
        "cwd": view_dir,
    });
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    // The embedded hook only enters its `catch` on a non-zero exit, and reads
    // the reason off stdout. Exit 0 with a JSON body would let the write run.
    assert!(!out.exit_zero);
    assert!(out.body.contains("writable set:"));
}

#[test]
fn omp_adapter_denies_write_when_cwd_is_unpromoted_repo_worktree() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    // `api` is the fixture's only repo, promoted onto the feature branch; its
    // default-branch worktree is outside the writable set. A write there, from
    // a cwd inside it, must still be denied.
    let unpromoted_wt = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    crate::infra::fs::ensure_dir(&unpromoted_wt).unwrap();

    let payload = serde_json::json!({
        "tool": "edit",
        "args": { "filePath": unpromoted_wt.join("src/index.js") },
        "cwd": unpromoted_wt,
    });
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body.contains("writable set:")
            || out
                .body
                .contains("no ivar session resolves from the cwd or the target path")
    );
}

#[test]
fn claude_and_opencode_agree_with_omp_on_a_feature_document_target() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    let target = layout.feature_dir(&feature.name).join("requirements.md");

    // Claude Code
    let claude_payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": target },
        "cwd": root,
    });
    let claude_out = guard(Provider::ClaudeCode, &claude_payload.to_string(), None).unwrap();
    let claude_val: serde_json::Value = serde_json::from_str(&claude_out.body).unwrap();
    assert_eq!(
        claude_val["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );

    // OpenCode
    let opencode_payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": target },
        "cwd": root,
    });
    let opencode_out = guard(Provider::OpenCode, &opencode_payload.to_string(), None).unwrap();
    assert!(opencode_out.exit_zero);
    assert_eq!(opencode_out.body, "");
}
