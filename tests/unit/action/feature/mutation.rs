//! Unit tests for `crate::action::feature::mutation` — the scoped mutation
//! boundaries for a feature that has begun integrating.
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
use crate::action::feature::lifecycle::write_close;
use crate::domain::feature::{
    Feature, IntegrationReceipt, IntegrationStrategy, IntegrationVia, PromotionOutcome,
    VerificationEvidence, VerificationResult, WorkstreamDef, WorkstreamStatus,
};
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::store::layout::Layout;
use crate::test_support::hall_root;

fn layout() -> Layout {
    let (_guard, root) = hall_root();
    Layout::at(&root)
}

fn feature_with(parent: Option<&str>, name: &str) -> Feature {
    let mut feature = Feature::new(
        FeatureName::new(name).unwrap(),
        BranchName::new(name).unwrap(),
    );
    feature.parent = parent.map(|p| FeatureName::new(p).unwrap());
    feature
}

fn failing_receipt() -> IntegrationReceipt {
    IntegrationReceipt {
        source_sha: "111".to_owned(),
        target_branch: BranchName::new("parent").unwrap(),
        result_sha: "222".to_owned(),
        via: IntegrationVia::Local,
        strategy: IntegrationStrategy::Squash,
        pr_url: None,
        verification: VerificationEvidence {
            command_fingerprint: "checks-v1".to_owned(),
            child: vec![VerificationResult::failed("true", Some(1), "boom")],
            parent: Vec::new(),
            pr_checks: Vec::new(),
            verified_at: "2026-08-14T12:00:00Z".to_owned(),
        },
    }
}

fn passing_receipt() -> IntegrationReceipt {
    let mut receipt = failing_receipt();
    receipt.verification.child = Vec::new();
    receipt
}

fn workstream(contract: &[&str]) -> WorkstreamDef {
    WorkstreamDef {
        id: "ws1".to_owned(),
        title: "WS one".to_owned(),
        operations: vec!["op1".to_owned()],
        depends_on: Vec::new(),
        write_contract: contract.iter().map(|s| s.to_string()).collect(),
        status: WorkstreamStatus::Waiting,
        provider: None,
        model: None,
        agent: None,
    }
}

// -- the whole child: `integrated` freezes everything ----------------------

#[test]
fn an_integrated_close_record_blocks_every_mutation_guard() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    child.promote(api.clone());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing_receipt());
    child.write(&layout).unwrap();
    write_close(
        &layout,
        &FeatureName::new("child").unwrap(),
        PromotionOutcome::Integrated,
    )
    .unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    let failure = ensure_not_fully_integrated(&layout, &child).unwrap_err();
    assert_eq!(failure.code, "feature.integration_immutable");
    let failure = ensure_structure_mutable(&layout, &child).unwrap_err();
    assert_eq!(failure.code, "feature.integration_immutable");
    let failure = ensure_promotion_mutable(&layout, &child, &api).unwrap_err();
    assert_eq!(failure.code, "feature.integration_immutable");
    let failure = ensure_unrestricted_session_allowed(&layout, &child).unwrap_err();
    assert_eq!(failure.code, "feature.integration_immutable");
}

// -- structure freezes at the first receipt --------------------------------

#[test]
fn any_receipt_freezes_structure_but_failed_evidence_does_not_lock_the_promotion() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    let web = RepoName::new("web").unwrap();
    child.promote(api.clone());
    child.promote(web.clone());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing_receipt());
    child.promotions.get_mut(&web).unwrap().integration_receipt = Some(failing_receipt());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    // Structure is frozen by any receipt — even the failed one.
    let failure = ensure_structure_mutable(&layout, &child).unwrap_err();
    assert_eq!(failure.code, "feature.integration_structure_frozen");

    // The successful promotion is locked; the failed-evidence one is not.
    let failure = ensure_promotion_mutable(&layout, &child, &api).unwrap_err();
    assert_eq!(failure.code, "feature.promotion_integration_immutable");
    assert!(ensure_promotion_mutable(&layout, &child, &web).is_ok());
}

#[test]
fn a_successful_receipt_stays_locked_when_it_goes_stale() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    child.promote(api.clone());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing_receipt());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    // Freshness is irrelevant to the lock: the recorded evidence passed.
    assert!(ensure_promotion_mutable(&layout, &child, &api).is_err());
}

// -- plan/board/journal mutations stay legal during partial state ----------

#[test]
fn plan_only_mutations_are_allowed_during_partial_integration() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    child.promote(api.clone());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(failing_receipt());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    // Failed-evidence-only partial state: plan/board mutations remain legal.
    assert!(ensure_not_fully_integrated(&layout, &child).is_ok());

    // But structure is still frozen, because a receipt exists.
    assert!(ensure_structure_mutable(&layout, &child).is_err());
}

