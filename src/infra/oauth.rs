//! OAuth 2.0 primitives for Ivar's own authorization-code + PKCE flow.
//!
//! Ivar performs this exchange itself for Figma on OpenCode (`R-FIGMA-FLOW`)
//! instead of delegating to `opencode mcp auth`. This module is the
//! network-adjacent half: PKCE, OAuth `state`, the authorize-URL builder, and
//! the code-for-token exchange. It holds no provider or domain knowledge — the
//! server name, client id, and redirect URI all arrive as arguments.
//!
//! # Module boundary
//!
//! `infra` may import [`crate::error`] and external crates only. This module
//! uses `ureq` (blocking, like [`crate::infra::figma`]), `sha2` for PKCE `S256`,
//! and `base64` for the base64url encoding both PKCE and `state` need.
//!
//! # Secrets
//!
//! `CodeVerifier`, `CodeChallenge`, `State`, and [`Tokens`] never derive a
//! `Debug` that prints their value. A `Failure` produced here never carries an
//! authorization code, PKCE verifier, or token into its `actual` — those are
//! redacted or replaced with a generic category.

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::error::Failure;

/// A PKCE code verifier. The value is redacted from `Debug`.
#[derive(Clone)]
pub struct CodeVerifier(pub String);

impl std::fmt::Debug for CodeVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeVerifier(<redacted>)")
    }
}

/// A PKCE code challenge (`base64url(SHA-256(verifier))`, no padding). Redacted
/// from `Debug`.
#[derive(Clone)]
pub struct CodeChallenge(pub String);

impl std::fmt::Debug for CodeChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeChallenge(<redacted>)")
    }
}

/// An OAuth `state` value. Redacted from `Debug`.
#[derive(Clone)]
pub struct State(pub String);

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("State(<redacted>)")
    }
}

/// Tokens returned by a token exchange, in the shape OpenCode's
/// `mcp-auth.json` store expects (`accessToken`, `refreshToken?`, `expiresAt?`,
/// `scope?`). `expiresAt` is absolute Unix seconds. Redacted from `Debug`.
#[derive(Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tokens(<redacted>)")
    }
}

/// The raw token-endpoint response, snake_case as the server sends it. Private —
/// the public shape is [`Tokens`] with absolute `expiresAt`.
#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
}

/// 32 bytes of OS-entropy from `uuid::Uuid::new_v4()` (two UUIDs, 16 bytes
/// each). `uuid` is already a dependency with the `v4` feature and its RNG is
/// the platform CSPRNG.
fn random_bytes_32() -> [u8; 32] {
    let mut out = [0u8; 32];
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    out[..16].copy_from_slice(a.as_bytes());
    out[16..].copy_from_slice(b.as_bytes());
    out
}

/// Generate a PKCE verifier (43 chars of base64url-encoded 32 random bytes)
/// and its `S256` challenge (base64url of the SHA-256 digest, no padding).
pub fn pkce_pair() -> (CodeVerifier, CodeChallenge) {
    let verifier_bytes = random_bytes_32();
    let verifier = CodeVerifier(URL_SAFE_NO_PAD.encode(verifier_bytes.as_slice()));
    (verifier.clone(), challenge_from_verifier(&verifier))
}

/// Compute the PKCE challenge for a given verifier string, per RFC 7636 §4.2:
/// `code_challenge = BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))`.
pub fn challenge_from_verifier(verifier: &CodeVerifier) -> CodeChallenge {
    let hash = Sha256::digest(verifier.0.as_bytes());
    CodeChallenge(URL_SAFE_NO_PAD.encode(hash.as_slice()))
}

/// Generate a random OAuth `state` value (43 chars of base64url-encoded 32
/// random bytes, no padding).
pub fn state() -> State {
    State(URL_SAFE_NO_PAD.encode(random_bytes_32().as_slice()))
}

