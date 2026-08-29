//! Unit tests for `crate::action::session::guard`.
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

#[test]
fn writable_set_is_view_dir_plus_promoted_worktrees() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature =
        Feature::read(&layout, &FeatureName::new("checkout").unwrap())
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
    let api_worktree =
        layout.repo_worktree(&RepoName::new("api").unwrap(), &feature.branch);
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
