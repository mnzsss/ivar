#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::convert::Infallible;

use super::*;

fn print(id: i64, session: &str, day: &str, text: &str) -> Fingerprint {
    Fingerprint {
        id,
        observation_id: id * 10,
        session_id: session.to_owned(),
        feature: String::new(),
        day: day.to_owned(),
        normalized: text.to_owned(),
    }
}

/// Offers every text as a possible match, so the grouping alone decides.
fn all_texts(
    prints: &[Fingerprint],
) -> impl FnMut(&BTreeSet<String>) -> Result<Vec<String>, Infallible> + use<> {
    let texts: Vec<String> = prints.iter().map(|f| f.normalized.clone()).collect();
    move |_| Ok(texts.clone())
}

fn grouped(prints: Vec<Fingerprint>) -> Vec<Group> {
    let similar = all_texts(&prints);
    group(prints, similar).unwrap()
}

fn set(words: &[&str]) -> BTreeSet<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

#[test]
fn tokens_are_words_and_whole_placeholders() {
    assert_eq!(
        tokens("fix <path> tests, then <id> <n> <token> v2"),
        set(&[
            "fix", "<path>", "tests", "then", "<id>", "<n>", "<token>", "v2"
        ])
    );
}

#[test]
fn a_placeholder_is_not_the_word_it_spells() {
    assert_ne!(tokens("<path>"), tokens("path"));
    assert_eq!(tokens("<unknown> thing"), set(&["unknown", "thing"]));
}

#[test]
fn tokens_of_nothing_are_empty() {
    assert!(tokens("").is_empty());
    assert!(tokens(" - ? ").is_empty());
}

#[test]
fn jaccard_is_intersection_over_union() {
    assert!(
        (jaccard(&set(&["a", "b", "c"]), &set(&["a", "b", "c", "d", "e"])) - 0.6).abs()
            < f64::EPSILON
    );
    assert!((jaccard(&set(&["a"]), &set(&["a"])) - 1.0).abs() < f64::EPSILON);
    assert!(jaccard(&set(&["a"]), &set(&["b"])).abs() < f64::EPSILON);
    assert!(jaccard(&set(&[]), &set(&[])).abs() < f64::EPSILON);
}

#[test]
fn identical_texts_form_one_group() {
    let groups = grouped(vec![
        print(1, "s1", "2026-01-01", "release notes"),
        print(2, "s2", "2026-01-02", "release notes"),
    ]);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].members().len(), 2);
}

#[test]
fn texts_at_the_similarity_threshold_join() {
    let groups = grouped(vec![
        print(1, "s1", "2026-01-01", "alpha beta gamma"),
        print(2, "s2", "2026-01-02", "alpha beta gamma delta epsilon"),
    ]);

    assert_eq!(groups.len(), 1, "3 of 5 tokens is exactly 0.6");
}

#[test]
fn texts_below_the_threshold_stay_apart() {
    let groups = grouped(vec![
        print(1, "s1", "2026-01-01", "alpha beta gamma"),
        print(2, "s2", "2026-01-02", "alpha beta gamma delta epsilon zeta"),
        print(3, "s3", "2026-01-03", "write release notes"),
    ]);

    assert_eq!(
        groups.len(),
        3,
        "3 of 6 is 0.5, and the third shares nothing"
    );
}

#[test]
fn similarity_chains_by_single_linkage() {
    // a~b and b~c, but a and c share too little to be similar themselves.
    let a = "one two three four five";
    let b = "one two three four five six seven";
    let c = "three four five six seven eight";
    assert!(jaccard(&tokens(a), &tokens(c)) < MIN_SIMILARITY);

    let groups = grouped(vec![
        print(1, "s1", "2026-01-01", a),
        print(2, "s2", "2026-01-02", b),
        print(3, "s3", "2026-01-03", c),
    ]);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].members().len(), 3);
}

#[test]
fn a_fingerprint_with_no_text_is_left_out() {
    let groups = grouped(vec![
        print(1, "s1", "2026-01-01", ""),
        print(2, "s2", "2026-01-02", ""),
    ]);

    assert!(groups.is_empty());
}

#[test]
fn a_text_the_lookup_never_offers_cannot_join() {
    let prints = vec![
        print(1, "s1", "2026-01-01", "alpha beta"),
        print(2, "s2", "2026-01-02", "alpha beta gamma"),
    ];

    let groups = group(prints, |_| {
        Ok::<_, Infallible>(vec!["not stored".to_owned()])
    })
    .unwrap();

    assert_eq!(groups.len(), 2);
}

#[test]
fn the_lookup_error_is_returned() {
    let prints = vec![print(1, "s1", "2026-01-01", "alpha")];

    assert_eq!(
        group(prints, |_| Err::<Vec<String>, _>("boom")),
        Err("boom")
    );
}

fn candidate_of(rows: &[(&str, &str)]) -> bool {
    let prints = rows
        .iter()
        .zip(1..)
        .map(|((session, day), id)| print(id, session, day, "same request"))
        .collect();
    let groups = grouped(prints);
    assert_eq!(groups.len(), 1);
    groups[0].is_candidate()
}

#[test]
fn three_sessions_over_two_days_is_a_candidate() {
    assert!(candidate_of(&[
        ("s1", "2026-01-01"),
        ("s2", "2026-01-01"),
        ("s3", "2026-01-02"),
    ]));
}

#[test]
fn three_sessions_on_one_day_is_not() {
    assert!(!candidate_of(&[
        ("s1", "2026-01-01"),
        ("s2", "2026-01-01"),
        ("s3", "2026-01-01"),
    ]));
}

#[test]
fn two_sessions_is_not() {
    assert!(!candidate_of(&[("s1", "2026-01-01"), ("s2", "2026-01-02")]));
}

#[test]
fn repeats_inside_one_session_count_once() {
    assert!(!candidate_of(&[
        ("s1", "2026-01-01"),
        ("s1", "2026-01-02"),
        ("s1", "2026-01-03"),
        ("s2", "2026-01-04"),
    ]));
}

#[test]
fn a_prompt_outside_any_session_is_not_a_session_or_a_day() {
    assert!(!candidate_of(&[
        ("s1", "2026-01-01"),
        ("s2", "2026-01-01"),
        ("", "2026-01-02"),
        ("", "2026-01-03"),
    ]));
}

#[test]
fn the_group_is_described_by_its_members() {
    let mut recent = print(5, "s3", "2026-01-02", "release notes v2");
    recent.feature = "checkout".to_owned();
    let mut old = print(1, "s1", "2026-01-01", "release notes");
    old.feature = "billing".to_owned();
    let same = print(3, "s2", "2026-01-01", "release notes");
    let group = Group::new(vec![old, recent, same]);

    assert_eq!(group.latest().unwrap().id, 5);
    assert_eq!(
        group.members().iter().map(|f| f.id).collect::<Vec<_>>(),
        [5, 3, 1]
    );
    assert_eq!(group.sessions(), BTreeSet::from(["s1", "s2", "s3"]));
    assert_eq!(group.days(), BTreeSet::from(["2026-01-01", "2026-01-02"]));
    assert_eq!(group.features(), BTreeSet::from(["billing", "checkout"]));
    assert_eq!(
        group.samples(MAX_SAMPLES),
        ["release notes v2", "release notes"]
    );
    assert_eq!(group.samples(1), ["release notes v2"]);
}

#[test]
fn recency_is_the_day_and_then_the_id() {
    let later_day = print(1, "s", "2026-01-02", "x");
    let later_id = print(9, "s", "2026-01-01", "x");

    assert!(later_day.recency() > later_id.recency());
}
