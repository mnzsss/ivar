#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

// -- summarize_error_body -----------------------------------------------

#[test]
fn error_detail_includes_error_and_message_strings() {
    let body = r#"{"error":"forbidden","message":"not allowed"}"#;
    let summary = summarize_error_body(body);
    assert_eq!(
        summary.detail,
        Some("error=\"forbidden\", message=\"not allowed\"".to_owned())
    );
}

#[test]
fn error_detail_omits_non_allowlisted_keys() {
    let body = r#"{"error":"e","i18n":"val","message":"m","status":400}"#;
    let summary = summarize_error_body(body);
    assert_eq!(
        summary.detail,
        Some("error=\"e\", message=\"m\"".to_owned())
    );
}

#[test]
fn error_detail_truncates_long_values() {
    let long_error = "a".repeat(250);
    let body = format!(r#"{{"error":"{long_error}"}}"#);
    let summary = summarize_error_body(&body);
    let expected = format!("error=\"{}…\"", "a".repeat(200));
    assert_eq!(summary.detail, Some(expected));
}

#[test]
fn error_detail_strips_newlines_and_control_chars() {
    let body = r#"{"error":"line\nend\u0000"}"#;
    let summary = summarize_error_body(body);
    assert_eq!(summary.detail, Some("error=\"line end \"".to_owned()));
}

#[test]
fn error_detail_absent_when_no_string_fields() {
    let body = r#"{"status":400}"#;
    let summary = summarize_error_body(body);
    assert_eq!(summary.detail, None);
}

#[test]
fn error_detail_ignores_non_string_values() {
    let body = r#"{"error":123,"message":["a"]}"#;
    let summary = summarize_error_body(body);
    assert_eq!(summary.detail, None);
}

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
    // SHA-256 of the verifier string -> 43 base64url chars, no padding.
    assert_eq!(challenge.0.len(), 43);
    assert!(!challenge.0.contains('='), "challenge must not be padded");

    let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.0.as_bytes()).as_slice());
    assert_eq!(
        challenge.0, expected,
        "challenge must be base64url(SHA-256(verifier))"
    );
}

#[test]
fn pkce_challenge_matches_rfc7636_appendix_b_vector() {
    let verifier = CodeVerifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned());
    let expected_challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    let challenge = challenge_from_verifier(&verifier);
    assert_eq!(challenge.0, expected_challenge);
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

#[test]
fn exchange_code_captures_oauth_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut reader = BufReader::new(stream.try_clone().unwrap());

        // Read headers
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if line.to_ascii_lowercase().starts_with("content-length:") {
                content_length = line.split(':').nth(1).unwrap().trim().parse().unwrap();
            }
        }

        // Read body
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();

        // Return a 400 error
        let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n\r\n{\"error\":\"invalid_grant\"}";
        stream.write_all(response.as_bytes()).unwrap();
        stream.shutdown(std::net::Shutdown::Both).unwrap();
    });

    let token_endpoint = format!("http://127.0.0.1:{port}");
    let verifier = CodeVerifier("v".to_owned());
    let err = exchange_code(
        &token_endpoint,
        "code",
        "uri",
        &verifier,
        "id",
        Some("secret"),
        AuthMode::ClientSecretPost,
        None,
    )
    .unwrap_err();

    assert!(err.actual.unwrap().contains("invalid_grant"));
}

#[test]
fn token_exchange_includes_resource_when_present() {
    exchange_code_with_resource(Some("my-resource"), |body| {
        assert!(body.contains("resource=my-resource"));
    });
}

#[test]
fn token_exchange_omits_resource_when_absent() {
    exchange_code_with_resource(None, |body| {
        assert!(!body.contains("resource="));
    });
}

#[test]
fn error_category_read_from_message_field() {
    let body = r#"{"error":"non-oauth-error","message":"invalid_grant"}"#;
    let summary = summarize_error_body(body);
    assert_eq!(summary.category, "invalid_grant");
}

#[test]
fn error_category_prefers_first_allowlisted_match() {
    let body = r#"{"reason":"invalid_client","message":"invalid_grant"}"#;
    let summary = summarize_error_body(body);
    assert_eq!(summary.category, "invalid_client");
}

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

#[test]
fn exchange_code_request_structure_none() {
    exchange_code_with_mode(AuthMode::None, |is_post, _, _, has_secret| {
        assert!(is_post);
        assert!(!has_secret);
    });
}

#[test]
fn exchange_code_request_structure_post() {
    exchange_code_with_mode(AuthMode::ClientSecretPost, |is_post, _, _, has_secret| {
        assert!(is_post);
        assert!(has_secret);
    });
}

#[test]
fn exchange_code_request_structure_basic() {
    exchange_code_with_mode(AuthMode::ClientSecretBasic, |is_post, has_basic, _, _| {
        assert!(is_post);
        assert!(has_basic);
    });
}

fn exchange_code_with_mode<F>(mode: AuthMode, assertion: F)
where
    F: FnOnce(bool, bool, bool, bool) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        let mut content_length = 0;

        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if line.to_ascii_lowercase().starts_with("content-length:") {
                content_length = line.split(':').nth(1).unwrap().trim().parse().unwrap();
            }
            request.push_str(&line);
        }

        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        request.push_str(&String::from_utf8_lossy(&body));

        let is_post = request.starts_with("POST");
        let has_basic_auth = request.to_lowercase().contains("authorization: basic ");
        let has_client_id = request.contains("client_id=");
        let has_client_secret = request.contains("client_secret=");

        let response =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"access_token\":\"at\"}";
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.shutdown(std::net::Shutdown::Both);

        assertion(is_post, has_basic_auth, has_client_id, has_client_secret);
    });

    let token_endpoint = format!("http://127.0.0.1:{port}");
    let verifier = CodeVerifier("v".to_owned());
    let _ = exchange_code(
        &token_endpoint,
        "code",
        "uri",
        &verifier,
        "id",
        Some("secret"),
        mode,
        None,
    )
    .unwrap();
    handle.join().unwrap();
}

fn exchange_code_with_resource<F>(resource: Option<&str>, assertion: F)
where
    F: FnOnce(&str) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut content_length = 0;

        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if line.to_ascii_lowercase().starts_with("content-length:") {
                content_length = line.split(':').nth(1).unwrap().trim().parse().unwrap();
            }
        }

        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assertion(&body_str);

        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"access_token\":\"at\"}").unwrap();
    });

    let ep = format!("http://127.0.0.1:{port}");
    let verifier = CodeVerifier("v".to_owned());
    exchange_code(
        &ep,
        "c",
        "uri",
        &verifier,
        "id",
        None,
        AuthMode::None,
        resource,
    )
    .unwrap();
    handle.join().unwrap();
}
