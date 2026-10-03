#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use anstream::ColorChoice;
use anstream::adapter::strip_str;

fn sample_failure() -> Failure {
    Failure::blocked("repo.dirty", "api has uncommitted changes")
        .expected("a clean worktree")
        .actual("3 modified files")
        .fix(FixAction::safe("commit", "commit the changes").command("git commit -a"))
        .fix(FixAction::unsafe_("discard", "discard the changes"))
}

#[test]
fn blocked_and_failed_render_different_labels() {
    let blocked = Failure::blocked("repo.dirty", "dirty");
    let failed = Failure::failed("repo.dirty", "dirty");
    assert_eq!(blocked.label(), "blocked:");
    assert_eq!(failed.label(), "error:");
}

#[test]
fn human_form_orders_fixes_and_marks_the_unsafe_one() {
    let failure = sample_failure();
    let mut buf = Vec::new();
    failure.write_human(&mut buf).unwrap();
    let rendered = String::from_utf8(buf).unwrap();

    let plain = "blocked: api has uncommitted changes\n  expected: a clean worktree\n  actual:   3 modified files\n  try:\n    1. commit the changes\n       $ git commit -a\n    2. discard the changes (needs you)\n";
    assert_eq!(strip_str(&rendered).to_string(), plain);
    assert!(rendered.contains("\x1b[31mblocked:\x1b[0m"));
    assert!(rendered.contains("\x1b[2mexpected:\x1b[0m"));
    assert!(rendered.contains("\x1b[2mactual:\x1b[0m"));
    assert!(rendered.contains("\x1b[2mtry:\x1b[0m"));
    assert!(rendered.contains("\x1b[36m$\x1b[0m"));
    assert!(rendered.contains("\x1b[33m(needs you)\x1b[0m"));
}

#[test]
fn failure_write_human_renders_styled_output_matching_stripped_plain() {
    let failure = sample_failure();
    let mut buf = Vec::new();
    failure.write_human(&mut buf).unwrap();
    let rendered = String::from_utf8(buf).unwrap();

    let plain = "blocked: api has uncommitted changes\n  expected: a clean worktree\n  actual:   3 modified files\n  try:\n    1. commit the changes\n       $ git commit -a\n    2. discard the changes (needs you)\n";
    assert_eq!(strip_str(&rendered).to_string(), plain);
}

#[test]
fn values_never_sit_inside_style_spans() {
    let failure = sample_failure();
    let mut buf = Vec::new();
    failure.write_human(&mut buf).unwrap();
    let rendered = String::from_utf8(buf).unwrap();

    for value in [
        "api has uncommitted changes",
        "a clean worktree",
        "3 modified files",
        "commit the changes",
        "git commit -a",
        "discard the changes",
    ] {
        assert!(
            rendered.contains(value),
            "value `{value}` was broken up or painted"
        );
    }
}

#[test]
fn auto_stream_with_color_choice_never_emits_no_escape_bytes() {
    let failure = sample_failure();
    let mut out = Vec::new();
    {
        let mut stream = anstream::AutoStream::new(&mut out, ColorChoice::Never);
        failure.write_human(&mut stream).unwrap();
    }
    assert!(
        !out.contains(&0x1b),
        "AutoStream with ColorChoice::Never must not write escape byte 0x1b"
    );
    let plain = "blocked: api has uncommitted changes\n  expected: a clean worktree\n  actual:   3 modified files\n  try:\n    1. commit the changes\n       $ git commit -a\n    2. discard the changes (needs you)\n";
    assert_eq!(String::from_utf8(out).unwrap(), plain);
}

#[test]
fn warning_write_human_renders_styled_output_and_strips_cleanly() {
    let warning = Warning::new("repo.unreachable", "api", "remote did not answer");
    let mut buf = Vec::new();
    warning.write_human(&mut buf).unwrap();
    let rendered = String::from_utf8(buf).unwrap();

    assert_eq!(
        strip_str(&rendered).to_string(),
        "warning: api: remote did not answer\n"
    );
    assert!(rendered.starts_with("\x1b[33mwarning:\x1b[0m"));
}

