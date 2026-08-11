//! Unit tests for `crate::action::session::start`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::plan::create::{self as plan_create, CreateInput as PlanCreateInput};
use crate::action::session::conversion::{self, ConvertInput};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{git, hall_root, seeded_repo};

fn hall_with_promoted_feature() -> (tempfile::TempDir, Utf8PathBuf) {
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

    let origin = seeded_repo(&root.parent().unwrap().join("origins").join("api"), "main");
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
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

#[test]
fn materialise_view_dir_symlinks_promoted_repos() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap(),
    );

    crate::action::session::view::materialise(
        &layout,
        &manifest,
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    let link = view_dir.join("api");
    assert!(
        fs::is_dir(&link).unwrap(),
        "the api symlink must resolve to a dir"
    );
    let target = match fs::read_symlink(&link).unwrap() {
        fs::SymlinkTarget::Target(path) => path,
        other => panic!("expected a symlink, got {other:?}"),
    };
    assert!(
        target.as_str().contains(".ivar/repos/api/checkout"),
        "the symlink must point at the feature worktree: {target}"
    );
}

/// `<view_dir>/.claude` must be a
/// real directory, never a symlink to the hall's own — a symlinked one
/// would send a later per-session `settings.json` write into
/// `hall/.claude`, applying one workstream's write guard to every session
/// sharing the hall.
#[test]
fn materialise_view_dir_makes_the_config_dir_real_not_a_symlink() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
    );

    crate::action::session::view::materialise(
        &layout,
        &manifest,
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    let config_dir = view_dir.join(Provider::ClaudeCode.config_dir());
    assert!(
        fs::is_dir(&config_dir).unwrap(),
        "the config dir must exist and resolve to a directory"
    );
    assert_eq!(
        fs::read_symlink(&config_dir).unwrap(),
        fs::SymlinkTarget::NotASymlink,
        "the config dir itself must be a real directory, not a symlink"
    );
}

/// `commands/` inside the real config dir must still be the hall's own,
/// so the hall's shipped `/ivar-*` commands reach the agent.
#[test]
fn materialise_view_dir_symlinks_hall_commands_into_the_config_dir() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
    );

    crate::action::session::view::materialise(
        &layout,
        &manifest,
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    let commands_link = view_dir
        .join(Provider::ClaudeCode.config_dir())
        .join("commands");
    let target = read_link_target(&commands_link);
    assert_eq!(
        target,
        layout.commands_dir(&Provider::ClaudeCode),
        "commands/ must resolve to the hall's own commands dir"
    );
    assert!(
        fs::is_file(&commands_link.join("ivar-execute.md")).unwrap(),
        "a shipped ivar-* command must be reachable through the symlink"
    );
}

/// The view dir is re-materialised on every `session connect`; the config
/// dir and its commands symlink must stay stable across repeated
/// materialisation, not churn or error out the second time.
#[test]
fn materialise_view_dir_is_stable_across_repeated_materialisation() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
    );

    crate::action::session::view::materialise(
        &layout,
        &manifest,
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();
    let commands_link = view_dir
        .join(Provider::ClaudeCode.config_dir())
        .join("commands");
    let first_target = read_link_target(&commands_link);

    // Re-materialise, exactly as `session connect` would.
    crate::action::session::view::materialise(
        &layout,
        &manifest,
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    assert!(
        fs::is_dir(&view_dir.join(Provider::ClaudeCode.config_dir())).unwrap(),
        "the config dir must still be a real directory after re-materialisation"
    );
    assert_eq!(
        fs::read_symlink(&view_dir.join(Provider::ClaudeCode.config_dir())).unwrap(),
        fs::SymlinkTarget::NotASymlink,
        "the config dir must still not be a symlink after re-materialisation"
    );
    let second_target = read_link_target(&commands_link);
    assert_eq!(
        first_target, second_target,
        "the commands symlink must point at the same place after re-materialisation"
    );
}

#[test]
fn resolve_provider_uses_the_manifest_default_when_absent() {
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![],
        None,
    )
    .unwrap();

    assert_eq!(
        resolve_provider(&manifest, None).unwrap(),
        Provider::ClaudeCode
    );
    assert_eq!(
        resolve_provider(&manifest, Some("opencode")).unwrap(),
        Provider::OpenCode
    );
    assert!(resolve_provider(&manifest, Some("nope")).is_err());
}