/// Build the `authorize` URL for the authorization-code flow.
///
/// `resource` and `scope` are included only when `Some` — the OAuth server may
/// require either (RFC 8707 resource indicator).
#[allow(clippy::too_many_arguments)]
pub fn authorize_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &State,
    code_challenge: &CodeChallenge,
    resource: Option<&str>,
    scope: Option<&str>,
) -> String {
    let mut params: Vec<(&str, String)> = vec![
        ("response_type", "code".to_owned()),
        ("client_id", client_id.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("state", state.0.clone()),
        ("code_challenge", code_challenge.0.clone()),
        ("code_challenge_method", "S256".to_owned()),
    ];
    if let Some(resource) = resource {
        params.push(("resource", resource.to_owned()));
    }
    if let Some(scope) = scope {
        params.push(("scope", scope.to_owned()));
    }

    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", query_encode(key), query_encode(value)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{authorization_endpoint}?{query}")
}

/// Token endpoint authentication method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    None,
    ClientSecretPost,
    ClientSecretBasic,
}

impl std::fmt::Display for AuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthMode::None => write!(f, "none"),
            AuthMode::ClientSecretPost => write!(f, "client_secret_post"),
            AuthMode::ClientSecretBasic => write!(f, "client_secret_basic"),
        }
    }
}

/// Exchange an authorization code for tokens at `token_endpoint`, using PKCE.
///
/// `client_secret` is `Some` only when `auth_mode` is `ClientSecretPost` or
/// `ClientSecretBasic`.
/// `resource` is included only when `Some` (RFC 8707).
#[allow(clippy::too_many_arguments)]
pub fn exchange_code(
    token_endpoint: &str,
    authorization_code: &str,
    redirect_uri: &str,
    code_verifier: &CodeVerifier,
    client_id: &str,
    client_secret: Option<&str>,
    auth_mode: AuthMode,
    resource: Option<&str>,
) -> Result<Tokens, Failure> {
    let mut params = vec![
        ("grant_type", "authorization_code".to_owned()),
        ("code", authorization_code.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("code_verifier", code_verifier.0.clone()),
        ("client_id", client_id.to_owned()),
    ];

    if let Some(resource) = resource {
        params.push(("resource", resource.to_owned()));
    }

    if let Some(secret) = client_secret
        && auth_mode == AuthMode::ClientSecretPost
    {
        params.push(("client_secret", secret.to_owned()));
    }

    let form = params
        .iter()
        .map(|(key, value)| format!("{}={}", key, form_encode(value)))
        .collect::<Vec<_>>()
        .join("&");

    // `http_status_as_error(false)` maintains control over the response for
    // better diagnostic reporting.
    let mut request = ureq::post(token_endpoint)
        .config()
        .http_status_as_error(false)
        .build();

    if auth_mode == AuthMode::ClientSecretBasic
        && let Some(secret) = client_secret
    {
        let auth = format!("{}:{}", client_id, secret);
        let b64 = base64::engine::general_purpose::STANDARD.encode(auth);
        request = request.header("Authorization", &format!("Basic {}", b64));
    }

    let response = request
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form)
        .map_err(|e| {
            Failure::failed(
                "oauth.exchange_code",
                format!("token exchange request failed: {e}"),
            )
            .expected("the OAuth token endpoint to respond")
            .actual(format!("HTTP transport error: {e}"))
        })?;

    let status = response.status();
    let content_type = response
        .headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = read_body(response.into_body().as_reader())?;

    if !status.is_success() {
        let summary = summarize_error_body(&body);
        let field_names = params
            .iter()
            .map(|(k, _)| *k)
            .collect::<Vec<_>>()
            .join(", ");

        let mut oauth_err = format!("token endpoint returned {status}: {}", summary.category);
        if let Some(detail) = summary.detail {
            oauth_err.push_str(&format!(", {detail}"));
        }

        return Err(Failure::failed("oauth.exchange_code_http", oauth_err)
            .expected("HTTP 2xx")
            .actual(format!(
                "HTTP {status}, endpoint: {token_endpoint}, content-type: {}, body-len: {}, \
             oauth-error: {}, auth-mode: {auth_mode}, fields: [{field_names}], body-keys: [{}]",
                classify_content_type(&content_type),
                body.len(),
                summary.category,
                summary.keys
            )));
    }

    tokens_from_json(&body, now_unix())
}

