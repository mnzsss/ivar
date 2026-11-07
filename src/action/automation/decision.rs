//! `ivar automation decide | renew | decisions | undo`: the human's answers to
//! proposals to automate a recurring request.
//!
//! `decide`, `renew` and `undo` are the only commands that write the decisions;
//! `scan` never does, and the memory sweep only maintains them.

use std::collections::BTreeSet;

use serde::Serialize;

use super::{has_database, open_read_only};
use crate::action::memory::drain::{self, epoch_secs_now};
use crate::action::{Ctx, discover_hall};
pub use crate::domain::automation::Form;
use crate::domain::automation::{
    DEFAULT_DEFERRAL_DAYS, Decision, DecisionKind, clean_artifact, intent_problem, is_day, utc_day,
};
use crate::domain::repetition::WINDOW_DAYS;
use crate::domain::session::rfc3339_from_epoch_secs;
use crate::error::{Failure, Outcome, Report, Warning, WriteHuman};
use crate::store::layout::Layout;
use crate::store::memory::automation::{self as store, NewDecision};
use crate::store::memory::db::MemoryDb;
use crate::store::memory::fingerprints::grouped_since;
use crate::store::memory::retention::cutoff_day_before;

/// What `decide` records for the group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The automation exists as `artifact`, a hall-relative path.
    Accepted {
        intent: String,
        form: Form,
        artifact: String,
    },
    Declined {
        intent: String,
    },
    /// Not now. `until` is the last day of the deferral, `YYYY-MM-DD`; it
    /// defaults to 30 days from now.
    Deferred {
        intent: String,
        until: Option<String>,
    },
    /// Add the group's texts to this declined or deferred decision.
    SameAs {
        decision_id: i64,
    },
}

/// What `ivar automation decide` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecideInput {
    /// The `group_ref` a scan reported: the id of a request fingerprint.
    pub group_ref: i64,
    pub answer: Answer,
}

/// A decision as the commands report it.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionView {
    pub id: i64,
    pub kind: DecisionKind,
    pub intent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Form>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// How many normalized texts the decision covers.
    pub text_count: usize,
}

impl From<&Decision> for DecisionView {
    fn from(decision: &Decision) -> Self {
        Self {
            id: decision.id,
            kind: decision.kind,
            intent: decision.intent.clone(),
            form: decision.form,
            artifact: decision.artifact.clone(),
            until: decision.until.clone(),
            created_at: decision.created_at.clone(),
            updated_at: decision.updated_at.clone(),
            text_count: decision.texts.len(),
        }
    }
}

