#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn equal_hashes_are_approved() {
    assert_eq!(state(Some("abc"), "abc"), ApprovalState::Approved);
    assert!(state(Some("abc"), "abc").is_approved());
    assert_eq!(state(Some("abc"), "abc").reason(), None);
}

#[test]
fn a_different_hash_is_unreviewed_because_it_changed() {
    let found = state(Some("abc"), "abd");

    assert_eq!(
        found,
        ApprovalState::Unreviewed(UnreviewedReason::ChangedSinceApproval)
    );
    assert_eq!(found.as_str(), "unreviewed");
    assert_eq!(found.reason().unwrap().as_str(), "changed since approval");
}

#[test]
fn no_record_is_unreviewed_for_lack_of_approval() {
    let found = state(None, "abc");

    assert_eq!(
        found,
        ApprovalState::Unreviewed(UnreviewedReason::NoApproval)
    );
    assert_eq!(found.reason().unwrap().as_str(), "no approval");
}

fn files(source: &str) -> Vec<(usize, String)> {
    secret_file_references(source)
        .into_iter()
        .map(|r| (r.line, r.file))
        .collect()
}

#[test]
fn env_files_are_found_with_their_line() {
    let source = "#!/bin/bash\nset -a\nsource .env\ncat \"$HOME/.ivar/secrets/agents.env\" | wc\nload_dotenv('.env.local')\nsource ../prod.env\ncd . && . ./.envrc\n";

    assert_eq!(
        files(source),
        vec![
            (3, ".env".to_owned()),
            (4, "agents.env".to_owned()),
            (5, ".env.local".to_owned()),
            (6, "prod.env".to_owned()),
            (7, ".envrc".to_owned()),
        ]
    );
}

#[test]
fn ordinary_words_that_contain_env_are_not_secret_files() {
    let source = "echo $ENVIRONMENT\nos.getenv('HOME')\nprocess.env.FOO\nenvsubst < a.tmpl\nvenv/bin/python\n";

    assert!(files(source).is_empty(), "{:?}", files(source));
}

#[test]
fn example_env_files_are_not_secrets() {
    assert!(files("cp .env.example .env.sample\ncat .env.template\n").is_empty());
}

#[test]
fn a_file_name_is_matched_case_insensitively_and_reported_as_written() {
    assert_eq!(files("source .ENV\n"), vec![(1, ".ENV".to_owned())]);
}
