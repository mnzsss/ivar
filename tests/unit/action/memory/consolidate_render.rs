#![allow(clippy::unwrap_used)]

use super::*;
use crate::domain::memory_page::PageMeta;
use crate::store::memory::conversations::{ConversationHead, ConversationRow};
use crate::store::memory::page_source::parse_page;

fn head() -> ConversationHead {
    ConversationHead {
        native_session_id: "n1".into(),
        session_id: "s1".into(),
        first_at: "2026-11-01T10:00:00.000000Z".into(),
        last_at: "2026-11-01T10:09:00.000000Z".into(),
        last_id: 9,
        ended: true,
    }
}

fn row(id: i64, event: &str, origin: &str, importance: u8, body: &str) -> ConversationRow {
    ConversationRow {
        id,
        source_event: event.into(),
        occurred_at: format!("2026-11-01T10:0{id}:00.000000Z"),
        session_id: "s1".into(),
        native_session_id: "n1".into(),
        provider: "claude-code".into(),
        feature: "memory-consolidation".into(),
        repo: "ivar".into(),
        origin: origin.into(),
        importance,
        body: body.into(),
    }
}

#[test]
fn a_conversation_with_work_renders_a_valid_session_page() {
    let rows = [
        row(1, "session.prompted", "harness", 1, "<system-reminder>x"),
        row(
            2,
            "session.prompted",
            "user",
            5,
            "Quero fazer a fase 6 do mapa\ncom detalhes",
        ),
        row(
            3,
            "session.tool",
            "",
            4,
            r#"{"tool":"Edit","input":{"file_path":"src/a.rs"},"result":""}"#,
        ),
        row(
            4,
            "session.tool",
            "",
            6,
            r#"{"tool":"Bash","input":{"command":"cargo test"},"result":"error: 1 failed\nmore"}"#,
        ),
    ];
    let attached = [row(5, "delivery.pr", "", 4, r#"{"url":"https://x/pr/1"}"#)];
    let page = render(
        &Conversation {
            head: &head(),
            rows: &rows,
            attached: &attached,
        },
        &PageMeta::default(),
    )
    .unwrap();

    let parsed = parse_page(".ivar/memory/session/n1.md", &page).unwrap();
    assert_eq!(parsed.title, "Quero fazer a fase 6 do mapa");
    assert!(parsed.meta.expires_at.is_none());
    assert!(page.contains("`src/a.rs`"));
    assert!(page.contains("Bash: error: 1 failed"));
    assert!(page.contains("delivery.pr"));
    assert!(page.contains("n1"));
    assert!(page.contains("memory-consolidation"));
}

#[test]
fn pinned_and_expires_at_are_carried_over() {
    let rows = [row(2, "session.prompted", "user", 5, "oi")];
    let preserved = PageMeta {
        pinned: true,
        expires_at: Some("2027-01-01T00:00:00Z".into()),
        ..PageMeta::default()
    };
    let page = render(
        &Conversation {
            head: &head(),
            rows: &rows,
            attached: &[],
        },
        &preserved,
    )
    .unwrap();
    let parsed = parse_page(".ivar/memory/session/n1.md", &page).unwrap();
    assert!(parsed.meta.pinned);
    assert_eq!(
        parsed.meta.expires_at.as_deref(),
        Some("2027-01-01T00:00:00Z")
    );
}

#[test]
fn a_conversation_without_importance_four_renders_nothing() {
    let rows = [
        row(1, "session.started", "", 1, "{}"),
        row(2, "session.tool", "", 2, r#"{"tool":"Read","path":"a"}"#),
    ];
    assert!(
        render(
            &Conversation {
                head: &head(),
                rows: &rows,
                attached: &[]
            },
            &PageMeta::default()
        )
        .is_none()
    );
}

#[test]
fn without_a_user_prompt_the_title_names_the_provider_and_day() {
    let rows = [row(
        3,
        "session.tool",
        "",
        4,
        r#"{"tool":"Write","input":{"filePath":"b.md"},"result":""}"#,
    )];
    let page = render(
        &Conversation {
            head: &head(),
            rows: &rows,
            attached: &[],
        },
        &PageMeta::default(),
    )
    .unwrap();
    assert_eq!(
        parse_page(".ivar/memory/session/n1.md", &page)
            .unwrap()
            .title,
        "Session claude-code 2026-11-01"
    );
}