// -- detached sessions -----------------------------------------------------

/// A detached session must not spawn a provider: the view dir and its
/// session record exist when `start` returns, and no PTY was opened.
#[test]
fn detached_start_creates_the_view_dir_without_launching_a_provider() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    let report = start(
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
    assert!(report.is_clean());

    let outcome = &report.value;
    assert!(outcome.detached);
    assert!(fs::is_dir(&outcome.view_dir).unwrap());
    assert!(fs::is_dir(&outcome.view_dir.join("api")).unwrap());

    let state = SessionState::read(&outcome.view_dir).unwrap().unwrap();
    assert_eq!(state.provider(), Provider::ClaudeCode);
    assert_eq!(state.feature().unwrap().as_str(), "checkout");
    assert!(state.feature_bound_at().is_some());
}

// -- discovery sessions ----------------------------------------------------

/// No feature named: the session materialises in the hall's own session
/// tree, its record binds nothing, and the promoted repo is linked to its
/// read-only default-branch worktree rather than the feature worktree.
#[test]
fn start_without_a_feature_creates_a_discovery_session() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    let report = start(
        &ctx,
        StartInput {
            feature: None,
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap();
    assert!(report.is_clean());

    let outcome = &report.value;
    assert!(outcome.feature.is_none());
    assert!(
        outcome
            .view_dir
            .as_str()
            .contains(&format!(".ivar/sessions/{}", outcome.session_id)),
        "the view dir must live in the hall session tree: {}",
        outcome.view_dir
    );
    assert!(fs::is_dir(&outcome.view_dir).unwrap());

    let target = read_link_target(&outcome.view_dir.join("api"));
    assert!(
        target.as_str().contains(".ivar/repos/api/main"),
        "a discovery session links the default worktree: {target}"
    );

    let state = SessionState::read(&outcome.view_dir).unwrap().unwrap();
    assert!(state.is_discovery());
    assert_eq!(state.feature_bound_at(), None);
    assert_eq!(state.provider(), Provider::ClaudeCode);
}

/// The two halves of the flow meet: `session convert` binds the session
/// `start` produced.
#[test]
fn a_started_discovery_session_can_be_converted() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    let started = start(
        &ctx,
        StartInput {
            feature: None,
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap()
    .value;

    let converted = conversion::convert(
        &ctx,
        ConvertInput {
            session_id: started.session_id.clone(),
            feature: "checkout".to_owned(),
        },
    )
    .unwrap()
    .value;

    assert_eq!(converted.session_id, started.session_id);
    assert!(
        !fs::is_dir(&started.view_dir).unwrap(),
        "the discovery view dir moves into the feature tree"
    );
}

/// A relay hands one feature's work over; with no feature there is
/// nothing to hand over.
#[test]
fn relay_without_a_feature_is_blocked() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    let failure = start(
        &ctx,
        StartInput {
            feature: None,
            resume: false,
            provider: Some("opencode".to_owned()),
            detached: true,
            relay: true,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.relay_needs_feature");
}

// -- smart fetch -----------------------------------------------------------

/// The fetch-and-fast-forward on session start is real, not just a report:
/// the default worktree catches up to a commit the origin gained after
/// sync, while the promoted repo's feature worktree is untouched.
#[test]
fn smart_fetch_advances_default_branches_and_never_touches_feature_worktrees() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let default_worktree = layout.repo_worktree(
        &RepoName::new("api").unwrap(),
        &BranchName::new("main").unwrap(),
    );
    let feature_worktree = layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);

    // The origin gains a commit after sync.
    let origin = root.parent().unwrap().join("origins").join("api");
    std::fs::write(origin.join("CHANGELOG.md"), "v1\n").unwrap();
    git(&origin, &["add", "CHANGELOG.md"]);
    git(&origin, &["commit", "-m", "v1"]);

    let report = start(
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
    assert!(report.is_clean());

    assert_eq!(
        std::fs::read_to_string(default_worktree.join("CHANGELOG.md")).unwrap(),
        "v1\n",
        "smart fetch must fast-forward the default worktree"
    );
    assert!(
        !feature_worktree.join("CHANGELOG.md").exists(),
        "smart fetch must never touch a promoted repo's feature worktree"
    );
}

/// Best-effort: one repo whose refresh fails (no worktree) warns and the
/// session still starts.
#[test]
fn smart_fetch_warns_and_continues_when_a_repo_fails() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    // A second declared repo that was never synced: no worktree, so its
    // refresh fails — the session must still start.
    let layout = Layout::at(root.clone());
    let manifest = Manifest::read(&layout).unwrap().unwrap();
    let mut repos = manifest.repos().to_vec();
    repos.push(Repo::new(
        RepoName::new("ghost").unwrap(),
        root.join("no-such-origin").as_str(),
        BranchName::new("main").unwrap(),
    ));
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        repos,
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();

    let report = start(
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

    assert!(!report.is_clean());
    assert!(report.warnings.iter().any(|warning| {
        warning.subject == "ghost" && warning.code == "session.smart_fetch_failed"
    }));
    assert!(
        fs::is_dir(&report.value.view_dir).unwrap(),
        "one failed repo must not block session start"
    );
}

// -- relay -----------------------------------------------------------------

/// Relay: a new session on the same feature under a different provider,
/// sharing the feature's worktrees, with its own fresh conversation.
#[test]
fn relay_starts_a_new_session_with_a_different_provider() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    // The session to relay from: the hall's default provider, detached so
    // no provider binary is spawned.
    let first = start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )
    .unwrap()
    .value;

    let report = start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: Some("opencode".to_owned()),
            detached: true,
            relay: true,
        },
    )
    .unwrap();
    assert!(report.is_clean());

    let relayed = &report.value;
    assert_ne!(
        relayed.session_id, first.session_id,
        "a relay is a new session, never a resume"
    );
    assert!(fs::is_dir(&relayed.view_dir).unwrap());

    // Reuses the same feature worktrees.
    let first_link = read_link_target(&first.view_dir.join("api"));
    let relayed_link = read_link_target(&relayed.view_dir.join("api"));
    assert_eq!(first_link, relayed_link);

    // Fresh conversation: the relayed session's record is its own.
    let state = SessionState::read(&relayed.view_dir).unwrap().unwrap();
    assert_eq!(state.provider(), Provider::OpenCode);
}

