#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use serde_json::{Value, json};

use super::*;

const HALL: &str = "/hall";

fn fact(source_event: &str, at: &str, importance: u8, body: Value) -> ObservationFact {
    ObservationFact {
        source_event: source_event.to_owned(),
        occurred_at: at.to_owned(),
        importance,
        body,
    }
}

fn tool_write(at: &str, path: &str, kind: &str, action: &str) -> ObservationFact {
    fact(
        "memory.page_written",
        at,
        4,
        json!({"path": path, "kind": kind, "action": action, "sha256": "h"}),
    )
}

fn captured(at: &str, tool: &str, input: Value, importance: u8) -> ObservationFact {
    fact(
        "session.tool",
        at,
        importance,
        json!({"tool": tool, "input": input, "result": ""}),
    )
}

fn bash(at: &str, command: &str) -> ObservationFact {
    captured(at, "Bash", json!({"command": command}), 4)
}

fn query(at: &str) -> ObservationFact {
    fact(
        "session.tool",
        at,
        2,
        json!({"tool": "mcp__acme-ivar__memory_query", "path": ""}),
    )
}

#[test]
fn each_provider_write_key_under_docs_memory_is_a_page_target() {
    for key in ["file_path", "filePath", "path"] {
        assert_eq!(
            memory_page_target(&json!({key: "/hall/docs/memory/gotcha/x.md"}), HALL),
            Some(("docs/memory/gotcha/x.md".to_owned(), "gotcha".to_owned())),
            "{key}"
        );
    }
    for outside in [
        "/hall/docs/memory/session/x.md",
        "/hall/docs/memory/nope/x.md",
        "/hall/docs/other/x.md",
        "/elsewhere/docs/memory/gotcha/x.md",
        "/hall/docs/memory/gotcha/deep/x.md",
        "/hall/docs/memory/gotcha/x.txt",
    ] {
        assert_eq!(
            memory_page_target(&json!({"file_path": outside}), HALL),
            None,
            "{outside}"
        );
    }
}

#[test]
fn a_hall_relative_write_path_is_a_page_target_under_the_same_confinement() {
    assert_eq!(
        memory_page_target(&json!({"file_path": "docs/memory/gotcha/x.md"}), HALL),
        Some(("docs/memory/gotcha/x.md".to_owned(), "gotcha".to_owned()))
    );
    for outside in [
        "../x",
        "../docs/memory/gotcha/x.md",
        "docs/memory/../x.md",
        "docs/memory/gotcha/../x.md",
        "docs/memory/nope/x.md",
        "docs/memory/session/x.md",
        "docs/memory/gotcha/deep/x.md",
    ] {
        assert_eq!(
            memory_page_target(&json!({"file_path": outside}), HALL),
            None,
            "{outside}"
        );
    }
}

#[test]
fn tool_writes_and_captured_writes_merge_by_path_and_the_last_action_wins() {
    let facts = [
        tool_write(
            "2026-11-05T10:00:00Z",
            "docs/memory/decision/a.md",
            "decision",
            "written",
        ),
        captured(
            "2026-11-05T10:01:00Z",
            "Edit",
            json!({"file_path": "/hall/docs/memory/decision/a.md"}),
            4,
        ),
        captured(
            "2026-11-05T10:02:00Z",
            "write",
            json!({"filePath": "/hall/docs/memory/gotcha/b.md"}),
            4,
        ),
        tool_write(
            "2026-11-05T10:03:00Z",
            "docs/memory/decision/a.md",
            "decision",
            "patched",
        ),
        captured(
            "2026-11-05T10:04:00Z",
            "Read",
            json!({"file_path": "/hall/docs/memory/concept/c.md"}),
            2,
        ),
        fact(
            "session.tool",
            "2026-11-05T10:05:00Z",
            4,
            Value::String("{\"tool\":\"Write\",\"input\":{\"content\":\"…".to_owned()),
        ),
        tool_write(
            "2026-11-05T10:06:00Z",
            "docs/memory/update/d.md",
            "update",
            "deleted",
        ),
        captured(
            "2026-11-05T10:07:00Z",
            "Write",
            json!({"file_path": "/hall/docs/memory/update/d.md"}),
            4,
        ),
    ];

    let written = written_pages(&facts, HALL);

    assert_eq!(
        written,
        vec![
            WrittenPage {
                path: "docs/memory/decision/a.md".to_owned(),
                kind: "decision".to_owned(),
                action: PageAction::Patched,
                source: WriteSource::Tool,
            },
            WrittenPage {
                path: "docs/memory/gotcha/b.md".to_owned(),
                kind: "gotcha".to_owned(),
                action: PageAction::Written,
                source: WriteSource::Capture,
            },
            WrittenPage {
                path: "docs/memory/update/d.md".to_owned(),
                kind: "update".to_owned(),
                action: PageAction::Written,
                source: WriteSource::Capture,
            },
        ]
    );
}

#[test]
fn a_cut_body_still_yields_the_page_when_its_path_survived() {
    let raw = "{\"tool\":\"Edit\",\"input\":{\"file_path\":\"/hall/docs/memory/procedure/p.md\",\"new_string\":\"…";
    let facts = [fact("session.tool", "t", 4, Value::String(raw.to_owned()))];
    assert_eq!(
        written_pages(&facts, HALL)[0].path,
        "docs/memory/procedure/p.md"
    );
}

