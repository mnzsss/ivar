//! MCP OAuth metadata discovery (RFC 9728, RFC 8414) and dynamic client
//! registration (RFC 7591).
//!
//! This is the vendor-neutral half of what `infra::figma` used to hold. A
//! server's 401 names its resource metadata; that names an authorization
//! server; that publishes the concrete endpoints. Nothing here knows which
//! server it is talking to.
//!
//! # Module boundaries
//!
//! `infra` may import [`crate::error`] and nothing else from this crate.
//! This module reaches into `ureq` — the network boundary.

use std::io::Read;

use crate::error::Failure;
use crate::infra::oauth::AuthMode;

/// A registered OAuth client, as dynamic client registration returns it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ClientInfo {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub client_secret_expires_at: Option<i64>,
    #[serde(default)]
    pub token_endpoint_auth_method: Option<String>,
}

impl ClientInfo {
    /// The token-endpoint auth mode to use for this registration.
    ///
    /// Figma echoes back the `token_endpoint_auth_method: "none"` it was sent
    /// while still issuing a `client_secret`, and its token endpoint then
    /// rejects an exchange that omits it with `Client secret is required`
    /// (measured 2026-08-26). A registration carrying a secret is a
    /// confidential client, so `none` plus a secret means `client_secret_post`.
    pub fn auth_mode(&self) -> AuthMode {
        match self.token_endpoint_auth_method.as_deref() {
            Some("client_secret_post") => AuthMode::ClientSecretPost,
            Some("client_secret_basic") => AuthMode::ClientSecretBasic,
            _ if self.client_secret.is_some() => AuthMode::ClientSecretPost,
            _ => AuthMode::None,
        }
    }
}

/// Read a response body into a String using the ureq 3.x Response API.
pub(crate) fn read_body<T: Read>(mut reader: T) -> Result<String, Failure> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf).map_err(|e| {
        Failure::failed(
            "mcp_oauth.read_body",
            format!("could not read response body: {e}"),
        )
    })?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// OAuth metadata discovery (RFC 9728 / RFC 8414)
// ---------------------------------------------------------------------------

/// What discovery found. A server that serves `initialize` without a
/// challenge is not a failure to authenticate — it is a server that needs no
/// authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryOutcome {
    Endpoints(OAuthEndpoints),
    NoAuthRequired,
}

/// Discovered OAuth authorization and token endpoints for an MCP server.
///
/// `resource` and `scopes_supported` are optional — included when the
/// server's metadata provides them, absent otherwise. No secrets or tokens
/// appear in this struct or its `Debug` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthEndpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub resource: Option<String>,
    pub scopes_supported: Option<Vec<String>>,
    /// The RFC 7591 dynamic client registration endpoint, when the
    /// authorization server advertises one.
    pub registration_endpoint: Option<String>,
}

/// Resource metadata as described by RFC 9728 (`.well-known/oauth-protected-resource`).
#[derive(serde::Deserialize)]
struct ResourceMetadata {
    authorization_servers: Vec<String>,
    #[serde(default)]
    resource: Option<String>,
}

/// Authorization server metadata as described by RFC 8414.
#[derive(serde::Deserialize)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    scopes_supported: Option<Vec<String>>,
    #[serde(default)]
    registration_endpoint: Option<String>,
}

/// Extract the `resource_metadata` URL from a `WWW-Authenticate` header
/// value. The parser supports variable parameter ordering and preserves
/// quoted URLs.
///
/// `None` when the header is absent or contains no `resource_metadata`
/// parameter.
pub(crate) fn parse_www_authenticate_resource_metadata(header: &str) -> Option<String> {
    // Split the scheme from parameters: "Bearer realm=..., resource_metadata=..."
    let params_part = header
        .trim()
        .split_once(char::is_whitespace)
        .map_or("", |(_, p)| p);

    for param in params_part.split(',') {
        let param = param.trim();
        if let Some(rest) = param.strip_prefix("resource_metadata=") {
            let value = rest.trim();
            // Strip surrounding quotes if present, preserving inner content.
            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                &value[1..value.len() - 1]
            } else {
                value
            };
            return Some(value.to_owned());
        }
    }
    None
}

