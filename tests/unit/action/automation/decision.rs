#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use rusqlite::params;

use super::*;
use crate::action::automation::scan::{ScanInput, scan_in};
use crate::domain::session::rfc3339_from_epoch_secs;
use crate::test_support::seeded_hall;

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;

fn at(days_ago: u64) -> String {
    rfc3339_from_epoch_secs((NOW - days_ago * DAY) as f64)
}

fn hall() -> (tempfile::TempDir, Layout) {
    let (guard, root) = seeded_hall();
    (guard, Layout::at(root))
}

fn plant(layout: &Layout, rows: &[(&str, u64, &str)]) {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    for (session, days_ago, text) in rows {
        let when = at(*days_ago);
        db.conn()
            .execute(
                "INSERT INTO request_fingerprints
                   (observation_id, session_id, feature, day, normalized, created_at)
                 VALUES (0, ?1, '', substr(?2, 1, 10), ?3, ?2)",
                params![session, when, text],
            )
            .unwrap();
    }
}

/// Three sessions over two days asking for the same thing.
fn recurring(layout: &Layout, text: &str) {
    plant(layout, &[("s1", 6, text), ("s2", 6, text), ("s3", 2, text)]);
}

fn group_ref(layout: &Layout, text: &str) -> i64 {
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    db.conn()
        .query_row(
            "SELECT max(id) FROM request_fingerprints WHERE normalized = ?1",
            [text],
            |row| row.get(0),
        )
        .unwrap()
}

fn decide_at(layout: &Layout, group_ref: i64, answer: Answer, now: u64) -> Outcome<DecideOutcome> {
    decide_in(layout, &DecideInput { group_ref, answer }, now)
}

fn declined(intent: &str) -> Answer {
    Answer::Declined {
        intent: intent.to_owned(),
    }
}

fn deferred(intent: &str, until: Option<&str>) -> Answer {
    Answer::Deferred {
        intent: intent.to_owned(),
        until: until.map(str::to_owned),
    }
}

fn scanned(layout: &Layout, now: u64) -> crate::action::automation::scan::ScanOutcome {
    scan_in(
        layout,
        &ScanInput {
            feature: None,
            drain: false,
        },
        now,
    )
    .unwrap()
    .value
}

fn code(result: Outcome<impl std::fmt::Debug>) -> &'static str {
    result.unwrap_err().code
}

#[test]
fn a_decline_stores_the_groups_current_texts_and_suppresses_it() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    plant(&layout, &[("s4", 1, "update changelog entry")]);
    let reference = group_ref(&layout, "update changelog");

    let report = decide_at(&layout, reference, declined("changelog"), NOW).unwrap();

    let view = &report.value.decision;
    assert_eq!(view.kind, DecisionKind::Declined);
    assert_eq!(view.intent, "changelog");
    assert_eq!(
        (view.form, &view.artifact, &view.until),
        (None, &None, &None)
    );
    assert_eq!(view.text_count, 2);
    assert_eq!(report.value.texts_added, 2);
    let outcome = scanned(&layout, NOW);
    assert!(outcome.candidates.is_empty());
    let [suppressed] = outcome.suppressed.as_slice() else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        (
            suppressed.decision_id,
            suppressed.intent.as_str(),
            suppressed.groups
        ),
        (view.id, "changelog", 1)
    );
}

#[test]
fn the_ref_may_be_any_fingerprint_of_the_group() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    let oldest = {
        let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
        db.conn()
            .query_row("SELECT min(id) FROM request_fingerprints", [], |r| r.get(0))
            .unwrap()
    };

    decide_at(&layout, oldest, declined("changelog"), NOW).unwrap();

    assert!(scanned(&layout, NOW).candidates.is_empty());
}

#[test]
fn a_decline_survives_the_group_gaining_new_texts() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        declined("changelog"),
        NOW,
    )
    .unwrap();

    plant(&layout, &[("s4", 0, "update changelog quickly")]);

    let outcome = scanned(&layout, NOW);
    assert!(outcome.candidates.is_empty());
    assert_eq!(outcome.suppressed.len(), 1);
}

#[test]
fn a_deferral_defaults_to_thirty_days_and_takes_a_given_date() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    recurring(&layout, "rotate the keys");
    let changelog = group_ref(&layout, "update changelog");
    let keys = group_ref(&layout, "rotate the keys");

    let default = decide_at(&layout, changelog, deferred("changelog", None), NOW).unwrap();
    let given = decide_at(&layout, keys, deferred("keys", Some("2027-03-01")), NOW).unwrap();

    // NOW is 2027-01-15.
    assert_eq!(default.value.decision.until.as_deref(), Some("2027-02-14"));
    assert_eq!(given.value.decision.until.as_deref(), Some("2027-03-01"));
    assert_eq!(scanned(&layout, NOW).suppressed.len(), 2);
}