impl DecisionView {
    fn write_line(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        write!(out, "decision {}: {} {}", self.id, self.kind, self.intent)?;
        if let (Some(form), Some(artifact)) = (self.form, &self.artifact) {
            write!(out, " as {form} `{artifact}`")?;
        }
        if let Some(until) = &self.until {
            write!(out, " until {until}")?;
        }
        writeln!(
            out,
            " ({} text(s), updated {})",
            self.text_count, self.updated_at
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DecideOutcome {
    pub decision: DecisionView,
    /// The texts of the group that the decision did not cover before.
    pub texts_added: usize,
}

impl WriteHuman for DecideOutcome {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        write!(out, "recorded ")?;
        self.decision.write_line(out)?;
        writeln!(out, "added {} text(s)", self.texts_added)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RenewOutcome {
    pub decision: DecisionView,
}

impl WriteHuman for RenewOutcome {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        write!(out, "renewed ")?;
        self.decision.write_line(out)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UndoOutcome {
    pub removed: DecisionView,
}

impl WriteHuman for UndoOutcome {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        write!(out, "removed ")?;
        self.removed.write_line(out)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionsOutcome {
    pub decisions: Vec<DecisionView>,
}

impl WriteHuman for DecisionsOutcome {
    fn write_human(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        writeln!(out, "{} automation decision(s)", self.decisions.len())?;
        for decision in &self.decisions {
            write!(out, "  ")?;
            decision.write_line(out)?;
        }
        Ok(())
    }
}

fn not_found(id: i64) -> Failure {
    Failure::blocked(
        "automation.decision_not_found",
        format!("there is no automation decision {id}"),
    )
    .fix(
        crate::error::FixAction::safe(
            "automation.list_decisions",
            "List the decisions to find the id.",
        )
        .command("ivar automation decisions"),
    )
}

fn stored(error: impl std::fmt::Display) -> Failure {
    Failure::failed(
        "automation.decision_store_failed",
        format!("could not use the automation decisions in memory.db: {error}"),
    )
}

fn epoch_text(now_epoch_secs: u64) -> String {
    rfc3339_from_epoch_secs(now_epoch_secs as f64)
}

/// Opens `memory.db` for writing, when there is one. A hall with no database
/// has no fingerprint and no decision to name.
fn open_for_writing(layout: &Layout) -> Result<Option<MemoryDb>, Failure> {
    if !has_database(layout) {
        return Ok(None);
    }
    MemoryDb::open(layout.memory_db().as_std_path())
        .map(Some)
        .map_err(stored)
}

/// Records the human's answer for the group that holds fingerprint
/// `input.group_ref`. Drains the capture spool first, so the group is the one
/// the next scan sees.
///
/// # Errors
///
/// Outside a hall; for an intent, artifact or date that is not valid; when the
/// fingerprint no longer exists, or `--same-as` names no declined or deferred
/// decision; and when `memory.db` cannot be used.
pub fn decide(ctx: &Ctx, input: &DecideInput) -> Outcome<DecideOutcome> {
    let layout = discover_hall(ctx)?;
    let mut warnings = Vec::new();
    if let Err(failure) = drain::drain(&layout) {
        warnings.push(Warning::new(
            "automation.drain_skipped",
            "memory spool",
            format!(
                "the capture spool was not drained, so the newest prompts may be missing: {}",
                failure.what
            ),
        ));
    }
    let mut report = decide_in(&layout, input, epoch_secs_now())?;
    warnings.append(&mut report.warnings);
    report.warnings = warnings;
    Ok(report)
}

/// [`decide`] in a known hall at `now_epoch_secs`, without draining the spool.
///
/// # Errors
///
/// As [`decide`].
pub fn decide_in(
    layout: &Layout,
    input: &DecideInput,
    now_epoch_secs: u64,
) -> Outcome<DecideOutcome> {
    let plan = Plan::new(&input.answer, now_epoch_secs)?;
    let Some(mut db) = open_for_writing(layout)? else {
        return Err(group_gone(input.group_ref));
    };
    let now = epoch_text(now_epoch_secs);
    // One immediate transaction: the group is read and the decision written
    // against the same data.
    let tx = rusqlite::Transaction::new(db.conn_mut(), rusqlite::TransactionBehavior::Immediate)
        .map_err(stored)?;
    let (_, groups) =
        grouped_since(&tx, &cutoff_day_before(now_epoch_secs, WINDOW_DAYS)).map_err(stored)?;
    let group = groups
        .iter()
        .find(|group| group.members().iter().any(|f| f.id == input.group_ref))
        .ok_or_else(|| group_gone(input.group_ref))?;
    let texts: BTreeSet<String> = group.texts().into_iter().map(str::to_owned).collect();

    let mut warnings = Vec::new();
    let (id, texts_added) = match &plan {
        Plan::New {
            kind,
            intent,
            form,
            artifact,
            until,
        } => {
            let id = store::insert(
                &tx,
                &NewDecision {
                    kind: *kind,
                    intent,
                    form: *form,
                    artifact: artifact.as_deref(),
                    until: until.as_deref(),
                    texts: &texts,
                },
                &now,
            )
            .map_err(stored)?;
            if let Some(artifact) = artifact
                && !layout.root().join(artifact).exists()
            {
                warnings.push(Warning::new(
                    "automation.artifact_missing",
                    artifact.clone(),
                    "the artifact does not exist yet, so the group stays a candidate until it does",
                ));
            }
            (id, texts.len())
        }
        Plan::SameAs(id) => {
            let existing = store::get(&tx, *id)
                .map_err(stored)?
                .ok_or_else(|| not_found(*id))?;
            if existing.kind == DecisionKind::Accepted {
                return Err(Failure::blocked(
                    "automation.same_as_accepted",
                    format!(
                        "decision {id} is accepted: only a declined or deferred decision can take more texts"
                    ),
                ));
            }
            let added = store::add_texts(&tx, *id, &texts).map_err(stored)?;
            store::touch(&tx, *id, &now).map_err(stored)?;
            (*id, added)
        }
    };
    let decision = store::get(&tx, id)
        .map_err(stored)?
        .ok_or_else(|| not_found(id))?;
    tx.commit().map_err(stored)?;
    Ok(Report::with_warnings(
        DecideOutcome {
            decision: DecisionView::from(&decision),
            texts_added,
        },
        warnings,
    ))
}

fn group_gone(group_ref: i64) -> Failure {
    Failure::blocked(
        "automation.group_ref_gone",
        format!(
            "no request fingerprint {group_ref} in the last {WINDOW_DAYS} days: it expired or never existed"
        ),
    )
    .fix(
        crate::error::FixAction::safe(
            "automation.rescan",
            "Scan again and use a current `group_ref`.",
        )
        .command("ivar automation scan"),
    )
}

/// An answer, validated.
enum Plan {
    New {
        kind: DecisionKind,
        intent: String,
        form: Option<Form>,
        artifact: Option<String>,
        until: Option<String>,
    },
    SameAs(i64),
}

impl Plan {
    fn new(answer: &Answer, now_epoch_secs: u64) -> Result<Self, Failure> {
        let invalid = |what: String| Failure::blocked("automation.invalid_decision", what);
        let checked = |intent: &str| match intent_problem(intent) {
            Some(problem) => Err(invalid(problem.to_owned())),
            None => Ok(intent.to_owned()),
        };
        Ok(match answer {
            Answer::Accepted {
                intent,
                form,
                artifact,
            } => Self::New {
                kind: DecisionKind::Accepted,
                intent: checked(intent)?,
                form: Some(*form),
                artifact: Some(clean_artifact(artifact).map_err(|why| invalid(why.to_owned()))?),
                until: None,
            },
            Answer::Declined { intent } => Self::New {
                kind: DecisionKind::Declined,
                intent: checked(intent)?,
                form: None,
                artifact: None,
                until: None,
            },
            Answer::Deferred { intent, until } => {
                let today = utc_day(now_epoch_secs);
                let until = match until {
                    Some(until) if !is_day(until) => {
                        return Err(invalid(format!(
                            "`{until}` is not a date: write it as YYYY-MM-DD"
                        )));
                    }
                    Some(until) if *until < today => {
                        return Err(invalid(format!("`{until}` is already past")));
                    }
                    Some(until) => until.clone(),
                    None => utc_day(now_epoch_secs + DEFAULT_DEFERRAL_DAYS * 86_400),
                };
                Self::New {
                    kind: DecisionKind::Deferred,
                    intent: checked(intent)?,
                    form: None,
                    artifact: None,
                    until: Some(until),
                }
            }
            Answer::SameAs { decision_id } => Self::SameAs(*decision_id),
        })
    }
}

/// Marks an accepted decision as renewed by rewriting its `updated_at`.
///
/// # Errors
///
/// Outside a hall, when the decision does not exist or is not accepted, and
/// when `memory.db` cannot be used.
pub fn renew(ctx: &Ctx, decision_id: i64) -> Outcome<RenewOutcome> {
    renew_in(&discover_hall(ctx)?, decision_id, epoch_secs_now())
}

/// [`renew`] in a known hall at `now_epoch_secs`.
///
/// # Errors
///
/// As [`renew`].
pub fn renew_in(layout: &Layout, decision_id: i64, now_epoch_secs: u64) -> Outcome<RenewOutcome> {
    let Some(db) = open_for_writing(layout)? else {
        return Err(not_found(decision_id));
    };
    let existing = store::get(db.conn(), decision_id)
        .map_err(stored)?
        .ok_or_else(|| not_found(decision_id))?;
    if existing.kind != DecisionKind::Accepted {
        return Err(Failure::blocked(
            "automation.renew_not_accepted",
            format!(
                "decision {decision_id} is {}: only an accepted decision is renewed",
                existing.kind
            ),
        ));
    }
    store::touch(db.conn(), decision_id, &epoch_text(now_epoch_secs)).map_err(stored)?;
    let decision = store::get(db.conn(), decision_id)
        .map_err(stored)?
        .ok_or_else(|| not_found(decision_id))?;
    Ok(Report::new(RenewOutcome {
        decision: DecisionView::from(&decision),
    }))
}

/// Lists the decisions, oldest first. Read-only.
///
/// # Errors
///
/// Outside a hall, and when `memory.db` is outdated, newer or unreadable.
pub fn decisions(ctx: &Ctx) -> Outcome<DecisionsOutcome> {
    decisions_in(&discover_hall(ctx)?)
}

/// [`decisions`] in a known hall.
///
/// # Errors
///
/// As [`decisions`].
pub fn decisions_in(layout: &Layout) -> Outcome<DecisionsOutcome> {
    let decisions = match open_read_only(layout)? {
        Some(conn) => store::list(&conn).map_err(stored)?,
        None => Vec::new(),
    };
    Ok(Report::new(DecisionsOutcome {
        decisions: decisions.iter().map(DecisionView::from).collect(),
    }))
}

/// Deletes a decision; its group is a candidate again when it recurs enough.
///
/// # Errors
///
/// Outside a hall, when the decision does not exist, and when `memory.db`
/// cannot be used.
pub fn undo(ctx: &Ctx, decision_id: i64) -> Outcome<UndoOutcome> {
    undo_in(&discover_hall(ctx)?, decision_id)
}

/// [`undo`] in a known hall.
///
/// # Errors
///
/// As [`undo`].
pub fn undo_in(layout: &Layout, decision_id: i64) -> Outcome<UndoOutcome> {
    let Some(mut db) = open_for_writing(layout)? else {
        return Err(not_found(decision_id));
    };
    let tx = rusqlite::Transaction::new(db.conn_mut(), rusqlite::TransactionBehavior::Immediate)
        .map_err(stored)?;
    let existing = store::get(&tx, decision_id)
        .map_err(stored)?
        .ok_or_else(|| not_found(decision_id))?;
    store::delete(&tx, decision_id).map_err(stored)?;
    tx.commit().map_err(stored)?;
    Ok(Report::new(UndoOutcome {
        removed: DecisionView::from(&existing),
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/automation/decision.rs"]
mod tests;
