//! Unit tests for `crate::domain::feature::integration` — the pure
//! nested-integration vocabulary: via/strategy/override/policy resolution,
//! receipts and verification evidence, and the derived integration-state
//! classifier.
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
use crate::domain::name::BranchName;

// -- embedded defaults and parsers ----------------------------------------

#[test]
fn embedded_policy_is_local_squash() {
    assert_eq!(IntegrationPolicy::default().via, IntegrationVia::Local);
    assert_eq!(
        IntegrationPolicy::default().strategy,
        IntegrationStrategy::Squash
    );
}

#[rstest]
#[case("pr", IntegrationVia::Pr)]
#[case("local", IntegrationVia::Local)]
fn via_accepts_only_public_spellings(#[case] raw: &str, #[case] expected: IntegrationVia) {
    assert_eq!(IntegrationVia::parse(raw).unwrap(), expected);
    assert!(IntegrationVia::parse("github").is_err());
}

#[rstest]
#[case("squash", IntegrationStrategy::Squash)]
#[case("merge", IntegrationStrategy::Merge)]
#[case("rebase", IntegrationStrategy::Rebase)]
fn strategy_accepts_only_the_three_strategies(
    #[case] raw: &str,
    #[case] expected: IntegrationStrategy,
) {
    assert_eq!(IntegrationStrategy::parse(raw).unwrap(), expected);
    assert!(IntegrationStrategy::parse("cherry-pick").is_err());
}

#[test]
fn display_names_are_the_cli_surface() {
    assert_eq!(IntegrationVia::Pr.to_string(), "pr");
    assert_eq!(IntegrationVia::Local.to_string(), "local");
    assert_eq!(IntegrationStrategy::Squash.to_string(), "squash");
    assert_eq!(IntegrationStrategy::Merge.to_string(), "merge");
    assert_eq!(IntegrationStrategy::Rebase.to_string(), "rebase");
}

#[test]
fn serde_names_are_snake_case_and_never_github() {
    assert_eq!(serde_json::to_value(IntegrationVia::Pr).unwrap(), serde_json::json!("pr"));
    assert_eq!(
        serde_json::to_value(IntegrationStrategy::Rebase).unwrap(),
        serde_json::json!("rebase")
    );
    assert!(serde_json::from_str::<IntegrationVia>("\"github\"").is_err());
}

// -- per-field precedence ---------------------------------------------------

#[test]
fn policy_resolves_each_field_cli_then_feature_then_hall_then_embedded() {
    let resolved = IntegrationPolicy::resolve(
        IntegrationOverride {
            via: Some(IntegrationVia::Pr),
            strategy: None,
        },
        IntegrationOverride {
            via: Some(IntegrationVia::Local),
            strategy: Some(IntegrationStrategy::Merge),
        },
        IntegrationPolicy {
            via: IntegrationVia::Pr,
            strategy: IntegrationStrategy::Rebase,
        },
    );
    assert_eq!(
        resolved,
        IntegrationPolicy {
            via: IntegrationVia::Pr,
            strategy: IntegrationStrategy::Merge,
        }
    );
}

#[test]
fn resolve_falls_back_to_embedded_defaults_when_nothing_is_set() {
    let resolved = IntegrationPolicy::resolve(
        IntegrationOverride::default(),
        IntegrationOverride::default(),
        IntegrationPolicy::default(),
    );
    assert_eq!(resolved, IntegrationPolicy::default());
}

#[test]
fn resolve_treats_hall_as_the_last_word_above_embedded() {
    let resolved = IntegrationPolicy::resolve(
        IntegrationOverride::default(),
        IntegrationOverride::default(),
        IntegrationPolicy {
            via: IntegrationVia::Local,
            strategy: IntegrationStrategy::Merge,
        },
    );
    assert_eq!(resolved.via, IntegrationVia::Local);
    assert_eq!(resolved.strategy, IntegrationStrategy::Merge);
}

#[test]
fn override_round_trips_through_serde_omitting_unset_fields() {
    let override_value = IntegrationOverride {
        via: Some(IntegrationVia::Pr),
        strategy: None,
    };
    let rendered = serde_json::to_value(&override_value).unwrap();
    assert_eq!(rendered, serde_json::json!({ "via": "pr" }));
    assert_eq!(
        serde_json::from_value::<IntegrationOverride>(rendered).unwrap(),
        override_value
    );
}

// -- receipts and evidence --------------------------------------------------

