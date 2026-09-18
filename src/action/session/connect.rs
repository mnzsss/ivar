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

use crate::action::feature::promote::{self, PromoteInput};
use crate::domain::feature::{Feature, GateState};
use crate::domain::name::{FeatureName, RepoName};
use crate::domain::session::{SessionRef, SessionState};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
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
    /// Repos the approved plan declared that this connect promoted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub promoted: Vec<RepoName>,
}

impl WriteHuman for ConnectOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        // Callers `eval` this output, so the promotion report goes out as a
        // shell comment: visible to a human, a no-op to the shell.
        if !self.promoted.is_empty() {
            let names: Vec<&str> = self.promoted.iter().map(RepoName::as_str).collect();
            writeln!(w, "# ivar: promoted {}", names.join(", "))?;
        }
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
pub fn connect(ctx: &Ctx, input: &ConnectInput) -> Outcome<ConnectOutcome> {
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

    let (promoted, promote_warnings) = match &feature {
        Some(record) => promote_declared_repos(ctx, &layout, record)?,
        None => (Vec::new(), Vec::new()),
    };
    warnings.extend(promote_warnings);
    let feature = match &session.feature {
        Some(name) if !promoted.is_empty() => Feature::read(&layout, name)?,
        _ => feature,
    };

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
            promoted,
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

/// Promote every repo the **approved** plan declares that is not promoted yet.
///
/// A repo that fails to promote is a warning, not a refused connect: a
/// half-promoted session is recoverable with `ivar feature promote`, a
/// refused connect blocks all work.
fn promote_declared_repos(
    ctx: &Ctx,
    layout: &Layout,
    feature: &Feature,
) -> Result<(Vec<RepoName>, Vec<Warning>), Failure> {
    if crate::action::plan::effective_plan_gate(layout, &feature.name)? != GateState::Approved {
        return Ok((Vec::new(), Vec::new()));
    }
    let declared = crate::action::plan::plan_declared_repos(layout, &feature.name)?;
    if declared.is_empty() {
        let warnings = if feature.promotions.is_empty() {
            vec![Warning::new(
                "connect.no_repos_declared",
                feature.name.as_str(),
                format!(
                    "the approved plan declares no `repos:` and nothing is promoted; run `ivar feature promote {} <repo>` for each repo this feature edits",
                    feature.name
                ),
            )]
        } else {
            Vec::new()
        };
        return Ok((Vec::new(), warnings));
    }

    let mut promoted = Vec::new();
    let mut warnings = Vec::new();
    for repo in declared
        .into_iter()
        .filter(|repo| !feature.is_promoted(repo))
    {
        match promote::promote(
            ctx,
            PromoteInput {
                feature: feature.name.to_string(),
                repo: repo.to_string(),
                base: None,
            },
        ) {
            Ok(report) => {
                warnings.extend(report.warnings);
                promoted.push(repo);
            }
            Err(failure) => warnings.push(Warning::new(
                "connect.promote_failed",
                repo.as_str(),
                format!(
                    "could not promote `{repo}`: {}: {}; run `ivar feature promote {} {repo}` once that is fixed",
                    failure.code, failure.what, feature.name
                ),
            )),
        }
    }
    Ok((promoted, warnings))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/connect.rs"]
mod tests;
