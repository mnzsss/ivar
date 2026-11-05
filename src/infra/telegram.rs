//! The Telegram Bot API calls ivar makes. No error leaves this module carrying the bot token.

use std::time::Duration;

use serde_json::{Value, json};

use super::webhook;

pub const API: &str = "https://api.telegram.org";
const TIMEOUT: Duration = Duration::from_secs(10);
const CHAT_KEYS: [&str; 4] = [
    "message",
    "edited_message",
    "channel_post",
    "my_chat_member",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub id: String,
    pub name: String,
    pub kind: String,
}

/// # Errors
///
/// Telegram's refusal or a transport failure, without the token.
pub fn get_me(api: &str, token: &str) -> Result<String, String> {
    let me = call(api, token, "getMe", &json!({}))?;
    me.get("username")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "Telegram's getMe answer has no username".to_owned())
}

/// # Errors
///
/// Telegram's refusal or a transport failure, without the token.
pub fn get_updates(api: &str, token: &str) -> Result<Vec<Chat>, String> {
    let updates = call(api, token, "getUpdates", &json!({}))?;
    let mut chats: Vec<Chat> = Vec::new();
    for chat in updates.as_array().into_iter().flatten().filter_map(chat_of) {
        if !chats.iter().any(|known| known.id == chat.id) {
            chats.push(chat);
        }
    }
    Ok(chats)
}

/// # Errors
///
/// Telegram's refusal or a transport failure, without the token.
pub fn send_message(api: &str, token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    call(
        api,
        token,
        "sendMessage",
        &json!({ "chat_id": chat_id, "text": text }),
    )
    .map(|_| ())
}

/// Sends `html`, falling back to `plain` when Telegram refuses the markup,
/// with a button that copies `copy_text` to the clipboard.
///
/// # Errors
///
/// Telegram's refusal or a transport failure, without the token.
pub fn send_card(
    api: &str,
    token: &str,
    chat_id: &str,
    html: &str,
    plain: &str,
    copy: (&str, &str),
) -> Result<(), String> {
    let (label, copy_text) = copy;
    let keyboard =
        json!({ "inline_keyboard": [[{ "text": label, "copy_text": { "text": copy_text } }]] });
    let html_body =
        json!({ "chat_id": chat_id, "text": html, "parse_mode": "HTML", "reply_markup": keyboard });
    match call(api, token, "sendMessage", &html_body) {
        Err(error) if error.contains("(HTTP 400)") => {
            let plain_body = json!({ "chat_id": chat_id, "text": plain, "reply_markup": keyboard });
            call(api, token, "sendMessage", &plain_body).map(|_| ())
        }
        result => result.map(|_| ()),
    }
}

fn chat_of(update: &Value) -> Option<Chat> {
    let chat = CHAT_KEYS
        .iter()
        .find_map(|key| update.get(key)?.get("chat"))?;
    let field = |key: &str| chat.get(key).and_then(Value::as_str);
    Some(Chat {
        id: chat.get("id")?.as_i64()?.to_string(),
        name: field("title")
            .or_else(|| field("first_name"))
            .or_else(|| field("username"))
            .unwrap_or("?")
            .to_owned(),
        kind: field("type").unwrap_or("?").to_owned(),
    })
}

fn call(api: &str, token: &str, method: &str, body: &Value) -> Result<Value, String> {
    call_unscrubbed(api, token, method, body).map_err(|error| {
        if token.is_empty() {
            error
        } else {
            error.replace(token, "[redacted]")
        }
    })
}

fn call_unscrubbed(api: &str, token: &str, method: &str, body: &Value) -> Result<Value, String> {
    let mut response = webhook::send_json(
        &format!("{api}/bot{token}/{method}"),
        &body.to_string(),
        TIMEOUT,
    )
    .map_err(|e| format!("Telegram {method} failed: {e}"))?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("Telegram {method} answer could not be read: {e}"))?;
    let reply: Value = serde_json::from_str(&text)
        .map_err(|_| format!("Telegram {method} answered HTTP {status} without JSON"))?;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    } else {
        let reason = reply
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("no description");
        Err(format!(
            "Telegram refused {method} (HTTP {status}): {reason}"
        ))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/infra/telegram.rs"]
mod tests;
