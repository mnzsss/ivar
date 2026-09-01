#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

fn repo(base: &str) -> DeliveryRepo {
    DeliveryRepo {
        repo: RepoName::new("api").unwrap(),
        local_branch: BranchName::new("checkout").unwrap(),
        remote: "https://example.invalid/api.git".to_owned(),
        push_refspec: "checkout:refs/heads/checkout".to_owned(),
        action: DeliveryAction::PushOnly,
        base_branch: BranchName::new(base).unwrap(),
        dependencies: Vec::new(),
        blockers: Vec::new(),
        pr_url: None,
        default_branch: None,
        ff_possible: None,
        remote_default_tip: None,
        pr_title: None,
        pr_body: None,
        draft: None,
    }
}

fn main_branch() -> BranchName {
    BranchName::new("main").unwrap()
}
fn delivery_repo(repo: &str) -> DeliveryRepo {
    DeliveryRepo {
        repo: RepoName::new(repo).unwrap(),
        local_branch: BranchName::new("checkout").unwrap(),
        remote: "git@example.com:acme/api.git".to_owned(),
        push_refspec: "checkout:refs/heads/checkout".to_owned(),
        action: DeliveryAction::PushOnly,
        base_branch: BranchName::new("main").unwrap(),
        dependencies: Vec::new(),
        blockers: Vec::new(),
        pr_url: None,
        default_branch: None,
        ff_possible: None,
        remote_default_tip: None,
        pr_title: None,
        pr_body: None,
        draft: None,
    }
}

#[test]
fn a_delivery_preview_round_trips_through_serde() {
    let mut repo = delivery_repo("api");
    repo.pr_title = Some("feat: custom title".to_owned());
    repo.pr_body = Some("custom body".to_owned());
    let preview = DeliveryPreview {
        feature: FeatureName::new("checkout").unwrap(),
        mode: DeliveryMode::Push,
        plan_gate: GateState::Approved,
        repos: vec![repo],
        tree_blockers: Vec::new(),
        fingerprint: "abc123".to_owned(),
    };

    let parsed: DeliveryPreview =
        serde_json::from_value(serde_json::to_value(&preview).unwrap()).unwrap();

    assert_eq!(parsed, preview);
}

#[test]
fn delivery_repo_serializes_explicit_nulls_for_absent_pr_metadata() {
    let repo = delivery_repo("api");
    let json = serde_json::to_value(&repo).unwrap();
    assert_eq!(json.get("pr_title"), Some(&serde_json::Value::Null));
    assert_eq!(json.get("pr_body"), Some(&serde_json::Value::Null));
}

#[test]
fn delivery_action_serialises_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&DeliveryAction::PushOnly).unwrap(),
        r#""push_only""#
    );
    assert_eq!(
        serde_json::to_string(&DeliveryAction::NewPr).unwrap(),
        r#""new_pr""#
    );
    assert_eq!(
        serde_json::to_string(&DeliveryAction::UpdatePr).unwrap(),
        r#""update_pr""#
    );
}

#[test]
fn an_unknown_field_in_a_delivery_repo_is_refused() {
    let repo = delivery_repo("api");
    let rendered = serde_json::to_value(&repo).unwrap();
    let mut with_bogus = rendered.as_object().unwrap().clone();
    with_bogus.insert("bogus".to_owned(), serde_json::json!(true));

    assert!(serde_json::from_value::<DeliveryRepo>(serde_json::Value::Object(with_bogus)).is_err());
}

#[test]
fn classify_base_is_ok_when_the_base_is_present_and_still_an_ancestor() {
    assert_eq!(
        classify_base(Ok(Some("deadbeef".to_owned())), Ok(true)),
        BaseVerdict::Ok
    );
}

#[test]
fn classify_base_is_merged_and_deleted_when_absent_but_merged_into_default() {
    assert_eq!(
        classify_base(Ok(None), Ok(true)),
        BaseVerdict::BaseMergedAndDeleted
    );
}

