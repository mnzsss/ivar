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
use crate::action::session::start::{self as session_start, StartInput};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

/// A hall with two registered repos — `api` promoted into `checkout`,
/// `web` left read-only — plus a detached session on `checkout`.
fn hall_with_detached_session() -> (tempfile::TempDir, Utf8PathBuf) {
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

    session_start::start(
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

    (guard, root)
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

fn session_id_of(root: &camino::Utf8Path) -> String {
    let layout = Layout::at(root.to_path_buf());
    let dir = layout
        .feature_dir(&FeatureName::new("checkout").unwrap())
        .join("sessions");
    let entry = fs::read_dir(&dir).unwrap();
    let session_dir = &entry[0];
    session_dir.file_name().unwrap().to_owned()
}

#[test]
fn connect_locates_a_session_by_id_prefix() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let id = session_id_of(&root);

    let report = connect(
        &ctx,
        ConnectInput {
            session_id: Some(id[..8].to_owned()),
            feature: None,
        },
    )
    .unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.session_id, id);
    assert_eq!(report.value.feature.unwrap().as_str(), "checkout");
    unguard_worktrees(&root);
}

#[test]
fn connect_locates_a_session_by_feature_name() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let id = session_id_of(&root);

    let report = connect(
        &ctx,
        ConnectInput {
            session_id: None,
            feature: Some("checkout".to_owned()),
        },
    )
    .unwrap();

    assert_eq!(report.value.session_id, id);
    unguard_worktrees(&root);
}

/// Connect re-materialises the view dir: a symlink drifted by a stray
/// write (or a promote in another session) is pointed back at the right
/// worktree.
#[test]
fn connect_repairs_drifted_symlinks() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let id = session_id_of(&root);
    let session_id = crate::domain::name::SessionId::new(id.clone()).unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);
    let link = view_dir.join("api");
    let wrong = root.join("nowhere");
    fs::replace_symlink(&wrong, &link).unwrap();

    connect(
        &ctx,
        ConnectInput {
            session_id: Some(id),
            feature: None,
        },
    )
    .unwrap();

    assert!(
        fs::is_dir(&link).unwrap(),
        "the drifted api symlink must resolve again"
    );
    let target = match fs::read_symlink(&link).unwrap() {
        fs::SymlinkTarget::Target(path) => path,
        other => panic!("expected a symlink, got {other:?}"),
    };
    assert!(
        target.as_str().contains(".ivar/repos/api/checkout"),
        "the symlink must point back at the feature worktree: {target}"
    );
    unguard_worktrees(&root);
}

/// Connect re-applies the read-only guard on non-promoted worktrees — the
/// stale state a crashed guard-lift leaves behind.
#[test]
fn connect_repairs_read_only_guards_on_non_promoted_worktrees() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let web_worktree = root.join(".ivar/repos/web/main");

    // The guard materialisation applied...
    assert_eq!(
        fs::unix_mode(&web_worktree).unwrap().unwrap() & 0o222,
        0,
        "the non-promoted worktree starts read-only-guarded"
    );
    // ...drifts: a crashed lift leaves the write bits back.
    fs::restore_write_bits(&web_worktree).unwrap();
    let id = session_id_of(&root);

    connect(
        &ctx,
        ConnectInput {
            session_id: Some(id),
            feature: None,
        },
    )
    .unwrap();

    assert_eq!(
        fs::unix_mode(&web_worktree).unwrap().unwrap() & 0o222,
        0,
        "connect must re-apply the read-only guard"
    );
    unguard_worktrees(&root);
}

/// Idempotent: connecting to an already-correct view dir must not rename
/// any symlink (each rename opens a transient resolution race — see
/// `infra::fs`). An unchanged link keeps its inode.
#[test]
fn connect_on_an_unchanged_view_dir_is_a_no_op() {
    use std::os::unix::fs::MetadataExt;

    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let id = session_id_of(&root);
    let session_id = crate::domain::name::SessionId::new(id.clone()).unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);
    let link = view_dir.join("api");
    let inode_before = fs_err::symlink_metadata(link.as_std_path()).unwrap().ino();

    connect(
        &ctx,
        ConnectInput {
            session_id: Some(id),
            feature: None,
        },
    )
    .unwrap();

    let inode_after = fs_err::symlink_metadata(link.as_std_path()).unwrap().ino();
    assert_eq!(
        inode_before, inode_after,
        "an unchanged symlink must not be replaced"
    );
    unguard_worktrees(&root);
}

/// The binding connect emits — the `IVAR_*` env-var contract.
#[test]
fn connect_emits_the_session_binding_env_vars() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let id = session_id_of(&root);

    let report = connect(
        &ctx,
        ConnectInput {
            session_id: Some(id.clone()),
            feature: None,
        },
    )
    .unwrap();
    let outcome = &report.value;

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains(&format!("export IVAR_SESSION_ID={id}")),
        "was: {text}"
    );
    assert!(text.contains("export IVAR_FEATURE=checkout"), "was: {text}");
    assert!(
        text.contains(&format!("export IVAR_SESSION_PATH={}", outcome.view_dir)),
        "was: {text}"
    );
    unguard_worktrees(&root);
}

