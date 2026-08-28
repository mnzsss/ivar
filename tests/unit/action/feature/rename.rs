//! Unit tests for `crate::action::feature::rename`.
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
use crate::action::feature::create::CreateInput;
use crate::action::feature::create::create as create_action;
use crate::error::Status;
use crate::test_support::seeded_hall;

fn rename_input(feature: &str, name: Option<&str>, branch: Option<&str>) -> RenameInput {
    RenameInput {
        feature: feature.to_owned(),
        name: name.map(str::to_owned),
        branch: branch.map(str::to_owned),
    }
}

#[test]
fn rename_refuses_when_name_and_branch_unchanged() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    create_action(
        &ctx,
        CreateInput {
            name: "my-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let err = rename(&ctx, rename_input("my-feat", Some("my-feat"), None)).unwrap_err();
    assert_eq!(err.status, Status::Blocked);
    assert_eq!(err.code, "feature.rename_noop");
}

#[test]
fn rename_feature_name_success() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root);

    create_action(
        &ctx,
        CreateInput {
            name: "old-feat".to_owned(),
            branch: None,
            base: None,
            parent: None,
            via: None,
            strategy: None,
        },
    )
    .unwrap();

    let report = rename(&ctx, rename_input("old-feat", Some("new-feat"), None)).unwrap();
    assert!(report.is_clean());
    let outcome = report.value;
    assert_eq!(outcome.old_name.as_str(), "old-feat");
    assert_eq!(outcome.new_name.as_str(), "new-feat");
}
