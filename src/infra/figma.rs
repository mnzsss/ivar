//! Figma's MCP dynamic client registration and OAuth metadata discovery.
//!
//! # Why this exists
//!
//! Figma's MCP server does not accept OpenCode's default dynamic client
//! registration request — the `client_name` it sends is not on Figma's
//! allowlist, and the flow fails before a browser ever opens. Figma's docs
//! carry a workaround: register a client with one specific `client_name`
//! ahead of time. That is [`register_client`] — a single request, run once
//! per machine, before the harness's own auth command owns the terminal.
//!
//! When Ivar performs the OAuth flow itself (OpenCode + Figma), it also
//! needs Figma's authorization and token endpoints. These are discovered
//! dynamically via the MCP server's OAuth resource metadata (RFC 9728):
//! the server's 401 `WWW-Authenticate` header points to resource metadata,
//! which points to an authorization server, whose `.well-known` metadata
//! provides the concrete endpoints. [`discover_oauth_endpoints`] chains
//! these steps.
//!
//! # Which hosts need this
//!
//! Today exactly one: `mcp.figma.com`. [`needs_preregistration`] is the
//! lookup the caller uses to decide whether to bother; it and
//! [`CLIENT_NAME`] are the whole allowlist entry. When Figma's allowlist
//! changes — expected, per Figma's own docs — this is the one file that
//! changes with it (`R-CONTAINED`).
//!
//! # Module boundaries
//!
//! `infra` may import [`crate::error`] and nothing else from this crate.
//! This module reaches into `ureq` — the network boundary. It holds no
//! domain knowledge: the server name being registered and the provider
//! doing the registering both stay in the caller.
//!
//! # Rejected: caching the registration here
//!
//! `register_client` always performs the network call; it does not check
//! whether a client already exists. That check needs a place to look
//! (OpenCode's `mcp-auth.json`), and `infra` cannot know about that file
//! without importing something above it. `R-IDEMPOTENT` is the caller's
//! job, not this module's.

use std::io::Read;

use crate::error::Failure;

/// The one host that needs a pre-registered client today.
const PREREGISTRATION_HOSTS: &[&str] = &["mcp.figma.com"];

/// The `client_name` Figma's allowlist accepts.
///
/// Unverified beyond "it worked when this was written" — Figma owns the
/// list and can change it without notice. A 403 from [`register_client`]
/// means this name fell off the list.
pub const CLIENT_NAME: &str = "Codex";

/// Figma's dynamic client registration endpoint.
const REGISTER_URL: &str = "https://api.figma.com/v1/oauth/mcp/register";

/// Whether `host` is one that needs a pre-registered client before dynamic
/// registration will succeed.
#[must_use]
pub fn needs_preregistration(host: &str) -> bool {
    PREREGISTRATION_HOSTS.contains(&host)
}

/// A registered OAuth client, as Figma's registration endpoint returns it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ClientInfo {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub client_secret_expires_at: Option<i64>,
}

/// Register an OAuth client for `redirect_uri` with Figma's MCP server.
///
/// A registration is not an authentication (`R-HONEST`) — it only clears
/// the allowlist obstacle so the harness's own auth command can proceed.
///
/// A non-2xx response is a [`Failure`] carrying the response body. A 403
/// specifically means Figma's `client_name` allowlist changed — this
/// module's `CLIENT_NAME` no longer matches, not a mistake on the caller's
/// part.
pub fn register_client(redirect_uri: &str) -> Result<ClientInfo, Failure> {
    register_client_as(redirect_uri, CLIENT_NAME)
}

