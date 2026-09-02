//! Unit tests for `crate::action::session::relay`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;

use super::*;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::session::start::{self as session_start, StartInput};
use crate::domain::feature::{RunBaseline, RunId, RunProvenance, RunReceipt, RunStatus};
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::test_support::{hall_root, seeded_repo};

fn hall_with_provider_session() -> (tempfile::TempDir, Utf8PathBuf) {
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
    let layout = Layout::at(root.clone());
    let manifest = Manifest::new(
        HallName::new("acme").unwrap(),
        Providers::new(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::ClaudeCode,
        ),
        vec![Repo::new(
            RepoName::new("api").unwrap(),
            api_origin.as_str(),
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

    // A session on the default provider (claude-code).
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

fn legacy_board() -> serde_json::Value {
    serde_json::json!({
        "version": 3,
        "status": "completed",
        "workstreams": [],
        "sessions": {},
        "journal": []
    })
}

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

/// `session relay` and `session start --relay` produce the same outcome on
/// the same state: same feature, same provider, same session record, same
/// worktree reuse — differing only in the fresh session id. This is the
/// test that prevents the two paths from diverging (the whole point of the
/// verb being a thin alias).
#[test]
fn relay_and_start_relay_produce_the_same_outcome() {
    // Two identical halls: relay via the verb in one, via `start --relay`
    // in the other.
    let (_guard_a, root_a) = hall_with_provider_session();
    let ctx_a = Ctx::new(root_a.clone());
    let via_relay = relay(
        &ctx_a,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap();

    let (_guard_b, root_b) = hall_with_provider_session();
    let ctx_b = Ctx::new(root_b.clone());
    let via_start = session_start::start(
        &ctx_b,
        StartInput {
            feature: Some("checkout".to_owned()),
            resume: false,
            provider: Some("opencode".to_owned()),
            detached: true,
            relay: true,
        },
    )
    .unwrap();

    let a = &via_relay.value;
    let b = &via_start.value;
    assert_eq!(Some(a.feature.clone()), b.feature, "same feature");
    assert_eq!(a.provider, b.provider, "same provider");
    assert_eq!(
        via_relay.is_clean(),
        via_start.is_clean(),
        "same warning set"
    );

    // Both created a real, fresh session bound to opencode on checkout.
    let state_a = session_state_of(&root_a, &a.session_id);
    let state_b = session_state_of(&root_b, &b.session_id);
    assert_eq!(state_a.provider(), Provider::OpenCode);
    assert_eq!(state_b.provider(), Provider::OpenCode);
    assert_eq!(state_a.feature().unwrap().as_str(), "checkout");
    assert_eq!(state_b.feature().unwrap().as_str(), "checkout");

    // Both reuse the feature worktree the previous session linked.
    assert!(
        api_link_target(&root_a, &a.session_id).contains(".ivar/repos/api/checkout"),
        "relay must reuse the feature worktree"
    );
    assert!(
        api_link_target(&root_b, &b.session_id).contains(".ivar/repos/api/checkout"),
        "start --relay must reuse the feature worktree"
    );

    unguard_worktrees(&root_a);
    unguard_worktrees(&root_b);
}

/// The four-line output is contract with the landing's first fold: session
/// line, provider line, board line, blank separator.
#[test]
fn relay_emits_the_four_line_output_contract() {
    let (_guard, root) = hall_with_provider_session();
    let ctx = Ctx::new(root.clone());

    let relay_report = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap();

    assert!(relay_report.is_clean());
    assert_eq!(relay_report.value.feature.as_str(), "checkout");
    assert_eq!(relay_report.value.provider, Provider::OpenCode);

    // Verify exactly 4 lines of output.
    let mut out = Vec::new();
    relay_report.value.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "must produce exactly 4 lines, got {}: {:?}",
        lines.len(),
        lines
    );
    assert!(lines[0].contains("relayed"));
    assert!(lines[1].starts_with("Provider:"));
    assert!(lines[2].starts_with("plan preserved"));
    assert!(lines[3].is_empty(), "fourth line must be blank");
    unguard_worktrees(&root);
}

/// Relay imports historical execution evidence before reading the receipt.
#[test]
fn relay_imports_legacy_board_before_reading_the_receipt() {
    let (_guard, root) = hall_with_provider_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    crate::infra::fs::ensure_dir(&layout.execution_dir(&feature)).unwrap();
    crate::infra::json::write_canonical(
        &layout.execution_dir(&feature).join("board.json"),
        &legacy_board(),
    )
    .unwrap();

    let report = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(report.value.run_status, None);
    assert!(RunReceipt::read(&layout, &feature).unwrap().is_none());
    let history = crate::store::feature::run::history(&layout, &feature).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].provenance, RunProvenance::LegacyImport);
    unguard_worktrees(&root);
}

/// Relay exposes receipt identity and state, never inferred progress.
#[test]
fn relay_reports_the_current_receipt_without_progress_counts() {
    let (_guard, root) = hall_with_provider_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());
    let feature = FeatureName::new("checkout").unwrap();
    let receipt = RunReceipt::start(
        RunId::new("00000000-0000-0000-0000-000000000001").unwrap(),
        feature.clone(),
        "plans/checkout/plan.md",
        "abc123",
        RunBaseline::default(),
        crate::domain::name::SessionId::new("00000000-0000-0000-0000-000000000002").unwrap(),
        Provider::ClaudeCode,
        "2026-01-01T00:00:00Z",
    );
    receipt.write(&layout).unwrap();

    let report = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap();
    assert_eq!(
        report.value.run_id.as_deref(),
        Some("00000000-0000-0000-0000-000000000001")
    );
    assert_eq!(report.value.run_status, Some(RunStatus::Active));

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("run 00000000-0000-0000-0000-000000000001 is active"));
    assert!(!text.contains("steps"));
    unguard_worktrees(&root);
}

