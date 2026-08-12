#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn read_parses_the_protocol_until_the_blank_line() {
    let input = b"protocol=https\nhost=github.com\nusername=oauth2\n\n";
    let cred = Credential::read(&input[..]).unwrap();

    assert_eq!(cred.protocol, "https");
    assert_eq!(cred.host, "github.com");
    assert_eq!(cred.username, "oauth2");
    assert!(cred.password.is_empty());
}

#[test]
fn read_ignores_unknown_keys_and_stops_at_the_separator() {
    let input = b"protocol=https\nhost=github.com\nunknown=whatever\n\n";
    let cred = Credential::read(&input[..]).unwrap();

    assert_eq!(cred.host, "github.com");
    // Everything after the blank line is a fresh request, not this one.
    let second = b"protocol=http\nhost=example.com\n\n";
    let parsed = Credential::read(&second[..]).unwrap();
    assert_eq!(parsed.protocol, "http");
    assert_eq!(parsed.host, "example.com");
}

#[test]
fn write_emits_the_protocol_format_with_trailing_blank_line() {
    let cred = Credential {
        protocol: "https".to_owned(),
        host: "github.com".to_owned(),
        username: "x-access-token".to_owned(),
        password: "ghp_secret".to_owned(),
        ..Credential::default()
    };

    let mut out = Vec::new();
    cred.write(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    assert!(text.contains("protocol=https\n"));
    assert!(text.contains("host=github.com\n"));
    assert!(text.contains("username=x-access-token\n"));
    assert!(text.contains("password=ghp_secret\n"));
    assert!(
        text.ends_with("\n\n"),
        "protocol output ends with a blank line"
    );
}

#[test]
fn approved_credential_ends_with_a_blank_line() {
    let cred = Credential {
        protocol: "https".to_owned(),
        host: "github.com".to_owned(),
        approval: Approval::Approved,
        ..Credential::default()
    };

    let mut out = Vec::new();
    cred.write(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.ends_with("\n\n"), "approved ends with the separator");
}

// -- the operation argument ---------------------------------------------------
//
// git never invokes a credential helper bare: it appends the operation it
// wants as the final argument (`get`, `store`, `erase`). A helper that does
// not accept one fails on every invocation git makes of it.

#[test]
fn the_three_operations_git_names_are_understood() {
    assert!(matches!(Operation::from_arg(Some("get")), Operation::Get));
    assert!(matches!(
        Operation::from_arg(Some("store")),
        Operation::Store
    ));
    assert!(matches!(
        Operation::from_arg(Some("erase")),
        Operation::Erase
    ));
}

/// gitcredentials(7): a helper that is handed an operation it does not
/// implement must ignore the request, not fail. Anything else turns a future
/// git release into an error on every push.
#[test]
fn an_unknown_operation_is_ignored_rather_than_refused() {
    assert!(matches!(
        Operation::from_arg(Some("capability")),
        Operation::Unknown
    ));
    assert!(matches!(Operation::from_arg(None), Operation::Get));
}

#[test]
fn store_consumes_the_request_and_answers_nothing() {
    let input = b"protocol=https\nhost=github.com\nusername=x\npassword=secret\n\n";
    let mut out = Vec::new();

    respond(Operation::Store, &input[..], &mut out, || {
        panic!("store must not reach for a token")
    })
    .unwrap();

    assert!(
        out.is_empty(),
        "a store answers nothing; git ignores output here: {out:?}"
    );
}

#[test]
fn erase_answers_nothing() {
    let input = b"protocol=https\nhost=github.com\n\n";
    let mut out = Vec::new();

    respond(Operation::Erase, &input[..], &mut out, || {
        panic!("erase must not reach for a token")
    })
    .unwrap();

    assert!(out.is_empty());
}

#[test]
fn get_answers_with_the_token_from_the_cascade() {
    let input = b"protocol=https\nhost=github.com\n\n";
    let mut out = Vec::new();

    respond(Operation::Get, &input[..], &mut out, || {
        Some("ghp_token".to_owned())
    })
    .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("username=x-access-token\n"), "was: {text}");
    assert!(text.contains("password=ghp_token\n"), "was: {text}");
}

/// git reads an empty `username=`/`password=` as *set*, and stops asking the
/// remaining helpers. Answering nothing is what keeps a user's own helper
/// (`gh`, a keychain) reachable when ivar has no token of its own.
#[test]
fn a_get_with_no_token_answers_nothing_so_the_next_helper_is_asked() {
    let input = b"protocol=https\nhost=github.com\n\n";
    let mut out = Vec::new();

    respond(Operation::Get, &input[..], &mut out, || None).unwrap();

    assert!(
        out.is_empty(),
        "an empty answer would shadow the next helper: {out:?}"
    );
}

#[test]
fn a_get_for_a_host_that_is_not_github_answers_nothing() {
    let input = b"protocol=https\nhost=gitlab.com\n\n";
    let mut out = Vec::new();

    respond(Operation::Get, &input[..], &mut out, || {
        panic!("a non-GitHub host must not reach for a GitHub token")
    })
    .unwrap();

    assert!(out.is_empty());
}

/// The response carries what this helper *adds*, not an echo of the request
/// with blanks in it: git takes `path=` at face value and writes it into the
/// filled credential.
#[test]
fn the_answer_omits_the_fields_it_has_no_value_for() {
    let input = b"protocol=https\nhost=github.com\n\n";
    let mut out = Vec::new();

    respond(Operation::Get, &input[..], &mut out, || {
        Some("ghp_token".to_owned())
    })
    .unwrap();

    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("port="), "empty port emitted: {text}");
    assert!(!text.contains("path="), "empty path emitted: {text}");
}
