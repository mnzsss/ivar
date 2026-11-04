use super::*;
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName};
use crate::domain::observation::{Importance, SourceEvent, delivery_event_id, merged_event_id};

fn scope() -> DeliveryScope {
    DeliveryScope {
        feature: "login".to_owned(),
        parent_feature: String::new(),
        repo: "api".to_owned(),
        branch: "login".to_owned(),
        head_sha: "abc123".to_owned(),
    }
}

#[test]
fn importance_follows_the_event_and_its_outcome() {
    let refused = DeliveryEvent::Refused {
        code: "deliver.x".into(),
        blockers: vec![],
        fingerprint: String::new(),
    };
    let pushed = |ok| DeliveryEvent::Pushed {
        ok,
        detail: None,
        base: "main".into(),
    };
    let checks = |passed| DeliveryEvent::Checks {
        passed,
        failed: vec![],
    };
    let landed = |merged, pushed| DeliveryEvent::Landed {
        merged,
        pushed,
        detail: None,
    };
    let rebased = |conflicted| DeliveryEvent::Rebased {
        conflicted,
        onto: None,
        base_source: None,
        sha_before: None,
        sha_after: None,
    };
    let integrated = |ok| DeliveryEvent::Integrated {
        ok,
        via: "pr".into(),
        source_sha: "s".into(),
        merge_commit: None,
    };
    let merged = DeliveryEvent::Merged {
        number: 7,
        url: "u".into(),
        merge_commit: None,
        merged_at: None,
        observed_by: ObservedBy::Cleanup,
    };
    assert_eq!(refused.importance().get(), 7);
    assert_eq!(
        (
            pushed(true).importance().get(),
            pushed(false).importance().get()
        ),
        (4, 7)
    );
    assert_eq!(
        (
            checks(true).importance().get(),
            checks(false).importance().get()
        ),
        (4, 7)
    );
    assert_eq!(
        (
            landed(true, true).importance().get(),
            landed(true, false).importance().get()
        ),
        (4, 7)
    );
    assert_eq!(
        (
            rebased(false).importance().get(),
            rebased(true).importance().get()
        ),
        (4, 7)
    );
    assert_eq!(
        (
            integrated(true).importance().get(),
            integrated(false).importance().get()
        ),
        (4, 7)
    );
    assert_eq!(merged.importance().get(), 6);
    assert_eq!(
        DeliveryEvent::Closed {
            outcome: "delivered".into()
        }
        .importance()
        .get(),
        4
    );
    assert_eq!(
        DeliveryEvent::Deleted { repos: vec![] }.importance(),
        Importance::delivery_routine()
    );
}

#[test]
fn each_event_names_its_source_event() {
    let cleaned = DeliveryEvent::Cleaned {
        record_path: "docs/updates/x.md".into(),
        docs: "written".into(),
    };
    assert_eq!(cleaned.source_event(), SourceEvent::DeliveryCleaned);
    assert_eq!(cleaned.source_event().as_str(), "delivery.cleaned");
}

#[test]
fn body_carries_branch_head_and_stays_valid_json_when_lists_overflow() {
    let event = DeliveryEvent::Refused {
        code: "deliver.blocked".into(),
        blockers: (0..100)
            .map(|n| format!("{n}-{}", "x".repeat(1000)))
            .collect(),
        fingerprint: "fp".into(),
    };
    let body = event.body(&scope());
    assert!(body.len() < 4096, "body is {} bytes", body.len());
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["branch"], "login");
    assert_eq!(value["head_sha"], "abc123");
    assert_eq!(value["blockers"].as_array().unwrap().len(), 10);
}

fn feature(parent: Option<&str>) -> Feature {
    let mut feature = Feature::new(
        FeatureName::new("login").unwrap(),
        BranchName::new("feat/login").unwrap(),
    );
    feature.parent = parent.map(|name| FeatureName::new(name).unwrap());
    feature
}

#[test]
fn scopes_copy_feature_parent_branch_repo_and_head() {
    let child = DeliveryScope::for_repo(&feature(Some("epic")), "api", "abc123");
    assert_eq!(
        (
            child.feature.as_str(),
            child.parent_feature.as_str(),
            child.repo.as_str(),
            child.branch.as_str(),
            child.head_sha.as_str()
        ),
        ("login", "epic", "api", "feat/login", "abc123")
    );
    let root = DeliveryScope::for_feature(&feature(None));
    assert_eq!(
        (
            root.parent_feature.as_str(),
            root.repo.as_str(),
            root.head_sha.as_str()
        ),
        ("", "", "")
    );
}

#[test]
fn a_merged_body_names_who_observed_the_merge() {
    let merged = |observed_by| DeliveryEvent::Merged {
        number: 7,
        url: "u".into(),
        merge_commit: Some("m".into()),
        merged_at: Some("t".into()),
        observed_by,
    };
    let cleanup: serde_json::Value =
        serde_json::from_str(&merged(ObservedBy::Cleanup).body(&scope())).unwrap();
    let integrate: serde_json::Value =
        serde_json::from_str(&merged(ObservedBy::Integrate).body(&scope())).unwrap();
    assert_eq!(
        (
            cleanup["observed_by"].as_str(),
            integrate["observed_by"].as_str()
        ),
        (Some("cleanup"), Some("integrate"))
    );
    assert_eq!(cleanup["merged_at"], "t");
}

#[test]
fn merged_id_ignores_attempt_time_and_process() {
    let a = merged_event_id("api", "https://x/pull/7", "deadbeef");
    assert_eq!(a, merged_event_id("api", "https://x/pull/7", "deadbeef"));
    assert_ne!(a, merged_event_id("api", "https://x/pull/8", "deadbeef"));
}

#[test]
fn delivery_ids_separate_attempts_events_repos_and_order() {
    let base = delivery_event_id("att-1", SourceEvent::DeliveryPushed, "api", 0);
    assert_eq!(
        base,
        delivery_event_id("att-1", SourceEvent::DeliveryPushed, "api", 0)
    );
    assert_ne!(
        base,
        delivery_event_id("att-2", SourceEvent::DeliveryPushed, "api", 0)
    );
    assert_ne!(
        base,
        delivery_event_id("att-1", SourceEvent::DeliveryChecks, "api", 0)
    );
    assert_ne!(
        base,
        delivery_event_id("att-1", SourceEvent::DeliveryPushed, "web", 0)
    );
    assert_ne!(
        base,
        delivery_event_id("att-1", SourceEvent::DeliveryPushed, "api", 1)
    );
}
