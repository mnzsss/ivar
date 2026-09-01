#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

// -- pkce_pair ----------------------------------------------------------

#[test]
fn pkce_verifier_is_43_to_128_chars() {
    let (verifier, _challenge) = pkce_pair();
    let len = verifier.0.len();
    assert!(
        (43..=128).contains(&len),
        "PKCE verifier must be 43–128 chars, got {len}"
    );
}

#[test]
fn pkce_verifier_is_base64url_without_padding() {
    let (verifier, _challenge) = pkce_pair();
    assert_eq!(
        verifier.0.len(),
        43,
        "32 random bytes -> 43 base64url chars"
    );
    for byte in verifier.0.as_bytes() {
        assert!(
            byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_',
            "verifier char {byte:02x} is not base64url-safe"
        );
    }
}

#[test]
fn pkce_challenge_is_sha256_base64url_no_pad() {
    let (verifier, challenge) = pkce_pair();
    // SHA-256 of the raw verifier bytes -> 43 base64url chars, no padding.
    assert_eq!(challenge.0.len(), 43);
    assert!(!challenge.0.contains('='), "challenge must not be padded");

    let raw = URL_SAFE_NO_PAD.decode(verifier.0.as_bytes()).unwrap();
    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(raw.as_slice()).as_slice());
    assert_eq!(
        challenge.0, expected,
        "challenge must be base64url(SHA-256(verifier))"
    );
}

// -- state --------------------------------------------------------------

#[test]
fn state_is_base64url_no_pad() {
    let s = state();
    assert_eq!(s.0.len(), 43, "32 random bytes -> 43 base64url chars");
    for byte in s.0.as_bytes() {
        assert!(
            byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_',
            "state char {byte:02x} is not base64url-safe"
        );
    }
}

// -- authorize_url ------------------------------------------------------

#[test]
fn authorize_url_has_required_params() {
    let (_, challenge) = pkce_pair();
    let st = state();
    let url = authorize_url(
        "https://www.figma.com/oauth",
        "test-client-id",
        "http://127.0.0.1:19876/callback",
        &st,
        &challenge,
        None,
        None,
    );
    assert!(url.starts_with("https://www.figma.com/oauth?"));
    assert!(
        url.contains("response_type=code"),
        "missing response_type: {url}"
    );
    assert!(
        url.contains("client_id=test-client-id"),
        "missing client_id: {url}"
    );
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A19876%2Fcallback"),
        "redirect_uri not percent-encoded: {url}"
    );
    assert!(
        url.contains("code_challenge_method=S256"),
        "missing method: {url}"
    );
    assert!(
        url.contains(&format!("state={}", st.0)),
        "state mismatch: {url}"
    );
    assert!(
        url.contains(&format!("code_challenge={}", challenge.0)),
        "challenge mismatch: {url}"
    );
}

#[test]
fn authorize_url_includes_resource_when_present() {
    let (_, challenge) = pkce_pair();
    let st = state();
    let url = authorize_url(
        "https://www.figma.com/oauth",
        "id",
        "http://127.0.0.1:19876/callback",
        &st,
        &challenge,
        Some("https://mcp.figma.com/mcp"),
        None,
    );
    assert!(
        url.contains("resource=https%3A%2F%2Fmcp.figma.com%2Fmcp"),
        "resource not encoded: {url}"
    );
}

#[test]
fn authorize_url_encodes_scope_spaces_as_pct20() {
    let (_, challenge) = pkce_pair();
    let st = state();
    let url = authorize_url(
        "https://www.figma.com/oauth",
        "id",
        "http://127.0.0.1:19876/callback",
        &st,
        &challenge,
        None,
        Some("openid profile"),
    );
    assert!(
        url.contains("scope=openid%20profile"),
        "scope not encoded: {url}"
    );
}

// -- tokens_from_json ---------------------------------------------------

