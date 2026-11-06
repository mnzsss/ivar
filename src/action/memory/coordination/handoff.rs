//! `memory_handoff_*`: begin, list, accept and cancel a session handoff.

use serde_json::{Value, json};

use super::{
    CoordinationCaller, UNTRUSTED_NOTICE, address_json, clean_field, clean_list, now_secs, open_db,
    parse_address, queue_json, resolve_queue, ttl_days,
};
use crate::action::memory::events::report_memory_event;
use crate::domain::coordination::{Handoff, HandoffState, NewHandoff, Queue, Viewer};
use crate::domain::observation::SourceEvent;
use crate::store::layout::Layout;
use crate::store::memory::coordination;

pub const HANDOFF_LIST_DEFAULT: usize = 20;
pub const HANDOFF_LIST_MAX: usize = 100;

/// Open handoffs the `viewer` sees, newest first, narrowed to `filter` after visibility applies.
///
/// # Errors
///
/// When the memory database cannot be opened or read.
pub fn visible_open(
    layout: &Layout,
    viewer: &Viewer,
    filter: Option<&Queue>,
    now: i64,
) -> Result<Vec<Handoff>, String> {
    let db = open_db(layout)?;
    Ok(coordination::open_handoffs(db.conn(), now)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|handoff| viewer.sees(&handoff.queue, handoff.address.as_ref()))
        .filter(|handoff| filter.is_none_or(|queue| *queue == handoff.queue))
        .collect())
}

pub(super) fn render(handoff: &Handoff) -> Value {
    json!({
        "id": handoff.id,
        "queue": queue_json(&handoff.queue),
        "to": address_json(handoff.address.as_ref()),
        "owner": handoff.owner.key,
        "owner_provider": handoff.owner.provider,
        "created_at": handoff.created_at,
        "expires_at": handoff.expires_at,
        "summary": handoff.summary,
        "pending_tasks": handoff.pending_tasks,
        "open_decisions": handoff.open_decisions,
        "modified_files": handoff.modified_files,
    })
}

/// # Errors
///
/// When the queue filter is invalid or the memory database cannot be read.
pub fn list(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(HANDOFF_LIST_DEFAULT, |n| {
            usize::try_from(n).unwrap_or(usize::MAX)
        })
        .min(HANDOFF_LIST_MAX);
    let filter = if args.get("feature").is_some() || args.get("hall").is_some() {
        Some(resolve_queue(layout, caller, args)?)
    } else {
        None
    };
    let handoffs: Vec<Value> = visible_open(layout, &caller.viewer, filter.as_ref(), now_secs())?
        .iter()
        .take(limit)
        .map(render)
        .collect();
    Ok(json!({ "handoffs": handoffs, "notice": UNTRUSTED_NOTICE }))
}

/// # Errors
///
/// When `summary` is empty, an argument is invalid or oversized, or the handoff cannot be stored.
pub fn begin(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if summary.trim().is_empty() {
        return Err("`summary` is required".to_owned());
    }
    let now = now_secs();
    let handoff = NewHandoff {
        id: uuid::Uuid::new_v4().to_string(),
        queue: resolve_queue(layout, caller, args)?,
        address: parse_address(args.get("to"))?,
        owner: caller.owner.clone(),
        summary: clean_field(layout, "summary", summary)?,
        pending_tasks: clean_list(layout, "pending_tasks", args)?,
        open_decisions: clean_list(layout, "open_decisions", args)?,
        modified_files: clean_list(layout, "modified_files", args)?,
        created_at: now,
        expires_at: now.saturating_add(i64::from(ttl_days(layout)) * 86_400),
    };
    coordination::insert_handoff(open_db(layout)?.conn(), &handoff)
        .map_err(|error| error.to_string())?;
    report_memory_event(
        layout,
        &caller.scope,
        SourceEvent::HandoffOpened,
        &json!({
            "id": handoff.id,
            "queue": handoff.queue.column(),
            "to": address_json(handoff.address.as_ref()),
            "owner": handoff.owner.key,
            "owner_provider": handoff.owner.provider,
        }),
    );
    Ok(json!({ "handoff_id": handoff.id, "expires_at": handoff.expires_at }))
}

/// # Errors
///
/// When the memory database cannot be opened, read or updated.
pub fn accept(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let now = now_secs();
    let db = open_db(layout)?;
    let target = match args.get("handoff_id").and_then(Value::as_str) {
        Some(id) => match coordination::get_handoff(db.conn(), id)
            .map_err(|error| error.to_string())?
        {
            Some(handoff) if !caller.viewer.sees(&handoff.queue, handoff.address.as_ref()) => None,
            Some(handoff) if handoff.state == HandoffState::Accepted => {
                return Ok(json!({ "status": "already_claimed" }));
            }
            Some(handoff) if handoff.state != HandoffState::Open || handoff.expires_at <= now => {
                None
            }
            found => found,
        },
        None => visible_open(layout, &caller.viewer, None, now)?
            .into_iter()
            .next(),
    };
    let Some(handoff) = target else {
        return Ok(json!({ "status": "none_pending" }));
    };
    if !coordination::claim_handoff(
        db.conn(),
        &handoff.id,
        &caller.owner.key,
        &caller.viewer,
        now,
    )
    .map_err(|error| error.to_string())?
    {
        return Ok(json!({ "status": "already_claimed" }));
    }
    report_memory_event(
        layout,
        &caller.scope,
        SourceEvent::HandoffAccepted,
        &json!({
            "id": handoff.id,
            "queue": handoff.queue.column(),
            "owner": handoff.owner.key,
            "accepted_by": caller.owner.key,
        }),
    );
    Ok(json!({ "status": "claimed", "handoff": render(&handoff), "notice": UNTRUSTED_NOTICE }))
}

/// # Errors
///
/// When `handoff_id` is missing, or the handoff is not open, does not exist, or belongs to another owner.
pub fn cancel(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let id = args
        .get("handoff_id")
        .and_then(Value::as_str)
        .ok_or("`handoff_id` is required")?;
    let cancelled = coordination::cancel_handoff(
        open_db(layout)?.conn(),
        id,
        Some(&caller.owner.key),
        now_secs(),
    )
    .map_err(|error| error.to_string())?;
    if !cancelled {
        return Err(format!(
            "handoff `{id}` is not open, does not exist, or is not yours"
        ));
    }
    report_memory_event(
        layout,
        &caller.scope,
        SourceEvent::HandoffCancelled,
        &json!({ "id": id, "owner": caller.owner.key }),
    );
    Ok(json!({ "cancelled": id }))
}
