//! Allow/deny decisions and the guidance in denial messages.

use super::*;

/// Build a `WritableSet` whose view dir is a real temp directory so
/// `allows` canonicalises to real paths.
fn writable_set_fixture() -> (WritableSet, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let view = Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap();
    let set = WritableSet::from_parts(view, None, &[]);
    (set, dir)
}

#[test]
fn reads_are_never_denied() {
    let targets = vec![Utf8PathBuf::from("/etc/passwd")];
    let req = ToolRequest {
        tool: "Read".into(),
        targets: targets.clone(),
        writes: false,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };
    assert!(matches!(
        decide(
            &Resolution::Unresolved {
                scoped_scratch_dirs: Vec::new(),
                live_count: 0,
            },
            &req,
            &targets
        ),
        GuardDecision::Allow
    ));
}

#[test]
fn writes_outside_the_set_are_denied_with_a_reason_naming_the_set() {
    let (set, _guard) = writable_set_fixture();
    let targets = vec![Utf8PathBuf::from("/etc/passwd")];
    let req = ToolRequest {
        tool: "Write".into(),
        targets: targets.clone(),
        writes: true,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };
    match decide(&Resolution::Resolved(&set), &req, &targets) {
        GuardDecision::Deny { reason } => {
            assert!(
                reason.contains("writable"),
                "reason must name the set: {reason}"
            );
        }
        GuardDecision::Allow => panic!("an out-of-set write must be denied"),
    }
}

/// Every structured write tool a provider can send, not just the two the
/// guard was originally written against. `NotebookEdit` and `MultiEdit` are
/// the ones that leaked: they fell through to the permissive arm and wrote
/// wherever they liked.
#[test]
fn every_structured_write_tool_is_denied_outside_the_set() {
    let (set, _guard) = writable_set_fixture();
    for tool in [
        "Write",
        "Edit",
        "MultiEdit",
        "NotebookEdit",
        "ApplyPatch",
        "apply_patch",
        "patch",
    ] {
        let targets = vec![Utf8PathBuf::from("/etc/passwd")];
        let req = ToolRequest {
            tool: tool.to_owned(),
            targets: targets.clone(),
            writes: true,
            search_pattern: None,
            input: serde_json::Value::Null,
            agent: None,
            call_id: None,
        };
        match decide(&Resolution::Resolved(&set), &req, &targets) {
            GuardDecision::Deny { reason } => assert!(
                reason.contains("writable"),
                "`{tool}` must name the set: {reason}"
            ),
            GuardDecision::Allow => panic!("`{tool}` outside the set must be denied"),
        }
    }
}

/// A discovery session binds no feature, and every repo under it is mounted
/// read-only on its default branch. Resolving to `None` made the guard inert
/// exactly there — the session where nothing at all may be written.
#[test]
fn a_discovery_session_resolves_to_an_empty_writable_set() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-00000000dddd").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let env = crate::action::session::env::SessionEnv {
        hall: root.clone(),
        session_id: session_id.to_string(),
        view_dir: view_dir.clone(),
        provider: Provider::ClaudeCode,
        feature: None,
    };

    let set = resolve_writable_set(&env).expect("a discovery session must resolve to a set");
    assert!(
        set.worktrees.is_empty(),
        "a discovery session promotes nothing: {:?}",
        set.worktrees
    );

    // The view dir is still the agent's own scratch space.
    assert!(set.allows(&view_dir.join("notes.md")));

    // A repo mounted read-only under it is not writable.
    let api_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    assert!(
        !set.allows(&api_worktree.join("src/lib.rs")),
        "a discovery session must not write into a read-only worktree"
    );
}

#[test]
fn writes_inside_the_set_are_allowed_and_shell_is_never_classified() {
    let (set, _guard) = writable_set_fixture();
    let in_set = set.view_dir().to_path_buf();
    assert!(matches!(
        decide(
            &Resolution::Resolved(&set),
            &ToolRequest {
                tool: "Edit".into(),
                targets: vec![in_set.clone()],
                writes: true,
                search_pattern: None,
                input: serde_json::Value::Null,
                agent: None,
                call_id: None,
            },
            &[in_set]
        ),
        GuardDecision::Allow
    ));
    assert!(matches!(
        decide(
            &Resolution::Resolved(&set),
            &ToolRequest {
                tool: "Bash".into(),
                targets: Vec::new(),
                writes: false,
                search_pattern: None,
                input: serde_json::Value::Null,
                agent: None,
                call_id: None,
            },
            &[]
        ),
        GuardDecision::Allow
    ));
}