/// A discovery session (no feature bound) connects too — the feature env
/// var is simply absent.
#[test]
fn connect_resolves_a_discovery_session() {
    let (_guard, root) = hall_root();
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
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(vec![Provider::ClaudeCode], Provider::ClaudeCode),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            api_origin.as_str(),
            BranchName::new("main").unwrap(),
        )],
        None,
    )
    .unwrap();
    Manifest::write(&layout, &manifest).unwrap();
    crate::action::sync::sync(&ctx, Default::default()).unwrap();

    // Materialise a discovery session directly rather than through
    // `session start`: connect only cares about the shape on disk.
    let session_id =
        crate::domain::name::SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c".to_owned())
            .unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::action::session::view::materialise(
        &layout,
        &manifest,
        None,
        Provider::ClaudeCode,
        &view_dir,
    )
    .unwrap();
    let state = crate::domain::session::SessionState::new(
        Provider::ClaudeCode,
        "2026-01-01T00:00:00.000000000Z",
    );
    state.write(&view_dir).unwrap();

    let report = connect(
        &ctx,
        ConnectInput {
            session_id: Some("2c6e6f1e".to_owned()),
            feature: None,
        },
    )
    .unwrap();

    assert_eq!(report.value.feature, None);
    assert_eq!(report.value.session_id, session_id.to_string());
    assert!(fs::is_dir(&report.value.view_dir.join("api")).unwrap());
    unguard_worktrees(&root);
}

#[test]
fn connect_with_no_filter_is_blocked() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());

    let failure = connect(
        &ctx,
        ConnectInput {
            session_id: None,
            feature: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.lookup_needs_filter");
    unguard_worktrees(&root);
}

/// Connect re-materialises the whole view dir, so a session created before
/// plan projection and bootstrap instructions existed — or whose entries were
/// deleted behind ivar's back — is repaired: the plan link, the provider's
/// commands symlink and the session instruction file all come back.
#[test]
fn connect_repairs_the_projected_plan_commands_and_instructions() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    // Scaffold the plan so the projected path resolves to real artifacts.
    plan_create::create(
        &ctx,
        PlanCreateInput {
            feature: "checkout".to_owned(),
            artifacts: Vec::new(),
        },
    )
    .unwrap();
    let id = session_id_of(&root);
    let session_id = crate::domain::name::SessionId::new(id.clone()).unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session_id);

    // Drift: the whole projected plan, the commands symlink and the session
    // instruction file disappear.
    fs::remove_path(&view_dir.join("plans")).unwrap();
    fs::remove_file(&view_dir.join(".claude/commands")).unwrap();
    fs::remove_file(&view_dir.join("CLAUDE.md")).unwrap();
    assert!(!fs::exists(&view_dir.join("CLAUDE.md")).unwrap());

    connect(
        &ctx,
        ConnectInput {
            session_id: Some(id),
            feature: None,
        },
    )
    .unwrap();

    // The projected plan is back and resolves to the hall's plan directory.
    assert!(
        fs::is_file(&view_dir.join("plans/checkout/requirements.md")).unwrap(),
        "connect must restore the projected plan"
    );
    // The provider's commands reach the agent again.
    assert!(
        fs::is_file(&view_dir.join(".claude/commands/ivar-execute.md")).unwrap(),
        "connect must restore the provider's commands symlink"
    );
    // The session instruction file is back, with the bootstrap block.
    let instructions = fs::read_text(&view_dir.join("CLAUDE.md")).unwrap().unwrap();
    assert!(
        instructions.contains("ivar session — feature `checkout`"),
        "connect must restore the bootstrap instructions: {instructions}"
    );
    unguard_worktrees(&root);
}

#[test]
fn connect_with_an_unknown_session_is_blocked() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());

    let failure = connect(
        &ctx,
        ConnectInput {
            session_id: Some("deadbeef".to_owned()),
            feature: None,
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.not_found");
    unguard_worktrees(&root);
}

#[test]
fn connect_with_an_ambiguous_prefix_is_blocked() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    // A second session under the same feature makes the feature-name
    // lookup ambiguous.
    session_start::start(
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

    let failure = connect(
        &ctx,
        ConnectInput {
            session_id: None,
            feature: Some("checkout".to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.ambiguous");
    unguard_worktrees(&root);
}

/// The bug this fixes: a `--feature` search only looks under
/// `.ivar/features/<f>/sessions/`, so a discovery session already holding that
/// feature's work is invisible to it. Reporting only `session.start_first`
/// sent agents off to open a second session beside the one with the work; the
/// failure must name the candidates and point at `session convert`.
#[test]
fn connect_by_feature_names_discovery_sessions_as_convert_candidates() {
    let (_guard, root) = hall_with_detached_session();
    let ctx = Ctx::new(root.clone());
    // A discovery session: no feature, so it lands under `.ivar/sessions/`.
    let discovery = session_start::start(
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
    .value
    .session_id;
    // A feature with no session of its own is the case that used to dead-end.
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

    let failure = connect(
        &ctx,
        ConnectInput {
            session_id: None,
            feature: Some("billing".to_owned()),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.not_found");
    let actual = failure
        .actual
        .clone()
        .expect("the failure names what it found");
    assert!(
        actual.contains(&discovery),
        "the discovery session must be named as a candidate: {actual}"
    );
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.code == "session.convert"),
        "convert must be offered before start: {:?}",
        failure.fix_actions
    );
    unguard_worktrees(&root);
}
