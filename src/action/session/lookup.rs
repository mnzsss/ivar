//! Finding sessions on disk — shared by `connect`, `convert`, and relay.
//!
//! A session is live while its View Dir exists. Where the View Dir lives
//! decides what kind of session it is:
//!
//! - `.ivar/sessions/<id>/` — a discovery session (no feature bound)
//! - `.ivar/features/<feature>/sessions/<id>/` — a feature session
//!
//! This module enumerates both trees, resolves id-prefixes, and picks the most
//! recent session of a feature (for relay). Read-only: nothing here mutates
//! state.

use camino::Utf8Path;

use crate::domain::name::{FeatureName, SessionId};
use crate::domain::session::{SessionRef, SessionState};
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::store::layout::Layout;

/// Every live discovery session in the hall.
pub(crate) fn list_discovery(layout: &Layout) -> Result<Vec<SessionRef>, Failure> {
    sessions_in(&layout.discovery_sessions_dir(), None)
}

/// Every live session bound to `feature`.
pub(crate) fn list_feature(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Vec<SessionRef>, Failure> {
    sessions_in(
        &layout.feature_dir(feature).join("sessions"),
        Some(feature.clone()),
    )
}

/// Resolve a session by id-prefix and/or feature, to exactly one session.
///
/// `Blocked` when neither filter is given, when nothing matches, or when more
/// than one session matches (naming the candidates so the caller can narrow).
pub(crate) fn resolve(
    layout: &Layout,
    id_prefix: Option<&str>,
    feature: Option<&str>,
) -> Result<SessionRef, Failure> {
    if id_prefix.is_none() && feature.is_none() {
        return Err(Failure::blocked(
            "session.lookup_needs_filter",
            "name a session id (or a unique prefix) and/or a feature to search in",
        )
        .expected("at least one of `session_id` or `--feature`")
        .actual("neither given")
        .fix(FixAction::safe(
            "session.connect_filter",
            "Pass the session id (or a prefix of it) and/or `--feature`.",
        )));
    }

    let mut candidates = Vec::new();
    let feature_filter = feature.map(FeatureName::new).transpose()?;
    match &feature_filter {
        Some(name) => candidates.extend(list_feature(layout, name)?),
        None => {
            candidates.extend(list_discovery(layout)?);
            if fs::is_dir(&layout.features_dir())? {
                for entry in fs::read_dir(&layout.features_dir())? {
                    let Some(name) = entry.file_name() else {
                        continue;
                    };
                    let Ok(name) = FeatureName::new(name) else {
                        continue;
                    };
                    candidates.extend(list_feature(layout, &name)?);
                }
            }
        }
    }

    let matches: Vec<SessionRef> = candidates
        .into_iter()
        .filter(|session| id_prefix.is_none_or(|prefix| session.id.as_str().starts_with(prefix)))
        .collect();

    match matches.len() {
        0 => Err(
            Failure::blocked("session.not_found", "no live session matches")
                .expected("a live session — one whose View Dir exists")
                .actual(describe_request(id_prefix, feature))
                .fix(FixAction::safe(
                    "session.start_first",
                    "Start a session first with `ivar session start`.",
                )),
        ),
        1 => matches.into_iter().next().ok_or_else(|| {
            Failure::failed(
                "session.lookup_internal",
                "a session matched but its record vanished",
            )
        }),
        _ => Err(Failure::blocked(
            "session.ambiguous",
            format!("{} sessions match", matches.len()),
        )
        .expected("exactly one session")
        .actual(format!(
            "matching sessions: {}",
            matches
                .iter()
                .map(|session| session.id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .fix(FixAction::safe(
            "session.narrow_filter",
            "Add `--feature`, or use the full session id.",
        ))),
    }
}

/// The most recent live session bound to `feature`, by `started_at`.
///
/// Sessions without a readable `state.json` are skipped — without a record
/// there is no provider to compare, which is all relay needs this for.
pub(crate) fn most_recent(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Option<SessionRef>, Failure> {
    let mut sessions: Vec<SessionRef> = list_feature(layout, feature)?
        .into_iter()
        .filter(|session| session.state.is_some())
        .collect();
    sessions.sort_by(|a, b| {
        let a = a.state.as_ref().map_or("", SessionState::started_at);
        let b = b.state.as_ref().map_or("", SessionState::started_at);
        b.cmp(a) // descending: most recent first
    });
    Ok(sessions.into_iter().next())
}

/// What `resolve` was asked for, for the "nothing matched" message.
fn describe_request(id_prefix: Option<&str>, feature: Option<&str>) -> String {
    match (id_prefix, feature) {
        (Some(id), Some(feature)) => format!("session id `{id}` in feature `{feature}`"),
        (Some(id), None) => format!("session id `{id}`"),
        (None, Some(feature)) => format!("any session in feature `{feature}`"),
        (None, None) => "any session".to_owned(),
    }
}

/// The live sessions under `dir` — one [`SessionRef`] per entry that is a
/// directory named as a valid session id.
///
/// A session whose `state.json` is missing or unreadable still counts: its
/// location identifies it, and the verbs that actually need the record
/// (`convert`, relay) re-read it strictly where it matters.
fn sessions_in(dir: &Utf8Path, feature: Option<FeatureName>) -> Result<Vec<SessionRef>, Failure> {
    if !fs::is_dir(dir)? {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for entry in fs::read_dir(dir)? {
        let Some(name) = entry.file_name() else {
            continue;
        };
        let Ok(id) = SessionId::new(name) else {
            continue;
        };
        if !fs::is_dir(&entry)? {
            continue;
        }
        let state = SessionState::read(&entry).ok().flatten();
        sessions.push(SessionRef {
            id,
            feature: feature.clone(),
            view_dir: entry,
            state,
        });
    }
    Ok(sessions)
}