/// [`register_client`], parameterised on `client_name` so the 403 branch is
/// reachable from a test without waiting for Figma's allowlist to change —
/// `"opencode"` (the name OpenCode's own dynamic registration sends) is
/// confirmed to 403 against this endpoint, while [`CLIENT_NAME`] 200s.
fn register_client_as(redirect_uri: &str, client_name: &str) -> Result<ClientInfo, Failure> {
    let body = serde_json::json!({
        "client_name": client_name,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });

    let body = serde_json::to_string(&body).map_err(|e| {
        Failure::failed(
            "figma.register_client_encode",
            format!("could not encode registration request: {e}"),
        )
    })?;

    // `http_status_as_error(false)`: ureq 3.x defaults to turning any non-2xx
    // status into `Err(ureq::Error::StatusCode(_))` before we ever see the
    // response, which would make the 403-means-allowlist-changed branch
    // below dead code. Disabling it keeps the response — status *and*
    // body — in hand so `registration_failure` can read both.
    let response = ureq::post(REGISTER_URL)
        .config()
        .http_status_as_error(false)
        .build()
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| {
            Failure::failed(
                "figma.register_client",
                format!("could not register OAuth client: {e}"),
            )
            .expected("Figma's registration endpoint to accept the request")
            .actual(format!("HTTP error: {e}"))
        })?;

    let status = response.status();
    let body = read_body(response.into_body().as_reader())?;

    if !status.is_success() {
        return Err(registration_failure(status.as_u16(), body));
    }

    serde_json::from_str(&body).map_err(|e| {
        Failure::failed(
            "figma.register_client_parse",
            format!("could not parse registration response: {e}"),
        )
        .expected("a JSON object with client_id")
        .actual(body)
    })
}

/// Build the [`Failure`] for a non-2xx registration response. A 403 gets a
/// `fix` that names the allowlist rather than blaming the caller; every
/// other status gets a generic failure carrying the body.
fn registration_failure(status: u16, body: String) -> Failure {
    let failure = Failure::failed(
        "figma.register_client_http",
        format!("Figma's registration endpoint returned {status}"),
    )
    .expected("HTTP 2xx")
    .actual(body);

    if status == 403 {
        failure.fix(crate::error::FixAction::unsafe_(
            "figma.allowlist_changed",
            format!(
                "Figma's client_name allowlist changed — \"{CLIENT_NAME}\" is no longer accepted. Update infra::figma::CLIENT_NAME to the name Figma's docs now list."
            ),
        ))
    } else {
        failure
    }
}

/// Read a response body into a String using the ureq 3.x Response API.
fn read_body<T: Read>(mut reader: T) -> Result<String, Failure> {
    let mut buf = String::new();
    reader.read_to_string(&mut buf).map_err(|e| {
        Failure::failed(
            "figma.read_body",
            format!("could not read response body: {e}"),
        )
    })?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// OAuth metadata discovery (RFC 9728 / RFC 8414)
// ---------------------------------------------------------------------------

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
            "figma.resource_metadata_parse",
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
                "figma.resource_metadata_no_authorization_server",
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
            "figma.auth_metadata_parse",
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
pub fn discover_oauth_endpoints(server_url: &str) -> Result<OAuthEndpoints, Failure> {
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
            "figma.discover_server_encode",
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
                "figma.discover_server",
                format!("could not reach MCP server: {e}"),
            )
            .expected("the MCP server to respond")
            .actual(format!("HTTP transport error: {e}"))
        })?;

    let status = response.status();
    if status.as_u16() != 401 {
        return Err(Failure::failed(
            "figma.discover_unexpected_status",
            format!("MCP server returned {status} instead of 401"),
        )
        .expected("HTTP 401 with WWW-Authenticate")
        .actual(format!("HTTP {status}")));
    }

    let www_auth = response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            Failure::failed(
                "figma.discover_no_www_authenticate",
                "401 response missing WWW-Authenticate header",
            )
            .expected("a WWW-Authenticate header with resource_metadata")
        })?;

    let resource_metadata_url =
        parse_www_authenticate_resource_metadata(www_auth).ok_or_else(|| {
            Failure::failed(
                "figma.discover_no_resource_metadata",
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
                "figma.discover_resource_metadata",
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
                "figma.discover_auth_metadata",
                format!("could not fetch authorization server metadata: {e}"),
            )
            .expected("the authorization server metadata endpoint to respond")
        })?;

    let auth_body = read_body(auth_response.into_body().as_reader())?;
    let mut endpoints = parse_authorization_metadata(&auth_body)?;
    endpoints.resource = resource;

    Ok(endpoints)
}

#[cfg(test)]
#[path = "../../tests/unit/infra/figma.rs"]
mod tests;
