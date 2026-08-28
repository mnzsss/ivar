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
fn disabled_reporter_returns_blocked_failure_listing_options_on_select_many() {
    let confirmer = reporter(false);
    let options = vec![
        SelectOption {
            id: "opt1".to_owned(),
            description: Some("First option".to_owned()),
            path_if_any: "skills/opt1".to_owned(),
        },
        SelectOption {
            id: "opt2".to_owned(),
            description: None,
            path_if_any: "skills/opt2".to_owned(),
        },
    ];
    let err = confirmer.select_many("Select:", &options).unwrap_err();
    assert_eq!(err.code, "skill.add.multiple_choices");
    assert!(
        err.what
            .contains("opt1 (--path skills/opt1) — First option")
    );
    assert!(err.what.contains("opt2 (--path skills/opt2)"));
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
fn fixed_select_returns_preset_indices() {
    let f = fixed_select(true, vec![0, 2]);
    let options = vec![
        SelectOption {
            id: "a".to_owned(),
            description: None,
            path_if_any: "".to_owned(),
        },
        SelectOption {
            id: "b".to_owned(),
            description: None,
            path_if_any: "".to_owned(),
        },
        SelectOption {
            id: "c".to_owned(),
            description: None,
            path_if_any: "".to_owned(),
        },
    ];
    assert_eq!(f.select_many("Select:", &options).unwrap(), vec![0, 2]);
}

#[test]
fn the_seam_carries_onto_ctx_and_defaults_to_never() {
    let (_tmp, root) = crate::test_support::utf8_temp_dir();
    let ctx = crate::action::Ctx::new(root);
    assert!(!ctx.confirm("Anything?", None).unwrap());
}
