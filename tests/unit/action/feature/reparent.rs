//! Unit tests for `crate::action::feature::reparent` — the one allowed
//! pristine lineage transition.
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
use crate::action::feature::create::create as create_action;
use crate::action::feature::create::{CreateInput, CreateOutcome};
use crate::action::feature::list::list as list_action;
use crate::domain::name::RepoName;
use crate::error::Status;
use crate::test_support::seeded_hall;

fn create(ctx: &Ctx, name: &str, parent: Option<&str>) -> CreateOutcome {
    create_action(
        ctx,
        CreateInput {
            name: name.to_owned(),
            branch: None,
            base: None,
            parent: parent.map(str::to_owned),
            via: None,
            strategy: None,
        },
    )
    .unwrap()
    .value
}

fn reparent_input(child: &str, parent: &str) -> ReparentInput {
    ReparentInput {
        child: child.to_owned(),
        parent: parent.to_owned(),
    }
}

#[test]
fn reparent_updates_parent_and_base_in_one_feature_write() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(&ctx, "parent-a", None);
    create(&ctx, "parent-b", None);
    let child = create(&ctx, "child", Some("parent-a"));
    assert_eq!(child.parent.as_ref().map(|n| n.as_str()), Some("parent-a"));

    let report = reparent(&ctx, reparent_input("child", "parent-b")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.child.as_str(), "child");
    assert_eq!(
        report.value.old_parent.as_ref().map(|n| n.as_str()),
        Some("parent-a")
    );
    assert_eq!(report.value.new_parent.as_str(), "parent-b");

    // The resulting canonical feature.json carries both the new parent and
    // the derived base — one record write, one shape.
    let layout = Layout::at(&root);
    let on_disk = fs::read_text(
        &layout
            .feature_dir(&FeatureName::new("child").unwrap())
            .join("feature.json"),
    )
    .unwrap()
    .unwrap();
    assert!(on_disk.contains("\"parent\": \"parent-b\""), "{on_disk}");
    assert!(on_disk.contains("\"base\": \"parent-b\""), "{on_disk}");
    assert!(!on_disk.contains("parent-a"), "{on_disk}");
}

#[test]
fn reparent_requires_an_existing_target() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(&ctx, "child", None);

    let failure = reparent(&ctx, reparent_input("child", "ghost")).unwrap_err();
    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "feature.reparent_parent_not_found");
}

#[test]
fn reparent_refuses_self_and_descendant_targets_with_bytes_unchanged() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(&ctx, "root", None);
    create(&ctx, "child", Some("root"));
    create(&ctx, "leaf", Some("child"));
    let layout = Layout::at(&root);
    let path = layout
        .feature_dir(&FeatureName::new("child").unwrap())
        .join("feature.json");
    let before = fs::read_text(&path).unwrap().unwrap();

    // Self-parent.
    let failure = reparent(&ctx, reparent_input("child", "child")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_self_parent");
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), before);

    // A target below the child would cycle the tree.
    let failure = reparent(&ctx, reparent_input("child", "leaf")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_cycle");
    assert_eq!(fs::read_text(&path).unwrap().unwrap(), before);
}

