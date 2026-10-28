use super::*;

fn terms(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

#[test]
fn words_split_camel_snake_and_kebab_case() {
    assert_eq!(words_of("duplicateNode"), ["duplicate", "node"]);
    assert_eq!(words_of("HTTPServer"), ["http", "server"]);
    assert_eq!(
        words_of("reply_comment-queue"),
        ["reply", "comment", "queue"]
    );
}

#[test]
fn plurals_fold_to_one_stem() {
    assert_eq!(stem("changesets"), stem("changeset"));
    assert_eq!(stem("entries"), stem("entry"));
    assert_eq!(stem("class"), "class");
    assert_eq!(stem("status"), "status");
}

#[test]
fn generic_words_and_extensions_are_not_counted() {
    let query = QueryWords::from_terms(&terms(&["how", "does", "reply-comment.ts", "work"]));
    assert_eq!(query.words, ["reply", "comment"]);
}

#[test]
fn a_query_of_only_generic_words_keeps_them() {
    let query = QueryWords::from_terms(&terms(&["how", "it", "works"]));
    assert!(!query.is_empty());
}

#[test]
fn half_the_words_is_a_match_and_a_third_is_not() {
    let query = QueryWords::from_terms(&terms(&["alpha", "beta", "gamma"]));
    assert!(!query.is_covered(&query.covered_by("alpha")));
    assert!(query.is_covered(&query.covered_by("alpha gamma")));
    let short = QueryWords::from_terms(&terms(&["alpha", "beta"]));
    assert!(short.is_covered(&short.covered_by("beta")));
}

#[test]
fn natural_language_has_no_path_and_no_identifier() {
    assert!(is_natural_language(
        &terms(&["changeset", "required"]),
        false
    ));
    assert!(!is_natural_language(
        &terms(&["changeset", "useRealtime"]),
        false
    ));
    assert!(!is_natural_language(&terms(&["changeset"]), true));
}

#[test]
fn docs_workflows_and_config_are_recognised() {
    assert!(is_doc_or_config_path(".github/workflows/ci.yml"));
    assert!(is_doc_or_config_path("docs/guide/setup.md"));
    assert!(is_doc_or_config_path("package.json"));
    assert!(!is_doc_or_config_path("src/session/guard.rs"));
}

#[test]
fn digits_are_words_of_their_own() {
    assert_eq!(words_of("session0"), ["session", "0"]);
    assert_eq!(words_of("Base64Encode"), ["base", "64", "encode"]);
}

#[test]
fn words_far_apart_in_a_long_file_are_not_covered_together() {
    let query = QueryWords::from_terms(&terms(&["alpha", "beta", "gamma"]));
    let filler = "unrelated line\n".repeat(CONTENT_WINDOW_LINES * 2);
    let spread = format!("alpha\n{filler}beta\n{filler}gamma\n");
    assert_eq!(query.covered_together(&spread).len(), 1);
    assert!(!query.is_covered(&query.covered_together(&spread)));

    let together = format!("{filler}alpha beta\nunrelated\ngamma\n{filler}");
    assert_eq!(query.covered_together(&together).len(), 3);
}

#[test]
fn a_short_file_is_one_window() {
    let query = QueryWords::from_terms(&terms(&["alpha", "beta"]));
    assert_eq!(query.covered_together("alpha\n\n\nbeta").len(), 2);
    assert!(query.covered_together("").is_empty());
}
