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
