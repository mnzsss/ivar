//! Types for sessions and how they bind to features.
//!
//! A **Session** is live while its **View Dir** exists — liveness is a
//! filesystem fact, not a process one (see `store::layout`). What this module
//! holds is the session's *record*: which provider launched it, when, and
//! whether it is bound to a feature.
//!
//! # Discovery vs feature sessions
//!
//! A session starts as a **discovery session** (`feature: None`): every repo
//! is symlinked to its read-only default-branch worktree. Conversion binds it
//! to a feature exactly once — one-way, irreversible — and the View Dir moves
//! from `.ivar/sessions/<id>/` to `.ivar/features/<feature>/sessions/<id>/`.
//!
//! # What lives here
//!
//! [`SessionState`] — the record written to `state.json` inside the View Dir
//! (`provider`, `started_at`, optional feature binding). [`SessionRef`] — a
//! session located on disk: its id, where it lives (which implies its
//! feature), and its state. All pure, no I/O: reading and writing `state.json`
//! is `store::session`'s job, discovering sessions on disk is
//! `action::session::lookup`'s.

use serde::{Deserialize, Serialize};

use super::name::{FeatureName, SessionId};
use super::provider::Provider;

/// The schema version of `state.json`, stamped by `store::session`.
const CURRENT_VERSION: u32 = 1;

/// One session's record: how it was launched and what it is bound to.
///
/// Written to `state.json` inside the session's View Dir — which is what lets
/// `session convert` preserve `provider` and `started_at` for free: the file
/// moves with the directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionState {
    version: u32,
    /// The feature this session is bound to. `None` = a discovery session.
    pub feature: Option<FeatureName>,
    /// The provider that launched (or will launch) the session's agent.
    pub provider: Provider,
    /// When the session started, as an RFC 3339 timestamp. Fixed-width and
    /// zero-padded, so two timestamps sort lexically exactly as they sort
    /// chronologically — `lookup::most_recent` relies on that.
    pub started_at: String,
    /// When the session was bound to its feature — set by conversion, and by
    /// a session started directly on a feature. `None` for a discovery
    /// session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature_bound_at: Option<String>,
}

impl SessionState {
    /// A fresh discovery-session record: no feature, launched by `provider`
    /// at `started_at`.
    #[must_use]
    pub fn new(provider: Provider, started_at: impl Into<String>) -> Self {
        Self {
            version: CURRENT_VERSION,
            feature: None,
            provider,
            started_at: started_at.into(),
            feature_bound_at: None,
        }
    }

    /// Bind this session to `feature`. Idempotent: re-binding an already-bound
    /// session is a no-op, so an interrupted conversion resumes cleanly.
    pub fn bind(&mut self, feature: FeatureName, bound_at: impl Into<String>) {
        if self.feature.is_none() {
            self.feature = Some(feature);
            self.feature_bound_at = Some(bound_at.into());
        }
    }

    /// Whether this is a discovery session (no feature bound).
    #[must_use]
    pub fn is_discovery(&self) -> bool {
        self.feature.is_none()
    }

    /// The provider that launched this session.
    #[must_use]
    pub fn provider(&self) -> Provider {
        self.provider
    }

    /// When the session started.
    #[must_use]
    pub fn started_at(&self) -> &str {
        &self.started_at
    }

    /// The bound feature, if any.
    #[must_use]
    pub fn feature(&self) -> Option<&FeatureName> {
        self.feature.as_ref()
    }

    /// When the session was bound to its feature, if it is bound.
    #[must_use]
    pub fn feature_bound_at(&self) -> Option<&str> {
        self.feature_bound_at.as_deref()
    }

    /// The schema version — always [`CURRENT_VERSION`] for a value built
    /// through [`Self::new`] or read by `store::session`.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }
}

