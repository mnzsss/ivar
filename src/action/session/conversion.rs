//! `ivar session convert` — one-way conversion of a discovery session into a
//! feature session.
//!
//! Valhalla's **Session Conversion**: bind a discovery session (no feature)
//! to an existing feature, moving its View Dir from `.sessions/<id>/` to
//! `.features/<feature>/sessions/<id>/` and rebuilding the symlinks for the
//! target feature. The Session ID, provider, and original `started_at` are
//! preserved — the session's `state.json` moves with the directory.
//!
//! # The transition state
//!
//! Conversion is not atomic, so an interrupted run must not leave the session
//! in an ambiguous half-moved condition. Before any step runs, a `.converting`
//! marker is written under the destination feature's directory
//! (`.features/<feature>/.converting`) naming the session, its source path,
//! and the feature. On retry, the marker is detected first and the conversion
//! is resumed: every step is idempotent and re-derived from disk, so "resume
//! from the last completed step" falls out of re-running them.
//!
//! Once converted, a session can never revert to discovery: a second convert
//! of the same session is refused.

use std::io;

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::session::{SessionState, rfc3339_now};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::infra::{fs, json};
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::super::{discover_hall, read_manifest};
use super::lookup;
use super::view;
use crate::action::Ctx;

/// What `ivar session convert` needs.
#[derive(Debug, Clone)]
pub struct ConvertInput {
    /// The discovery session's id, or a unique prefix of one.
    ///
    /// No feature is named: conversion promotes the name the session
    /// already has, found through the discovery doc that lists this
    /// session (ADR-0002, D9). Renaming at promotion would orphan a name
    /// already written into branches, commits, and issues.
    pub session_id: String,
}

/// What `ivar session convert` did.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertOutcome {
    /// The session's id — unchanged by conversion.
    pub session_id: String,
    /// The feature the session is now bound to.
    pub feature: FeatureName,
    /// The session's new view dir, under the feature's session tree.
    pub view_dir: Utf8PathBuf,
}

impl WriteHuman for ConvertOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Converted session `{}` to feature `{}`. View dir: {}",
            self.session_id, self.feature, self.view_dir
        )
    }
}

/// The marker file's name, under the destination feature's directory.
const CONVERTING_FILE: &str = ".converting";

/// The transition record of an in-flight conversion.
///
/// `step` is not used to drive resume — disk state is the truth, and every
/// step below is idempotent — but it documents how far the interrupted run
/// got, which is what a human reading `.converting` wants to see.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transition {
    /// The session being converted.
    session_id: SessionId,
    /// The View Dir's path before the move — where resume looks for the
    /// session first (it may already be gone if the move completed).
    source: Utf8PathBuf,
    /// The feature the session is being bound to.
    feature: FeatureName,
    /// How far the interrupted run got.
    step: Step,
}

/// Which phase an in-flight conversion is in. Diagnostic only — see
/// [`Transition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    /// About to move the View Dir.
    MoveSession,
    /// About to bind the session state.
    UpdateState,
    /// About to re-materialise the View Dir for the target feature.
    Rematerialize,
}

