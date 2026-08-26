//! Figma's MCP dynamic client registration.
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

#[cfg(test)]
#[path = "../../tests/unit/infra/figma.rs"]
mod tests;
