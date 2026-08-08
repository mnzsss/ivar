//! The error envelope, and the warning channel that sits beside it.
//!
//! Every failure the binary reports renders through [`Failure`]. That is the
//! contract: `--json` consumers and the human surface see the same value, so the
//! two cannot drift.
//!
//! Two distinctions carry the design.
//!
//! [`Status::Blocked`] versus [`Status::Failed`] is "refused before anything
//! happened" versus "broke in flight". It is what tells a caller — often an agent
//! — whether retrying is safe.
//!
//! [`FixAction::safe`] is what lets an agent recover on its own without being
//! handed permission to force-push. `true` means it may run this unattended;
//! `false` means the action can lose work or touch a remote, so a human decides.
//!
//! Warnings are **not** a severity level of error. A verb crossing eight repos
//! where one has uncommitted changes returns seven successes and one
//! [`Warning`] — inside `Ok`. [`Failure`] is reserved for "the whole operation is
//! unsalvageable".
//!
//! Colour lives here too, as [`Palette`], for one reason: the layout of a
//! failure must have exactly **one** code path. A second, colour-aware renderer
//! elsewhere would be a copy of this module's line ordering that drifts from it
//! the first time either side is edited. So the layout stays here and takes a
//! palette; [`Palette::plain`] is byte-for-byte what it always was, and colour
//! is decoration applied around the same `writeln!` calls, never a second pass
//! over a different shape. `infra::term` decides *whether* to colour; this
//! module decides *what* the paint means.
//!
//! Module error types live with their module, as `thiserror` enums, and convert
//! here via `From`. That conversion is where a mechanical error acquires a code
//! and a fix action, so it belongs to the module that knows what went wrong — not
//! to this one.

use std::borrow::Cow;
use std::fmt;
use std::io;

use serde::Serialize;

/// Which roles the human surface paints, and whether it paints at all.
///
/// Hand-rolled SGR rather than a colour crate. The whole vocabulary is the five
/// constants below, they have no edge cases at this size, and a binary whose
/// pitch is "read the source and check" is the wrong place to spend a
/// dependency on `"\x1b[31m"`. See [`crate::infra::term`] for the decision of
/// *whether* to emit any of it.
///
/// [`Palette::plain`] must stay byte-identical to an unpainted render — there is
/// a test for exactly that, because it is what lets every existing
/// byte-for-byte assertion keep its meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    colour: bool,
}

/// Reset every attribute. Emitted after each painted span so a span never
/// leaks into the text after it — including when a write is cut short.
const RESET: &str = "\x1b[0m";
/// Red: the failure label itself.
const RED: &str = "\x1b[31m";
/// Yellow: something that needs a human.
const YELLOW: &str = "\x1b[33m";
/// Dim: structural labels that guide the eye but carry no news.
const DIM: &str = "\x1b[2m";
/// Cyan: a command the reader can copy and run.
const CYAN: &str = "\x1b[36m";

impl Palette {
    /// A painting palette.
    #[must_use]
    pub const fn colour() -> Self {
        Self { colour: true }
    }

    /// No escape codes at all. The default, and what every byte-for-byte test
    /// asserts against.
    #[must_use]
    pub const fn plain() -> Self {
        Self { colour: false }
    }

    /// Build from an already-made decision — typically
    /// [`crate::infra::term::colour_for`].
    #[must_use]
    pub const fn from_decision(colour: bool) -> Self {
        if colour {
            Self::colour()
        } else {
            Self::plain()
        }
    }

    /// Whether this palette emits anything.
    #[must_use]
    pub const fn is_colour(&self) -> bool {
        self.colour
    }

    /// Wrap `text` in `code`, or hand it back untouched when plain.
    fn paint<'a>(&self, code: &str, text: &'a str) -> Cow<'a, str> {
        if self.colour {
            Cow::Owned(format!("{code}{text}{RESET}"))
        } else {
            Cow::Borrowed(text)
        }
    }

    /// The failure label — `blocked:` / `error:`.
    fn danger<'a>(&self, text: &'a str) -> Cow<'a, str> {
        self.paint(RED, text)
    }

    /// Something a human has to decide: the `warning:` label, `(needs you)`.
    fn caution<'a>(&self, text: &'a str) -> Cow<'a, str> {
        self.paint(YELLOW, text)
    }

    /// A structural label: `expected:`, `actual:`, `try:`.
    fn muted<'a>(&self, text: &'a str) -> Cow<'a, str> {
        self.paint(DIM, text)
    }

    /// A runnable command.
    fn command<'a>(&self, text: &'a str) -> Cow<'a, str> {
        self.paint(CYAN, text)
    }
}