#[test]
fn tokens_from_json_parses_full_response_and_computes_absolute_expiry() {
    let body =
        r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"scope":"files:read"}"#;
    let tokens = tokens_from_json(body, 1_000_000.0).unwrap();
    assert_eq!(
        tokens,
        Tokens {
            access_token: "at".to_owned(),
            refresh_token: Some("rt".to_owned()),
            expires_at: Some(1_003_600.0),
            scope: Some("files:read".to_owned()),
        }
    );
}

#[test]
fn tokens_from_json_minimal_response() {
    let body = r#"{"access_token":"minimal"}"#;
    let tokens = tokens_from_json(body, 5.0).unwrap();
    assert_eq!(
        tokens,
        Tokens {
            access_token: "minimal".to_owned(),
            refresh_token: None,
            expires_at: None,
            scope: None,
        }
    );
}

#[test]
fn tokens_from_json_fails_without_leaking_the_body() {
    // Truncated JSON that still carries a token-shaped value: the failure must
    // report a generic parse error, never echo the body.
    let body = r#"{"access_token":"should-not-leak""#;
    let err = tokens_from_json(body, 0.0).unwrap_err();
    let rendered = format!("{err:?}");
    assert!(
        !rendered.contains("should-not-leak"),
        "failure must not leak the response body: {rendered}"
    );
}

// -- redaction ----------------------------------------------------------

#[test]
fn secret_types_redact_their_value_in_debug() {
    let (verifier, challenge) = pkce_pair();
    let st = state();
    for rendered in [
        format!("{verifier:?}"),
        format!("{challenge:?}"),
        format!("{st:?}"),
    ] {
        assert!(rendered.contains("<redacted>"), "unredacted: {rendered}");
    }
    assert!(!format!("{verifier:?}").contains(&verifier.0));
    assert!(!format!("{challenge:?}").contains(&challenge.0));
    assert!(!format!("{st:?}").contains(&st.0));
}

#[test]
fn tokens_redact_their_value_in_debug() {
    let tokens = Tokens {
        access_token: "super-secret-access".to_owned(),
        refresh_token: Some("super-secret-refresh".to_owned()),
        expires_at: Some(1.0),
        scope: None,
    };
    let rendered = format!("{tokens:?}");
    assert_eq!(rendered, "Tokens(<redacted>)");
}

// -- Tokens serialization (store shape) ---------------------------------

#[test]
fn tokens_serialize_to_opencode_camel_case() {
    let tokens = Tokens {
        access_token: "at".to_owned(),
        refresh_token: Some("rt".to_owned()),
        expires_at: Some(1_000.0),
        scope: Some("files:read".to_owned()),
    };
    let value = serde_json::to_value(&tokens).unwrap();
    assert_eq!(
        value.get("accessToken").and_then(|v| v.as_str()),
        Some("at")
    );
    assert_eq!(
        value.get("refreshToken").and_then(|v| v.as_str()),
        Some("rt")
    );
    assert_eq!(
        value.get("expiresAt").and_then(|v| v.as_f64()),
        Some(1_000.0)
    );
    assert_eq!(
        value.get("scope").and_then(|v| v.as_str()),
        Some("files:read")
    );
}

#[test]
fn tokens_omit_none_fields_when_serializing() {
    let tokens = Tokens {
        access_token: "at".to_owned(),
        refresh_token: None,
        expires_at: None,
        scope: None,
    };
    let value = serde_json::to_value(&tokens).unwrap();
    assert!(value.get("refreshToken").is_none());
    assert!(value.get("expiresAt").is_none());
    assert!(value.get("scope").is_none());
}

// -- encoding -----------------------------------------------------------

#[test]
fn query_encode_uses_pct20_for_spaces() {
    assert_eq!(query_encode("a b"), "a%20b");
    assert_eq!(query_encode("http://x/y"), "http%3A%2F%2Fx%2Fy");
    assert_eq!(query_encode("a-z_A.Z~0"), "a-z_A.Z~0");
}

#[test]
fn form_encode_uses_plus_for_spaces() {
    assert_eq!(form_encode("a b"), "a+b");
    assert_eq!(form_encode("http://x/y"), "http%3A%2F%2Fx%2Fy");
}