/// Construct the `.well-known/oauth-authorization-server` URL for an issuer.
///
/// For an issuer at root (`https://auth.example.com`), the well-known URL is
/// `https://auth.example.com/.well-known/oauth-authorization-server`. For an
/// issuer with a path (`https://auth.example.com/issuer/v1`), the well-known
/// path is appended to the issuer's path, per RFC 8414 §3.
///
/// `None` when the issuer URL cannot be parsed.
pub(crate) fn build_well_known_url(issuer: &str) -> Result<String, Failure> {
    let base = issuer.trim_end_matches('/');
    Ok(format!("{base}/.well-known/oauth-authorization-server"))
}

/// Parse the resource metadata JSON to extract the first authorization server
/// issuer URL and optional resource identifier.
///
/// Returns `(issuer, resource)`. Fails when the `authorization_servers`
/// array is absent or empty.
pub(crate) fn parse_resource_metadata(json_str: &str) -> Result<(String, Option<String>), Failure> {
    let meta: ResourceMetadata = serde_json::from_str(json_str).map_err(|e| {
        Failure::failed(
            "mcp_oauth.resource_metadata_parse",
            format!("could not parse resource metadata: {e}"),
        )
        .expected("JSON with an authorization_servers array")
        .actual("invalid resource metadata JSON")
    })?;

    let issuer = meta
        .authorization_servers
        .into_iter()
        .next()
        .ok_or_else(|| {
            Failure::failed(
                "mcp_oauth.resource_metadata_no_authorization_server",
                "resource metadata has no authorization servers",
            )
            .expected("a non-empty authorization_servers array")
            .actual("authorization_servers is empty or absent")
        })?;

    Ok((issuer, meta.resource))
}

/// Parse authorization server metadata JSON (RFC 8414) to extract the
/// endpoints needed for the OAuth flow.
///
/// Fails when required fields (`authorization_endpoint`, `token_endpoint`)
/// are absent.
pub(crate) fn parse_authorization_metadata(json_str: &str) -> Result<OAuthEndpoints, Failure> {
    let meta: AuthorizationServerMetadata = serde_json::from_str(json_str).map_err(|e| {
        Failure::failed(
            "mcp_oauth.auth_metadata_parse",
            format!("could not parse authorization server metadata: {e}"),
        )
        .expected("JSON with authorization_endpoint and token_endpoint")
        .actual("invalid authorization server metadata JSON")
    })?;

    Ok(OAuthEndpoints {
        authorization_endpoint: meta.authorization_endpoint,
        token_endpoint: meta.token_endpoint,
        resource: None,
        scopes_supported: meta.scopes_supported,
        registration_endpoint: meta.registration_endpoint,
    })
}

