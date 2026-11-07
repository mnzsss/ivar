#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

const TODAY: &str = "2026-11-07";

fn set(texts: &[&str]) -> BTreeSet<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

fn decision(id: i64, kind: DecisionKind, texts: &[&str], until: Option<&str>) -> Decision {
    let accepted = kind == DecisionKind::Accepted;
    Decision {
        id,
        kind,
        intent: format!("intent-{id}"),
        form: accepted.then_some(Form::Script),
        artifact: accepted.then(|| "a.sh".to_owned()),
        until: until.map(str::to_owned),
        created_at: "then".to_owned(),
        updated_at: "then".to_owned(),
        texts: set(texts),
    }
}

#[test]
fn a_group_matches_when_any_one_of_its_texts_is_in_the_set() {
    let declined = decision(
        1,
        DecisionKind::Declined,
        &["bump version", "bump the version"],
        None,
    );

    assert!(declined.matches(["bump version"]));
    assert!(declined.matches(["unrelated", "bump the version", "other"]));
    assert!(!declined.matches(["unrelated", "other"]));
    assert!(!declined.matches([]));
}

#[test]
fn a_decision_survives_its_group_gaining_texts() {
    let declined = decision(1, DecisionKind::Declined, &["bump version"], None);

    assert!(declined.matches(["bump version", "bump version again", "bump version now"]));
}

#[test]
fn an_accepted_decision_without_its_artifact_is_not_in_force() {
    let accepted = decision(1, DecisionKind::Accepted, &["a"], None);

    assert!(accepted.is_in_force(TODAY, true));
    assert!(!accepted.is_in_force(TODAY, false));
}

#[test]
fn a_deferral_holds_through_its_last_day_and_not_after() {
    let deferred = decision(1, DecisionKind::Deferred, &["a"], Some(TODAY));

    assert!(deferred.is_in_force(TODAY, true));
    assert!(deferred.is_in_force("2026-11-06", true));
    assert!(!deferred.is_in_force("2026-11-08", true));
}

#[test]
fn a_decline_is_always_in_force() {
    assert!(decision(1, DecisionKind::Declined, &["a"], None).is_in_force("2099-01-01", false));
}

#[test]
fn the_sweep_extends_a_decision_with_the_texts_of_the_groups_that_match_it() {
    let declined = decision(1, DecisionKind::Declined, &["a"], None);
    let deferred = decision(2, DecisionKind::Deferred, &["x"], Some("2026-12-01"));
    let groups = [set(&["a", "a2"]), set(&["x", "x2", "x3"]), set(&["other"])];

    let plan = plan_sweep(&[declined, deferred], &groups, TODAY);

    assert!(plan.delete.is_empty());
    assert_eq!(
        plan.extend,
        vec![(1, set(&["a2"])), (2, set(&["x2", "x3"]))]
    );
}

#[test]
fn the_sweep_takes_the_texts_of_every_group_that_matches() {
    let declined = decision(1, DecisionKind::Declined, &["a", "b"], None);
    let groups = [set(&["a", "a2"]), set(&["b", "b2"])];

    let plan = plan_sweep(&[declined], &groups, TODAY);

    assert_eq!(plan.extend, vec![(1, set(&["a2", "b2"]))]);
}

#[test]
fn the_sweep_adds_nothing_when_the_group_has_no_new_text() {
    let declined = decision(1, DecisionKind::Declined, &["a", "b"], None);

    let plan = plan_sweep(&[declined], &[set(&["a"])], TODAY);

    assert_eq!(plan, SweepPlan::default());
}

#[test]
fn the_sweep_deletes_a_decline_that_nothing_in_the_window_matches() {
    let declined = decision(1, DecisionKind::Declined, &["a"], None);

    let plan = plan_sweep(std::slice::from_ref(&declined), &[set(&["other"])], TODAY);
    assert_eq!(plan.delete, vec![1]);
    assert!(plan.extend.is_empty());

    let plan = plan_sweep(&[declined], &[], TODAY);
    assert_eq!(plan.delete, vec![1]);
}

