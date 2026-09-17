//! Unit tests for `crate::action::session::guard`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::domain::name::{BranchName, FeatureName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};
use camino::Utf8PathBuf;

fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = crate::action::Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        crate::domain::name::HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    (guard, root)
}

// ---------------------------------------------------------------------------
// WritableSet tests
// ---------------------------------------------------------------------------

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

    // Paths outside the set are NOT writable.
    let hall_root_path = layout.root().to_path_buf();
    assert!(!set.allows(&hall_root_path));
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

    // Hall root is NOT writable.
    let hall_root_path = layout.root().to_path_buf();
    assert!(!set.allows(&hall_root_path));
}

#[test]
fn discovery_and_feature_sessions_allow_only_canonical_hall_sources() {
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

    for set in [&discovery, &feature_set] {
        assert!(set.allows(&layout.root().join("HALL.md")));
        assert!(set.allows(&layout.hall_skills().join("custom/SKILL.md")));
        assert!(set.allows(&layout.hall_skills_local().join("private/SKILL.md")));
        assert!(!set.allows(&layout.state()));
        assert!(!set.allows(&layout.root().join(".claude/skills/custom/SKILL.md")));
        assert!(!set.allows(&layout.root().join("ivar.json")));
    }

    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    assert!(!discovery.allows(&default_worktree.join("src/lib.rs")));
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

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();
    let roots = set.roots();

    assert!(
        roots.contains(
            &layout
                .root()
                .join("HALL.md")
                .canonicalize_utf8()
                .unwrap()
                .as_path()
        )
    );
    assert!(roots.contains(&layout.hall_skills().canonicalize_utf8().unwrap().as_path()));
    assert!(
        roots.contains(
            &layout
                .hall_skills_local()
                .canonicalize_utf8()
                .unwrap()
                .as_path()
        )
    );
    assert_eq!(roots.len(), 6);
}

#[test]
fn discovery_writable_set_roots_include_only_view_and_canonical_hall_sources() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills()).unwrap();
    crate::infra::fs::ensure_dir(&layout.hall_skills_local()).unwrap();

    let set = WritableSet::from_discovery(&layout, &view_dir).unwrap();
    let roots = set.roots();

    assert_eq!(roots.len(), 4);
    assert!(roots.contains(&view_dir.canonicalize_utf8().unwrap().as_path()));
    assert!(
        roots.contains(
            &layout
                .root()
                .join("HALL.md")
                .canonicalize_utf8()
                .unwrap()
                .as_path()
        )
    );
    assert!(roots.contains(&layout.hall_skills().canonicalize_utf8().unwrap().as_path()));
    assert!(
        roots.contains(
            &layout
                .hall_skills_local()
                .canonicalize_utf8()
                .unwrap()
                .as_path()
        )
    );
}

#[test]
fn discovery_guard_allows_canonical_source_and_denies_generated_mirror() {
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
    assert!(guard(Provider::Omp, &allow.to_string()).unwrap().exit_zero);

    let deny = serde_json::json!({
        "tool": "write",
        "args": { "filePath": layout.root().join(".omp/skills/custom/SKILL.md") },
        "cwd": layout.discovery_session(&session_id),
    });
    assert!(!guard(Provider::Omp, &deny.to_string()).unwrap().exit_zero);
}

// ---------------------------------------------------------------------------
// GuardDecision tests
// ---------------------------------------------------------------------------

/// Build a `WritableSet` whose view dir is a real temp directory so
/// `allows` canonicalises to real paths.
fn writable_set_fixture() -> (WritableSet, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let view = Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap();
    let set = WritableSet::from_parts(view, None, vec![]);
    (set, dir)
}

#[test]
fn reads_are_never_denied() {
    let req = ToolRequest {
        tool: "Read".into(),
        file_path: Some("/etc/passwd".into()),
    };
    assert!(matches!(
        decide(&Resolution::Unresolved { scratch_dirs: Vec::new() }, &req),
        GuardDecision::Allow
    ));
}

#[test]
fn writes_outside_the_set_are_denied_with_a_reason_naming_the_set() {
    let (set, _guard) = writable_set_fixture();
    let req = ToolRequest {
        tool: "Write".into(),
        file_path: Some("/etc/passwd".into()),
    };
    match decide(&Resolution::Resolved(&set), &req) {
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
        let req = ToolRequest {
            tool: tool.to_owned(),
            file_path: Some("/etc/passwd".into()),
        };
        match decide(&Resolution::Resolved(&set), &req) {
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
                file_path: Some(in_set),
            }
        ),
        GuardDecision::Allow
    ));
    assert!(matches!(
        decide(
            &Resolution::Resolved(&set),
            &ToolRequest {
                tool: "Bash".into(),
                file_path: None,
            }
        ),
        GuardDecision::Allow
    ));
}