/// Discover OAuth authorization and token endpoints for an MCP server.
///
/// The discovery follows the MCP OAuth flow (RFC 9728):
/// 1. GET the server URL; expect a 401 with `WWW-Authenticate` containing
///    a `resource_metadata` URL.
/// 2. GET the resource metadata; extract the first authorization server.
/// 3. GET the authorization server's `.well-known/oauth-authorization-server`
///    metadata; extract the authorization and token endpoints.
///
/// No secrets, tokens, or codes appear in errors or their `actual` fields.
pub fn discover_oauth_endpoints(server_url: &str) -> Result<DiscoveryOutcome, Failure> {
    // Step 1: POST to the server URL to trigger the 401 challenge.
    // The MCP Server expects a POST request (JSON-RPC initialize) to initiate
    // the stream/connection, and will respond with 401 + WWW-Authenticate.
    let request_body = serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "ivar",
                "version": "0.8.0"
            }
        },
        "id": 1
    }))
    .map_err(|e| {
        Failure::failed(
            "mcp_oauth.discover_server_encode",
            format!("could not encode initialize request: {e}"),
        )
    })?;

    let response = ureq::post(server_url)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Content-Type", "application/json")
        .send(request_body)
        .map_err(|e| {
            Failure::failed(
                "mcp_oauth.discover_server",
                format!("could not reach MCP server: {e}"),
            )
            .expected("the MCP server to respond")
            .actual(format!("HTTP transport error: {e}"))
        })?;

    let status = response.status();
    if status.is_success() {
        return Ok(DiscoveryOutcome::NoAuthRequired);
    }
    if status.as_u16() != 401 {
        return Err(Failure::failed(
            "mcp_oauth.discover_unexpected_status",
            format!("MCP server returned {status} instead of 401"),
        )
        .expected("HTTP 401 with WWW-Authenticate, or a 2xx needing no auth")
        .actual(format!("HTTP {status}")));
    }

    let www_auth = response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            Failure::failed(
                "mcp_oauth.discover_no_www_authenticate",
                "401 response missing WWW-Authenticate header",
            )
            .expected("a WWW-Authenticate header with resource_metadata")
        })?;

    let resource_metadata_url =
        parse_www_authenticate_resource_metadata(www_auth).ok_or_else(|| {
            Failure::failed(
                "mcp_oauth.discover_no_resource_metadata",
                "WWW-Authenticate header has no resource_metadata parameter",
            )
            .expected("resource_metadata URL in the WWW-Authenticate header")
            .actual(format!("header value: {www_auth}"))
        })?;

    // Step 2: GET the resource metadata.
    let resource_response = ureq::get(&resource_metadata_url)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| {
            Failure::failed(
                "mcp_oauth.discover_resource_metadata",
                format!("could not fetch resource metadata: {e}"),
            )
            .expected("the resource metadata endpoint to respond")
        })?;

    let resource_body = read_body(resource_response.into_body().as_reader())?;
    let (issuer, resource) = parse_resource_metadata(&resource_body)?;

    // Step 3: GET the authorization server's well-known metadata.
    let well_known_url = build_well_known_url(&issuer)?;
    let auth_response = ureq::get(&well_known_url)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| {
            Failure::failed(
                "mcp_oauth.discover_auth_metadata",
                format!("could not fetch authorization server metadata: {e}"),
            )
            .expected("the authorization server metadata endpoint to respond")
        })?;

    let auth_body = read_body(auth_response.into_body().as_reader())?;
    let mut endpoints = parse_authorization_metadata(&auth_body)?;
    endpoints.resource = resource;

    Ok(DiscoveryOutcome::Endpoints(endpoints))
}

/// Register an OAuth client at `registration_endpoint` (RFC 7591).
///
/// The endpoint is what the authorization server advertised in its RFC 8414
/// metadata — never a constant, and never a caller-supplied URL. A
/// registration is not an authentication: it only obtains a `client_id`.
pub fn register_client(
    registration_endpoint: &str,
    redirect_uri: &str,
    client_name: &str,
) -> Result<ClientInfo, Failure> {
    let body = serde_json::json!({
        "client_name": client_name,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let body = serde_json::to_string(&body).map_err(|e| {
        Failure::failed(
            "mcp_oauth.register_client_encode",
            format!("could not encode registration request: {e}"),
        )
    })?;

    let response = ureq::post(registration_endpoint)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| {
            Failure::failed(
                "mcp_oauth.register_client",
                format!("could not register OAuth client: {e}"),
            )
            .expected("the registration endpoint to accept the request")
            .actual(format!("HTTP error: {e}"))
        })?;

    let status = response.status();
    let body = read_body(response.into_body().as_reader())?;
    if !status.is_success() {
        return Err(Failure::failed(
            "mcp_oauth.register_client_http",
            format!("the registration endpoint returned {status}"),
        )
        .expected("HTTP 2xx")
        .actual(body));
    }
    parse_registration_response(&body)
}

/// Parse an RFC 7591 registration response. Pure: no network, so the
/// public/confidential decision is unit-testable offline.
pub(crate) fn parse_registration_response(body: &str) -> Result<ClientInfo, Failure> {
    serde_json::from_str(body).map_err(|e| {
        Failure::failed(
            "mcp_oauth.register_client_parse",
            format!("could not parse registration response: {e}"),
        )
        .expected("a JSON object with client_id")
        .actual(body.to_owned())
    })
}

#[cfg(test)]
#[path = "../../tests/unit/infra/mcp_oauth.rs"]
mod tests;