#[test]
fn the_sweep_keeps_a_deferral_nothing_matches_until_its_date() {
    let deferred = decision(1, DecisionKind::Deferred, &["a"], Some("2026-12-01"));

    let plan = plan_sweep(&[deferred], &[], TODAY);

    assert_eq!(plan, SweepPlan::default());
}

#[test]
fn the_sweep_deletes_a_deferral_past_its_date_even_when_it_still_matches() {
    let deferred = decision(1, DecisionKind::Deferred, &["a"], Some("2026-11-06"));

    let plan = plan_sweep(&[deferred], &[set(&["a", "b"])], TODAY);

    assert_eq!(plan.delete, vec![1]);
    assert!(plan.extend.is_empty());
}

#[test]
fn the_sweep_never_touches_an_accepted_decision() {
    let accepted = decision(1, DecisionKind::Accepted, &["a"], None);

    assert_eq!(
        plan_sweep(std::slice::from_ref(&accepted), &[], "2099-01-01"),
        SweepPlan::default()
    );
    assert_eq!(
        plan_sweep(&[accepted], &[set(&["a", "b"])], TODAY),
        SweepPlan::default()
    );
}

#[test]
fn only_a_decline_or_a_live_deferral_needs_the_groups() {
    let accepted = decision(1, DecisionKind::Accepted, &["a"], None);
    let due = decision(2, DecisionKind::Deferred, &["a"], Some("2026-11-06"));
    let live = decision(3, DecisionKind::Deferred, &["a"], Some("2026-12-01"));
    let declined = decision(4, DecisionKind::Declined, &["a"], None);

    assert!(!needs_groups(&[accepted.clone(), due.clone()], TODAY));
    assert!(needs_groups(&[accepted.clone(), live], TODAY));
    assert!(needs_groups(&[accepted, due, declined], TODAY));
}

#[test]
fn a_day_is_a_real_calendar_date() {
    assert!(is_day("2026-11-07"));
    assert!(is_day("2028-02-29"));
    assert!(!is_day("2026-02-29"));
    assert!(!is_day("2026-13-01"));
    assert!(!is_day("2026-1-07"));
    assert!(!is_day("tomorrow"));
    assert!(!is_day("2026-11-07T00:00:00Z"));
    assert!(!is_day("+026-11-07"));
}

#[test]
fn the_utc_day_of_an_epoch() {
    assert_eq!(utc_day(0), "1970-01-01");
    assert_eq!(utc_day(1_800_000_000), "2027-01-15");
    assert_eq!(utc_day(1_800_000_000 + 30 * 86_400), "2027-02-14");
}

#[test]
fn an_artifact_is_a_clean_relative_path_inside_the_hall() {
    assert_eq!(
        clean_artifact(".ivar/scripts/a.sh").unwrap(),
        ".ivar/scripts/a.sh"
    );
    assert_eq!(clean_artifact("./a//b.sh").unwrap(), "a/b.sh");
    assert!(clean_artifact("/etc/passwd").is_err());
    assert!(clean_artifact("../outside.sh").is_err());
    assert!(clean_artifact("a/../../b").is_err());
    assert!(clean_artifact("").is_err());
    assert!(clean_artifact(".").is_err());
}

#[test]
fn an_intent_is_a_short_single_line_name() {
    assert_eq!(intent_problem("update changelog"), None);
    assert!(intent_problem("  ").is_some());
    assert!(intent_problem("two\nlines").is_some());
    assert!(intent_problem(&"x".repeat(101)).is_some());
}

#[test]
fn kinds_and_forms_round_trip_through_their_names() {
    for kind in [
        DecisionKind::Accepted,
        DecisionKind::Declined,
        DecisionKind::Deferred,
    ] {
        assert_eq!(DecisionKind::parse(kind.as_str()), Some(kind));
    }
    for form in [Form::Script, Form::Skill, Form::Agent, Form::Procedure] {
        assert_eq!(Form::parse(form.as_str()), Some(form));
    }
    assert_eq!(DecisionKind::parse("maybe"), None);
    assert_eq!(Form::parse("tool"), None);
}