#[test]
fn reparent_refuses_once_any_work_fact_exists() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(&ctx, "root", None);
    create(&ctx, "target", None);
    let layout = Layout::at(&root);
    let path = |name: &str| {
        layout
            .feature_dir(&FeatureName::new(name).unwrap())
            .join("feature.json")
    };
    let bytes = |name: &str| fs::read_bytes(&path(name)).unwrap().unwrap();

    // A promotion is work.
    create(&ctx, "promoted", Some("root"));
    let mut feature = Feature::read(&layout, &FeatureName::new("promoted").unwrap())
        .unwrap()
        .unwrap();
    feature.promote(RepoName::new("api").unwrap());
    feature.write(&layout).unwrap();
    let before = bytes("promoted");
    let failure = reparent(&ctx, reparent_input("promoted", "target")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_work_started");
    assert_eq!(
        bytes("promoted"),
        before,
        "refusal must not touch the record"
    );

    // A plan directory entry is work.
    create(&ctx, "planned", Some("root"));
    fs::ensure_dir(&layout.plan_dir(&FeatureName::new("planned").unwrap())).unwrap();
    fs::write_text(
        &layout
            .plan_dir(&FeatureName::new("planned").unwrap())
            .join("plan.md"),
        "# plan",
    )
    .unwrap();
    let before = bytes("planned");
    let failure = reparent(&ctx, reparent_input("planned", "target")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_work_started");
    assert_eq!(
        bytes("planned"),
        before,
        "refusal must not touch the record"
    );

    // An execution board is work.
    create(&ctx, "executed", Some("root"));
    fs::ensure_dir(&layout.execution_dir(&FeatureName::new("executed").unwrap())).unwrap();
    fs::write_text(
        &layout
            .execution_dir(&FeatureName::new("executed").unwrap())
            .join("board.json"),
        "{}",
    )
    .unwrap();
    let before = bytes("executed");
    let failure = reparent(&ctx, reparent_input("executed", "target")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_work_started");
    assert_eq!(
        bytes("executed"),
        before,
        "refusal must not touch the record"
    );

    // A live (or detached) session dir is work.
    create(&ctx, "sessioned", Some("root"));
    fs::ensure_dir(
        &layout
            .feature_sessions_dir(&FeatureName::new("sessioned").unwrap())
            .join("sess-1"),
    )
    .unwrap();
    let before = bytes("sessioned");
    let failure = reparent(&ctx, reparent_input("sessioned", "target")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_work_started");
    assert_eq!(
        bytes("sessioned"),
        before,
        "refusal must not touch the record"
    );

    // A close record is work.
    create(&ctx, "closed", Some("root"));
    crate::action::feature::lifecycle::write_close(
        &layout,
        &FeatureName::new("closed").unwrap(),
        crate::domain::feature::PromotionOutcome::Abandoned,
    )
    .unwrap();
    let before = bytes("closed");
    let failure = reparent(&ctx, reparent_input("closed", "target")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_work_started");
    assert_eq!(bytes("closed"), before, "refusal must not touch the record");

    // A descendant is work.
    create(&ctx, "with-descendant", Some("root"));
    create(&ctx, "grandchild", Some("with-descendant"));
    let before = bytes("with-descendant");
    let failure = reparent(&ctx, reparent_input("with-descendant", "target")).unwrap_err();
    assert_eq!(failure.code, "feature.reparent_work_started");
    assert_eq!(
        bytes("with-descendant"),
        before,
        "refusal must not touch the record"
    );
}

#[test]
fn a_just_created_child_can_be_reparented_repeatedly_until_work_starts() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(&ctx, "a", None);
    create(&ctx, "b", None);
    create(&ctx, "c", None);
    create(&ctx, "child", None);

    let first = reparent(&ctx, reparent_input("child", "a")).unwrap().value;
    assert_eq!(first.new_parent.as_str(), "a");

    let second = reparent(&ctx, reparent_input("child", "b")).unwrap().value;
    assert_eq!(second.old_parent.as_ref().map(|n| n.as_str()), Some("a"));
    assert_eq!(second.new_parent.as_str(), "b");

    let third = reparent(&ctx, reparent_input("child", "c")).unwrap().value;
    assert_eq!(third.old_parent.as_ref().map(|n| n.as_str()), Some("b"));
    assert_eq!(third.new_parent.as_str(), "c");
}

#[test]
fn list_exposes_parent_depth_and_state_after_reparenting() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    create(&ctx, "root", None);
    create(&ctx, "child", None);

    reparent(&ctx, reparent_input("child", "root")).unwrap();

    let report = list_action(&ctx).unwrap();
    let child_summary = report
        .value
        .features
        .iter()
        .find(|summary| summary.name.as_str() == "child")
        .unwrap();
    assert_eq!(
        child_summary.parent.as_ref().map(|n| n.as_str()),
        Some("root")
    );
    assert_eq!(child_summary.depth, 1);
    assert_eq!(child_summary.state.to_string(), "active");
}