/// Convert a discovery session into a feature session.
///
/// `Blocked` when the session is not a discovery session (already converted),
/// when the destination feature does not exist, or when the session cannot be
/// located. `Failed` when a step breaks mid-flight — the `.converting` marker
/// then lets the next attempt resume.
pub fn convert(ctx: &Ctx, input: ConvertInput) -> Outcome<ConvertOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;

    // 1. An interrupted conversion? Resume it first, before anything reads
    //    the session's location — a half-moved session already sits under
    //    its destination feature, so every check below would misread it as
    //    already bound. The marker wins, as it did when the feature was an
    //    argument; the only change is that the name is now found by
    //    scanning for the marker rather than taken from the caller.
    if let Some((feature_name, transition)) = find_pending_transition(&layout, &input.session_id)? {
        let outcome = resume(&layout, &manifest, &feature_name, transition)?;
        return mark_discovery_converted(ctx, &feature_name, outcome);
    }

    // 2. Locate the session and verify it is a discovery session.
    let session = lookup::resolve(&layout, Some(&input.session_id), None)?;
    let state = session.state.as_ref().ok_or_else(|| {
        Failure::blocked(
            "session.state_missing",
            format!("session `{}` has no session record", session.id),
        )
        .expected("a session with a `state.json` in its view dir")
        .actual("the view dir exists but no state.json does")
        .fix(FixAction::safe(
            "session.start_fresh",
            "Start a fresh session instead — conversion needs the session's record.",
        ))
    })?;
    if session.feature.is_some() || !state.is_discovery() {
        return Err(Failure::blocked(
            "session.convert_already_bound",
            format!("session `{}` is already bound to a feature", session.id),
        )
        .expected("a discovery session (no feature bound)")
        .actual(format!(
            "the session is bound to `{}`",
            session
                .feature
                .as_ref()
                .map_or("an unknown feature", |f| f.as_str())
        ))
        .fix(FixAction::safe(
            "session.convert_once",
            "Conversion is one-way; a bound session cannot be converted again.",
        )));
    }

    // The name is the session's, not the caller's: find the discovery doc
    // that lists this session. Conversion promotes a name; it never
    // chooses one (ADR-0002, D9).
    let listed = crate::action::discovery::list::list(
        ctx,
        crate::action::discovery::list::ListInput { status: None },
    )?
    .value;
    let mut matching_names: Vec<FeatureName> = listed
        .discoveries
        .iter()
        .map(|summary| summary.name.clone())
        .filter(|name| {
            crate::action::discovery::load(&layout, name).is_ok_and(|doc| {
                doc.frontmatter
                    .sessions
                    .iter()
                    .any(|id| id == session.id.as_str())
            })
        })
        .collect();
    let feature_name = match matching_names.len() {
        0 => {
            return Err(Failure::blocked(
                "session.convert_no_discovery",
                format!("no discovery doc names session `{}`", session.id),
            )
            .expected("a session recorded in exactly one discovery doc's `sessions`")
            .actual("no discovery doc lists this session")
            .fix(FixAction::safe(
                "discovery.amend_first",
                "Write the discovery first — `ivar discovery amend <name>` from inside the session — then convert.",
            )));
        }
        1 => match matching_names.pop() {
            Some(name) => name,
            None => unreachable!("length checked"),
        },
        count => {
            return Err(Failure::blocked(
                "session.convert_discovery_ambiguous",
                format!("session `{}` is listed by {count} discovery docs", session.id),
            )
            .expected("a session recorded in exactly one discovery doc")
            .actual("more than one discovery doc claims the session")
            .fix(FixAction::safe(
                "discovery.remove_duplicate_session",
                "Remove the session id from every discovery doc except the one it belongs to, then convert again.",
            )));
        }
    };

    // 3. The feature is created when it does not exist. Under D9 that is
    //    the normal case: the discovery came first, and conversion is what
    //    promotes it. An existing feature of the same name is bound as-is
    //    (D3: the reverse order is allowed too).
    let feature = match Feature::read(&layout, &feature_name)? {
        Some(feature) => feature,
        None => {
            crate::action::feature::create::create(
                ctx,
                crate::action::feature::create::CreateInput {
                    name: feature_name.as_str().to_owned(),
                    branch: None,
                    base: None,
                    parent: None,
                    via: None,
                    strategy: None,
                },
            )?;
            Feature::read(&layout, &feature_name)?.ok_or_else(|| {
                Failure::failed(
                    "session.convert_feature_vanished",
                    format!("created feature `{feature_name}` but could not read it back"),
                )
            })?
        }
    };

    // Converting a discovery session into an unrestricted feature session
    // must not hand it a locked promotion; refused before the transition
    // marker or any view move.
    crate::action::feature::ensure_unrestricted_session_allowed(&layout, &feature)?;

    // 4. Record the transition, then run the (idempotent, resumable) steps.
    let transition = Transition {
        session_id: session.id.clone(),
        source: session.view_dir.clone(),
        feature: feature_name.clone(),
        step: Step::MoveSession,
    };
    write_transition(&layout, &feature_name, &transition)?;
    let outcome = run_conversion(&layout, &manifest, &feature_name, &feature, transition)?;
    mark_discovery_converted(ctx, &feature_name, outcome)
}