#[test]
fn classify_base_is_never_delivered_when_absent_and_not_confirmed_merged() {
    assert_eq!(
        classify_base(Ok(None), Ok(false)),
        BaseVerdict::BaseNeverDelivered
    );
}

/// When the base is absent and local ancestry cannot even be checked (its
/// ref is genuinely gone), refusing as "never delivered" is the safe
/// default — there is no evidence it was ever merged.
#[test]
fn classify_base_is_never_delivered_when_absent_and_ancestry_cannot_be_checked() {
    assert_eq!(
        classify_base(Ok(None), Err(())),
        BaseVerdict::BaseNeverDelivered
    );
}

#[test]
fn classify_base_is_unconfirmed_when_the_remote_does_not_answer() {
    assert_eq!(
        classify_base(Err(()), Ok(true)),
        BaseVerdict::BaseUnconfirmed
    );
}

#[test]
fn classify_base_is_moved_when_present_but_no_longer_an_ancestor() {
    assert_eq!(
        classify_base(Ok(Some("deadbeef".to_owned())), Ok(false)),
        BaseVerdict::BaseMoved
    );
}

#[test]
fn classify_base_is_moved_when_present_and_ancestry_cannot_be_checked() {
    assert_eq!(
        classify_base(Ok(Some("deadbeef".to_owned())), Err(())),
        BaseVerdict::BaseMoved
    );
}

#[test]
fn check_base_allows_delivery_when_the_verdict_is_ok() {
    let delivery_repo = repo("develop");

    let refusal =
        delivery_repo.check_base(Ok(Some("deadbeef".to_owned())), Ok(true), &main_branch());

    assert!(refusal.is_none());
}

#[test]
fn check_base_refuses_a_merged_and_deleted_base_with_a_rebase_onto_default_fix() {
    let delivery_repo = repo("develop");

    let failure = delivery_repo
        .check_base(Ok(None), Ok(true), &main_branch())
        .expect("a merged-and-deleted base refuses");

    assert_eq!(failure.code, "feature.base_merged_and_deleted");
    assert!(failure.what.contains("develop"));
    assert_eq!(failure.fix_actions.len(), 1);
    assert!(
        !failure.fix_actions[0].safe,
        "rebasing onto a new target is a judgment call — a human decides"
    );
    assert!(failure.fix_actions[0].what.contains("--onto main"));
}

#[test]
fn check_base_refuses_a_never_delivered_base_with_a_deliver_parent_first_fix() {
    let delivery_repo = repo("develop");

    let failure = delivery_repo
        .check_base(Ok(None), Ok(false), &main_branch())
        .expect("a never-delivered base refuses");

    assert_eq!(failure.code, "feature.base_never_delivered");
    assert!(failure.what.contains("develop"));
    assert!(failure.fix_actions[0].what.contains("Deliver"));
    assert!(!failure.fix_actions[0].safe);
}

/// The remote not answering must read as an open question, never as the
/// base being gone — those are different facts with different fixes.
#[test]
fn check_base_refuses_as_unconfirmed_never_as_absent_when_the_remote_does_not_answer() {
    let delivery_repo = repo("develop");

    let failure = delivery_repo
        .check_base(Err(()), Ok(true), &main_branch())
        .expect("an unanswered remote refuses");

    assert_eq!(failure.code, "feature.base_unconfirmed");
    assert!(
        !failure.what.to_lowercase().contains("absent"),
        "an unanswered remote must never be reported as an absent base: {}",
        failure.what
    );
    assert!(!failure.fix_actions[0].safe);
}

#[test]
fn check_base_refuses_a_moved_base_with_a_rebase_the_feature_fix() {
    let delivery_repo = repo("develop");

    let failure = delivery_repo
        .check_base(Ok(Some("deadbeef".to_owned())), Ok(false), &main_branch())
        .expect("a moved base refuses");

    assert_eq!(failure.code, "feature.base_moved");
    assert!(failure.what.contains("develop"));
    assert!(failure.fix_actions[0].what.contains("rebase"));
    assert!(!failure.fix_actions[0].safe);
}
