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
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

const DISCOVERY_ID: &str = "2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c";
const STARTED_AT: &str = "2026-01-01T00:00:00.000000000Z";

/// A hall with `api` promoted into `checkout`, and a discovery session
/// whose view dir materialises every repo read-only.
fn hall_with_discovery_session() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();

    let origins = root.parent().unwrap().join("origins");
    let api_origin = seeded_repo(&origins.join("api"), "main");
    let web_origin = seeded_repo(&origins.join("web"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![
            Repo::new(
                RepoName::new("api").unwrap(),
                api_origin.as_str(),
                BranchName::new("main").unwrap(),
            ),
            Repo::new(
                RepoName::new("web").unwrap(),
                web_origin.as_str(),
                BranchName::new("main").unwrap(),
            ),
        ],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    feature_create::create(
        &ctx,
        CreateInput {
            name: "checkout".to_owned(),
            branch: None,
        },
    )
    .unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();
    feature_promote::promote(
        &ctx,
        PromoteInput {
            feature: "checkout".to_owned(),
            repo: "api".to_owned(),
        },
    )
    .unwrap();

    (guard, root)
}

/// Materialise a discovery view dir with a session record, as a session
/// created outside ivar (or before session records existed) would leave.
fn discovery_view_dir(layout: &Layout) -> Utf8PathBuf {
    let session_id = SessionId::new(DISCOVERY_ID).unwrap();
    let view_dir = layout.discovery_session(&session_id);
    let manifest = Manifest::read(layout).unwrap().unwrap();
    crate::action::session::view::materialise(
        layout,
        &manifest,
        None,
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();
    let state = SessionState::new(Provider::ClaudeCode, STARTED_AT);
    state.write(&view_dir).unwrap();
    view_dir
}

/// Undo the read-only guards materialisation applied, so the TempDir can
/// clean up after the test.
fn unguard_worktrees(root: &camino::Utf8Path) {
    let repos = root.join(".ivar/repos");
    if !fs::is_dir(&repos).unwrap() {
        return;
    }
    for repo in fs::read_dir(&repos).unwrap() {
        for worktree in fs::read_dir(&repo).unwrap() {
            let _ = fs::restore_write_bits(&worktree);
        }
    }
}

fn feature_name() -> FeatureName {
    FeatureName::new("checkout").unwrap()
}

#[test]
fn convert_moves_the_view_dir_and_rebuilds_symlinks() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let old_dir = discovery_view_dir(&layout);
    assert!(fs::is_dir(&old_dir).unwrap());

    let report = convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let session_id = SessionId::new(DISCOVERY_ID).unwrap();
    let new_dir = layout.feature_session(&feature_name(), &session_id);
    assert_eq!(report.value.view_dir, new_dir);
    assert!(
        !fs::is_dir(&old_dir).unwrap(),
        "the discovery view dir must move"
    );
    assert!(fs::is_dir(&new_dir).unwrap());

    // Symlinks rebuilt for the target feature: api → feature worktree,
    // web → read-only default worktree.
    let api_target = match fs::read_symlink(&new_dir.join("api")).unwrap() {
        fs::SymlinkTarget::Target(path) => path,
        other => panic!("expected a symlink, got {other:?}"),
    };
    assert!(
        api_target.as_str().contains(".ivar/repos/api/checkout"),
        "api must point at the feature worktree: {api_target}"
    );
    let web_target = match fs::read_symlink(&new_dir.join("web")).unwrap() {
        fs::SymlinkTarget::Target(path) => path,
        other => panic!("expected a symlink, got {other:?}"),
    };
    assert!(
        web_target.as_str().contains(".ivar/repos/web/main"),
        "web must point at the read-only default worktree: {web_target}"
    );

    // The transition marker is gone.
    assert!(!fs::exists(&transition_path(&layout, &feature_name())).unwrap());
    unguard_worktrees(&root);
}

/// Conversion binds the discovery session to the feature, and the rematerialised
/// View Dir gains what a feature session carries: the projected plan and the
/// bootstrap instructions. The plan was *not* reachable from the discovery
/// view dir before the conversion.
#[test]
fn convert_projects_the_plan_and_writes_bootstrap_instructions() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let old_dir = discovery_view_dir(&layout);
    assert_eq!(
        fs::read_symlink(&old_dir.join("plans")).unwrap(),
        fs::SymlinkTarget::Absent,
        "a discovery session carries no plan projection"
    );
    let discovery_instructions = fs::read_text(&old_dir.join("CLAUDE.md")).unwrap().unwrap();
    assert!(
        !discovery_instructions.contains("ivar session — feature"),
        "a discovery session carries no bootstrap instructions"
    );
    assert_eq!(
        discovery_instructions,
        fs::read_text(&root.join("HALL.md")).unwrap().unwrap(),
        "a discovery session's instruction file is the canonical content"
    );

    convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let session_id = SessionId::new(DISCOVERY_ID).unwrap();
    let new_dir = layout.feature_session(&feature_name(), &session_id);
    assert!(
        fs::is_file(&new_dir.join("plans/checkout/requirements.md")).unwrap(),
        "the converted view dir must project the feature's plan"
    );
    let instructions = fs::read_text(&new_dir.join("CLAUDE.md")).unwrap().unwrap();
    assert!(
        instructions.contains("ivar session — feature `checkout`"),
        "the converted view dir must carry the bootstrap instructions: {instructions}"
    );
    unguard_worktrees(&root);
}

