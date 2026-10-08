//! `WritableSet` roots, the protected hall paths and symlink containment.

use super::*;

#[test]
fn writable_set_is_view_dir_plus_promoted_worktrees_plus_feature_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();

    // The view dir itself is writable.
    assert!(set.allows(&view_dir));
    assert!(set.allows(&view_dir.join("notes.txt")));

    // The feature directory and its working documents are writable.
    let feature_dir = layout.feature_dir(&feature.name);
    assert!(set.allows(&feature_dir));
    assert!(set.allows(&feature_dir.join("plan.md")));
    assert!(set.allows(&feature_dir.join("discovery.md")));
    assert!(set.allows(&feature_dir.join("planning/approvals.json")));

    // A promoted repo's worktree is writable.
    let api_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);
    assert!(set.allows(&api_worktree));

    assert!(!set.allows(&layout.state()));
}

#[test]
fn guard_denies_session_writes_to_feedback_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();

    let feedback_file = layout.feedback_doc("001-bug");
    assert!(!set.allows(&feedback_file));
}

#[test]
fn discovery_session_writable_set_does_not_include_any_feature_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();

    // The view dir itself is writable.
    assert!(set.allows(&view_dir));
    assert!(set.allows(&view_dir.join("scratch.txt")));

    // Feature directories and their docs are NOT writable from discovery.
    let feature_dir = layout.feature_dir(&FeatureName::new("checkout").unwrap());
    assert!(!set.allows(&feature_dir));
    assert!(!set.allows(&feature_dir.join("plan.md")));
    assert!(!set.allows(&feature_dir.join("discovery.md")));

    // Promoted repo worktrees are NOT writable from discovery.
    let api_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("checkout").unwrap(),
    );
    assert!(!set.allows(&api_worktree));

    assert!(!set.allows(&layout.state()));
}

#[test]
fn every_session_may_write_the_hall_root_outside_dot_ivar() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let discovery_view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000001").unwrap());
    let feature_view = layout.feature_session(
        &feature.name,
        &SessionId::new("6f0c9d5f-0000-4000-8000-000000000002").unwrap(),
    );
    crate::infra::fs::ensure_dir(&discovery_view).unwrap();
    crate::infra::fs::ensure_dir(&feature_view).unwrap();

    let discovery = WritableSet::from_discovery(&layout, &discovery_view).unwrap();
    let feature_set = WritableSet::from_session(&layout, &feature, &feature_view).unwrap();
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let foreign_session = "6f0c9d5f-0000-4000-8000-000000000099";
    let foreign_view = layout
        .discovery_sessions_dir()
        .join(foreign_session)
        .join("notes.md");

    for set in [&discovery, &feature_set] {
        assert!(set.allows(&layout.root().join("HALL.md")));
        assert!(set.allows(&layout.root().join("docs/product/001-topic.md")));
        assert!(set.allows(&layout.root().join("ivar.json")));
        assert!(set.allows(&layout.root().join(".claude/skills/custom/SKILL.md")));
        assert!(set.allows(&layout.root().join("new-top-level.md")));
        assert!(set.allows(&layout.hall_skills().join("custom/SKILL.md")));
        assert!(set.allows(&layout.hall_skills_local().join("private/SKILL.md")));
        assert!(set.allows(&layout.hall_setups().join("valhalla.sh")));
        assert!(set.allows(&layout.hall_setups().join("valhalla.session.sh")));
        assert!(!set.allows(&layout.state()));
        assert!(!set.allows(&layout.ivar_dir().join("cache/target/x")));
        assert!(!set.allows(&default_worktree.join("src/lib.rs")));
        assert!(!set.allows(&foreign_view));
    }

    assert!(!discovery.allows(&layout.feature_dir(&feature.name).join("plan.md")));
    assert!(
        !feature_set.allows(
            &layout
                .feature_sessions_dir(&feature.name)
                .join(foreign_session)
                .join("notes.md")
        )
    );
}

#[cfg(unix)]
#[test]
fn a_hall_root_symlink_cannot_escape_the_hall_or_reach_dot_ivar() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000013").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.join("outside")).unwrap();
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    symlink(&default_worktree, root.join("api-link")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view).unwrap();

    assert!(!set.allows(&root.join("outside/file.md")));
    assert!(!set.allows(&root.join("api-link/src/lib.rs")));
}