/// A session located on disk: its id, where it lives, and its state.
///
/// `feature` comes from the session's **location** — a View Dir under
/// `.features/<f>/sessions/` is a feature session, one under `.sessions/` is
/// a discovery session. Location is authoritative over whatever `state` says,
/// which can be absent (a View Dir that predates `state.json`) or stale (a
/// crash between the View Dir move and the state write during conversion).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRef {
    /// The session id — the View Dir's name.
    pub id: SessionId,
    /// The feature implied by the View Dir's location.
    pub feature: Option<FeatureName>,
    /// The View Dir itself — where this session lives on disk.
    pub view_dir: camino::Utf8PathBuf,
    /// The session's record, when one exists.
    pub state: Option<SessionState>,
}

/// The current time as an RFC 3339 UTC timestamp with nanosecond precision,
/// e.g. `2026-08-07T12:34:56.789012345Z`.
///
/// Fixed-width, zero-padded fields make two such timestamps compare
/// lexicographically exactly as they compare chronologically — which is why
/// `lookup::most_recent` sorts sessions by their `started_at` strings without
/// parsing them. Written here (rather than pulling in `chrono`) because it is
/// the one timestamp the crate needs, and the civil-from-days algorithm is
/// small and standard.
#[must_use]
pub fn rfc3339_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let nanos = now.subsec_nanos();

    let days = secs.div_euclid(86_400);
    let seconds_of_day = secs.rem_euclid(86_400);
    let (hours, minutes, seconds) = (
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60,
    );
    let (year, month, day) = civil_from_days(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{nanos:09}Z")
}

/// Days since 1970-01-01 to a civil (year, month, day), in UTC.
///
/// Howard Hinnant's `civil_from_days` algorithm — the standard, well-tested
/// way to do this without a date library.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::domain::name::FeatureName;

    fn discovery_state() -> SessionState {
        SessionState::new(Provider::ClaudeCode, "2026-01-01T00:00:00.000000000Z")
    }

    #[test]
    fn new_creates_a_discovery_record() {
        let state = discovery_state();
        assert!(state.is_discovery());
        assert_eq!(state.provider(), Provider::ClaudeCode);
        assert_eq!(state.started_at(), "2026-01-01T00:00:00.000000000Z");
        assert_eq!(state.feature(), None);
        assert_eq!(state.feature_bound_at(), None);
        assert_eq!(state.version(), 1);
    }

    #[test]
    fn bind_attaches_the_feature_once_and_is_idempotent() {
        let mut state = discovery_state();
        let feature = FeatureName::new("checkout").unwrap();

        state.bind(feature.clone(), "2026-02-02T00:00:00.000000000Z");
        state.bind(feature.clone(), "2026-03-03T00:00:00.000000000Z");

        assert_eq!(state.feature(), Some(&feature));
        assert_eq!(
            state.feature_bound_at(),
            Some("2026-02-02T00:00:00.000000000Z")
        );
        assert!(!state.is_discovery());
    }

    #[test]
    fn session_state_round_trips_through_serde_without_unknown_fields() {
        let mut state = discovery_state();
        state.bind(
            FeatureName::new("checkout").unwrap(),
            "2026-02-02T00:00:00.000000000Z",
        );

        let rendered = serde_json::to_string(&state).unwrap();
        let parsed: SessionState = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed, state);
    }

    #[test]
    fn an_unknown_field_in_session_state_is_refused() {
        let raw = r#"{"version":1,"feature":null,"provider":"claude-code","started_at":"2026-01-01T00:00:00.000000000Z","bogus":true}"#;
        assert!(serde_json::from_str::<SessionState>(raw).is_err());
    }

    #[test]
    fn rfc3339_now_is_fixed_width_and_zero_padded() {
        let now = rfc3339_now();
        assert_eq!(now.len(), 30, "was: {now}");
        assert!(now.ends_with('Z'), "was: {now}");
        assert!(now.starts_with("20"), "was: {now}");
    }

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // 2000-03-01: 30 years from 1970 with seven leap days in between, then
        // Jan (31) + Feb (29, 2000 is a leap year).
        assert_eq!(civil_from_days(30 * 365 + 7 + 31 + 29), (2000, 3, 1));
    }
}