#[derive(Debug)]
struct ErrorSummary {
    category: String,
    keys: String,
    detail: Option<String>,
}

fn summarize_error_body(body: &str) -> ErrorSummary {
    let json: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let keys = if let serde_json::Value::Object(map) = &json {
        map.keys().take(10).cloned().collect::<Vec<_>>().join(", ")
    } else {
        "non-object".to_owned()
    };

    let Some(obj) = json.as_object() else {
        return ErrorSummary {
            category: "unknown_error".to_owned(),
            keys,
            detail: None,
        };
    };

    let mut category = "unknown_error".to_owned();
    for key in &["error", "reason", "message", "err"] {
        if let Some(serde_json::Value::String(s)) = obj.get(*key)
            && [
                "invalid_request",
                "invalid_client",
                "invalid_grant",
                "unauthorized_client",
                "unsupported_grant_type",
                "invalid_scope",
            ]
            .contains(&s.as_str())
        {
            category = s.clone();
            break;
        }
    }

    let mut details = Vec::new();
    for key in &["error", "reason", "message", "err"] {
        if let Some(serde_json::Value::String(s)) = obj.get(*key) {
            let sanitized = s.replace(|c: char| c.is_control() || c == '\n' || c == '\r', " ");
            let truncated = if sanitized.len() > 200 {
                format!("{}…", &sanitized[..200])
            } else {
                sanitized
            };
            details.push(format!("{key}=\"{truncated}\""));
        }
    }

    let detail = if details.is_empty() {
        None
    } else {
        Some(details.join(", "))
    };

    ErrorSummary {
        category,
        keys,
        detail,
    }
}

fn classify_content_type(ct: &str) -> &'static str {
    if ct.contains("application/json") {
        "application/json"
    } else if ct.contains("application/x-www-form-urlencoded") {
        "application/x-www-form-urlencoded"
    } else if ct.contains("text/plain") {
        "text/plain"
    } else if ct.contains("text/html") {
        "text/html"
    } else {
        "other"
    }
}

/// Parse a token-endpoint JSON body into [`Tokens`], turning the server's
/// relative `expires_in` seconds into an absolute `expiresAt` using `now_unix`.
/// Pure and testable without a network round-trip.
fn tokens_from_json(body: &str, now_unix: f64) -> Result<Tokens, Failure> {
    let raw: TokenResponse = serde_json::from_str(body).map_err(|e| {
        Failure::failed(
            "oauth.exchange_code_parse",
            format!("could not parse token response: {e}"),
        )
        .expected("a JSON object with an access_token")
        .actual("invalid JSON token response".to_owned())
    })?;

    let expires_at = raw.expires_in.map(|seconds| now_unix + seconds as f64);
    Ok(Tokens {
        access_token: raw.access_token,
        refresh_token: raw.refresh_token,
        expires_at,
        scope: raw.scope,
    })
}

/// Current Unix time as seconds (`f64`), for `expiresAt`.
fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// Read a response body into a `String`, matching [`crate::infra::figma`].
fn read_body<T: Read>(mut reader: T) -> Result<String, Failure> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf).map_err(|e| {
        Failure::failed(
            "oauth.read_body",
            format!("could not read token response body: {e}"),
        )
    })?;
    Ok(buf)
}

/// Percent-encode a query-string component per RFC 3986: unreserved characters
/// stay, space becomes `%20`, everything else is `%XX`.
fn query_encode(s: &str) -> String {
    encode_component(s, false)
}

/// Percent-encode a form body component per `application/x-www-form-urlencoded`:
/// unreserved characters stay, space becomes `+`, everything else is `%XX`.
fn form_encode(s: &str) -> String {
    encode_component(s, true)
}

fn encode_component(s: &str, space_as_plus: bool) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' if space_as_plus => out.push('+'),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
#[path = "../../tests/unit/infra/oauth.rs"]
mod tests;