/// The session record of `session_id` in a hall whose `checkout` feature
/// holds it.
fn session_state_of(
    root: &camino::Utf8Path,
    session_id: &str,
) -> crate::domain::session::SessionState {
    let layout = Layout::at(root.to_path_buf());
    let session = crate::domain::name::SessionId::new(session_id.to_owned()).unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session);
    crate::domain::session::SessionState::read(&view_dir)
        .unwrap()
        .unwrap()
}

/// Where the view dir's `api` symlink points.
fn api_link_target(root: &camino::Utf8Path, session_id: &str) -> String {
    let layout = Layout::at(root.to_path_buf());
    let session = crate::domain::name::SessionId::new(session_id.to_owned()).unwrap();
    let view_dir = layout.feature_session(&FeatureName::new("checkout").unwrap(), &session);
    let link = view_dir.join("api");
    match fs::read_symlink(&link).unwrap() {
        fs::SymlinkTarget::Target(target) => target.to_string(),
        other => panic!("expected a symlink, got {other:?}"),
    }
}

/// The symlink target `link` points at, panicking on anything else.
fn read_link_target(link: &camino::Utf8Path) -> camino::Utf8PathBuf {
    match fs::read_symlink(link).unwrap() {
        fs::SymlinkTarget::Target(target) => target,
        other => panic!("expected a symlink, got {other:?}"),
    }
}

#[test]
fn relay_without_a_previous_session_is_blocked() {
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

    // No previous session → relay blocked.
    let failure = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.relay_no_previous");
    unguard_worktrees(&root);
}

#[test]
fn relay_with_same_provider_as_previous_is_blocked() {
    let (_guard, root) = hall_with_provider_session();
    let ctx = Ctx::new(root.clone());

    // Previous session uses claude-code (the default). Try to relay to it.
    let failure = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "claude-code".to_owned(),
        },
    )
    .unwrap_err();

    assert_eq!(failure.code, "session.relay_same_provider");
    unguard_worktrees(&root);
}

#[test]
fn relay_output_without_a_receipt_mentions_no_progress() {
    let (_guard, root) = hall_with_provider_session();
    let ctx = Ctx::new(root.clone());

    let report = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap();

    let mut out = Vec::new();
    report.value.write_human(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("plan preserved"));
    assert!(!text.contains("steps"));
    unguard_worktrees(&root);
}

/// The relay's View Dir is materialised for the **relayed** provider, not the
/// hall's default: a Claude → OpenCode relay must land on `.opencode/` (with
/// OpenCode's commands) and `AGENTS.md`, even though the hall's default — and
/// the session being relayed from — is claude-code. This is the fix for the
/// relay leaving the new session with the previous provider's harness
/// materialised.
#[test]
fn relay_materialises_the_relayed_providers_config() {
    let (_guard, root) = hall_with_provider_session();
    let ctx = Ctx::new(root.clone());
    let layout = Layout::at(root.clone());

    let report = relay(
        &ctx,
        RelayInput {
            feature: "checkout".to_owned(),
            provider: "opencode".to_owned(),
        },
    )
    .unwrap();
    let view_dir = layout.feature_session(
        &report.value.feature,
        &crate::domain::name::SessionId::new(report.value.session_id.clone()).unwrap(),
    );

    // The relayed session's own provider materialises its config dir…
    assert!(
        fs::is_dir(&view_dir.join(".opencode")).unwrap(),
        "the relayed session must materialise .opencode, not .claude"
    );
    assert_eq!(
        fs::read_symlink(&view_dir.join(".opencode")).unwrap(),
        fs::SymlinkTarget::NotASymlink,
        "the config dir must be a real directory, not a symlink"
    );
    // …with OpenCode's commands reachable through the symlink…
    let commands = read_link_target(&view_dir.join(".opencode/commands"));
    assert_eq!(
        commands,
        layout.commands_dir(&Provider::OpenCode),
        "commands must resolve to the hall's opencode commands dir"
    );
    // …and the previous provider's config must not appear.
    assert_eq!(
        fs::read_symlink(&view_dir.join(".claude")).unwrap(),
        fs::SymlinkTarget::Absent,
        "the relayed session must not carry the previous provider's config"
    );

    // The provider-native instruction file exists and carries the session
    // bootstrap: this is the continuation contract the relay exists to
    // deliver.
    let agents = fs::read_text(&view_dir.join("AGENTS.md")).unwrap().unwrap();
    assert!(
        agents.contains("ivar session — feature `checkout`"),
        "AGENTS.md must carry the session bootstrap block: {agents}"
    );
    assert!(
        agents.contains("ivar plan status plans/checkout/plan.md"),
        "AGENTS.md must tell the agent how to re-derive the SPDD stage: {agents}"
    );

    // The active plan is projected into the view dir.
    let plan_link = read_link_target(&view_dir.join("plans/checkout"));
    assert_eq!(
        plan_link,
        layout.plan_dir(&report.value.feature),
        "plans/checkout must resolve to the hall's committed plan directory"
    );
    unguard_worktrees(&root);
}