#[test]
fn a_deferral_stops_suppressing_the_day_after_its_date() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        deferred("changelog", Some("2027-01-20")),
        NOW,
    )
    .unwrap();

    assert_eq!(scanned(&layout, NOW + 5 * DAY).suppressed.len(), 1);
    let after = scanned(&layout, NOW + 6 * DAY);
    assert!(after.suppressed.is_empty());
    assert_eq!(after.candidates.len(), 1);
}

#[test]
fn an_accepted_group_is_no_candidate_and_is_not_reported_as_suppressed() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    std::fs::create_dir_all(layout.root().join(".ivar/scripts").as_std_path()).unwrap();
    std::fs::write(
        layout
            .root()
            .join(".ivar/scripts/changelog.sh")
            .as_std_path(),
        "",
    )
    .unwrap();

    let report = decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        Answer::Accepted {
            intent: "changelog".to_owned(),
            form: Form::Script,
            artifact: "./.ivar/scripts/changelog.sh".to_owned(),
        },
        NOW,
    )
    .unwrap();

    let view = &report.value.decision;
    assert_eq!(view.form, Some(Form::Script));
    assert_eq!(view.artifact.as_deref(), Some(".ivar/scripts/changelog.sh"));
    assert!(report.is_clean());
    let outcome = scanned(&layout, NOW);
    assert!(outcome.candidates.is_empty());
    assert!(outcome.suppressed.is_empty());
}

#[test]
fn an_accepted_decision_whose_artifact_is_missing_is_ignored_and_warned_about() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");

    let report = decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        Answer::Accepted {
            intent: "changelog".to_owned(),
            form: Form::Skill,
            artifact: ".ivar/skills/changelog/SKILL.md".to_owned(),
        },
        NOW,
    )
    .unwrap();

    assert_eq!(report.warnings[0].code, "automation.artifact_missing");
    assert_eq!(scanned(&layout, NOW).candidates.len(), 1);
}

#[test]
fn same_as_extends_a_decision_and_keeps_its_kind_and_until() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    recurring(&layout, "write release notes");
    let first = decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        deferred("docs", Some("2027-03-01")),
        NOW,
    )
    .unwrap()
    .value
    .decision;

    let report = decide_at(
        &layout,
        group_ref(&layout, "write release notes"),
        Answer::SameAs {
            decision_id: first.id,
        },
        NOW + DAY,
    )
    .unwrap();

    let view = &report.value.decision;
    assert_eq!(view.id, first.id);
    assert_eq!(view.kind, DecisionKind::Deferred);
    assert_eq!(view.until.as_deref(), Some("2027-03-01"));
    assert_eq!(view.intent, "docs");
    assert_eq!(view.text_count, 2);
    assert_eq!(report.value.texts_added, 1);
    let outcome = scanned(&layout, NOW + DAY);
    assert!(outcome.candidates.is_empty());
    assert_eq!(outcome.suppressed.len(), 1);
    assert_eq!(outcome.suppressed[0].groups, 2);
}

#[test]
fn same_as_refuses_an_accepted_or_unknown_decision() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    recurring(&layout, "write release notes");
    let accepted = decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        Answer::Accepted {
            intent: "changelog".to_owned(),
            form: Form::Agent,
            artifact: "agents/a.md".to_owned(),
        },
        NOW,
    )
    .unwrap()
    .value
    .decision;
    let notes = group_ref(&layout, "write release notes");

    let result = decide_at(
        &layout,
        notes,
        Answer::SameAs {
            decision_id: accepted.id,
        },
        NOW,
    );
    assert_eq!(code(result), "automation.same_as_accepted");
    let result = decide_at(&layout, notes, Answer::SameAs { decision_id: 99 }, NOW);
    assert_eq!(code(result), "automation.decision_not_found");
    assert_eq!(decisions_in(&layout).unwrap().value.decisions.len(), 1);
}

#[test]
fn a_ref_that_no_longer_exists_fails_with_a_clear_message() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");

    let failure = decide_at(&layout, 999, declined("x"), NOW).unwrap_err();

    assert_eq!(failure.code, "automation.group_ref_gone");
    assert!(failure.what.contains("999"));
    assert!(decisions_in(&layout).unwrap().value.decisions.is_empty());
}