#[cfg(unix)]
#[test]
fn a_dangling_hall_root_symlink_cannot_create_its_target_outside_the_set() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000014").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside = Utf8PathBuf::try_from(outside.path().to_path_buf()).unwrap();
    symlink(outside.join("new.md"), root.join("outside-new.md")).unwrap();
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    symlink(default_worktree.join("src/new.rs"), root.join("api-new.rs")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view).unwrap();

    assert!(!set.allows(&root.join("outside-new.md")));
    assert!(!set.allows(&root.join("api-new.rs")));
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_in_the_hall_root_is_denied() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000017").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    symlink(root.join("loop-b"), root.join("loop-a")).unwrap();
    symlink(root.join("loop-a"), root.join("loop-b")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view).unwrap();

    assert!(!set.allows(&root.join("loop-a")));
}

#[cfg(unix)]
#[test]
fn canonical_hall_source_symlink_cannot_escape_to_a_default_worktree() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root);
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000003").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills()).unwrap();
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    symlink(&default_worktree, layout.hall_skills().join("escaped")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view).unwrap();
    assert!(!set.allows(&layout.hall_skills().join("escaped/src/lib.rs")));
}

#[test]
fn writable_set_roots_include_canonical_hall_sources() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills()).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills_local()).unwrap();
    crate::infra::fs::write_text(&layout.root().join("HALL.md"), "# Hall\n").unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();
    let roots = set.roots().unwrap();

    assert!(roots.contains(&layout.root().join("HALL.md").canonicalize_utf8().unwrap()));
    assert!(roots.contains(&layout.hall_skills().canonicalize_utf8().unwrap()));
    assert!(roots.contains(&layout.hall_skills_local().canonicalize_utf8().unwrap()));
}

#[test]
fn discovery_writable_set_roots_expand_the_hall_root_without_covering_dot_ivar() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills()).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills_local()).unwrap();
    crate::infra::fs::ensure_dir(&layout.root().join("docs")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let roots = set.roots().unwrap();
    let canonical_root = layout.root().canonicalize_utf8().unwrap();
    let repos = layout.repos_dir().canonicalize_utf8().unwrap();

    assert!(roots.contains(&view_dir.canonicalize_utf8().unwrap()));
    assert!(roots.contains(&canonical_root.join("docs")));
    assert!(roots.contains(&layout.hall_skills().canonicalize_utf8().unwrap()));
    assert!(!roots.contains(&canonical_root));
    assert!(!roots.iter().any(|r| repos.starts_with(r)));
}

const PROTECTED_HALL_PATHS: [&str; 7] = [
    ".git/hooks",
    ".git/config",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".opencode/plugins",
    ".omp/hooks",
    ".omp/extensions",
];

#[test]
fn the_hall_root_denies_git_hooks_git_config_and_provider_hook_config() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000021").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    seed_protected_hall_paths(&root);

    let set = WritableSet::from_discovery(&layout, &view).unwrap();

    for denied in [
        ".git/hooks/pre-commit",
        ".git/config",
        ".claude/settings.json",
        ".claude/settings.local.json",
        ".opencode/plugins/ivar.js",
        ".opencode/plugins/other.js",
        ".omp/hooks/pre/ivar.js",
        ".omp/extensions/ivar.js",
    ] {
        assert!(!set.allows(&root.join(denied)), "{denied} must be denied");
    }
    assert!(set.allows(&root.join(".git/index")));
    assert!(set.allows(&root.join(".git/objects/ab/cdef")));
    assert!(set.allows(&root.join(".claude/skills/custom/SKILL.md")));
    assert!(set.allows(&root.join("docs/x.md")));
}

#[test]
fn hall_root_entries_never_grant_a_protected_path_or_its_ancestor() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000022").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    seed_protected_hall_paths(&root);

    let set = WritableSet::from_discovery(&layout, &view).unwrap();
    let roots = set.roots().unwrap();
    let canonical_root = root.canonicalize_utf8().unwrap();
    let protected = PROTECTED_HALL_PATHS.map(|path| canonical_root.join(path));

    for granted in &roots {
        assert!(
            !protected
                .iter()
                .any(|p| p.starts_with(granted) || granted.starts_with(p)),
            "{granted} covers a protected path: {roots:?}"
        );
    }
    assert!(roots.contains(&canonical_root.join(".git/objects")));
    assert!(roots.contains(&canonical_root.join(".git/index")));
    assert!(roots.contains(&canonical_root.join(".claude/skills")));
    assert!(roots.contains(&canonical_root.join("docs")));
}