impl Default for Palette {
    /// Plain. Colour is something a surface opts into after asking
    /// `infra::term`, never a default that leaks into a pipe.
    fn default() -> Self {
        Self::plain()
    }
}

/// Whether a failure happened before or after the operation began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// A precondition was refused and nothing was mutated. Retrying after the
    /// fix action is safe.
    Blocked,
    /// The operation began and then failed. Some work may have landed; whether a
    /// retry is safe depends on the fix actions.
    Failed,
}

/// A concrete way out of a [`Failure`].
///
/// Ordered most-recommended first by whoever builds the failure.
#[derive(Debug, Clone, Serialize)]
pub struct FixAction {
    /// Stable, machine-matchable identifier. Never localised, never reworded.
    pub code: &'static str,
    /// One sentence, imperative, addressed to whoever has to act.
    pub what: String,
    /// The command that performs it, if there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// `true`: an agent may run this unattended. `false`: it can lose work or
    /// touch a remote, so a human decides.
    pub safe: bool,
}

impl FixAction {
    /// A fix an agent may take on its own.
    #[must_use]
    pub fn safe(code: &'static str, what: impl Into<String>) -> Self {
        Self {
            code,
            what: what.into(),
            command: None,
            safe: true,
        }
    }

    /// A fix that can lose work or touch a remote. Needs a human.
    #[must_use]
    pub fn unsafe_(code: &'static str, what: impl Into<String>) -> Self {
        Self {
            code,
            what: what.into(),
            command: None,
            safe: false,
        }
    }

    /// Attach the command that performs this fix.
    #[must_use]
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }
}

/// The one shape every reported failure takes.
#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub status: Status,
    /// Stable, machine-matchable identifier, e.g. `hall.already_initialised`.
    pub code: &'static str,
    /// One sentence naming what went wrong, in the user's terms.
    pub what: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Ordered, most-recommended first. May be empty when there is genuinely
    /// nothing to suggest — an empty list is more honest than a vague one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix_actions: Vec<FixAction>,
    /// Structured context for a machine reader. Never required to understand the
    /// failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl Failure {
    /// A precondition was refused. Nothing was mutated.
    #[must_use]
    pub fn blocked(code: &'static str, what: impl Into<String>) -> Self {
        Self::new(Status::Blocked, code, what)
    }

    /// The operation began and then failed.
    #[must_use]
    pub fn failed(code: &'static str, what: impl Into<String>) -> Self {
        Self::new(Status::Failed, code, what)
    }

    fn new(status: Status, code: &'static str, what: impl Into<String>) -> Self {
        Self {
            status,
            code,
            what: what.into(),
            expected: None,
            actual: None,
            fix_actions: Vec::new(),
            details: None,
        }
    }

    /// Record what the operation required.
    #[must_use]
    pub fn expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    /// Record what it found instead.
    #[must_use]
    pub fn actual(mut self, actual: impl Into<String>) -> Self {
        self.actual = Some(actual.into());
        self
    }

    /// Append a fix action. Call order is the recommendation order.
    #[must_use]
    pub fn fix(mut self, action: FixAction) -> Self {
        self.fix_actions.push(action);
        self
    }

    /// Attach structured context for machine readers.
    #[must_use]
    pub fn details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Render the full human form, unpainted: the summary, the mismatch, then
    /// the ordered fixes.
    ///
    /// Equivalent to [`write_painted`](Self::write_painted) with
    /// [`Palette::plain`], and kept as its own name because most callers — and
    /// every byte-for-byte test — want exactly that.
    pub fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        self.write_painted(w, &Palette::plain())
    }

    /// The one layout for a failure. `palette` decorates it; it never changes
    /// which lines appear, their order, or their text.
    ///
    /// Only the labels are painted. `what`, `expected`, `actual` and each fix's
    /// sentence are *values*, and a value never gets an escape code inside it —
    /// that is what keeps this consistent with the `--json` surface, where the
    /// same strings appear raw.
    pub fn write_painted(&self, w: &mut impl io::Write, palette: &Palette) -> io::Result<()> {
        writeln!(w, "{} {}", palette.danger(self.label()), self.what)?;
        if let Some(expected) = &self.expected {
            writeln!(w, "  {} {expected}", palette.muted("expected:"))?;
        }
        if let Some(actual) = &self.actual {
            writeln!(w, "  {}   {actual}", palette.muted("actual:"))?;
        }
        if !self.fix_actions.is_empty() {
            writeln!(w, "  {}", palette.muted("try:"))?;
            for (index, action) in self.fix_actions.iter().enumerate() {
                // The space belongs outside the paint: a trailing space inside a
                // coloured span is invisible but still styled, and shows up as a
                // stray background cell on some terminals.
                let needs_human = if action.safe {
                    String::new()
                } else {
                    format!(" {}", palette.caution("(needs you)"))
                };
                writeln!(w, "    {}. {}{needs_human}", index + 1, action.what)?;
                if let Some(command) = &action.command {
                    writeln!(w, "       {} {command}", palette.command("$"))?;
                }
            }
        }
        Ok(())
    }

    /// The word this failure's status renders as. The single source for both
    /// [`fmt::Display`] and [`write_painted`](Self::write_painted), so the
    /// painted and unpainted forms cannot disagree about it.
    const fn label(&self) -> &'static str {
        match self.status {
            Status::Blocked => "blocked:",
            Status::Failed => "error:",
        }
    }
}