#[test]
fn relay_without_an_explicit_provider_is_blocked() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());

    let failure = start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: true,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.relay_needs_provider");
}

#[test]
fn relay_with_the_same_provider_as_the_previous_session_is_blocked() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: None, // claude-code, the hall default
            detached: true,
            relay: false,
        },
    )
    .unwrap();

    let failure = start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: Some("claude-code".to_owned()),
            detached: true,
            relay: true,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.relay_same_provider");
}

#[test]
fn relay_without_a_previous_session_is_blocked() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    // No session has ever been started on `checkout`.

    let failure = start(
        &ctx,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: Some("opencode".to_owned()),
            detached: true,
            relay: true,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.relay_no_previous");
}

/// The symlink target `link` points at, panicking on anything else.
fn read_link_target(link: &camino::Utf8Path) -> Utf8PathBuf {
    match fs::read_symlink(link).unwrap() {
        fs::SymlinkTarget::Target(target) => target,
        other => panic!("expected a symlink, got {other:?}"),
    }
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

// -- plan projection and bootstrap instructions ----------------------------

/// A feature session projects **only the active feature's plan** into the
/// view dir: `plans/<feature>/` resolves to the hall's committed plan
/// directory, and plans of other features are never reachable.
#[test]
fn a_feature_session_projects_only_the_active_plan() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    // Scaffold the plan so the projected path resolves to real artifacts.
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();
    // A second feature whose plan must stay out of the session.
    feature_create::create(
        &ctx,
        CreateInput {
            name: "web".to_owned(),
            branch: None,
        },
    )
    .unwrap();

    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
    );
    crate::action::session::view::materialise(
        &layout,
        &manifest_of(&root),
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    // The active plan resolves through the view dir…
    assert!(
        fs::is_file(&view_dir.join("plans/checkout/requirements.md")).unwrap(),
        "plans/checkout must resolve to the hall's plan directory"
    );
    // …and a sibling feature's plan is not projected.
    assert_eq!(
        fs::read_symlink(&view_dir.join("plans/web")).unwrap(),
        fs::SymlinkTarget::Absent,
        "a feature session must never project another feature's plan"
    );
    unguard_worktrees(&root);
}

