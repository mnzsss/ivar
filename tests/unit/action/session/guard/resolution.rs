//! Which session judges a write: cwd, target owner, ambient id and target fallbacks.

use super::*;

#[test]
fn hall_root_cwd_allows_write_into_the_target_feature_directory() {
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
        "args": { "filePath": layout.feature_dir(&feature.name).join("requirements.md") },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn hall_root_cwd_allows_write_into_a_promoted_worktree() {
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

    let wt = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);
    let payload = serde_json::json!({
        "tool": "edit",
        "args": { "filePath": wt.join("src/index.js") },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn hall_root_cwd_allows_a_new_nested_file_inside_the_writable_set() {
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
        "args": { "filePath": view_dir.join("notes/deep/new.md") },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn hall_root_cwd_denies_a_target_outside_every_session() {
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
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("no ivar session resolves from the cwd or the target path")
    );
}

#[test]
fn hall_root_cwd_denies_an_unpromoted_worktree_target() {
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

    let unpromoted_wt = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    crate::infra::fs::ensure_dir(&unpromoted_wt).unwrap();

    let payload = serde_json::json!({
        "tool": "edit",
        "args": { "filePath": unpromoted_wt.join("src/index.js") },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("no ivar session resolves from the cwd or the target path")
            || out.body.contains("writable set:")
    );
}

#[test]
fn hall_root_cwd_denies_a_relative_target() {
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
        "args": { "filePath": "requirements.md" },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(out.body.contains("relative path") && out.body.contains("belongs to no ivar session"));
}

#[test]
fn relative_write_in_session_uses_payload_cwd() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000071").unwrap();
    let view = layout.feature_session(&feature.name, &id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z");
    state.bind(feature.name, "2026-09-29T00:00:00Z");
    state.write(&view).unwrap();
    let payload =
        serde_json::json!({"tool":"write","args":{"filePath":"notes/deep/new.md"},"cwd":view});
    assert!(
        guard(Provider::Omp, &payload.to_string(), None)
            .unwrap()
            .exit_zero
    );
}

#[test]
fn relative_hall_target_does_not_choose_the_latest_discovery() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000072").unwrap();
    let view = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z")
        .write(&view)
        .unwrap();
    let payload = serde_json::json!({"tool":"write","args":{"filePath":"apps/new.rs"},"cwd":root});
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(out.body.contains("absolute"));
    assert!(!out.body.contains(view.as_str()));
}

#[test]
fn cwd_session_stays_authoritative_for_a_foreign_target() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature1 = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id1 = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir1 = layout.feature_session(&feature1.name, &session_id1);
    crate::infra::fs::ensure_dir(&view_dir1).unwrap();
    let mut state1 =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state1.bind(feature1.name.clone(), "2026-08-29T00:00:00Z");
    state1.write(&view_dir1).unwrap();

    let feature2_name = FeatureName::new("billing").unwrap();
    let ctx = crate::action::Ctx::new(root.clone());
    crate::action::feature::create::create(
        &ctx,
        crate::action::feature::create::CreateInput {
            name: "billing".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.feature_dir(&feature2_name).join("requirements.md") },
        "cwd": view_dir1,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(out.body.contains("writable set:"));
}

#[test]
fn discovery_cwd_denies_a_feature_document_target() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let discovery_session_id = SessionId::new("6f0c9d5f-1111-4000-8000-000000000000").unwrap();
    let discovery_view_dir = layout.discovery_session(&discovery_session_id);
    crate::infra::fs::ensure_dir(&discovery_view_dir).unwrap();
    let state = crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state.write(&discovery_view_dir).unwrap();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.feature_dir(&feature.name).join("plan.md") },
        "cwd": discovery_view_dir,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(out.body.contains("writable set:"));
}

#[test]
fn hall_root_cwd_selects_the_most_recent_session_of_the_feature() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let session_old_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000001").unwrap();
    let view_old = layout.feature_session(&feature.name, &session_old_id);
    crate::infra::fs::ensure_dir(&view_old).unwrap();
    let mut state_old =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state_old.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state_old.write(&view_old).unwrap();

    let session_new_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000002").unwrap();
    let view_new = layout.feature_session(&feature.name, &session_new_id);
    crate::infra::fs::ensure_dir(&view_new).unwrap();
    let mut state_new =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-30T00:00:00Z");
    state_new.bind(feature.name.clone(), "2026-08-30T00:00:00Z");
    state_new.write(&view_new).unwrap();

    // Target newer session's view_dir notes.txt -> allowed
    let payload_new = serde_json::json!({
        "tool": "write",
        "args": { "filePath": view_new.join("notes.txt") },
        "cwd": root,
    });
    let out_new = guard(Provider::Omp, &payload_new.to_string(), None).unwrap();
    assert!(out_new.exit_zero);
    assert_eq!(out_new.body, "");

    // Target older session's view_dir notes.txt -> allowed for its own view dir
    let payload_old = serde_json::json!({
        "tool": "write",
        "args": { "filePath": view_old.join("notes.txt") },
        "cwd": root,
    });
    let out_old = guard(Provider::Omp, &payload_old.to_string(), None).unwrap();
    assert!(out_old.exit_zero);
}

