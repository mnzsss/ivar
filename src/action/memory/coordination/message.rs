//! `memory_message_*`: send, list, pop and cancel a message between sessions.

use serde_json::{Value, json};

use super::{
    CoordinationCaller, UNTRUSTED_NOTICE, address_json, clean_field, now_secs, open_db,
    parse_address, queue_json, resolve_queue, ttl_days,
};
use crate::action::memory::events::report_memory_event;
use crate::domain::coordination::{Message, MessageState, NewMessage};
use crate::domain::observation::SourceEvent;
use crate::store::layout::Layout;
use crate::store::memory::coordination;

pub const MESSAGE_LIST_DEFAULT: usize = 20;
pub const MESSAGE_LIST_MAX: usize = 100;

fn render(message: &Message) -> Value {
    json!({
        "id": message.id,
        "queue": queue_json(&message.queue),
        "to": address_json(message.address.as_ref()),
        "sender": { "key": message.sender.key, "provider": message.sender.provider },
        "subject": message.subject,
        "body": message.body,
        "created_at": message.created_at,
        "expires_at": message.expires_at,
    })
}

fn visible(caller: &CoordinationCaller, message: &Message) -> bool {
    caller.viewer.sees(&message.queue, message.address.as_ref())
}

fn pending(layout: &Layout, now: i64) -> Result<Vec<Message>, String> {
    coordination::pending_messages(open_db(layout)?.conn(), now).map_err(|error| error.to_string())
}

/// # Errors
///
/// When `body` is empty, an argument is invalid or oversized, or the message cannot be stored.
pub fn send(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let body = args.get("body").and_then(Value::as_str).unwrap_or_default();
    if body.trim().is_empty() {
        return Err("`body` is required".to_owned());
    }
    let now = now_secs();
    let message = NewMessage {
        id: uuid::Uuid::new_v4().to_string(),
        queue: resolve_queue(layout, caller, args)?,
        address: parse_address(args.get("to"))?,
        sender: caller.owner.clone(),
        subject: args
            .get("subject")
            .and_then(Value::as_str)
            .map(|subject| clean_field(layout, "subject", subject))
            .transpose()?,
        body: clean_field(layout, "body", body)?,
        created_at: now,
        expires_at: now.saturating_add(i64::from(ttl_days(layout)) * 86_400),
    };
    coordination::insert_message(open_db(layout)?.conn(), &message)
        .map_err(|error| error.to_string())?;
    report_memory_event(
        layout,
        &caller.scope,
        SourceEvent::MessageSent,
        &json!({
            "id": message.id,
            "queue": message.queue.column(),
            "to": address_json(message.address.as_ref()),
            "sender": message.sender.key,
        }),
    );
    Ok(json!({ "message_id": message.id, "expires_at": message.expires_at }))
}

/// # Errors
///
/// When `box` is neither inbox nor outbox, the queue filter is invalid, or the memory database cannot be read.
pub fn list(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let outbox = match args.get("box").and_then(Value::as_str) {
        None | Some("inbox") => false,
        Some("outbox") => true,
        Some(_) => return Err("`box` must be inbox or outbox".to_owned()),
    };
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(MESSAGE_LIST_DEFAULT, |n| {
            usize::try_from(n).unwrap_or(usize::MAX)
        })
        .min(MESSAGE_LIST_MAX);
    let filter = if args.get("feature").is_some() || args.get("hall").is_some() {
        Some(resolve_queue(layout, caller, args)?)
    } else {
        None
    };
    let messages: Vec<Value> = pending(layout, now_secs())?
        .iter()
        .filter(|message| {
            if outbox {
                message.sender.key == caller.owner.key
            } else {
                visible(caller, message)
            }
        })
        .filter(|message| filter.as_ref().is_none_or(|queue| *queue == message.queue))
        .take(limit)
        .map(render)
        .collect();
    Ok(json!({ "messages": messages, "notice": UNTRUSTED_NOTICE }))
}

/// # Errors
///
/// When the memory database cannot be opened, read or updated.
pub fn pop(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let now = now_secs();
    let db = open_db(layout)?;
    let target = match args.get("message_id").and_then(Value::as_str) {
        Some(id) => {
            match coordination::get_message(db.conn(), id).map_err(|error| error.to_string())? {
                Some(message) if !visible(caller, &message) => None,
                Some(message) if message.state == MessageState::Popped => {
                    return Ok(json!({ "status": "already_claimed" }));
                }
                Some(message)
                    if message.state != MessageState::Pending || message.expires_at <= now =>
                {
                    None
                }
                found => found,
            }
        }
        None => pending(layout, now)?
            .into_iter()
            .find(|message| visible(caller, message)),
    };
    let Some(message) = target else {
        return Ok(json!({ "status": "none_pending" }));
    };
    if !coordination::pop_message(
        db.conn(),
        &message.id,
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
        SourceEvent::MessagePopped,
        &json!({
            "id": message.id,
            "queue": message.queue.column(),
            "sender": message.sender.key,
            "popped_by": caller.owner.key,
        }),
    );
    Ok(json!({ "status": "popped", "message": render(&message), "notice": UNTRUSTED_NOTICE }))
}

/// # Errors
///
/// When the memory database cannot be opened or updated.
pub fn cancel(layout: &Layout, caller: &CoordinationCaller, args: &Value) -> Result<Value, String> {
    let cancelled = coordination::cancel_messages(
        open_db(layout)?.conn(),
        &caller.owner.key,
        args.get("message_id").and_then(Value::as_str),
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({ "cancelled": cancelled }))
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/memory/coordination_message.rs"]
mod tests;