#[test]
fn scheme_prefixed_targets_are_allowed() {
    let (_guard, root) = hall_with_promoted_feature();

    let schemes = [
        "xd://ast_grep",
        "memory://scratchpad",
        "artifact://output-log",
        "agent://worker-1",
        "custom-scheme://resource/path",
    ];

    for uri in schemes {
        let payload = serde_json::json!({
            "tool": "write",
            "args": { "path": uri },
            "cwd": root,
        });

        let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
        assert!(
            out.exit_zero,
            "scheme URI `{uri}` must be allowed by guard, got exit_zero=false with body: {}",
            out.body
        );
        assert_eq!(out.body, "");
    }

    // Under R-GUARD-XD-OTHER, xd://ast_edit without parseable content fails closed and is denied
    let payload_ast = serde_json::json!({
        "tool": "write",
        "args": { "path": "xd://ast_edit" },
        "cwd": root,
    });
    let out_ast = guard(Provider::Omp, &payload_ast.to_string(), None).unwrap();
    assert!(
        !out_ast.exit_zero,
        "xd://ast_edit without parseable content must be denied"
    );
}

#[test]
fn windows_path_or_colon_in_filename_is_not_mistaken_for_uri_scheme() {
    let (_guard, root) = hall_with_promoted_feature();
    // A relative path containing a colon or Windows drive is not a valid RFC 3986 scheme with ://
    let not_schemes = [
        "file:name.txt",
        "123://invalid-scheme",
        "+invalid://foo",
        "./xd://foo",
    ];

    for path in not_schemes {
        let payload = serde_json::json!({
            "tool": "write",
            "args": { "path": path },
            "cwd": root,
        });

        let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
        assert!(
            !out.exit_zero,
            "non-scheme path `{path}` must be denied by guard when outside session"
        );
    }
}

/// The message that started this feature: a denial has to say where a write
/// *does* belong, not only where it does not.
#[test]
fn a_resolved_denial_names_the_scratch_dir_and_keeps_the_writable_set() {
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

    // cwd inside the view dir resolves the session, so the set is Resolved;
    // the target sits under `.ivar/`, which the hall-root rule excludes.
    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.ivar_dir().join("elsewhere.md") },
        "cwd": view_dir,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body.contains("writable set:"),
        "the existing prefix is load-bearing: {}",
        out.body
    );
    assert!(
        out.body
            .contains(Layout::session_scratch(&view_dir).as_str()),
        "the denial must name the scratch dir: {}",
        out.body
    );
    let protected = layout
        .guard_protected_paths()
        .iter()
        .map(|path| canonicalize_lenient(path).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let hall_entry = format!(
        "{} (except {}, {})",
        root.canonicalize_utf8().unwrap(),
        layout.ivar_dir().canonicalize_utf8().unwrap(),
        protected
    );
    assert!(
        out.body.contains(&hall_entry),
        "the denial must name the hall root, its exclusion, and the protected paths: {}",
        out.body
    );
    assert!(
        hall_entry.contains(".git/hooks"),
        "the protected paths must include .git/hooks: {}",
        hall_entry
    );
}

/// The exact call that started this feature: hall-root cwd, a target nowhere
/// near a session. The old reason named no path at all.
#[test]
fn unresolved_denial_when_target_in_feature_with_live_session_lists_only_that_features_scratch() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let feat_session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let feat_view = layout.feature_session(&feature.name, &feat_session_id);
    crate::infra::fs::ensure_dir(&feat_view).unwrap();
    let mut feat_state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    feat_state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    feat_state.write(&feat_view).unwrap();

    let disc_session_id = SessionId::new("7a1d0e60-0000-4000-8000-000000000000").unwrap();
    let disc_view = layout.discovery_session(&disc_session_id);
    crate::infra::fs::ensure_dir(&disc_view).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-08-30T00:00:00Z")
        .write(&disc_view)
        .unwrap();

    // Target inside checkout feature directory
    let target = layout.feature_dir(&feature.name).join("plan.md");
    let scoped = vec![Layout::session_scratch(&feat_view)];
    let req = ToolRequest {
        tool: "write".into(),
        targets: vec![target.clone()],
        writes: true,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };

    let decision = decide(
        &Resolution::Unresolved {
            scoped_scratch_dirs: scoped,
            live_count: 2,
        },
        &req,
        std::slice::from_ref(&target),
    );

    match decision {
        GuardDecision::Deny { reason } => {
            assert!(
                reason.contains("no ivar session resolves from the cwd or the target path"),
                "reason missing first sentence: {reason}"
            );
            assert!(
                reason.contains(Layout::session_scratch(&feat_view).as_str()),
                "must list the matching feature session's scratch dir: {reason}"
            );
            assert!(
                !reason.contains(Layout::session_scratch(&disc_view).as_str()),
                "must NOT list unrelated discovery session scratch dir: {reason}"
            );
        }
        GuardDecision::Allow => panic!("expected Deny, got Allow"),
    }
}