#[test]
fn hall_root_cwd_allows_a_read_outside_every_session() {
    let (_guard, root) = hall_with_promoted_feature();

    let payload = serde_json::json!({
        "tool": "read",
        "args": { "filePath": "/etc/passwd" },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn a_write_inside_the_scratch_dir_is_allowed() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&Layout::session_scratch(&view_dir)).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    state.write(&view_dir).unwrap();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": Layout::session_scratch(&view_dir).join("draft.md") },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn a_hall_root_write_from_no_session_cwd_resolves_to_a_live_session() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view_dir =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000014").unwrap());
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-09-23T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": root.join("docs/topic.md") },
        "cwd": root,
    });

    assert!(
        guard(Provider::Omp, &payload.to_string(), None)
            .unwrap()
            .exit_zero
    );
}

/// #152: the cwd is the hall root, the harness carries session A's id, and
/// the write targets session B's `.tmp/`. The path names B, so B's set
/// judges it and allows it.
#[test]
fn a_hall_root_write_into_another_sessions_tmp_resolves_to_that_session() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id_a = SessionId::new("6f0c9d5f-0000-4000-8000-000000000090").unwrap();
    let view_a = layout.discovery_session(&id_a);
    crate::infra::fs::ensure_dir(&view_a).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-08T00:00:00Z")
        .write(&view_a)
        .unwrap();
    let id_b = SessionId::new("6f0c9d5f-0000-4000-8000-000000000091").unwrap();
    let view_b = layout.discovery_session(&id_b);
    crate::infra::fs::ensure_dir(&view_b).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-08T00:00:01Z")
        .write(&view_b)
        .unwrap();

    let payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": view_b.join(".tmp/run-report.json"), "content": "{}" },
        "cwd": root,
    });
    let evaluation = evaluate(
        Provider::ClaudeCode,
        &payload.to_string(),
        Some(id_a.as_str()),
    )
    .unwrap();

    assert_eq!(
        evaluation
            .session_env
            .as_ref()
            .map(|env| env.session_id.as_str()),
        Some(id_b.as_str())
    );
    assert!(
        matches!(evaluation.decision, GuardDecision::Allow),
        "{:?}",
        evaluation.decision
    );
}

/// A read from the hall root keeps the ambient session: only writes are
/// attributed by their target.
#[test]
fn a_hall_root_read_of_another_sessions_file_keeps_the_ambient_session() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id_a = SessionId::new("6f0c9d5f-0000-4000-8000-000000000092").unwrap();
    let view_a = layout.discovery_session(&id_a);
    crate::infra::fs::ensure_dir(&view_a).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-08T00:00:00Z")
        .write(&view_a)
        .unwrap();
    let id_b = SessionId::new("6f0c9d5f-0000-4000-8000-000000000093").unwrap();
    let view_b = layout.discovery_session(&id_b);
    crate::infra::fs::ensure_dir(&view_b).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-08T00:00:01Z")
        .write(&view_b)
        .unwrap();

    let payload = serde_json::json!({
        "tool_name": "Read",
        "tool_input": { "file_path": view_b.join("discovery.md") },
        "cwd": root,
    });
    let evaluation = evaluate(
        Provider::ClaudeCode,
        &payload.to_string(),
        Some(id_a.as_str()),
    )
    .unwrap();

    assert_eq!(
        evaluation
            .session_env
            .as_ref()
            .map(|env| env.session_id.as_str()),
        Some(id_a.as_str())
    );
}