#[test]
fn the_hall_root_protects_mcp_config_opencode_node_modules_and_env() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000024").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::infra::fs::ensure_dir(&root.join(".opencode/node_modules/dep")).unwrap();
    crate::infra::fs::ensure_dir(&root.join(".opencode/commands")).unwrap();
    crate::infra::fs::ensure_dir(&root.join(".omp")).unwrap();
    for file in [".mcp.json", ".omp/mcp.json", "opencode.json", ".env"] {
        crate::infra::fs::write_text(&root.join(file), "").unwrap();
    }

    let set = WritableSet::from_discovery(&layout, &view).unwrap();
    let roots = set.roots().unwrap();
    let canonical_root = root.canonicalize_utf8().unwrap();

    for protected in [
        ".mcp.json",
        ".omp/mcp.json",
        "opencode.json",
        ".env",
        ".opencode/node_modules",
    ] {
        assert!(
            !set.allows(&root.join(protected)),
            "{protected} must be denied"
        );
        let protected = canonical_root.join(protected);
        assert!(
            !roots
                .iter()
                .any(|r| protected.starts_with(r) || r.starts_with(&protected)),
            "a root covers {protected}: {roots:?}"
        );
    }
    assert!(!set.allows(&root.join(".opencode/node_modules/dep/index.js")));
    assert!(roots.contains(&canonical_root.join(".opencode/commands")));
}

#[cfg(unix)]
#[test]
fn hall_root_symlinks_to_protected_paths_are_denied_and_never_granted() {
    use std::os::unix::fs::symlink;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000025").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    seed_protected_hall_paths(&root);
    symlink(root.join(".git/hooks"), root.join("x")).unwrap();
    symlink(root.join(".git"), root.join("g")).unwrap();
    symlink(root.join(".claude"), root.join("c")).unwrap();

    let set = WritableSet::from_discovery(&layout, &view).unwrap();

    for denied in [
        "x",
        "x/pre-commit",
        "g/hooks/pre-commit",
        "g/config",
        "c/settings.json",
    ] {
        assert!(!set.allows(&root.join(denied)), "{denied} must be denied");
    }

    let roots = set.roots().unwrap();
    let canonical_root = root.canonicalize_utf8().unwrap();
    for link in ["x", "g", "c"].map(|link| canonical_root.join(link)) {
        assert!(
            !roots.iter().any(|r| r.starts_with(&link)),
            "{link} granted: {roots:?}"
        );
    }
    for target in [".git/hooks", ".git", ".claude"].map(|target| canonical_root.join(target)) {
        assert!(
            !roots.iter().any(|r| target.starts_with(r)),
            "{target} granted: {roots:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_hall_dir_fails_the_roots_instead_of_granting_less() {
    use std::os::unix::fs::PermissionsExt;

    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000023").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    seed_protected_hall_paths(&root);
    let git_dir = root.join(".git");
    let mode = std::fs::metadata(&git_dir).unwrap().permissions().mode();
    std::fs::set_permissions(&git_dir, std::fs::Permissions::from_mode(0o000)).unwrap();

    let set = WritableSet::from_discovery(&layout, &view).unwrap();
    let roots = set.roots();
    std::fs::set_permissions(&git_dir, std::fs::Permissions::from_mode(mode)).unwrap();

    assert_eq!(roots.unwrap_err().code, "guard.unreadable_hall_root");
}

#[test]
fn discovery_guard_allows_a_hall_file_and_denies_ivar_state() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-1111-4000-8000-000000000005").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::domain::session::SessionState::new(Provider::Omp, "2026-09-16T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    let allow = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.hall_skills().join("custom/SKILL.md") },
        "cwd": view_dir,
    });
    assert!(
        guard(Provider::Omp, &allow.to_string(), None)
            .unwrap()
            .exit_zero
    );

    let docs = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.root().join("docs/product/001-topic.md") },
        "cwd": view_dir,
    });
    assert!(
        guard(Provider::Omp, &docs.to_string(), None)
            .unwrap()
            .exit_zero
    );

    let deny = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.state() },
        "cwd": layout.discovery_session(&session_id),
    });
    assert!(
        !guard(Provider::Omp, &deny.to_string(), None)
            .unwrap()
            .exit_zero
    );
}

#[test]
fn symlinked_claude_skills_remain_writable() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000079").unwrap();
    let view = layout.discovery_session(&id);
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-09-29T00:00:00Z")
        .write(&view)
        .unwrap();

    // .claude/skills/my-skill -> .ivar/skills/my-skill
    let hall_skills = layout.hall_skills();
    crate::infra::fs::ensure_dir(&hall_skills.join("my-skill")).unwrap();
    let claude_skills = root.join(".claude/skills");
    crate::infra::fs::ensure_dir(&claude_skills).unwrap();
    crate::infra::fs::create_symlink(
        &hall_skills.join("my-skill"),
        &claude_skills.join("my-skill"),
    )
    .unwrap();

    let payload = serde_json::json!({
        "tool_name":"Write",
        "tool_input":{"file_path":claude_skills.join("my-skill/SKILL.md")},
        "cwd":view
    });
    let out = guard(Provider::ClaudeCode, &payload.to_string(), None).unwrap();
    assert!(out.exit_zero);
}