fn receipt() -> IntegrationReceipt {
    IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("parent").unwrap(),
        result_sha: "222".to_owned(),
        via: IntegrationVia::Pr,
        strategy: IntegrationStrategy::Squash,
        pr_url: Some("https://github.com/acme/api/pull/7".to_owned()),
        verification: VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: vec![VerificationResult::passed("cargo test", Some(0), "")],
            parent: vec![VerificationResult::passed("cargo test", Some(0), "")],
            pr_checks: vec![PrCheckResult::passed("ci")],
            verified_at: "2026-08-14T12:00:00Z".to_owned(),
        },
    }
}

#[test]
fn a_passing_receipt_round_trips_through_serde_and_reports_passed() {
    let receipt = receipt();
    assert!(receipt.verification.passed());
    assert_eq!(
        serde_json::from_str::<IntegrationReceipt>(&serde_json::to_string(&receipt).unwrap())
            .unwrap(),
        receipt
    );
}

#[test]
fn failed_evidence_is_not_passed_even_without_a_stale_marker() {
    let mut receipt = receipt();
    receipt.verification.child = vec![VerificationResult::failed(
        "cargo test",
        Some(101),
        "test failed",
    )];
    assert!(!receipt.verification.passed());
}

#[test]
fn a_receipt_without_a_pr_url_round_trips() {
    let mut receipt = receipt();
    receipt.pr_url = None;
    assert_eq!(
        serde_json::from_str::<IntegrationReceipt>(&serde_json::to_string(&receipt).unwrap())
            .unwrap(),
        receipt
    );
}

#[test]
fn an_unknown_field_in_a_receipt_is_refused() {
    let mut value = serde_json::to_value(receipt()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("bogus".to_owned(), serde_json::json!(true));
    assert!(serde_json::from_value::<IntegrationReceipt>(value).is_err());
}

// -- derived classification --------------------------------------------------

#[test]
fn classifier_derives_every_state_from_explicit_facts() {
    assert_eq!(
        classify(None, ClassificationFacts::active()),
        FeatureIntegrationState::Active
    );
    assert_eq!(
        classify(None, ClassificationFacts::integrated()),
        FeatureIntegrationState::Integrated
    );
    assert_eq!(
        classify(None, ClassificationFacts::failed()),
        FeatureIntegrationState::Failed
    );
    assert_eq!(
        classify(None, ClassificationFacts::stale()),
        FeatureIntegrationState::Stale
    );
    assert_eq!(
        classify(
            Some(PromotionOutcome::Abandoned),
            ClassificationFacts::active()
        ),
        FeatureIntegrationState::Abandoned
    );
    assert_eq!(
        classify(
            Some(PromotionOutcome::Delivered),
            ClassificationFacts::active()
        ),
        FeatureIntegrationState::Delivered
    );
}

#[test]
fn a_closed_record_wins_over_receipt_facts() {
    // An abandoned history remains abandoned even if receipts exist; the close
    // record is the lifecycle fact, receipts only fill in the unclosed case.
    assert_eq!(
        classify(
            Some(PromotionOutcome::Abandoned),
            ClassificationFacts::integrated()
        ),
        FeatureIntegrationState::Abandoned
    );
}

#[test]
fn failed_outranks_stale_and_stale_outranks_integrated() {
    let failed_and_stale = ClassificationFacts {
        fully_receipted: true,
        any_failed_evidence: true,
        any_stale: true,
    };
    assert_eq!(classify(None, failed_and_stale), FeatureIntegrationState::Failed);

    let stale_only = ClassificationFacts {
        fully_receipted: true,
        any_failed_evidence: false,
        any_stale: true,
    };
    assert_eq!(classify(None, stale_only), FeatureIntegrationState::Stale);
}

#[test]
fn integration_state_serialises_as_snake_case_and_has_no_lifecycle_field() {
    assert_eq!(
        serde_json::to_value(FeatureIntegrationState::Integrated).unwrap(),
        serde_json::json!("integrated")
    );

    // The state is derived, never stored: a Feature value carries no `state`
    // key and no reverse child list.
    let feature = crate::domain::feature::Feature::new(
        crate::domain::name::FeatureName::new("child").unwrap(),
        BranchName::new("child").unwrap(),
    );
    let value = serde_json::to_value(&feature).unwrap();
    let object = value.as_object().unwrap();
    assert!(object.get("state").is_none());
    assert!(object.get("children").is_none());
}