// ---------------------------------------------------------------------------
// guard() adapter tests
// ---------------------------------------------------------------------------

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
    let deny_out = guard(Provider::ClaudeCode, &deny_payload.to_string()).unwrap();
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
    let allow_out = guard(Provider::ClaudeCode, &allow_payload.to_string()).unwrap();
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
    let deny_out = guard(Provider::OpenCode, &deny_payload.to_string()).unwrap();
    assert!(!deny_out.exit_zero);
    assert!(deny_out.body.contains("writable set:"));

    // Allow: exits zero, empty body
    let allow_payload = serde_json::json!({
        "tool": "read",
        "args": { "filePath": "/etc/passwd" },
        "cwd": cwd,
    });
    let allow_out = guard(Provider::OpenCode, &allow_payload.to_string()).unwrap();
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
    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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
    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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
    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body.contains("writable set:")
            || out
                .body
                .contains("no ivar session resolves from the cwd or the target path")
    );
}

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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("no ivar session resolves from the cwd or the target path")
            || out.body.contains("writable set:")
    );
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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
    let out_new = guard(Provider::Omp, &payload_new.to_string()).unwrap();
    assert!(out_new.exit_zero);
    assert_eq!(out_new.body, "");

    // Target older session's view_dir notes.txt -> denied
    let payload_old = serde_json::json!({
        "tool": "write",
        "args": { "filePath": view_old.join("notes.txt") },
        "cwd": root,
    });
    let out_old = guard(Provider::Omp, &payload_old.to_string()).unwrap();
    assert!(!out_old.exit_zero);
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
    let claude_out = guard(Provider::ClaudeCode, &claude_payload.to_string()).unwrap();
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
    let opencode_out = guard(Provider::OpenCode, &opencode_payload.to_string()).unwrap();
    assert!(opencode_out.exit_zero);
    assert_eq!(opencode_out.body, "");
}

#[test]
fn hall_root_cwd_allows_a_read_outside_every_session() {
    let (_guard, root) = hall_with_promoted_feature();

    let payload = serde_json::json!({
        "tool": "read",
        "args": { "filePath": "/etc/passwd" },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
}

#[test]
fn scheme_prefixed_targets_are_allowed() {
    let (_guard, root) = hall_with_promoted_feature();

    let schemes = [
        "xd://ast_edit",
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

        let out = guard(Provider::Omp, &payload.to_string()).unwrap();
        assert!(
            out.exit_zero,
            "scheme URI `{uri}` must be allowed by guard, got exit_zero=false with body: {}",
            out.body
        );
        assert_eq!(out.body, "");
    }
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

        let out = guard(Provider::Omp, &payload.to_string()).unwrap();
        assert!(
            !out.exit_zero,
            "non-scheme path `{path}` must be denied by guard when outside session"
        );
    }
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(out.exit_zero);
    assert_eq!(out.body, "");
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
    // the target sits at the hall root, outside every root of that set —
    // `hall_sources` admits `HALL.md` by exact match only — so it is denied.
    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": root.join("elsewhere.md") },
        "cwd": view_dir,
    });

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
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
}

/// The exact call that started this feature: hall-root cwd, a target nowhere
/// near a session. The old reason named no path at all.
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("no ivar session resolves from the cwd or the target path"),
        "the existing sentence is load-bearing: {}",
        out.body
    );
    assert!(
        out.body
            .contains(Layout::session_scratch(&view_dir).as_str()),
        "one live session means one named scratch dir: {}",
        out.body
    );
}

/// Two live sessions must be listed, never picked — naming one would send an
/// agent into another session's view dir.
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

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body.contains(Layout::session_scratch(&first).as_str()),
        "the feature session's scratch dir is missing: {}",
        out.body
    );
    assert!(
        out.body.contains(Layout::session_scratch(&second).as_str()),
        "the discovery session's scratch dir is missing: {}",
        out.body
    );
}

/// With no live session there is no path to offer, and inventing one would be
/// worse than saying so.
#[test]
fn an_unresolved_denial_with_no_live_session_names_no_path() {
    let (_guard, root) = hall_with_promoted_feature();

    let payload = serde_json::json!({
        "tool": "write",
        "args": { "filePath": "/etc/passwd" },
        "cwd": root,
    });

    let out = guard(Provider::Omp, &payload.to_string()).unwrap();
    assert!(!out.exit_zero);
    assert!(
        out.body
            .contains("no ivar session resolves from the cwd or the target path"),
        "{}",
        out.body
    );
    assert!(
        !out.body.contains(crate::domain::session::SCRATCH_DIR),
        "no live session means no scratch dir to name: {}",
        out.body
    );
}