impl fmt::Display for Failure {
    /// The one-line summary. The full form is [`Failure::write_human`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.label(), self.what)
    }
}

impl std::error::Error for Failure {}

/// One item of a batch had a problem; everything else ran.
///
/// This is ordinary data returned inside `Ok`, never routed through `Result`.
#[derive(Debug, Clone, Serialize)]
pub struct Warning {
    /// Stable, machine-matchable identifier.
    pub code: &'static str,
    /// What the warning is about — a repo name, a feature, a session id.
    pub subject: String,
    /// One sentence saying what happened to it.
    pub what: String,
}

impl Warning {
    #[must_use]
    pub fn new(code: &'static str, subject: impl Into<String>, what: impl Into<String>) -> Self {
        Self {
            code,
            subject: subject.into(),
            what: what.into(),
        }
    }
}

impl Warning {
    /// The one layout for a warning. As with [`Failure::write_painted`], only
    /// the label is painted — subject and text are values.
    pub fn write_painted(&self, w: &mut impl io::Write, palette: &Palette) -> io::Result<()> {
        writeln!(
            w,
            "{} {}: {}",
            palette.caution("warning:"),
            self.subject,
            self.what
        )
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "warning: {}: {}", self.subject, self.what)
    }
}

/// What a verb that crosses the hall returns: the value, plus what needs
/// attention.
#[derive(Debug, Clone, Serialize)]
pub struct Report<T> {
    #[serde(flatten)]
    pub value: T,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Warning>,
}

impl<T> Report<T> {
    /// A clean run.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            value,
            warnings: Vec::new(),
        }
    }

    /// A run where some items needed attention.
    #[must_use]
    pub fn with_warnings(value: T, warnings: Vec<Warning>) -> Self {
        Self { value, warnings }
    }

    /// Append one warning.
    pub fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Whether anything needs attention. Callers use this to pick an exit code.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Replace the value, keeping the warnings.
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Report<U> {
        Report {
            value: f(self.value),
            warnings: self.warnings,
        }
    }
}

/// How a value renders for a human.
///
/// Every action's outcome implements this, which is what lets the binary have
/// **one** rendering path rather than one per verb: `--json` serializes the
/// value and the human surface calls this on the same value, so the two cannot
/// acquire separate formatting logic. See ARCHITECTURE.md, "1. `action` is the
/// unit, and it has one output shape".
///
/// Colour is not applied here — that belongs to the surface doing the writing,
/// so implementations stay testable byte-for-byte.
pub trait WriteHuman {
    /// Write the human form. One line for a simple outcome; a short block for
    /// one that reports several facts.
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()>;
}

/// The return type of every action.
pub type Outcome<T> = Result<Report<T>, Failure>;

#[cfg(test)]
mod tests {
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
}
