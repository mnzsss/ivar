#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::store::layout::Layout;

#[test]
fn session_env_renders_shell_and_json_and_applies_to_command() {
    let hall = Utf8PathBuf::from("/tmp/acme");
    let layout = Layout::at(hall.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = Utf8PathBuf::from(
        "/tmp/acme/.ivar/features/checkout/sessions/6f0c9d5f-0000-4000-8000-000000000000",
    );
    let feature = FeatureName::new("checkout").unwrap();
    let env = SessionEnv::build(
        &layout,
        &session_id,
        &view_dir,
        Provider::ClaudeCode,
        Some(&feature),
    );

    let shell = env.render_shell();
    assert!(shell.contains(&format!("export IVAR_HALL={hall}")));
    assert!(shell.contains("export IVAR_SESSION_ID=6f0c9d5f-0000-4000-8000-000000000000"));
    assert!(shell.contains(&format!("export IVAR_SESSION_PATH={view_dir}")));
    assert!(shell.contains("export IVAR_PROVIDER=claude-code"));
    assert!(shell.contains("export IVAR_FEATURE=checkout"));

    let json = env.render_json();
    assert_eq!(
        json["IVAR_SESSION_ID"],
        "6f0c9d5f-0000-4000-8000-000000000000"
    );
    assert_eq!(json["IVAR_PROVIDER"], "claude-code");

    let command = env.apply(crate::infra::proc::Command::new("sh"));
    let envs = command.envs();
    assert!(
        envs.iter()
            .any(|(k, v)| k == "IVAR_SESSION_ID" && v == "6f0c9d5f-0000-4000-8000-000000000000")
    );

    // A discovery session carries no feature.
    let discovery_dir = layout.discovery_session(&session_id);
    let discovery = SessionEnv::build(
        &layout,
        &session_id,
        &discovery_dir,
        Provider::ClaudeCode,
        None,
    );
    assert!(!discovery.render_shell().contains("IVAR_FEATURE"));
    assert!(discovery.render_json().get("IVAR_FEATURE").is_none());
}

#[test]
fn omp_session_env_resolves_all_five_variables() {
    let hall = Utf8PathBuf::from("/tmp/acme");
    let layout = Layout::at(hall.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = Utf8PathBuf::from(
        "/tmp/acme/.ivar/features/checkout/sessions/6f0c9d5f-0000-4000-8000-000000000000",
    );
    let feature = FeatureName::new("checkout").unwrap();
    let env = SessionEnv::build(
        &layout,
        &session_id,
        &view_dir,
        Provider::Omp,
        Some(&feature),
    );

    let shell = env.render_shell();
    assert!(shell.contains(&format!("export IVAR_HALL={hall}")));
    assert!(shell.contains("export IVAR_SESSION_ID=6f0c9d5f-0000-4000-8000-000000000000"));
    assert!(shell.contains(&format!("export IVAR_SESSION_PATH={view_dir}")));
    assert!(shell.contains("export IVAR_PROVIDER=omp"));
    assert!(shell.contains("export IVAR_FEATURE=checkout"));

    let json = env.render_json();
    assert_eq!(json["IVAR_HALL"], "/tmp/acme");
    assert_eq!(
        json["IVAR_SESSION_ID"],
        "6f0c9d5f-0000-4000-8000-000000000000"
    );
    assert_eq!(json["IVAR_SESSION_PATH"], view_dir.as_str());
    assert_eq!(json["IVAR_PROVIDER"], "omp");
    assert_eq!(json["IVAR_FEATURE"], "checkout");

    let command = env.apply(crate::infra::proc::Command::new("omp"));
    let envs: std::collections::HashMap<_, _> = command
        .envs()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(envs.get("IVAR_HALL").copied(), Some("/tmp/acme"));
    assert_eq!(
        envs.get("IVAR_SESSION_ID").copied(),
        Some("6f0c9d5f-0000-4000-8000-000000000000")
    );
    assert_eq!(
        envs.get("IVAR_SESSION_PATH").copied(),
        Some(view_dir.as_str())
    );
    assert_eq!(envs.get("IVAR_PROVIDER").copied(), Some("omp"));
    assert_eq!(envs.get("IVAR_FEATURE").copied(), Some("checkout"));
}

use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::session::start::{self as session_start, StartInput};
use crate::domain::name::{BranchName, RepoName};
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

fn hall_with_promoted_feature_and_session() -> (tempfile::TempDir, Utf8PathBuf, String) {
    let (guard, root) = hall_root();
    let ctx = crate::action::Ctx::new(root.clone());
    hall::init(
        &ctx,
        &InitInput {
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
    crate::action::sync::sync(&ctx, &Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
            base: None,
        },
    )
    .unwrap();

    let start_report = session_start::start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();

    (guard, root, start_report.value.session_id)
}

#[test]
fn resolve_by_cwd_from_promoted_worktree() {
    let (_guard, root, session_id) = hall_with_promoted_feature_and_session();
    let layout = Layout::at(root.clone());
    let feature_name = FeatureName::new("checkout").unwrap();
    let feature = Feature::read(&layout, &feature_name).unwrap().unwrap();
    let worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);

    let env = SessionEnv::resolve_by_cwd(&worktree)
        .unwrap()
        .expect("session env should resolve from worktree cwd");
    assert_eq!(env.session_id, session_id);
    assert_eq!(env.feature, Some(feature_name));
}

#[test]
fn resolve_by_cwd_from_promoted_worktree_picks_most_recent_session() {
    let (_guard, root, first_session_id) = hall_with_promoted_feature_and_session();
    let ctx = crate::action::Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature_name = FeatureName::new("checkout").unwrap();
    let feature = Feature::read(&layout, &feature_name).unwrap().unwrap();
    let worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);

    // A feature accumulates sessions; the newest is the one an agent standing
    // in the worktree is running under.
    let second = session_start::start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();

    let env = SessionEnv::resolve_by_cwd(&worktree)
        .unwrap()
        .expect("a worktree cwd resolves even with several sessions");
    assert_eq!(env.session_id, second.value.session_id);
    assert_ne!(env.session_id, first_session_id);
}

/// `hall_with_promoted_feature_and_session` plus a subfeature `checkout-ui`
/// (parent `checkout`, nothing promoted) with one live session recorded on
/// disk. Returns the parent's and the child's session ids.
fn hall_with_child_session() -> (tempfile::TempDir, Utf8PathBuf, String, String) {
    let (guard, root, parent_id) = hall_with_promoted_feature_and_session();
    let ctx = crate::action::Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
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

    let child_name = FeatureName::new("checkout-ui").unwrap();
    let child_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000031").unwrap();
    let view_dir = layout.feature_session(&child_name, &child_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();
    let mut state =
        crate::domain::session::SessionState::new(Provider::ClaudeCode, "2026-10-07T00:00:00Z");
    state.bind(child_name, "2026-10-07T00:00:00Z");
    state.write(&view_dir).unwrap();

    (guard, root, parent_id, child_id.to_string())
}

fn parent_worktree(layout: &Layout) -> Utf8PathBuf {
    let parent = Feature::read(layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    layout.repo_worktree(&RepoName::new("api").unwrap(), &parent.branch)
}

/// A subfeature agent standing in its parent's worktree (its view links
/// the parent's worktree, see `base_view`) keeps its own session: the
/// promoted-worktree fallback would hand it the parent's.
#[test]
fn resolve_for_agent_prefers_the_ambient_session_over_the_promoted_worktree_fallback() {
    let (_guard, root, parent_id, child_id) = hall_with_child_session();
    let layout = Layout::at(root.clone());
    let worktree = parent_worktree(&layout);

    let env = SessionEnv::resolve_for_agent(&worktree, Some(&child_id))
        .unwrap()
        .expect("the ambient session resolves");
    assert_eq!(env.session_id, child_id);
    assert_eq!(env.feature, Some(FeatureName::new("checkout-ui").unwrap()));

    let fallback = SessionEnv::resolve_by_cwd(&worktree)
        .unwrap()
        .expect("resolve_by_cwd keeps the promoted-worktree fallback");
    assert_eq!(
        fallback.session_id, parent_id,
        "resolve_by_cwd must stay env-free and unchanged"
    );
}

/// A view dir found by the walk-up is a filesystem fact and wins over any
/// ambient id (ADR 0003 D1).
#[test]
fn resolve_for_agent_keeps_the_view_dir_walk_over_the_ambient_session() {
    let (_guard, root, parent_id, child_id) = hall_with_child_session();
    let layout = Layout::at(root.clone());
    let parent_view = crate::action::session::lookup::most_recent(
        &layout,
        &FeatureName::new("checkout").unwrap(),
    )
    .unwrap()
    .unwrap()
    .view_dir;

    // The view dir itself, not `<view>/api`: that link canonicalises into the
    // worktree, outside the view dir, where the walk-up finds nothing.
    let env = SessionEnv::resolve_for_agent(&parent_view, Some(&child_id))
        .unwrap()
        .expect("the view dir resolves");
    assert_eq!(env.session_id, parent_id);
}

/// An ambient id that is not exactly a live session of this hall — an
/// unknown id, or a mere prefix of one — is ignored, leaving the fallback.
#[test]
fn resolve_for_agent_ignores_an_ambient_id_that_is_not_a_session_of_this_hall() {
    let (_guard, root, parent_id, child_id) = hall_with_child_session();
    let layout = Layout::at(root.clone());
    let worktree = parent_worktree(&layout);

    for ambient in ["6f0c9d5f-0000-4000-8000-0000000000ff", &child_id[..8]] {
        let env = SessionEnv::resolve_for_agent(&worktree, Some(ambient))
            .unwrap()
            .expect("the fallback still resolves");
        assert_eq!(
            env.session_id, parent_id,
            "ambient `{ambient}` must not select a session"
        );
    }
}