/// Resume an interrupted conversion. The marker's record is authoritative —
/// the session's location on disk decides which steps still need to run.
fn resume(
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    transition: Transition,
) -> Outcome<ConvertOutcome> {
    // The destination feature must still exist — rematerialising a view dir
    // needs its promotion record.
    let feature = Feature::read(layout, feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{feature_name}` does not exist"),
        )
        .expected("the feature an interrupted conversion was targeting")
        .actual("its feature.json is gone")
        .fix(FixAction::safe(
            "feature.recreate",
            format!("Recreate the feature with `ivar feature create {feature_name}`, then retry."),
        ))
    })?;
    run_conversion(layout, manifest, feature_name, &feature, transition)
}

/// The conversion steps, in order. Each is idempotent and re-derived from
/// disk, so a retry after any interruption picks up exactly where the last
/// run stopped.
fn run_conversion(
    layout: &Layout,
    manifest: &Manifest,
    feature_name: &FeatureName,
    feature: &Feature,
    mut transition: Transition,
) -> Outcome<ConvertOutcome> {
    let dest = layout.feature_session(feature_name, &transition.session_id);

    // Step 1 — move the View Dir into the feature's session tree. Resume
    // handles the crash-after-move case: the source is gone and the
    // destination exists, so there is nothing to move.
    if transition.step == Step::MoveSession {
        match (fs::is_dir(&transition.source)?, fs::is_dir(&dest)?) {
            (true, _) => {
                let Some(parent) = dest.parent() else {
                    return Err(Failure::failed(
                        "session.convert_no_parent",
                        format!("`{dest}` has no parent directory"),
                    ));
                };
                fs::ensure_dir(parent)?;
                fs::rename(&transition.source, &dest)?;
            }
            (false, true) => {}
            (false, false) => {
                return Err(Failure::failed(
                    "session.convert_missing_view_dir",
                    format!(
                        "the session's view dir is neither at `{}` nor `{dest}`",
                        transition.source
                    ),
                )
                .expected("the session's view dir, before or after the move")
                .actual("both paths are absent")
                .fix(FixAction::safe(
                    "session.start_fresh",
                    "Start a fresh session — this one's view dir cannot be recovered.",
                )));
            }
        }
        transition.step = Step::UpdateState;
        write_transition(layout, feature_name, &transition)?;
    }

    // Step 2 — bind the session's record to the feature. `started_at` and
    // `provider` were carried along by the move and are preserved untouched.
    if transition.step == Step::UpdateState {
        let mut state = SessionState::read(&dest)?.ok_or_else(|| {
            Failure::blocked(
                "session.state_missing",
                format!("session `{}` has no session record", transition.session_id),
            )
            .expected("a session with a `state.json` in its view dir")
            .actual("the view dir exists but no state.json does")
            .fix(FixAction::safe(
                "session.start_fresh",
                "Start a fresh session instead — conversion needs the session's record.",
            ))
        })?;
        state.bind(feature_name.clone(), rfc3339_now());
        state.write(&dest)?;
        transition.step = Step::Rematerialize;
        write_transition(layout, feature_name, &transition)?;
    }

    // Step 3 — rebuild the View Dir for the target feature, and clear the
    // transition: the conversion is complete. The provider comes from the
    // session's own record, which the move carried along untouched — a
    // discovery session keeps the provider it started under.
    let mut warnings = Vec::new();
    if transition.step == Step::Rematerialize {
        let provider = SessionState::read(&dest)?
            .ok_or_else(|| {
                Failure::blocked(
                    "session.state_missing",
                    format!("session `{}` has no session record", transition.session_id),
                )
                .expected("a session with a `state.json` in its view dir")
                .actual("the view dir exists but no state.json does")
                .fix(FixAction::safe(
                    "session.start_fresh",
                    "Start a fresh session instead — conversion needs the session's record.",
                ))
            })?
            .provider();
        let materialise_report =
            view::materialise(layout, manifest, Some(feature), provider, &dest)?;
        warnings.extend(materialise_report.warnings);
        fs::remove_file(&transition_path(layout, feature_name))?;
    }

    Ok(Report::with_warnings(
        ConvertOutcome {
            session_id: transition.session_id.to_string(),
            feature: feature_name.clone(),
            view_dir: dest,
        },
        warnings,
    ))
}

/// `.features/<feature>/.converting` — the transition marker for that feature.
fn transition_path(layout: &Layout, feature: &FeatureName) -> Utf8PathBuf {
    layout.feature_dir(feature).join(CONVERTING_FILE)
}

/// Read the transition marker for `feature`. `Ok(None)` when none is pending.
fn read_transition(layout: &Layout, feature: &FeatureName) -> Result<Option<Transition>, Failure> {
    json::read(&transition_path(layout, feature)).map_err(Failure::from)
}

/// Write the transition marker for `feature`, atomically, in canonical form.
fn write_transition(
    layout: &Layout,
    feature: &FeatureName,
    transition: &Transition,
) -> Result<(), Failure> {
    json::write_canonical(&transition_path(layout, feature), transition).map_err(Failure::from)
}

/// Find a pending transition for `session_id`, whichever feature owns it.
///
/// `session convert` no longer receives a feature name, so the marker can
/// no longer be read directly. Every feature is checked instead, and the
/// one whose marker names this session wins. `session_id` may be a prefix,
/// matching `lookup::resolve`.
fn find_pending_transition(
    layout: &Layout,
    session_id: &str,
) -> Result<Option<(FeatureName, Transition)>, Failure> {
    let features_dir = layout.features_dir();
    if !fs::is_dir(&features_dir)? {
        return Ok(None);
    }
    let mut matches = Vec::new();
    for entry in fs::read_dir(&features_dir)? {
        let Some(basename) = entry.file_name() else {
            continue;
        };
        let Ok(name) = FeatureName::new(basename) else {
            continue;
        };
        if let Some(transition) = read_transition(layout, &name)?
            .filter(|t| t.session_id.as_str().starts_with(session_id))
        {
            matches.push((name, transition));
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        count => Err(Failure::blocked(
            "session.prefix_ambiguous",
            format!("session prefix `{session_id}` matches {count} pending conversions"),
        )
        .expected("a full session id or unique prefix")
        .actual("more than one pending conversion matches")
        .fix(FixAction::safe(
            "session.use_longer_prefix",
            "Use the full session id, or enough characters to identify one pending conversion.",
        ))),
    }
}

/// Mark a successfully completed conversion in the discovery doc.
///
/// A failure here must not undo a completed conversion — the session is
/// already bound — so it is reported as a warning, not an error.
fn mark_discovery_converted(
    ctx: &Ctx,
    feature_name: &FeatureName,
    mut outcome: Report<ConvertOutcome>,
) -> Outcome<ConvertOutcome> {
    let marked = crate::action::discovery::close::close(
        ctx,
        crate::action::discovery::close::CloseInput {
            name: feature_name.as_str().to_owned(),
            outcome: crate::domain::discovery::DiscoveryStatus::Converted,
        },
    );
    if let Err(failure) = marked {
        outcome.warnings.push(Warning::new(
            "discovery.mark_converted_failed",
            feature_name.as_str().to_owned(),
            format!(
                "converted, but could not mark the discovery doc: {}",
                failure.what
            ),
        ));
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/conversion.rs"]
mod tests;
