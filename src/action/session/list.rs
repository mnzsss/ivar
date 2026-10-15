//! `ivar session list` — every live session in the hall, read-only.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::domain::session::{SessionRef, SessionState};
use crate::error::{Outcome, Report, Warning, WriteHuman};

use super::super::discover_hall;
use super::lookup;

/// Where a session's View Dir lives: the hall's session tree or a feature's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Discovery,
    Feature,
}

impl SessionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Feature => "feature",
        }
    }
}

/// One live session as `session list` reports it. Field names are a
/// contract with external clients: add fields, never rename or remove.
#[derive(Debug, Clone, Serialize)]
pub struct ListedSession {
    pub id: SessionId,
    pub kind: SessionKind,
    pub feature: Option<FeatureName>,
    pub provider: Option<Provider>,
    pub started_at: Option<String>,
    pub feature_bound_at: Option<String>,
    pub view_dir: Utf8PathBuf,
}

impl From<SessionRef> for ListedSession {
    fn from(session: SessionRef) -> Self {
        let state = session.state.as_ref();
        Self {
            kind: if session.feature.is_some() {
                SessionKind::Feature
            } else {
                SessionKind::Discovery
            },
            provider: state.map(SessionState::provider),
            started_at: state.map(|state| state.started_at().to_owned()),
            feature_bound_at: state.and_then(|state| state.feature_bound_at().map(str::to_owned)),
            id: session.id,
            feature: session.feature,
            view_dir: session.view_dir,
        }
    }
}

/// What `ivar session list` found.
#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    pub sessions: Vec<ListedSession>,
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.sessions.is_empty() {
            return writeln!(w, "No live sessions.");
        }
        let mut table =
            crate::infra::table::new(&["SESSION", "KIND", "FEATURE", "PROVIDER", "STARTED"]);
        for session in &self.sessions {
            table.add_row(vec![
                session.id.to_string(),
                session.kind.as_str().to_owned(),
                session
                    .feature
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), ToString::to_string),
                session
                    .provider
                    .map_or_else(|| "-".to_owned(), |provider| provider.to_string()),
                session.started_at.clone().unwrap_or_else(|| "-".to_owned()),
            ]);
        }
        crate::infra::table::write(w, &table)
    }
}

/// List every live session, oldest first. Reads the session trees and each
/// `state.json`; writes nothing.
///
/// # Errors
///
/// When no hall is found, or a session tree cannot be read.
pub fn list(ctx: &Ctx) -> Outcome<ListOutcome> {
    let layout = discover_hall(ctx)?;
    let mut sessions: Vec<ListedSession> = lookup::list_all(&layout)?
        .into_iter()
        .map(ListedSession::from)
        .collect();
    sessions.sort_by(|a, b| (&a.started_at, &a.id).cmp(&(&b.started_at, &b.id)));
    let warnings = sessions
        .iter()
        .filter(|session| session.provider.is_none())
        .map(|session| {
            Warning::new(
                "session.state_unreadable",
                session.id.to_string(),
                "no readable state.json; `ivar session prune` removes it",
            )
        })
        .collect();
    Ok(Report::with_warnings(ListOutcome { sessions }, warnings))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/list.rs"]
mod tests;