#[test]
fn a_cut_body_with_a_hall_relative_path_still_yields_the_page() {
    let raw =
        "{\"tool\":\"Write\",\"input\":{\"file_path\":\"docs/memory/gotcha/g.md\",\"content\":\"…";
    let facts = [fact("session.tool", "t", 4, Value::String(raw.to_owned()))];
    assert_eq!(
        written_pages(&facts, HALL)[0].path,
        "docs/memory/gotcha/g.md"
    );
}

#[test]
fn a_failed_tool_without_a_gotcha_and_a_feature_without_an_update_are_gaps() {
    let facts = [captured(
        "2026-11-05T10:00:00Z",
        "Bash",
        json!({"command": "cargo test"}),
        6,
    )];

    let found: Vec<GapKind> = gaps(&facts, &[], "login", false)
        .iter()
        .map(|g| g.kind)
        .collect();

    assert_eq!(found, [GapKind::Gotcha, GapKind::Update]);
}

#[test]
fn written_gotcha_and_update_pages_close_their_gaps_but_a_deleted_one_does_not() {
    let facts = [
        captured(
            "2026-11-05T10:00:00Z",
            "Bash",
            json!({"command": "cargo test"}),
            6,
        ),
        tool_write(
            "2026-11-05T10:01:00Z",
            "docs/memory/gotcha/g.md",
            "gotcha",
            "deleted",
        ),
        tool_write(
            "2026-11-05T10:02:00Z",
            "docs/memory/update/150-x.md",
            "update",
            "written",
        ),
    ];
    let written = written_pages(&facts, HALL);

    let found: Vec<GapKind> = gaps(&facts, &written, "login", false)
        .iter()
        .map(|g| g.kind)
        .collect();

    assert_eq!(found, [GapKind::Gotcha]);
}

#[test]
fn analysis_without_query_counts_only_queries_before_the_captured_approval() {
    let approve = bash(
        "2026-11-05T11:00:00Z",
        "cd /hall && ivar plan approve login analysis",
    );
    let update = tool_write(
        "2026-11-05T09:00:00Z",
        "docs/memory/update/150-x.md",
        "update",
        "written",
    );
    let kinds = |facts: &[ObservationFact], approved: bool| -> Vec<GapKind> {
        let written = written_pages(facts, HALL);
        gaps(facts, &written, "login", approved)
            .iter()
            .map(|g| g.kind)
            .collect()
    };

    let before = [
        update.clone(),
        query("2026-11-05T10:00:00Z"),
        approve.clone(),
    ];
    let after = [
        update.clone(),
        approve.clone(),
        query("2026-11-05T12:00:00Z"),
    ];
    let by_cli = [
        update.clone(),
        bash("2026-11-05T10:00:00Z", "ivar memory query \"locks\" --json"),
        approve.clone(),
    ];
    let uncaptured_approval = [update.clone(), query("2026-11-05T12:00:00Z")];
    let no_query = [update.clone()];

    assert!(kinds(&before, true).is_empty());
    assert_eq!(kinds(&after, true), [GapKind::AnalysisWithoutQuery]);
    assert!(kinds(&by_cli, true).is_empty());
    assert!(kinds(&uncaptured_approval, true).is_empty());
    assert_eq!(kinds(&no_query, true), [GapKind::AnalysisWithoutQuery]);
    assert!(kinds(&no_query, false).is_empty());
}

#[test]
fn stale_pages_group_reasons_skip_written_pages_and_need_every_verdict() {
    let changed = [
        (
            "docs/memory/decision/a.md".to_owned(),
            "ivar".to_owned(),
            "src/a.rs".to_owned(),
        ),
        (
            "docs/memory/decision/a.md".to_owned(),
            "ivar".to_owned(),
            "src/b.rs".to_owned(),
        ),
        (
            "docs/memory/gotcha/w.md".to_owned(),
            "ivar".to_owned(),
            "src/a.rs".to_owned(),
        ),
    ];
    let broken = [(
        "docs/memory/concept/c.md".to_owned(),
        "ivar".to_owned(),
        "src/gone.rs".to_owned(),
        Some("gone".to_owned()),
    )];
    let written = written_pages(
        &[tool_write(
            "t",
            "docs/memory/gotcha/w.md",
            "gotcha",
            "patched",
        )],
        HALL,
    );

    let stale = stale_pages(&changed, &broken, &written);

    let paths: Vec<&str> = stale.iter().map(|p| p.path.as_str()).collect();
    assert_eq!(
        paths,
        ["docs/memory/concept/c.md", "docs/memory/decision/a.md"]
    );
    assert_eq!(stale[1].reasons.len(), 2);
    assert_eq!(
        serde_json::to_value(&stale[0].reasons[0]).unwrap(),
        json!({"reason": "broken_anchor", "repo": "ivar", "path": "src/gone.rs", "symbol": "gone"})
    );
    assert_eq!(
        missing_verdicts(
            &stale,
            &[("docs/memory/decision/a.md".to_owned(), Verdict::StillValid)]
        ),
        ["docs/memory/concept/c.md"]
    );
    assert!(
        missing_verdicts(
            &stale,
            &[
                ("docs/memory/decision/a.md".to_owned(), Verdict::StillValid),
                ("docs/memory/concept/c.md".to_owned(), Verdict::Updated),
            ]
        )
        .is_empty()
    );
}
