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
fn writable_set_is_view_dir_plus_promoted_worktrees() {
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

    // A promoted repo's worktree is writable.
    let api_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);
    assert!(set.allows(&api_worktree));

    // Paths outside the set are NOT writable.
    let hall_root_path = layout.root().to_path_buf();
    assert!(!set.allows(&hall_root_path));
}

// ---------------------------------------------------------------------------
// GuardDecision tests
// ---------------------------------------------------------------------------

/// Build a `WritableSet` whose view dir is a real temp directory so
/// `allows` canonicalises to real paths.
fn writable_set_fixture() -> (WritableSet, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let view = Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap();
    let set = WritableSet::from_parts(view, vec![]);
    (set, dir)
}

#[test]
fn reads_are_never_denied() {
    let req = ToolRequest {
        tool: "Read".into(),
        file_path: Some("/etc/passwd".into()),
    };
    assert!(matches!(decide(None, &req), GuardDecision::Allow));
}

#[test]
fn writes_outside_the_set_are_denied_with_a_reason_naming_the_set() {
    let (set, _guard) = writable_set_fixture();
    let req = ToolRequest {
        tool: "Write".into(),
        file_path: Some("/etc/passwd".into()),
    };
    match decide(Some(&set), &req) {
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
        match decide(Some(&set), &req) {
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
            Some(&set),
            &ToolRequest {
                tool: "Edit".into(),
                file_path: Some(in_set),
            }
        ),
        GuardDecision::Allow
    ));
    assert!(matches!(
        decide(
            Some(&set),
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
            || out.body.contains("no ivar session resolves from the cwd")
    );
}
