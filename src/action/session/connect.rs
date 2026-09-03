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
use crate::domain::session::{SessionRef, SessionState};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::proc;
use crate::providers;
use crate::store::layout::Layout;

use super::super::{discover_hall, read_manifest};
use super::lookup;
use super::start;
use super::view;
use crate::action::Ctx;

/// What `ivar session connect` needs. At least one of the two must be given.
#[derive(Debug, Clone)]
pub struct ConnectInput {
    /// The session id, or a unique prefix of one.
    pub session_id: Option<String>,
    /// Narrow the search to sessions bound to this feature.
    pub feature: Option<String>,
    /// Attach or create: with a `--feature` and no session id, take the
    /// feature's most recent session that no harness is running in, and start
    /// a fresh detached one when every candidate is busy or none exist.
    ///
    /// This is what makes `/ivar-connect <feature>` a single command with no
    /// dead end — without it, `connect` never creates and a missing session is
    /// a `Blocked` failure.
    pub create: bool,
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

    let (session, mut warnings) = match attach_or_create(ctx, &layout, &input)? {
        Some(report) => (report.value, report.warnings),
        None => (
            lookup::resolve(
                &layout,
                input.session_id.as_deref(),
                input.feature.as_deref(),
            )?,
            Vec::new(),
        ),
    };

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

    warnings.extend(materialise_report.warnings);
    Ok(Report::with_warnings(
        ConnectOutcome {
            session_id: session.id.to_string(),
            feature: session.feature.clone(),
            view_dir: session.view_dir.clone(),
        },
        warnings,
    ))
}

/// The `--create` path: the feature's most recent **free** session, or a fresh
/// detached one.
///
/// `None` means this is an ordinary lookup — `--create` was not asked for, or a
/// session id was given, which names one session exactly and leaves nothing to
/// choose.
///
/// "Free" is decided by the session's own harness binary, not by any process
/// at all: whenever an agent runs this, `ivar` and its shell are themselves
/// sitting inside a View Dir, so a process-agnostic check would report the
/// caller's own session as busy. A session whose record is unreadable is not a
/// candidate — `session prune` owns those.
fn attach_or_create(
    ctx: &Ctx,
    layout: &Layout,
    input: &ConnectInput,
) -> Result<Option<Report<SessionRef>>, Failure> {
    if !input.create || input.session_id.is_some() {
        return Ok(None);
    }
    let Some(feature) = input.feature.as_deref() else {
        return Ok(None);
    };
    let name = FeatureName::new(feature)?;

    for session in lookup::by_recency(layout, &name)? {
        let Some(state) = session.state.as_ref() else {
            continue;
        };
        let binary = providers::launch_contract(state.provider()).binary;
        if !proc::is_program_running_in(&session.view_dir, binary) {
            return Ok(Some(Report::new(session)));
        }
    }

    // Every candidate is busy, or there are none. Detached: the caller is
    // already an agent — it wants the View Dir and the bindings, not a second
    // provider launched underneath it.
    let started = start::start(
        ctx,
        start::StartInput {
            feature: Some(feature.to_owned()),
            resume: false,
            provider: None,
            detached: true,
            relay: false,
        },
    )?;
    let warnings = started.warnings;
    let session = lookup::resolve(layout, Some(&started.value.session_id), Some(feature))?;
    Ok(Some(Report::with_warnings(session, warnings)))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/connect.rs"]
mod tests;
