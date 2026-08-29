#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::name::SessionId;
use crate::domain::provider::Provider;
use crate::domain::session::SessionState;
use crate::store::layout::Layout;

#[test]
fn resolve_by_cwd_finds_the_view_dir_without_env_vars() {
    let (guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    // Ambient env is ignored because resolution walks up looking for state.json.

    let env = SessionEnv::resolve_by_cwd(&view_dir).unwrap().unwrap();
    assert_eq!(env.session_id, "6f0c9d5f-0000-4000-8000-000000000000");
    assert_eq!(env.view_dir, view_dir);
    assert_eq!(env.provider, Provider::ClaudeCode);

    // Above the view dir but below the hall: still resolves by walk-up.
    let nested = view_dir.join("src/pkg");
    crate::infra::fs::ensure_dir(&nested).unwrap();
    assert!(SessionEnv::resolve_by_cwd(&nested).unwrap().is_some());

    // Outside any view dir: None.
    let elsewhere = root.parent().unwrap().join("elsewhere");
    crate::infra::fs::ensure_dir(&elsewhere).unwrap();
    assert!(SessionEnv::resolve_by_cwd(&elsewhere).unwrap().is_none());

    drop(guard);
}

#[test]
fn env_command_returns_session_env_when_inside_session() {
    use crate::action::Ctx;
    use crate::action::session::env_cmd::{EnvInput, run};

    let (guard, root) = crate::test_support::seeded_hall();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-08-29T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    let ctx = Ctx::new(view_dir.clone());
    let report = run(&ctx, EnvInput::default()).unwrap();
    let outcome = report.value;

    assert_eq!(outcome.session_id, "6f0c9d5f-0000-4000-8000-000000000000");
    assert_eq!(outcome.provider, Provider::ClaudeCode);

    drop(guard);
}