/// A discovery session (no feature bound) gets no `plans/` projection at all.
#[test]
fn a_discovery_session_projects_no_plans() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let view_dir = layout.discovery_session(
        &crate::domain::name::SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c".to_owned())
            .unwrap(),
    );

    crate::action::session::view::materialise(
        &layout,
        &manifest_of(&root),
        None,
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    assert_eq!(
        fs::read_symlink(&view_dir.join("plans")).unwrap(),
        fs::SymlinkTarget::Absent,
        "a discovery session must not project any plan"
    );
    unguard_worktrees(&root);
}

/// Writing through the projected plan path lands in the hall's committed plan
/// directory — the projection is a view of the real artifact, not a copy.
#[test]
fn writing_through_the_projected_plan_lands_in_the_hall() {
    let (_guard, root) = hall_with_promoted_feature();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
        },
    )
    .unwrap();

    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
    );
    crate::action::session::view::materialise(
        &layout,
        &manifest_of(&root),
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    fs::write_text(
        &view_dir.join("plans/checkout/requirements.md"),
        "# Requirements\n\n- [x] edited through the session\n",
    )
    .unwrap();

    assert_eq!(
        fs::read_text(&layout.plan_dir(&feature.name).join("requirements.md"))
            .unwrap()
            .unwrap(),
        "# Requirements\n\n- [x] edited through the session\n",
        "the edit must land in the hall's committed plan directory"
    );
    unguard_worktrees(&root);
}

/// The feature session's instruction file carries the hall's standing
/// instructions plus the session bootstrap block — without ever modifying the
/// hall's own file.
#[test]
fn a_feature_session_instruction_file_combines_hall_and_bootstrap() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());

    // `sync` wrote the hall's managed block into CLAUDE.md.
    let hall_file = layout.instruction_file(&Provider::ClaudeCode);
    let hall_before = fs::read_text(&hall_file).unwrap().unwrap();
    assert!(
        hall_before.contains("managed:start"),
        "precondition: the hall instruction file carries the managed block"
    );

    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let view_dir = layout.feature_session(
        &FeatureName::new("checkout").unwrap(),
        &crate::domain::name::SessionId::new(uuid::Uuid::new_v4().to_string()).unwrap(),
    );
    crate::action::session::view::materialise(
        &layout,
        &manifest_of(&root),
        Some(&feature),
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();

    let view_instructions = fs::read_text(&view_dir.join("CLAUDE.md")).unwrap().unwrap();
    assert!(
        view_instructions.contains("ivar session — feature `checkout`"),
        "the view instruction file must carry the session bootstrap block: {view_instructions}"
    );
    assert!(
        view_instructions.contains("ivar plan status plans/checkout/plan.md"),
        "the bootstrap block must say how to re-derive the SPDD stage: {view_instructions}"
    );
    assert!(
        view_instructions.contains("managed:start"),
        "the hall's standing instructions must survive into the view file"
    );

    // The hall's own file is untouched.
    assert_eq!(
        fs::read_text(&hall_file).unwrap().unwrap(),
        hall_before,
        "the hall's instruction file must never be modified by materialisation"
    );

    // Discovery sessions get no instruction file of their own (the agent
    // reaches the hall's by walk-up, as before).
    let discovery = layout.discovery_session(
        &crate::domain::name::SessionId::new("3d7f7f2e-3e9b-4c4a-8d3b-7b8f8f0a2c3d".to_owned())
            .unwrap(),
    );
    crate::action::session::view::materialise(
        &layout,
        &manifest_of(&root),
        None,
        Provider::ClaudeCode,
        &discovery,
    )
    .unwrap();
    assert_eq!(
        fs::read_text(&discovery.join("CLAUDE.md")).unwrap(),
        None,
        "a discovery session gets no session instruction file"
    );
    unguard_worktrees(&root);
}

/// The manifest of the hall at `root`.
fn manifest_of(root: &camino::Utf8Path) -> Manifest {
    Manifest::read(&Layout::at(root.to_path_buf()))
        .unwrap()
        .unwrap()
}