#[test]
fn a_ref_that_aged_out_of_the_window_fails() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    let reference = group_ref(&layout, "update changelog");

    let result = decide_at(&layout, reference, declined("x"), NOW + 100 * DAY);

    assert_eq!(code(result), "automation.group_ref_gone");
}

#[test]
fn a_hall_without_a_memory_database_has_no_ref_and_gets_none_created() {
    let (_guard, layout) = hall();

    assert_eq!(
        code(decide_at(&layout, 1, declined("x"), NOW)),
        "automation.group_ref_gone"
    );
    assert_eq!(
        code(renew_in(&layout, 1, NOW)),
        "automation.decision_not_found"
    );
    assert_eq!(code(undo_in(&layout, 1)), "automation.decision_not_found");
    assert!(decisions_in(&layout).unwrap().value.decisions.is_empty());
    assert!(!layout.memory_db().exists());
}

#[test]
fn invalid_answers_are_refused_before_anything_is_written() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    let reference = group_ref(&layout, "update changelog");

    for answer in [
        declined(""),
        declined("two\nlines"),
        deferred("x", Some("soon")),
        deferred("x", Some("2027-02-30")),
        deferred("x", Some("2027-01-14")),
        Answer::Accepted {
            intent: "x".to_owned(),
            form: Form::Script,
            artifact: "../outside.sh".to_owned(),
        },
        Answer::Accepted {
            intent: "x".to_owned(),
            form: Form::Script,
            artifact: "/abs/a.sh".to_owned(),
        },
    ] {
        assert_eq!(
            code(decide_at(&layout, reference, answer.clone(), NOW)),
            "automation.invalid_decision",
            "{answer:?}"
        );
    }
    assert!(decisions_in(&layout).unwrap().value.decisions.is_empty());
}

#[test]
fn undo_makes_the_group_a_candidate_again() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    let id = decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        declined("changelog"),
        NOW,
    )
    .unwrap()
    .value
    .decision
    .id;

    let removed = undo_in(&layout, id).unwrap().value.removed;

    assert_eq!(removed.id, id);
    assert_eq!(scanned(&layout, NOW).candidates.len(), 1);
    assert!(decisions_in(&layout).unwrap().value.decisions.is_empty());
    let db = MemoryDb::open(layout.memory_db().as_std_path()).unwrap();
    let texts: i64 = db
        .conn()
        .query_row("SELECT count(*) FROM automation_decision_texts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(texts, 0);
    assert_eq!(code(undo_in(&layout, id)), "automation.decision_not_found");
}

#[test]
fn renew_rewrites_updated_at_of_an_accepted_decision_only() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    recurring(&layout, "rotate the keys");
    let accepted = decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        Answer::Accepted {
            intent: "changelog".to_owned(),
            form: Form::Procedure,
            artifact: "docs/changelog.md".to_owned(),
        },
        NOW,
    )
    .unwrap()
    .value
    .decision;
    let declined = decide_at(
        &layout,
        group_ref(&layout, "rotate the keys"),
        declined("keys"),
        NOW,
    )
    .unwrap()
    .value
    .decision;

    let renewed = renew_in(&layout, accepted.id, NOW + 10 * DAY)
        .unwrap()
        .value
        .decision;

    assert_eq!(renewed.created_at, accepted.created_at);
    assert_eq!(
        renewed.updated_at,
        rfc3339_from_epoch_secs((NOW + 10 * DAY) as f64)
    );
    assert_ne!(renewed.updated_at, accepted.updated_at);
    assert_eq!(
        code(renew_in(&layout, declined.id, NOW)),
        "automation.renew_not_accepted"
    );
    assert_eq!(
        code(renew_in(&layout, 99, NOW)),
        "automation.decision_not_found"
    );
}

#[test]
fn decisions_lists_every_kind_oldest_first() {
    let (_guard, layout) = hall();
    recurring(&layout, "update changelog");
    recurring(&layout, "rotate the keys");
    decide_at(
        &layout,
        group_ref(&layout, "update changelog"),
        declined("changelog"),
        NOW,
    )
    .unwrap();
    decide_at(
        &layout,
        group_ref(&layout, "rotate the keys"),
        deferred("keys", None),
        NOW,
    )
    .unwrap();

    let listed = decisions_in(&layout).unwrap().value.decisions;

    let kinds: Vec<_> = listed.iter().map(|view| (view.id, view.kind)).collect();
    assert_eq!(
        kinds,
        [(1, DecisionKind::Declined), (2, DecisionKind::Deferred)]
    );
}