#[test]
fn feature_scratch_dirs_returns_only_matching_feature_scratches() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let feat_session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let feat_view = layout.feature_session(&feature.name, &feat_session_id);
    crate::infra::fs::ensure_dir(&feat_view).unwrap();
    let mut feat_state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    feat_state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    feat_state.write(&feat_view).unwrap();

    let disc_session_id = SessionId::new("7a1d0e60-0000-4000-8000-000000000000").unwrap();
    let disc_view = layout.discovery_session(&disc_session_id);
    crate::infra::fs::ensure_dir(&disc_view).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-08-30T00:00:00Z")
        .write(&disc_view)
        .unwrap();

    let target = layout.feature_dir(&feature.name).join("plan.md");
    let scratches = feature_scratch_dirs(&layout, &target);
    assert_eq!(scratches, vec![Layout::session_scratch(&feat_view)]);
}

/// Target outside every feature with 1 live session states count rather than listing scratch dir.
#[test]
fn an_unresolved_denial_names_the_only_live_sessions_scratch_dir() {
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
            .contains("no ivar session resolves from the cwd or the target path"),
        "the existing sentence is load-bearing: {}",
        out.body
    );
    assert!(
        out.body.contains("; this hall has 1 live session"),
        "target outside every feature states live session count: {}",
        out.body
    );
}

/// Target outside every feature with multiple live sessions states count rather than listing all scratch dirs.
#[test]
fn an_unresolved_denial_lists_every_live_sessions_scratch_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();

    let first_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let first = layout.feature_session(&feature.name, &first_id);
    crate::infra::fs::ensure_dir(&first).unwrap();
    let mut first_state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-08-29T00:00:00Z");
    first_state.bind(feature.name.clone(), "2026-08-29T00:00:00Z");
    first_state.write(&first).unwrap();

    let second_id = SessionId::new("7a1d0e60-0000-4000-8000-000000000000").unwrap();
    let second = layout.discovery_session(&second_id);
    crate::infra::fs::ensure_dir(&second).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-08-30T00:00:00Z")
        .write(&second)
        .unwrap();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": "/etc/passwd" },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("no ivar session resolves from the cwd or the target path"),
        "{}",
        out.body
    );
    assert!(
        out.body.contains("; this hall has 2 live sessions"),
        "target outside every feature states live session count: {}",
        out.body
    );
}

#[test]
fn an_unresolved_denial_with_no_live_session_names_no_path() {
    let (_guard, root) = hall_with_promoted_feature();
    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": "/etc/passwd" },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body.contains("no ivar session resolves from the cwd or the target path; this hall has no live session"),
        "{}",
        out.body
    );
}

#[test]
fn claude_scratchpad_denial_directs_to_session_tmp() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000073").unwrap();
    let view = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-09-29T00:00:00Z")
        .write(&view)
        .unwrap();
    let payload = serde_json::json!({
        "tool_name":"Write",
        "tool_input":{"file_path":"/tmp/claude-1000/-home-user-hall/123/scratchpad/draft.md"},
        "cwd":view
    });
    let out = guard(Provider::ClaudeCode, &payload.to_string(), None).unwrap();
    assert!(!out.body.is_empty());
    assert!(out.body.contains("scratchpad"));
    assert!(out.body.contains(Layout::session_scratch(&view).as_str()));
}

