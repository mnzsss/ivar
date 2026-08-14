//! Unit tests for `crate::action::confirm` — the confirmation seam.
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

#[test]
fn a_disabled_reporter_never_consents_and_never_reads() {
    let confirmer = reporter(false);
    assert!(!confirmer.confirm("Delete everything?", None).unwrap());
    assert!(!confirmer.confirm("Rewrite it?", Some("careful")).unwrap());
}

#[test]
fn an_enabled_reporter_returns_an_interactive_confirmer() {
    let confirmer = reporter(true);
    // The seam shape is all that is asserted here — the interactive half reads
    // the real terminal, which the test suite never is.
    assert!(format!("{confirmer:?}").contains("Interactive"));
}

#[test]
fn fixed_answers_its_value_for_tests() {
    let yes = fixed(true);
    let no = fixed(false);
    assert!(yes.confirm("Migrate?", Some("careful")).unwrap());
    assert!(!no.confirm("Migrate?", Some("careful")).unwrap());
}

#[test]
fn the_seam_carries_onto_ctx_and_defaults_to_never() {
    let (_tmp, root) = crate::test_support::utf8_temp_dir();
    let ctx = crate::action::Ctx::new(root);
    assert!(!ctx.confirm("Anything?", None).unwrap());
}
