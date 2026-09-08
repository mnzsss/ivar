#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::memory::sanitizer::sanitize_text;

#[test]
fn sanitizer_redacts_private_keys_and_tokens() {
    let raw = "export GITHUB_TOKEN=ghp_ABC1234567890abcdefghijklmnopqrstuvwx";
    let sanitized = sanitize_text(raw);
    assert!(!sanitized.as_str().contains("ghp_ABC1234567890abcdefghijklmnopqrstuvwx"));
    assert!(sanitized.as_str().contains("[REDACTED]"));
}

#[test]
fn sanitizer_redacts_various_secret_patterns() {
    let github_pat = "github_pat_11ABCD123_4567890abcdefghijklmnopqrstuvwxyz1234567890";
    assert_eq!(sanitize_text(github_pat).as_str(), "[REDACTED]");

    let slack_token = "xoxb-123456789012-1234567890123-abcdefghijklmnopqrstuvwx";
    assert_eq!(sanitize_text(slack_token).as_str(), "[REDACTED]");

    let aws_access_key = "AKIA1234567890ABCDEF";
    assert_eq!(sanitize_text(aws_access_key).as_str(), "[REDACTED]");

    let rsa_key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----";
    assert_eq!(sanitize_text(rsa_key).as_str(), "[REDACTED]");

    let openssh_key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA...\n-----END OPENSSH PRIVATE KEY-----";
    assert_eq!(sanitize_text(openssh_key).as_str(), "[REDACTED]");

    let pgp_key = "-----BEGIN PGP PRIVATE KEY BLOCK-----\nVersion: ...\n-----END PGP PRIVATE KEY BLOCK-----";
    assert_eq!(sanitize_text(pgp_key).as_str(), "[REDACTED]");
}
