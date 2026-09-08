#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::action::Ctx;
use crate::action::feature::create::{self as feature_create, CreateInput};
use crate::action::feature::promote::{self as feature_promote, PromoteInput};
use crate::action::hall::{self, InitInput};
use crate::action::session::start::{self as session_start, StartInput};
use crate::domain::memory::handoff::HandoffPayload;
use crate::domain::name::{BranchName, FeatureName, HallName, RepoName, SessionId};
use crate::domain::provider::Provider;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::{Manifest, Providers, Repo};
use crate::store::memory::handoff::persist_handoff;
use crate::test_support::{hall_root, seeded_repo};
use camino::Utf8PathBuf;

fn setup_hall_with_feature() -> (tempfile::TempDir, Utf8PathBuf, FeatureName) {
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

    let feature_name = FeatureName::new("checkout").unwrap();
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

    (guard, root, feature_name)
}

#[test]
fn test_view_materialise_claims_handoffs_and_injects_hot_memory() {
    let (_guard, root, feature_name) = setup_hall_with_feature();
    let layout = Layout::at(root.clone());
    let ctx = Ctx::new(root.clone());

    // Write a handoff to the feature inbox
    let handoff = HandoffPayload::new(
        "handoff-xyz",
        SessionId::new("33333333-3333-3333-3333-333333333333").unwrap(),
        "Finished refactoring checkout models",
        vec!["Add integration tests".to_owned()],
        vec!["Use Postgres JSONB".to_owned()],
        vec!["src/models.rs".to_owned()],
    );
    persist_handoff(&layout, &feature_name, &handoff).unwrap();

    assert!(
        layout
            .feature_memory_inbox(&feature_name)
            .join("handoff-xyz.json")
            .exists()
    );

    // Start a new session on this feature
    let started = session_start::start(
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

    // The handoff should have been claimed and moved to archive
    assert!(
        !layout
            .feature_memory_inbox(&feature_name)
            .join("handoff-xyz.json")
            .exists()
    );
    assert!(
        layout
            .feature_memory_archive(&feature_name)
            .join("handoff-xyz.json")
            .exists()
    );

    // Check instruction file in session view dir
    let session_id = SessionId::new(started.value.session_id).unwrap();
    let view_dir = layout.feature_session(&feature_name, &session_id);
    let instruction_file = view_dir.join(Provider::ClaudeCode.instruction_file());

    let content = fs::read_text(&instruction_file).unwrap().unwrap();
    assert!(content.contains("<!-- ivar:hot-memory:start -->"));
    assert!(content.contains("## Recent Handoff Context"));
    assert!(content.contains("### Handoff from Session `33333333-3333-3333-3333-333333333333`"));
    assert!(content.contains("Finished refactoring checkout models"));
    assert!(content.contains("- Add integration tests"));
    assert!(content.contains("- Use Postgres JSONB"));
    assert!(content.contains("- `src/models.rs`"));
    assert!(content.contains("<!-- ivar:hot-memory:end -->"));
}
