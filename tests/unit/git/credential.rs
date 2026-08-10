#![allow(clippy::unwrap_used, clippy::expect_used)]

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