#[test]
fn a_dotdot_through_a_missing_directory_cannot_escape_the_view_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view =
        layout.discovery_session(&SessionId::new("6f0c9d5f-0000-4000-8000-000000000020").unwrap());
    crate::infra::fs::ensure_dir(&view).unwrap();
    let set = WritableSet::from_discovery(&layout, &view).unwrap();

    assert!(!set.allows(&view.join("missing/../../../../.git/hooks/pre-commit")));
    assert!(!set.allows(&view.join("missing/../../../../../escaped.md")));
    assert!(set.allows(&view.join("notes.md")));
}

#[test]
fn a_dotdot_through_a_missing_directory_is_denied_from_every_writable_root() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view = layout.feature_session(
        &feature.name,
        &SessionId::new("6f0c9d5f-0000-4000-8000-000000000023").unwrap(),
    );
    crate::infra::fs::ensure_dir(&view).unwrap();
    crate::infra::fs::ensure_dir(&root.join("docs")).unwrap();
    let set = WritableSet::from_session(&layout, &feature, &view).unwrap();
    let hall = root.canonicalize_utf8().unwrap();
    let roots = set.roots().unwrap();
    assert!(roots.len() >= 5, "{roots:?}");

    for writable in roots {
        let depth = writable.strip_prefix(&hall).unwrap().components().count();
        let to_hall = writable.join("missing").join("../".repeat(depth + 1));
        assert!(
            !set.allows(&to_hall.join(".git/hooks/pre-commit")),
            "escape from {writable}"
        );
        assert!(
            !set.allows(&to_hall.join("../escaped.md")),
            "escape from {writable}"
        );
    }
}

#[test]
fn a_root_that_resolves_to_the_empty_path_allows_nothing() {
    let (_tmp, dir) = crate::test_support::canonical_temp_dir();
    let view = dir.join("view");
    crate::infra::fs::ensure_dir(&view).unwrap();
    let set = WritableSet::from_parts(view.clone(), None, &[dir.join("missing/../worktree")]);

    assert!(!set.allows(Utf8Path::new("/etc/passwd")));
    assert!(!set.allows(&dir.join("elsewhere.md")));
    assert!(set.allows(&view.join("notes.md")));
}

#[test]
fn parent_session_writable_set_allows_descendant_feature_dir_and_worktrees_but_denies_siblings() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let ctx = crate::action::Ctx::new(root.clone());

    // Create child feature "checkout-child" under parent "checkout"
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

    // Create unrelated sibling feature "billing" (no parent)
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

    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let parent_view = layout.feature_session(&parent_feature.name, &session_id);
    crate::infra::fs::ensure_dir(&parent_view).unwrap();

    let set = WritableSet::from_session(&layout, &parent_feature, &parent_view).unwrap();

    // 1. Parent's own paths are writable.
    let parent_feat_dir = layout.feature_dir(&parent_feature.name);
    let parent_worktree =
        layout.repo_worktree(&RepoName::new("api").unwrap(), &parent_feature.branch);
    assert!(set.allows(&parent_view));
    assert!(set.allows(&parent_feat_dir.join("plan.md")));
    assert!(set.allows(&parent_worktree.join("src/lib.rs")));

    // 2. Child feature's dir and promoted worktrees are writable from parent session.
    let child_feat_dir = layout.feature_dir(&child_feature.name);
    let child_worktree =
        layout.repo_worktree(&RepoName::new("api").unwrap(), &child_feature.branch);
    assert!(set.allows(&child_feat_dir));
    assert!(set.allows(&child_feat_dir.join("plan.md")));
    assert!(set.allows(&child_feat_dir.join("tasks/01-init.md")));
    assert!(set.allows(&child_worktree));
    assert!(set.allows(&child_worktree.join("src/lib.rs")));

    // 3. Child's sessions dir is an exclusion boundary.
    let child_sessions_dir = layout.feature_sessions_dir(&child_feature.name);
    assert!(!set.allows(&child_sessions_dir.join("6f0c9d5f-0000-4000-8000-000000000001/view")));

    // 4. Sibling feature is strictly denied.
    let sibling_feat_dir = layout.feature_dir(&sibling_feature.name);
    let sibling_worktree =
        layout.repo_worktree(&RepoName::new("api").unwrap(), &sibling_feature.branch);
    assert!(!set.allows(&sibling_feat_dir));
    assert!(!set.allows(&sibling_feat_dir.join("plan.md")));
    assert!(!set.allows(&sibling_worktree));
    assert!(!set.allows(&sibling_worktree.join("src/lib.rs")));
}
