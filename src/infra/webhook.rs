//! Fire-and-forget JSON POSTs to a user-configured URL.

use std::time::Duration;

/// # Errors
///
/// A transport failure or a non-2xx status, as a message.
pub fn post_json(url: &str, body: &str, timeout: Duration) -> Result<(), String> {
    let status = send_json(url, body, timeout)
        .map_err(|e| e.to_string())?
        .status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{url} answered HTTP {}", status.as_u16()))
    }
}

/// POST `body` as JSON; any HTTP status is an answer, only a transport failure is an error.
///
/// # Errors
///
/// The transport failure.
pub fn send_json(
    url: &str,
    body: &str,
    timeout: Duration,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into();
    agent
        .post(url)
        .header("Content-Type", "application/json")
        .send(body)
}

#[cfg(test)]
#[path = "../../tests/unit/infra/webhook.rs"]
mod tests;