#[test]
fn paint_wraps_text_with_style_and_resets() {
    let s = paint(DANGER, "danger text");
    assert_eq!(s, "\x1b[31mdanger text\x1b[0m");
}

#[test]
fn empty_optional_fields_stay_out_of_the_json() {
    let json = serde_json::to_string(&Failure::blocked("a.b", "c")).unwrap();
    assert_eq!(
        json,
        r#"{"ok":false,"kind":"blocked","code":"a.b","what":"c"}"#
    );
}

#[test]
fn a_failed_failure_reports_its_kind_in_the_json() {
    let failure = Failure::failed("a.b", "c");
    let json = serde_json::to_value(&failure).unwrap();

    assert_eq!(json.get("ok"), Some(&serde_json::Value::Bool(false)));
    assert_eq!(json.get("kind"), Some(&serde_json::json!("failed")));
    assert!(json.get("status").is_none(), "{json}");
    assert_eq!(failure.status, Status::Failed);
}

#[test]
fn a_report_with_warnings_is_not_clean() {
    #[derive(Debug, Serialize)]
    struct Synced {
        repos: u8,
    }

    let mut report = Report::new(Synced { repos: 3 });
    assert!(report.is_clean());
    report.warn(Warning::new(
        "repo.unreachable",
        "api",
        "remote did not answer",
    ));
    assert!(!report.is_clean());

    // The value flattens, so --json sees one object, not a nested wrapper.
    let json = serde_json::to_string(&report).unwrap();
    assert_eq!(
        json,
        r#"{"ok":true,"repos":3,"warnings":[{"code":"repo.unreachable","subject":"api","what":"remote did not answer"}]}"#
    );
}

#[test]
fn a_report_inlines_the_outcome_beside_the_envelope_flag() {
    #[derive(Serialize)]
    struct Outcome {
        feature: &'static str,
    }

    let rendered = serde_json::to_string(&Report::with_warnings(
        Outcome { feature: "api" },
        vec![Warning::new("x.y", "api", "nope")],
    ))
    .unwrap();

    assert!(
        rendered.starts_with(r#"{"ok":true,"feature":"api","warnings":["#),
        "{rendered}"
    );
}

#[test]
fn an_outcome_carrying_its_own_ok_key_is_refused_rather_than_rendered() {
    #[derive(Serialize)]
    struct Colliding {
        ok: bool,
    }

    let error = serde_json::to_string(&Report::new(Colliding { ok: false })).unwrap_err();

    assert!(error.to_string().contains("`ok`"), "{error}");
}

#[test]
fn failure_with_source_span_serializes_location_in_json_and_skips_text_and_label() {
    let span = SourceSpan::new(
        "ivar.json",
        "{\n  \"name\": Foo,\n}\n",
        2,
        11,
        "expected value",
    );
    let failure = Failure::failed("json.parse_failed", "invalid JSON syntax").at(span);
    let json = serde_json::to_string(&failure).unwrap();
    assert!(json.contains(r#""location":{"path":"ivar.json","line":2,"column":11}"#));
    assert!(!json.contains("expected value"));
    assert!(!json.contains("Foo"));
}

#[test]
fn failure_without_source_span_omits_location_key_in_json() {
    let failure = Failure::failed("general.error", "something failed");
    let json = serde_json::to_string(&failure).unwrap();
    assert!(!json.contains("location"));
}

#[test]
fn failure_with_source_span_renders_exact_human_snippet_layout() {
    let source_text = "{\n  \"name\": Foo,\n}\n";
    let span = SourceSpan::new("ivar.json", source_text, 2, 11, "expected value");
    let failure = Failure::failed("json.parse_failed", "syntax error in ivar.json")
        .expected("valid JSON")
        .actual("expected value at line 2 column 11")
        .at(span);
    let mut out = Vec::new();
    failure.write_human(&mut out).unwrap();
    let rendered = String::from_utf8(out).unwrap();
    let stripped = anstream::adapter::strip_str(&rendered).to_string();

    let expected = "\
error: syntax error in ivar.json
 --> ivar.json:2:11
  |
2 |   \"name\": Foo,
  |           ^^^ expected value
  expected: valid JSON
  actual:   expected value at line 2 column 11
";
    assert_eq!(stripped, expected);
}
