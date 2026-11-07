//! The `existing_automations` section of `ivar automation scan`: every
//! `accepted` decision in force, with the state and the retirable mark the
//! domain derives for it. The only I/O here is reading the tool observations
//! of the sessions that asked for an automation.

use std::collections::BTreeMap;

use serde::Serialize;

use super::FeatureCut;
use crate::domain::automation::{Decision, DecisionKind, Form};
use crate::domain::existing_automation::{
    self as existing, Days, RETIRE_DAYS, Request, STATE_DAYS, State,
};
use crate::domain::repetition::Group;
use crate::domain::script::ScriptScope;
use crate::store::memory::observations::session_tool_bodies;

/// An `accepted` decision, with what the last 30 and 90 days say about it.
#[derive(Debug, Clone, Serialize)]
pub struct ExistingAutomation {
    pub decision_id: i64,
    pub intent: String,
    pub form: Form,
    /// The artifact's path, relative to the hall.
    pub artifact: String,
    /// `personal` inside `.ivar/scripts-local/` or `.ivar/skills-local/`,
    /// `team` otherwise.
    pub scope: ScriptScope,
    /// `in use`, `unused` or `no requests`.
    pub state: State,
    /// Nothing asked for it in 90 days: delete the artifact and `undo`, or
    /// `renew` to keep it. Never true for an agent.
    pub retirable: bool,
    /// Sessions of the last 30 days that asked for it and ran a tool citing it.
    pub sessions_with_use: Vec<String>,
    /// Sessions of the last 30 days that asked for it and did not use it.
    pub sessions_without_use: Vec<String>,
}

impl ExistingAutomation {
    fn write_line(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        write!(
            out,
            "  decision {}: {} as {} `{}` ({}, {})",
            self.decision_id,
            self.intent,
            self.form,
            self.artifact,
            self.scope.as_str(),
            self.state
        )?;
        if self.state == State::Unused {
            write!(
                out,
                ", not used in: {}",
                self.sessions_without_use.join(", ")
            )?;
        }
        writeln!(out)
    }
}

/// The existing automations that need a look, after the candidates: a count
/// line, then one line per unused and per retirable automation. The rest is
/// only counted.
pub(super) fn write_human(
    all: &[ExistingAutomation],
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    if all.is_empty() {
        return Ok(());
    }
    let count = |state: State| all.iter().filter(|entry| entry.state == state).count();
    writeln!(
        out,
        "\n{} existing automation(s): {} in use, {} unused, {} no requests; {} retirable",
        all.len(),
        count(State::InUse),
        count(State::Unused),
        count(State::NoRequests),
        all.iter().filter(|entry| entry.retirable).count(),
    )?;
    let unused: Vec<_> = all
        .iter()
        .filter(|entry| entry.state == State::Unused)
        .collect();
    if !unused.is_empty() {
        writeln!(
            out,
            "unused: asked for in the last {STATE_DAYS} days, never used by those sessions:"
        )?;
        for entry in unused {
            entry.write_line(out)?;
        }
    }
    let retirable: Vec<_> = all.iter().filter(|entry| entry.retirable).collect();
    if !retirable.is_empty() {
        writeln!(
            out,
            "retirable: not asked for in {RETIRE_DAYS} days. Retire it (delete the artifact, \
             then run `ivar automation undo` with its id) or keep it (`ivar automation \
             renew`):"
        )?;
        for entry in retirable {
            entry.write_line(out)?;
        }
    }
    Ok(())
}

/// Every accepted decision in force with its derived state, oldest first.
/// `decisions` are the decisions in force, so an accepted one whose artifact
/// is gone is not here: its group is a candidate again. `groups` are all the
/// groups of the 90-day window, recurring or not: a request that came back
/// once is still a request. With a feature `cut`, only the decisions with a
/// matching fingerprint in the feature's sessions.
pub(super) fn existing_automations(
    conn: &rusqlite::Connection,
    groups: &[Group],
    decisions: &[Decision],
    cut: Option<&FeatureCut<'_>>,
    days: Days<'_>,
) -> rusqlite::Result<Vec<ExistingAutomation>> {
    // The tool inputs of each session, read once however many decisions ask.
    let mut tool_bodies: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut found = Vec::new();
    for decision in decisions {
        let (DecisionKind::Accepted, Some(form), Some(artifact)) =
            (decision.kind, decision.form, decision.artifact.as_deref())
        else {
            continue;
        };
        let matching: Vec<_> = existing::matching_fingerprints(decision, groups).collect();
        if cut.is_some_and(|cut| !matching.iter().any(|member| cut.keeps(member))) {
            continue;
        }
        let requests: Vec<Request<'_>> = matching.into_iter().map(Request::from).collect();
        let needle = existing::cited_as(form, artifact);
        if needle.is_some() {
            for session in existing::recent_sessions(&requests, days) {
                if !tool_bodies.contains_key(session) {
                    tool_bodies.insert(session.to_owned(), session_tool_bodies(conn, session)?);
                }
            }
        }
        let assessment = existing::assess(decision, &requests, days, |session| {
            needle.as_deref().is_some_and(|needle| {
                tool_bodies.get(session).is_some_and(|bodies| {
                    bodies
                        .iter()
                        .any(|body| existing::body_cites(body, form, needle))
                })
            })
        });
        found.push(ExistingAutomation {
            decision_id: decision.id,
            intent: decision.intent.clone(),
            form,
            artifact: artifact.to_owned(),
            scope: existing::scope_of(artifact),
            state: assessment.state,
            retirable: assessment.retirable,
            sessions_with_use: assessment.sessions_with_use,
            sessions_without_use: assessment.sessions_without_use,
        });
    }
    Ok(found)
}
