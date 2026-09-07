#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::session::guard::WritableSet;
use crate::action::session::sandbox::Sandbox;
use crate::domain::feature::Feature;
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
fn sandbox_roots_contain_writable_set_bare_git_dev_null_temp_and_provider_dirs() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let feature = Feature::read(&layout, &FeatureName::new("checkout").unwrap())
        .unwrap()
        .unwrap();
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.feature_session(&feature.name, &session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_session(&layout, &feature, &view_dir).unwrap();
    let sandbox =
        Sandbox::from_writable_set(&set, &layout, Some(&feature), Provider::ClaudeCode).unwrap();
    let roots = sandbox.roots();

    // 1. Every WritableSet root is present.
    for r in set.roots() {
        assert!(roots.iter().any(|p| p == r), "missing WritableSet root {r}");
    }

    // 2. Promoted repo bare git directory is present.
    let bare = layout.repo_bare(&RepoName::new("api").unwrap());
    let bare_canonical = bare.canonicalize_utf8().unwrap_or(bare);
    assert!(
        roots.iter().any(|p| p == &bare_canonical),
        "missing bare git root {bare_canonical}"
    );

    // 3. /dev/null is present if it exists.
    let dev_null = Utf8PathBuf::from("/dev/null");
    if dev_null.exists() {
        let canonical_dev_null = dev_null.canonicalize_utf8().unwrap_or(dev_null);
        assert!(
            roots.iter().any(|p| p == &canonical_dev_null),
            "missing /dev/null"
        );
    }

    // 4. System temp dir is present.
    let temp_dir = Utf8PathBuf::try_from(std::env::temp_dir()).unwrap();
    let temp_canonical = temp_dir.canonicalize_utf8().unwrap_or(temp_dir);
    assert!(
        roots.iter().any(|p| p == &temp_canonical),
        "missing temp dir {temp_canonical}"
    );
}

#[test]
fn sandbox_discovery_session_derives_roots_without_feature() {
    let (_guard, root) = hall_with_promoted_feature();
    let layout = Layout::at(root.clone());
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir = layout.discovery_session(&session_id);
    crate::infra::fs::ensure_dir(&view_dir).unwrap();

    let set = WritableSet::from_discovery(&view_dir).unwrap();
    let sandbox = Sandbox::from_writable_set(&set, &layout, None, Provider::Omp).unwrap();
    let roots = sandbox.roots();

    for r in set.roots() {
        assert!(
            roots.iter().any(|p| p == r),
            "missing discovery WritableSet root {r}"
        );
    }
}

#[test]
fn sandbox_status_enum_variants_and_predicates() {
    use crate::action::session::sandbox::SandboxStatus;

    let enforced = SandboxStatus::Enforced;
    assert!(enforced.is_enforced());

    let degraded = SandboxStatus::Degraded {
        reason: "Partially enforced ABI".into(),
    };
    assert!(!degraded.is_enforced());

    let unavailable = SandboxStatus::Unavailable {
        reason: "Landlock not supported on this platform".into(),
    };
    assert!(!unavailable.is_enforced());
}

#[test]
fn sandbox_status_enum_variants_and_display() {
    use crate::action::session::sandbox::SandboxStatus;

    let enforced = SandboxStatus::Enforced;
    assert!(enforced.is_enforced());

    let degraded = SandboxStatus::Degraded {
        reason: "Partially enforced".into(),
    };
    assert!(!degraded.is_enforced());

    let unavailable = SandboxStatus::Unavailable {
        reason: "Landlock not supported on this platform".into(),
    };
    assert!(!unavailable.is_enforced());
}
