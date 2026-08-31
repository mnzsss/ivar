//! `ivar session connect` — re-bind to an existing live session.
//!
//! Valhalla's **Connect**: locate a live session by id-prefix and/or feature,
//! re-materialise its View Dir to match the feature's current promotion state
//! (idempotent — a no-op when nothing drifted, but it repairs symlinks and
//! read-only guards left stale), and emit the session binding
//! (`IVAR_SESSION_ID`, `IVAR_FEATURE`, `IVAR_SESSION_PATH`). Used to resume
//! after the agent restarted or a new conversation began.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::domain::session::SessionState;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};

use super::super::{discover_hall, read_manifest};
use super::lookup;
use super::view;
use crate::action::Ctx;

/// What `ivar session connect` needs. At least one of the two must be given.
#[derive(Debug, Clone)]
pub struct ConnectInput {
    /// The session id, or a unique prefix of one.
    pub session_id: Option<String>,
    /// Narrow the search to sessions bound to this feature.
    pub feature: Option<String>,
}

/// The session binding `connect` emits — the env-var contract of
/// ARCHITECTURE.md, as data.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectOutcome {
    /// The bound session's id.
    pub session_id: String,
    /// The feature the session is bound to, if it is a feature session.
    pub feature: Option<FeatureName>,
    /// The session's (re-materialised) view dir.
    pub view_dir: Utf8PathBuf,
}

impl WriteHuman for ConnectOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "export IVAR_SESSION_ID={}", self.session_id)?;
        if let Some(feature) = &self.feature {
            writeln!(w, "export IVAR_FEATURE={feature}")?;
        }
        writeln!(w, "export IVAR_SESSION_PATH={}", self.view_dir)?;
        Ok(())
    }
}

/// Re-bind to a live session: locate it, re-materialise its view dir, and
/// return the binding. Nothing is created — a session that never existed is a
/// `Blocked` failure, and an ambiguous prefix is a `Blocked` failure naming
/// the candidates.
pub fn connect(ctx: &Ctx, input: ConnectInput) -> Outcome<ConnectOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    let session = lookup::resolve(
        &layout,
        input.session_id.as_deref(),
        input.feature.as_deref(),
    )?;

    // The feature to materialise against, if the session is feature-bound.
    // A feature session whose feature record is gone cannot be re-materialised
    // into anything — name the way back.
    let feature = match &session.feature {
        Some(name) => Some(Feature::read(&layout, name)?.ok_or_else(|| {
            Failure::blocked(
                "feature.not_found",
                format!("feature `{name}` does not exist"),
            )
            .expected("the feature this session is bound to")
            .actual(format!(
                "`{name}` has no feature.json, but a session lives under its tree"
            ))
            .fix(FixAction::safe(
                "feature.recreate",
                format!("Recreate the feature with `ivar feature create {name}`."),
            ))
        })?),
        None => None,
    };

    // Re-binding an unrestricted session to a successful partial state would
    // hand it a locked promotion; refuse before the view is re-materialised.
    if let Some(feature) = &feature {
        crate::action::feature::ensure_unrestricted_session_allowed(&layout, feature)?;
    }

    // Re-materialise: repair drifted symlinks, the read-only guards, the
    // projected plan and the bootstrap instructions. A no-op when nothing
    // drifted. The provider is the session's own (its record's, or the hall's
    // default for a legacy session that predates session records) — a session
    // opened under OpenCode is re-materialised as an OpenCode session, never
    // as the hall's default provider.
    let provider = session
        .state
        .as_ref()
        .map(SessionState::provider)
        .unwrap_or_else(|| manifest.providers().default_provider());
    let materialise_report = view::materialise(
        &layout,
        &manifest,
        feature.as_ref(),
        provider,
        &session.view_dir,
    )?;

    Ok(Report::with_warnings(
        ConnectOutcome {
            session_id: session.id.to_string(),
            feature: session.feature.clone(),
            view_dir: session.view_dir.clone(),
        },
        materialise_report.warnings,
    ))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/connect.rs"]
mod tests;