/// Conversion preserves the session id, provider, and original
/// `started_at` — the state file moves with the directory, unchanged
/// except for the binding.
#[test]
fn convert_preserves_session_id_provider_and_started_at() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    discovery_view_dir(&layout);

    let report = convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.session_id, DISCOVERY_ID);
    let session_id = SessionId::new(DISCOVERY_ID).unwrap();
    let state = SessionState::read(&layout.feature_session(&feature_name(), &session_id))
        .unwrap()
        .unwrap();
    assert_eq!(state.provider(), Provider::ClaudeCode);
    assert_eq!(state.started_at(), STARTED_AT);
    assert_eq!(state.feature().unwrap().as_str(), "checkout");
    assert!(state.feature_bound_at().is_some());
    unguard_worktrees(&root);
}

/// Conversion is one-way: converting an already-converted session is
/// refused.
#[test]
fn convert_refuses_an_already_converted_session() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    discovery_view_dir(&layout);
    convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let failure = convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.convert_already_bound");
    unguard_worktrees(&root);
}

/// Converting a session that is already a feature session (started
/// directly on a feature) is refused the same way.
#[test]
fn convert_refuses_a_feature_session() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    discovery_view_dir(&layout);

    // A feature session: view dir under the feature + bound state.
    let feature_session_id = SessionId::new("3d7f7f2e-3e9b-4c4a-8d3b-7b8f8f0a2c3d").unwrap();
    let feature_dir = layout.feature_session(&feature_name(), &feature_session_id);
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature = Feature::read(&layout, &feature_name()).unwrap().unwrap();
    crate::action::session::view::materialise(
        &layout,
        &manifest,
        Some(&feature),
        Provider::ClaudeCode,
        &feature_dir,
    )
    .unwrap();
    let mut state = SessionState::new(Provider::ClaudeCode, STARTED_AT);
    state.bind(feature_name(), STARTED_AT);
    state.write(&feature_dir).unwrap();

    let failure = convert(
        &ctx,
        ConvertInput {
            session_id: "3d7f7f2e".to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.convert_already_bound");
    unguard_worktrees(&root);
}

#[test]
fn convert_refuses_a_missing_feature() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    discovery_view_dir(&layout);

    let failure = convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "ghost".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "feature.not_found");
    unguard_worktrees(&root);
}

/// The transition marker prevents double-conversion: once it exists, a
/// retry resumes the recorded conversion instead of starting a fresh one
/// (the marker wins over the request, exactly as bifrost does).
#[test]
fn an_interrupted_conversion_resumes_on_retry() {
    let (_guard, root) = hall_with_discovery_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let old_dir = discovery_view_dir(&layout);

    // Simulate a run interrupted after the move but before the state
    // update: the view dir is already in the feature tree, and the marker
    // says so.
    let session_id = SessionId::new(DISCOVERY_ID).unwrap();
    let dest = layout.feature_session(&feature_name(), &session_id);
    let Some(parent) = dest.parent() else {
        panic!("no parent");
    };
    fs::ensure_dir(parent).unwrap();
    fs::rename(&old_dir, &dest).unwrap();
    let transition = Transition {
        session_id: session_id.clone(),
        source: old_dir.clone(),
        feature: feature_name(),
        step: Step::MoveSession,
    };
    write_transition(&layout, &feature_name(), &transition).unwrap();

    // Retry: resumes, completing the move bookkeeping, the state bind,
    // and the re-materialisation.
    let report = convert(
        &ctx,
        ConvertInput {
            session_id: DISCOVERY_ID.to_owned(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.session_id, DISCOVERY_ID);
    assert!(fs::is_dir(&dest).unwrap());
    let state = SessionState::read(&dest).unwrap().unwrap();
    assert_eq!(state.provider(), Provider::ClaudeCode);
    assert_eq!(state.started_at(), STARTED_AT);
    assert_eq!(state.feature().unwrap().as_str(), "checkout");
    assert!(!fs::exists(&transition_path(&layout, &feature_name())).unwrap());
    unguard_worktrees(&root);
}
