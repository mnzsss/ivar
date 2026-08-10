#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn blocked_and_failed_render_different_labels() {
    assert_eq!(Failure::blocked("x.y", "nope").to_string(), "blocked: nope");
    assert_eq!(Failure::failed("x.y", "nope").to_string(), "error: nope");
}

#[test]
fn human_form_orders_fixes_and_marks_the_unsafe_one() {
    let failure = Failure::blocked("repo.dirty", "api has uncommitted changes")
        .expected("a clean worktree")
        .actual("3 modified files")
        .fix(FixAction::safe("commit", "commit the changes").command("git commit -a"))
        .fix(FixAction::unsafe_("discard", "discard the changes"));

    let mut out = Vec::new();
    failure.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "blocked: api has uncommitted changes\n\
             \x20 expected: a clean worktree\n\
             \x20 actual:   3 modified files\n\
             \x20 try:\n\
             \x20   1. commit the changes\n\
             \x20      $ git commit -a\n\
             \x20   2. discard the changes (needs you)\n"
    );
}

#[test]
fn empty_optional_fields_stay_out_of_the_json() {
    let json = serde_json::to_string(&Failure::blocked("a.b", "c")).unwrap();
    assert_eq!(json, r#"{"status":"blocked","code":"a.b","what":"c"}"#);
}

/// Strip every SGR sequence. Deliberately a separate, dumb implementation
/// rather than anything reused from the code under test — a stripper that
/// shared the writer's idea of an escape code could not catch a malformed
/// one.
fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Consume through the terminating 'm' of the CSI sequence.
            for inner in chars.by_ref() {
                if inner == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn sample_failure() -> Failure {
    Failure::blocked("repo.dirty", "api has uncommitted changes")
        .expected("a clean worktree")
        .actual("3 modified files")
        .fix(FixAction::safe("commit", "commit the changes").command("git commit -a"))
        .fix(FixAction::unsafe_("discard", "discard the changes"))
}

fn render(failure: &Failure, palette: &Palette) -> String {
    let mut out = Vec::new();
    failure.write_painted(&mut out, palette).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn a_plain_palette_is_byte_for_byte_the_unpainted_form() {
    let failure = sample_failure();

    let mut via_write_human = Vec::new();
    failure.write_human(&mut via_write_human).unwrap();

    assert_eq!(
        String::from_utf8(via_write_human).unwrap(),
        render(&failure, &Palette::plain()),
        "write_human must stay exactly Palette::plain, or every byte-for-byte \
             assertion in this crate silently changes meaning"
    );
}

#[test]
fn colour_adds_only_escape_codes_and_never_changes_the_text() {
    let failure = sample_failure();

    let painted = render(&failure, &Palette::colour());
    let plain = render(&failure, &Palette::plain());

    assert_ne!(painted, plain, "the colour palette painted nothing");
    assert_eq!(
        strip_ansi(&painted),
        plain,
        "colour altered the layout, not just its decoration"
    );
}

#[test]
fn every_painted_span_is_closed_by_a_reset() {
    let painted = render(&sample_failure(), &Palette::colour());

    // Every escape sequence is either an opening code or a reset, so a
    // balanced render has exactly twice as many as it has resets.
    let escapes = painted.matches("\x1b[").count();
    let resets = painted.matches(RESET).count();
    assert_eq!(
        escapes,
        resets * 2,
        "each painted span should be one opening code plus one reset; an \
             unbalanced count leaks colour into the text that follows"
    );
}

#[test]
fn values_never_carry_escape_codes() {
    let painted = render(&sample_failure(), &Palette::colour());

    // The value strings must appear verbatim, unpainted — the --json
    // surface shows these same strings raw, and the two must agree.
    for value in [
        "api has uncommitted changes",
        "a clean worktree",
        "3 modified files",
        "commit the changes",
        "git commit -a",
    ] {
        assert!(
            painted.contains(value),
            "value `{value}` was broken up or painted"
        );
    }
}

#[test]
fn the_unsafe_marker_keeps_its_space_outside_the_paint() {
    let painted = render(&sample_failure(), &Palette::colour());

    assert!(
        painted.contains(&format!(" {YELLOW}(needs you){RESET}")),
        "the separating space must precede the escape code, not sit inside it"
    );
}

#[test]
fn a_warning_paints_only_its_label() {
    let warning = Warning::new("repo.unreachable", "api", "remote did not answer");

    let mut plain = Vec::new();
    warning
        .write_painted(&mut plain, &Palette::plain())
        .unwrap();
    let plain = String::from_utf8(plain).unwrap();

    let mut painted = Vec::new();
    warning
        .write_painted(&mut painted, &Palette::colour())
        .unwrap();
    let painted = String::from_utf8(painted).unwrap();

    // The unpainted form is the Display form plus a newline: one wording,
    // so a caller using either cannot show the user something different.
    assert_eq!(plain, format!("{warning}\n"));
    assert_eq!(strip_ansi(&painted), plain);
    assert!(painted.starts_with(&format!("{YELLOW}warning:{RESET}")));
}

#[test]
fn a_plain_palette_is_the_default_so_a_pipe_never_gets_colour() {
    assert_eq!(Palette::default(), Palette::plain());
    assert!(!Palette::default().is_colour());
    assert!(Palette::from_decision(true).is_colour());
    assert!(!Palette::from_decision(false).is_colour());
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
        r#"{"repos":3,"warnings":[{"code":"repo.unreachable","subject":"api","what":"remote did not answer"}]}"#
    );
}
