use super::fixture::*;
use super::*;
use crate::domain::feature::{DeliveryAction, DeliveryRepo};
use crate::domain::name::BranchName;
use crate::domain::provider::Provider;
use crate::error::Status;
use crate::store::layout::Layout;
use crate::test_support::git;
use camino::Utf8Path;

mod execute;
mod permissions;
mod preflight;
mod preview;

#[test]
fn land_on_default_serialises_as_snake_case_and_has_a_word() {
    let action = DeliveryAction::LandOnDefault;
    assert_eq!(
        serde_json::to_value(action).unwrap(),
        serde_json::json!("land_on_default")
    );
    assert_eq!(action_word(action), "land on default");
}