// -- contracts must name literal repos and avoid locked promotions ---------

#[test]
fn contracts_are_unchanged_when_no_successful_receipt_exists() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    child.promote(RepoName::new("api").unwrap());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    // Even a globby contract is fine before the first successful receipt.
    let streams = vec![workstream(&["**/*.rs"])];
    assert!(ensure_contracts_avoid_locked_promotions(&layout, &child, &streams).is_ok());
}

#[test]
fn contracts_must_name_literal_promoted_repos_once_a_success_receipt_exists() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    let web = RepoName::new("web").unwrap();
    child.promote(api.clone());
    child.promote(web.clone());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing_receipt());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    let cases: &[(&[&str], &str)] = &[
        // A literal path under the unreceipted repo B passes.
        (&["web/src/lib.rs"], "ok"),
        (&["web/**"], "ok"),
        (&["web/"], "ok"),
        // A literal path under the successful repo A is refused.
        (&["api/src/lib.rs"], "feature.promotion_integration_immutable"),
        // Ambiguous first components refuse the whole wave.
        (&["*/src/lib.rs"], "feature.partial_contract_ambiguous"),
        (&["**/*.rs"], "feature.partial_contract_ambiguous"),
        (&["../a/x"], "feature.partial_contract_ambiguous"),
        (&["/a/x"], "feature.partial_contract_ambiguous"),
        (&["c/x"], "feature.partial_contract_ambiguous"),
    ];
    for (contract, expected) in cases {
        let streams = vec![workstream(contract)];
        let result = ensure_contracts_avoid_locked_promotions(&layout, &child, &streams);
        match *expected {
            "ok" => assert!(result.is_ok(), "contract {contract:?} should pass: {result:?}"),
            code => {
                let failure = result.unwrap_err();
                assert_eq!(failure.code, code, "contract {contract:?}");
                // Contract failures name the exact workstream and path.
                assert!(failure.what.contains("ws1"), "{failure:?}");
                assert!(failure.what.contains(contract[0]), "{failure:?}");
            }
        }
    }
}

#[test]
fn one_ambiguous_path_anywhere_refuses_the_whole_wave() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    child.promote(api.clone());
    child.promote(RepoName::new("web").unwrap());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing_receipt());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    let clean = workstream(&["web/src/lib.rs"]);
    let mut dirty = workstream(&["web/src/lib.rs"]);
    dirty.id = "ws2".to_owned();
    dirty.write_contract = vec!["*/src/lib.rs".to_owned()];

    let failure =
        ensure_contracts_avoid_locked_promotions(&layout, &child, &[clean, dirty]).unwrap_err();
    assert_eq!(failure.code, "feature.partial_contract_ambiguous");
    assert!(failure.what.contains("ws2"));
}

// -- unrestricted sessions --------------------------------------------------

#[test]
fn unrestricted_sessions_are_refused_once_a_successful_receipt_exists() {
    let layout = layout();
    let mut child = feature_with(Some("parent"), "child");
    let api = RepoName::new("api").unwrap();
    child.promote(api.clone());
    child.promotions.get_mut(&api).unwrap().integration_receipt = Some(passing_receipt());
    child.write(&layout).unwrap();
    let child = Feature::read(&layout, &FeatureName::new("child").unwrap())
        .unwrap()
        .unwrap();

    let failure = ensure_unrestricted_session_allowed(&layout, &child).unwrap_err();
    assert_eq!(failure.code, "feature.session_unrestricted_blocked");
}

#[test]
fn unrestricted_sessions_pass_for_fresh_and_failed_evidence_only_children() {
    let layout = layout();
    let fresh = feature_with(Some("parent"), "fresh");
    fresh.write(&layout).unwrap();
    let fresh = Feature::read(&layout, &FeatureName::new("fresh").unwrap())
        .unwrap()
        .unwrap();
    assert!(ensure_unrestricted_session_allowed(&layout, &fresh).is_ok());

    let mut failed = feature_with(Some("parent"), "failed");
    let api = RepoName::new("api").unwrap();
    failed.promote(api.clone());
    failed.promotions.get_mut(&api).unwrap().integration_receipt = Some(failing_receipt());
    failed.write(&layout).unwrap();
    let failed = Feature::read(&layout, &FeatureName::new("failed").unwrap())
        .unwrap()
        .unwrap();
    assert!(ensure_unrestricted_session_allowed(&layout, &failed).is_ok());
}