#[test]
fn ambiguous_target_matching_multiple_features_denies_and_names_all_conflicting_features() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let ctx = crate::action::Ctx::new(root.clone());

    // Create a second feature "billing" promoting the same repo "api"
    feature_create::create(
        &ctx,
        CreateInput {
            name: "billing".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "billing".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    // Session for checkout feature
    let feat_checkout = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let id_checkout = SessionId::new("6f0c9d5f-0000-4000-8000-000000000081").unwrap();
    let view_checkout = layout.feature_session(&feat_checkout.name, &id_checkout);
    crate::infra::fs::ensure_dir(&view_checkout).unwrap();
    let mut state_checkout =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z");
    state_checkout.bind(feat_checkout.name.clone(), "2026-09-29T00:00:00Z");
    state_checkout.write(&view_checkout).unwrap();

    // Session for billing feature
    let feat_billing = Feature::read(&layout, &FeatureName::new("billing").unwrap())
        .unwrap()
        .unwrap();
    let id_billing = SessionId::new("6f0c9d5f-0000-4000-8000-000000000082").unwrap();
    let view_billing = layout.feature_session(&feat_billing.name, &id_billing);
    crate::infra::fs::ensure_dir(&view_billing).unwrap();
    let mut state_billing =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z");
    state_billing.bind(feat_billing.name.clone(), "2026-09-29T00:00:00Z");
    state_billing.write(&view_billing).unwrap();

    // Test decide() directly with Resolution::Ambiguous
    let targets = vec![root.join("some/path.rs")];
    let req = ToolRequest {
        tool: "write".into(),
        targets: targets.clone(),
        writes: true,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };
    let decision = decide(
        &Resolution::Ambiguous {
            features: vec!["billing".to_owned(), "checkout".to_owned()],
        },
        &req,
        &targets,
    );
    match decision {
        GuardDecision::Deny { reason } => {
            assert!(reason.contains("conflicting features"));
            assert!(reason.contains("billing"));
            assert!(reason.contains("checkout"));
        }
        GuardDecision::Allow => panic!("expected deny for ambiguous resolution"),
    }

    // Verify decide() when multiple discovery sessions are also present
    let decision_multi = decide(
        &Resolution::Ambiguous {
            features: vec![
                "6f0c9d5f-0000-4000-8000-000000000001".to_owned(),
                "billing".to_owned(),
                "checkout".to_owned(),
            ],
        },
        &req,
        &targets,
    );
    match decision_multi {
        GuardDecision::Deny { reason } => {
            assert!(reason.contains("conflicting features"));
            assert!(reason.contains("6f0c9d5f-0000-4000-8000-000000000001"));
            assert!(reason.contains("billing"));
            assert!(reason.contains("checkout"));
        }
        GuardDecision::Allow => panic!("expected deny for ambiguous resolution"),
    }
}

#[test]
fn guard_tool_request_from_parent_session_allows_child_worktree_and_denies_sibling_worktree() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let ctx = crate::action::Ctx::new(root.clone());

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout-child".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout-child".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "billing".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "billing".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let parent_feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let child_feature = Feature::read(&layout, &FeatureName::new("checkout-child").unwrap())
        .unwrap()
        .unwrap();
    let sibling_feature = Feature::read(&layout, &FeatureName::new("billing").unwrap())
        .unwrap()
        .unwrap();

    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000010").unwrap();
    let parent_view = layout.feature_session(&parent_feature.name, &session_id);
    crate::infra::fs::ensure_dir(&parent_view).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-02T00:00:00Z");
    state.bind(parent_feature.name.clone(), "2026-10-02T00:00:00Z");
    state.write(&parent_view).unwrap();

    let child_worktree =
        layout.repo_worktree(&RepoName::new("api").unwrap(), &child_feature.branch);
    let sibling_worktree =
        layout.repo_worktree(&RepoName::new("api").unwrap(), &sibling_feature.branch);

    // Write to child worktree from parent session cwd -> allowed
    let child_payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": child_worktree.join("src/lib.rs") },
        "cwd": parent_view
    });
    let child_res = guard(Provider::ClaudeCode, &child_payload.to_string(), None).unwrap();
    let child_body: serde_json::Value = serde_json::from_str(&child_res.body).unwrap();
    assert_eq!(
        child_body["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );

    // Write to sibling worktree from parent session cwd -> denied
    let sibling_payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": sibling_worktree.join("src/lib.rs") },
        "cwd": parent_view
    });
    let sibling_res = guard(Provider::ClaudeCode, &sibling_payload.to_string(), None).unwrap();
    let sibling_body: serde_json::Value = serde_json::from_str(&sibling_res.body).unwrap();
    assert_eq!(
        sibling_body["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
}

/// A subfeature's view links an unpromoted repo to its parent's feature
/// worktree (`BaseView::Parent`), whose write bits stay on for the parent.
/// The agent's hook cwd canonicalises into that worktree, where the
/// promoted-worktree fallback names the parent's newest session; the
/// agent's own `IVAR_SESSION_ID` must win, or the parent's writable set
/// admits the write (R-PARENT-RO).
#[test]
fn a_child_agent_in_its_parents_worktree_is_guarded_as_the_child() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let ctx = crate::action::Ctx::new(root.clone());
    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout-ui".to_owned(),
            branch: None,
            base: None,
            parent: Some("checkout".to_owned()),
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let parent_name = FeatureName::new("checkout").unwrap();
    let parent = Feature::read(&layout, &parent_name).unwrap().unwrap();
    let parent_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &parent.branch);

    // The parent's live session: what the promoted-worktree fallback picks.
    let parent_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000020").unwrap();
    let parent_view = layout.feature_session(&parent_name, &parent_id);
    crate::infra::fs::ensure_dir(&parent_view).unwrap();
    let mut parent_state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-07T00:00:00Z");
    parent_state.bind(parent_name.clone(), "2026-10-07T00:00:00Z");
    parent_state.write(&parent_view).unwrap();

    // The child's session, its `api` linked to the parent's worktree the
    // way `view::materialise` links a `BaseView::Parent` repo.
    let child_name = FeatureName::new("checkout-ui").unwrap();
    let child_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000021").unwrap();
    let child_view = layout.feature_session(&child_name, &child_id);
    crate::infra::fs::ensure_dir(&child_view).unwrap();
    let mut child_state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-07T00:00:01Z");
    child_state.bind(child_name, "2026-10-07T00:00:01Z");
    child_state.write(&child_view).unwrap();
    std::os::unix::fs::symlink(&parent_worktree, child_view.join("api")).unwrap();

    // Through the view's link and from the physical path alike.
    for cwd in [child_view.join("api"), parent_worktree.clone()] {
        let payload = serde_json::json!({
            "tool_name": "Edit",
            "tool_input": {
                "file_path": cwd.join("src/lib.rs"),
                "old_string": "a",
                "new_string": "b",
            },
            "cwd": cwd,
        });
        let evaluation = evaluate(
            Provider::ClaudeCode,
            &payload.to_string(),
            Some(child_id.as_str()),
        )
        .unwrap();
        assert_eq!(
            evaluation
                .session_env
                .as_ref()
                .map(|env| env.session_id.as_str()),
            Some(child_id.as_str()),
            "the agent's own session must resolve from cwd `{cwd}`"
        );
        assert!(
            matches!(evaluation.decision, GuardDecision::Deny { .. }),
            "a child agent must not write its parent's worktree (cwd `{cwd}`): {:?}",
            evaluation.decision
        );
    }
}
