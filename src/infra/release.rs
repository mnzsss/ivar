//! Release source seam.
//!
//! Provides access to the latest release URL and location for update checks.
//! Resolving the latest release via GitHub's redirect avoids GitHub API rate
//! limits and excludes drafts and prereleases.

use std::time::Duration;

use crate::error::Failure;

pub const LATEST_RELEASE_URL: &str = "https://github.com/mnzsss/ivar/releases/latest";

pub trait LatestRelease {
    /// # Errors
    ///
    /// Returns [`Failure`] on any network error, a non-redirect status, or a
    /// redirect without a `Location` header.
    fn latest_location(&self, timeout: Duration) -> Result<String, Failure>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GithubRelease;

impl LatestRelease for GithubRelease {
    fn latest_location(&self, timeout: Duration) -> Result<String, Failure> {
        let response = ureq::get(LATEST_RELEASE_URL)
            .config()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build()
            .header("User-Agent", concat!("ivar/", env!("CARGO_PKG_VERSION")))
            .call()
            .map_err(|e| {
                Failure::failed(
                    "release.request_failed",
                    format!("could not reach GitHub: {e}"),
                )
            })?;
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok());
        location_from(response.status().as_u16(), location)
    }
}

fn location_from(status: u16, location: Option<&str>) -> Result<String, Failure> {
    if !(300..400).contains(&status) {
        return Err(Failure::failed(
            "release.no_redirect",
            format!("expected a redirect from {LATEST_RELEASE_URL}, got HTTP {status}"),
        ));
    }
    location.map(str::to_owned).ok_or_else(|| {
        Failure::failed(
            "release.no_location",
            format!("{LATEST_RELEASE_URL} redirected without a Location header"),
        )
    })
}

#[cfg(test)]
#[path = "../../tests/unit/infra/release.rs"]
mod tests;