#[test]
fn claude_auto_memory_denial_directs_outside_hall() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000074").unwrap();
    let view = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-09-29T00:00:00Z")
        .write(&view)
        .unwrap();
    let payload = serde_json::json!({
        "tool_name":"Write",
        "tool_input":{"file_path":"/home/user/.claude/projects/-home-user-hall/memory/notes.md"},
        "cwd":view
    });
    let out = guard(Provider::ClaudeCode, &payload.to_string(), None).unwrap();
    assert!(out.body.contains("deny"));
    assert!(
        out.body
            .contains("auto-memory writes outside the hall are not permitted")
    );
    assert!(out.body.contains("hall docs or .ivar/skills"));
}

#[test]
fn unpromoted_repo_denial_suggests_feature_promote() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000075").unwrap();
    let view = layout.feature_session(&feature.name, &id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z");
    state.bind(feature.name.clone(), "2026-09-29T00:00:00Z");
    state.write(&view).unwrap();

    let unpromoted_wt = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    crate::infra::fs::ensure_dir(&unpromoted_wt).unwrap();

    let payload = serde_json::json!({
        "tool":"edit",
        "args":{"filePath":unpromoted_wt.join("src/main.rs")},
        "cwd":view
    });
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(out.body.contains("ivar feature promote api"));
}

#[test]
fn protected_hook_config_denial_suggests_owning_ivar_command() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000076").unwrap();
    let view = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z")
        .write(&view)
        .unwrap();

    let payload = serde_json::json!({
        "tool":"write",
        "args":{"filePath":root.join(".git/hooks/pre-commit")},
        "cwd":view
    });
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(out.body.contains("protected"));
    assert!(out.body.contains("owning `ivar` command"));
}

#[test]
fn foreign_session_view_denial_directs_to_own_view() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let id1 = SessionId::new("6f0c9d5f-0000-4000-8000-000000000077").unwrap();
    let view1 = layout.discovery_session(&id1);
    crate::infra::fs::ensure_dir(&view1).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z")
        .write(&view1)
        .unwrap();

    let id2 = SessionId::new("6f0c9d5f-0000-4000-8000-000000000078").unwrap();
    let view2 = layout.discovery_session(&id2);
    crate::infra::fs::ensure_dir(&view2).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-09-29T00:00:00Z")
        .write(&view2)
        .unwrap();

    let payload = serde_json::json!({
        "tool":"write",
        "args":{"filePath":view2.join("scratch.md")},
        "cwd":view1
    });
    let out = guard(Provider::Omp, &payload.to_string(), None).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("writes to another session's view dir are not permitted")
    );
    assert!(out.body.contains(view1.as_str()));
}

#[test]
fn decide_multi_target_allows_only_if_all_targets_allowed_and_denies_naming_first_disallowed() {
    let (set, _guard) = writable_set_fixture();
    let inside = set.view_dir().join("notes.md");
    let outside = Utf8PathBuf::from("/etc/passwd");
    let targets = vec![inside.clone(), outside.clone()];

    let req = ToolRequest {
        tool: "write".into(),
        targets: targets.clone(),
        writes: true,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };

    let decision = decide(&Resolution::Resolved(&set), &req, &targets);
    match decision {
        GuardDecision::Deny { reason } => {
            assert!(
                reason.contains("/etc/passwd"),
                "denial must name the first disallowed target path: {reason}"
            );
        }
        GuardDecision::Allow => panic!("multi-target with one outside target must be denied"),
    }
}

#[test]
fn decide_all_uri_targets_are_allowed_but_empty_targets_on_write_is_denied() {
    let (set, _guard) = writable_set_fixture();

    let uri_targets = vec![
        Utf8PathBuf::from("local://notes.md"),
        Utf8PathBuf::from("agent://subagent"),
    ];
    let req_uri = ToolRequest {
        tool: "write".into(),
        targets: uri_targets.clone(),
        writes: true,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };
    assert!(matches!(
        decide(&Resolution::Resolved(&set), &req_uri, &uri_targets),
        GuardDecision::Allow
    ));

    let empty_targets: Vec<Utf8PathBuf> = Vec::new();
    let req_empty = ToolRequest {
        tool: "write".into(),
        targets: empty_targets.clone(),
        writes: true,
        search_pattern: None,
        input: serde_json::Value::Null,
        agent: None,
        call_id: None,
    };
    assert!(matches!(
        decide(&Resolution::Resolved(&set), &req_empty, &empty_targets),
        GuardDecision::Deny { .. }
    ));
}
