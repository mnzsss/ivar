//! `ivar memory handoff`: begin, list and cancel handoffs from the terminal.

use std::io;

use serde::Serialize;
use serde_json::{Value, json};

use super::handoff::{self, render};
use super::{
    CoordinationCaller, UNTRUSTED_NOTICE, now_secs, open_existing, open_existing_read_only,
};
use crate::action::memory::events::{MemoryEventScope, report_memory_event};
use crate::action::{Ctx, discover_hall};
use crate::domain::coordination::{Owner, Queue, Viewer};
use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::domain::observation::SourceEvent;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::layout::Layout;
use crate::store::memory::coordination;

const CLI_OWNER: &str = "cli";
const SUMMARY_WIDTH: usize = 80;

#[derive(Debug, Clone)]
pub struct BeginCliInput {
    pub summary: String,
    pub pending: Vec<String>,
    pub decision: Vec<String>,
    pub file: Vec<String>,
    pub feature: Option<String>,
    pub hall: bool,
    pub to_provider: Option<String>,
    pub to_session: Option<String>,
    pub session: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct HandoffCliOutput(pub Value);

fn cli_caller(
    layout: &Layout,
    ctx: &Ctx,
    session: Option<&str>,
) -> Result<CoordinationCaller, Failure> {
    let (key, provider, feature) = match session {
        Some(prefix) => {
            let found = crate::action::session::lookup::resolve(layout, Some(prefix), None)?;
            let provider = found
                .state
                .as_ref()
                .map_or("unknown", |state| state.provider().id())
                .to_owned();
            (found.id.as_str().to_owned(), provider, found.feature)
        }
        None => (
            CLI_OWNER.to_owned(),
            CLI_OWNER.to_owned(),
            hall_feature(layout, ctx.session_feature()),
        ),
    };
    Ok(CoordinationCaller {
        owner: Owner {
            key: key.clone(),
            provider: provider.clone(),
        },
        scope: MemoryEventScope {
            session_id: key.clone(),
            feature: feature
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            ..MemoryEventScope::default()
        },
        viewer: Viewer {
            session_key: Some(key),
            provider,
            feature,
        },
    })
}

/// `$IVAR_FEATURE` may come from a session of another hall; only a feature of this hall counts.
fn hall_feature(layout: &Layout, name: Option<&str>) -> Option<FeatureName> {
    let name = FeatureName::new(name?).ok()?;
    Feature::read(layout, &name).ok().flatten().map(|_| name)
}

/// # Errors
///
/// When the hall cannot be found, `--session` matches no single session, or the handoff is refused.
pub fn begin_from_cli(ctx: &Ctx, input: &BeginCliInput) -> Outcome<HandoffCliOutput> {
    let layout = discover_hall(ctx)?;
    let caller = cli_caller(&layout, ctx, input.session.as_deref())?;
    let to = match (&input.to_provider, &input.to_session) {
        (Some(provider), _) => json!({ "provider": provider }),
        (_, Some(session)) => json!({ "session": session }),
        _ => Value::Null,
    };
    let args = json!({
        "summary": input.summary,
        "pending_tasks": input.pending,
        "open_decisions": input.decision,
        "modified_files": input.file,
        "feature": input.feature,
        "hall": input.hall,
        "to": to,
    });
    handoff::begin(&layout, &caller, &args)
        .map(|value| Report::new(HandoffCliOutput(value)))
        .map_err(|what| Failure::blocked("memory.handoff_refused", what))
}

/// # Errors
///
/// When the hall cannot be found, `feature` names no feature of the hall, or the memory database cannot be read.
pub fn list_from_cli(ctx: &Ctx, feature: Option<&str>, hall: bool) -> Outcome<HandoffCliOutput> {
    let layout = discover_hall(ctx)?;
    let filter = match (feature, hall) {
        (Some(name), _) => {
            let feature = FeatureName::new(name)?;
            if Feature::read(&layout, &feature)?.is_none() {
                return Err(Failure::blocked(
                    "memory.handoff_feature_unknown",
                    format!("feature `{name}` does not exist"),
                ));
            }
            Some(Queue::Feature(feature))
        }
        (None, true) => Some(Queue::Hall),
        (None, false) => None,
    };
    let handoffs: Vec<Value> =
        if let Some(conn) = open_existing_read_only(&layout).map_err(db_unavailable)? {
            coordination::open_handoffs(&conn, now_secs())
                .map_err(|error| db_unavailable(error.to_string()))?
                .iter()
                .filter(|handoff| filter.as_ref().is_none_or(|queue| *queue == handoff.queue))
                .map(render)
                .collect()
        } else {
            Vec::new()
        };
    Ok(Report::new(HandoffCliOutput(
        json!({ "handoffs": handoffs, "notice": UNTRUSTED_NOTICE }),
    )))
}

/// # Errors
///
/// When the hall cannot be found, the memory database cannot be written, or no open handoff has this id.
pub fn cancel_from_cli(ctx: &Ctx, id: &str) -> Outcome<HandoffCliOutput> {
    let layout = discover_hall(ctx)?;
    let cancelled = match open_existing(&layout).map_err(db_unavailable)? {
        Some(db) => coordination::cancel_handoff(db.conn(), id, None, now_secs())
            .map_err(|error| db_unavailable(error.to_string()))?,
        None => false,
    };
    if !cancelled {
        return Err(Failure::blocked(
            "memory.handoff_not_open",
            format!("no open handoff `{id}`"),
        ));
    }
    report_memory_event(
        &layout,
        &MemoryEventScope {
            session_id: CLI_OWNER.to_owned(),
            ..MemoryEventScope::default()
        },
        SourceEvent::HandoffCancelled,
        &json!({ "id": id, "by": CLI_OWNER }),
    );
    Ok(Report::new(HandoffCliOutput(
        json!({ "handoff_id": id, "cancelled": true }),
    )))
}

fn db_unavailable(what: String) -> Failure {
    Failure::failed("memory.db_unavailable", what)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

impl WriteHuman for HandoffCliOutput {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let id = text(&self.0, "handoff_id");
        if self.0.get("cancelled").is_some() {
            return writeln!(w, "Cancelled handoff {id}.");
        }
        let Some(rows) = self.0.get("handoffs").and_then(Value::as_array) else {
            return writeln!(w, "Opened handoff {id}.");
        };
        if rows.is_empty() {
            return writeln!(w, "No open handoffs.");
        }
        let now = now_secs();
        for row in rows {
            let created = row.get("created_at").and_then(Value::as_i64).unwrap_or(now);
            let summary: String = text(row, "summary")
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(SUMMARY_WIDTH)
                .collect();
            writeln!(
                w,
                "{}  {}  {}  {summary}",
                text(row, "id"),
                text(row, "owner"),
                age(now.saturating_sub(created)),
            )?;
        }
        Ok(())
    }
}

fn age(secs: i64) -> String {
    if secs < 3_600 {
        format!("{}m", secs.max(0) / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/coordination_cli.rs"]
mod tests;
