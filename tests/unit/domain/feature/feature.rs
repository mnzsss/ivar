//! Unit tests for `crate::domain::feature::promotion` (`Feature`, `Promotion`).
//!
//! Physically located here but compiled inside the library crate via `#[path]`
//! so `use super::*` reaches private parent items.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::name::{BranchName, FeatureName, RepoName};

fn feature() -> Feature {
    Feature::new(
        FeatureName::new("checkout").unwrap(),
        BranchName::new("feat/checkout").unwrap(),
    )
}

#[test]
fn a_new_feature_has_no_declared_base() {
    assert_eq!(feature().base, None);
}

#[test]
fn a_new_promotion_has_no_declared_base() {
    let mut feature = feature();
    let repo = RepoName::new("api").unwrap();

    feature.promote(repo.clone());

    assert_eq!(feature.promotions.get(&repo).unwrap().base, None);
}

#[test]
fn a_feature_is_stamped_at_version_two() {
    assert_eq!(feature().version(), 2);
}

#[test]
fn base_round_trips_through_serde_when_set() {
    let mut feature = feature();
    feature.base = Some(BranchName::new("main").unwrap());

    let rendered = serde_json::to_string(&feature).unwrap();
    let parsed: Feature = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed, feature);
    assert_eq!(parsed.base, Some(BranchName::new("main").unwrap()));
}

#[test]
fn a_v1_feature_json_with_no_base_field_still_deserialises() {
    let raw = r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{}}"#;

    let parsed: Feature = serde_json::from_str(raw).unwrap();

    assert_eq!(parsed.base, None);
}

#[test]
fn a_v1_promotion_with_no_base_field_still_deserialises() {
    let raw = r#"{"version":1,"name":"checkout","branch":"feat/checkout","promotions":{"api":{"worktree":"pending"}}}"#;

    let parsed: Feature = serde_json::from_str(raw).unwrap();

    assert_eq!(
        parsed
            .promotions
            .get(&RepoName::new("api").unwrap())
            .unwrap()
            .base,
        None
    );
}

#[test]
fn a_new_feature_has_no_parent_and_no_integration_override() {
    let feature = feature();
    assert_eq!(feature.parent, None);
    assert_eq!(feature.integration, crate::domain::feature::IntegrationOverride::default());
}

#[test]
fn a_new_promotion_has_no_integration_receipt() {
    let mut feature = feature();
    feature.promote(RepoName::new("api").unwrap());
    assert!(feature
        .promotions
        .values()
        .all(|promotion| promotion.integration_receipt.is_none()));
}

#[test]
fn receipt_facts_answer_has_any_and_passing() {
    let mut feature = feature();
    let api = RepoName::new("api").unwrap();
    let web = RepoName::new("web").unwrap();
    feature.promote(api.clone());
    feature.promote(web.clone());

    assert!(!feature.has_any_receipt());
    assert!(!feature.all_promotions_have_passing_receipts());

    // One failed-evidence receipt is "any receipt" but not "successful" and
    // not "all passing".
    let failed = IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("parent").unwrap(),
        result_sha: "222".to_owned(),
        via: crate::domain::feature::IntegrationVia::Local,
        strategy: crate::domain::feature::IntegrationStrategy::Squash,
        pr_url: None,
        verification: crate::domain::feature::VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: vec![crate::domain::feature::VerificationResult::failed(
                "cargo test",
                Some(101),
                "boom",
            )],
            parent: Vec::new(),
            pr_checks: Vec::new(),
            verified_at: "2026-08-14T12:00:00Z".to_owned(),
        },
    };
    feature.promotions.get_mut(&api).unwrap().integration_receipt = Some(failed.clone());

    assert!(feature.has_any_receipt());
    assert!(!feature.promotion_has_successful_receipt(&api));
    assert!(!feature.all_promotions_have_passing_receipts());

    // Upgrade api's receipt to passing evidence: "successful" and — with the
    // other repo still receipted? no — only api receipted, web is not.
    let mut passing = failed;
    passing.verification.child = Vec::new();
    feature.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing);

    assert!(feature.promotion_has_successful_receipt(&api));
    assert!(!feature.all_promotions_have_passing_receipts());
}

#[test]
fn all_promotions_have_passing_receipts_needs_every_repo_and_nonempty() {
    let feature = feature();
    assert!(!feature.all_promotions_have_passing_receipts());
}

#[test]
fn parent_and_integration_round_trip_through_serde_when_set() {
    let mut feature = feature();
    feature.parent = Some(FeatureName::new("parent").unwrap());
    feature.integration = crate::domain::feature::IntegrationOverride {
        via: Some(crate::domain::feature::IntegrationVia::Pr),
        strategy: None,
    };

    let rendered = serde_json::to_string(&feature).unwrap();
    let parsed: Feature = serde_json::from_str(&rendered).unwrap();

    assert_eq!(parsed, feature);
    assert_eq!(
        parsed.parent,
        Some(FeatureName::new("parent").unwrap())
    );
    assert_eq!(
        parsed.integration.via,
        Some(crate::domain::feature::IntegrationVia::Pr)
    );
}

#[test]
fn a_v2_feature_json_with_no_parent_or_integration_field_still_deserialises() {
    let raw = r#"{"version":2,"name":"checkout","branch":"feat/checkout","promotions":{}}"#;

    let parsed: Feature = serde_json::from_str(raw).unwrap();

    assert_eq!(parsed.parent, None);
    assert_eq!(parsed.integration, crate::domain::feature::IntegrationOverride::default());
}
