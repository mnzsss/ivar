#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::FakeTelegram;

const TOKEN: &str = "123:secret";

#[test]
fn get_me_returns_the_bot_username() {
    let fake = FakeTelegram::start(vec![(
        200,
        r#"{"ok":true,"result":{"id":1,"is_bot":true,"username":"ivar_bot"}}"#,
    )]);
    assert_eq!(get_me(&fake.api, TOKEN).unwrap(), "ivar_bot");
    assert!(fake.requests()[0].starts_with("POST /bot123:secret/getMe "));
}

#[test]
fn a_refused_call_names_the_reason_without_the_token() {
    let fake = FakeTelegram::start(vec![(
        401,
        r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#,
    )]);
    let error = get_me(&fake.api, TOKEN).unwrap_err();
    fake.requests();
    assert!(error.contains("getMe"), "{error}");
    assert!(error.contains("Unauthorized"), "{error}");
    assert!(!error.contains(TOKEN), "{error}");
}

#[test]
fn a_transport_failure_never_carries_the_token() {
    let error = get_me("http://127.0.0.1:9", TOKEN).unwrap_err();
    assert!(!error.contains(TOKEN), "{error}");
}

#[test]
fn get_updates_lists_each_chat_once() {
    let fake = FakeTelegram::start(vec![(
        200,
        r#"{"ok":true,"result":[
            {"update_id":1,"message":{"chat":{"id":42,"type":"private","first_name":"Ana"}}},
            {"update_id":2,"message":{"chat":{"id":42,"type":"private","first_name":"Ana"}}},
            {"update_id":3,"my_chat_member":{"chat":{"id":-100123,"type":"supergroup","title":"Ops"}}},
            {"update_id":4,"channel_post":{"chat":{"id":-100456,"type":"channel","title":"Runs","username":"ivar_runs"}}}
        ]}"#,
    )]);
    let chats = get_updates(&fake.api, TOKEN).unwrap();
    fake.requests();
    let chat = |id: &str, name: &str, kind: &str| Chat {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
    };
    assert_eq!(
        chats,
        vec![
            chat("42", "Ana", "private"),
            chat("-100123", "Ops", "supergroup"),
            chat("-100456", "Runs", "channel"),
        ]
    );
}

#[test]
fn send_message_posts_the_chat_id_and_text() {
    let fake = FakeTelegram::start(vec![(200, r#"{"ok":true,"result":{"message_id":7}}"#)]);
    send_message(&fake.api, TOKEN, "-100123", "hello").unwrap();
    let request = &fake.requests()[0];
    assert!(
        request.starts_with("POST /bot123:secret/sendMessage "),
        "{request}"
    );
    assert!(request.contains(r#""chat_id":"-100123""#), "{request}");
    assert!(request.contains(r#""text":"hello""#), "{request}");
}

#[test]
fn send_card_posts_html_with_a_copy_button() {
    let fake = FakeTelegram::start(vec![(200, r#"{"ok":true,"result":{}}"#)]);
    send_card(&fake.api, TOKEN, "1", "<b>hi</b>", "hi", ("Copy", "do it")).unwrap();
    let request = &fake.requests()[0];
    assert!(request.contains(r#""parse_mode":"HTML""#), "{request}");
    assert!(
        request.contains(r#""copy_text":{"text":"do it"}"#),
        "{request}"
    );
}

#[test]
fn send_card_falls_back_to_plain_text_when_telegram_refuses_the_markup() {
    let fake = FakeTelegram::start(vec![
        (
            400,
            r#"{"ok":false,"error_code":400,"description":"Bad Request: can't parse entities"}"#,
        ),
        (200, r#"{"ok":true,"result":{}}"#),
    ]);
    send_card(
        &fake.api,
        TOKEN,
        "1",
        "<b>hi",
        "plain hi",
        ("Copy", "do it"),
    )
    .unwrap();
    let requests = fake.requests();
    assert!(!requests[1].contains("parse_mode"), "{}", requests[1]);
    assert!(
        requests[1].contains(r#""text":"plain hi""#),
        "{}",
        requests[1]
    );
    assert!(requests[1].contains(r#""copy_text""#), "{}", requests[1]);
}
