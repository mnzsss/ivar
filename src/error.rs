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
//! Colour lives here too, as style roles, for one reason: the layout of a
//! failure must have exactly **one** code path. A second, colour-aware renderer
//! elsewhere would be a copy of this module's line ordering that drifts from it
//! the first time either side is edited. So the layout stays here and uses
//! [`paint`] with style roles; stream filtering is handled at the stream
//! boundary via `anstream::AutoStream`, and colour is decoration applied around
//! the same `writeln!` calls, never a second pass over a different shape.
//! `infra::term` decides *whether* to colour; this module decides *what* the
//! paint means.
//!
//! Module error types live with their module, as `thiserror` enums, and convert
//! here via `From`. That conversion is where a mechanical error acquires a code
//! and a fix action, so it belongs to the module that knows what went wrong — not
//! to this one.

use std::fmt;
use std::io;

use anstyle::{AnsiColor, Effects, Style};
use serde::Serialize;
use serde::ser::Serializer;

/// Style roles for human CLI output.
pub const DANGER: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Red)));
pub const CAUTION: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Yellow)));
pub const MUTED: Style = Style::new().effects(Effects::DIMMED);
pub const COMMAND: Style = Style::new().fg_color(Some(anstyle::Color::Ansi(AnsiColor::Cyan)));
pub const HEADER: Style = Style::new().effects(Effects::BOLD);

/// Wrap `text` in `style` and reset after.
#[must_use]
pub fn paint(style: Style, text: &str) -> String {
    format!("{style}{text}{style:#}")
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
    /// Always `false` — the JSON surface's success flag.
    ok: bool,
    /// Drives the exit code and the human label; `"kind"` on the JSON surface.
    #[serde(rename(serialize = "kind"))]
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
            ok: false,
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

    /// Render the failure layout with anstyle style roles.
    ///
    /// Stream filtering (stripping styles on non-tty/Never) is performed
    /// by `anstream::AutoStream` at the stream boundary.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if `w` cannot be written to.
    pub fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "{} {}", paint(DANGER, self.label()), self.what)?;
        if let Some(expected) = &self.expected {
            writeln!(w, "  {} {expected}", paint(MUTED, "expected:"))?;
        }
        if let Some(actual) = &self.actual {
            writeln!(w, "  {}   {actual}", paint(MUTED, "actual:"))?;
        }
        if !self.fix_actions.is_empty() {
            writeln!(w, "  {}", paint(MUTED, "try:"))?;
            for (index, action) in self.fix_actions.iter().enumerate() {
                // The space belongs outside the paint: a trailing space inside a
                // coloured span is invisible but still styled, and shows up as a
                // stray background cell on some terminals.
                let needs_human = if action.safe {
                    String::new()
                } else {
                    format!(" {}", paint(CAUTION, "(needs you)"))
                };
                writeln!(w, "    {}. {}{needs_human}", index + 1, action.what)?;
                if let Some(command) = &action.command {
                    writeln!(w, "       {} {command}", paint(COMMAND, "$"))?;
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

    /// Render the warning layout with anstyle style roles.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if `w` cannot be written to.
    pub fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "{} {}: {}",
            paint(CAUTION, "warning:"),
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
#[derive(Debug, Clone)]
pub struct Report<T> {
    /// Always `true` — the JSON surface's success flag.
    ok: bool,
    pub value: T,
    pub warnings: Vec<Warning>,
}

/// The envelope's own key, which no outcome type may also serialize.
const ENVELOPE_FLAG: &str = "ok";

/// The envelope as it renders: the flag, the outcome's own fields inlined, and
/// the warnings when there are any.
#[derive(Serialize)]
struct Envelope<'a, T> {
    ok: bool,
    #[serde(flatten)]
    value: &'a T,
    #[serde(skip_serializing_if = "<[Warning]>::is_empty")]
    warnings: &'a [Warning],
}

/// The value's fields are inlined beside `ok`. A silent collision on `ok`
/// would flip a success into a failure for any reader that takes the last
/// duplicate key, so an outcome carrying one is refused rather than rendered.
impl<T: Serialize> Serialize for Report<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let probe = serde_json::to_value(&self.value).map_err(serde::ser::Error::custom)?;
        if matches!(&probe, serde_json::Value::Object(fields) if fields.contains_key(ENVELOPE_FLAG))
        {
            return Err(serde::ser::Error::custom(
                "an outcome type must not serialize an `ok` key: it collides with the report envelope's success flag",
            ));
        }
        Envelope {
            ok: self.ok,
            value: &self.value,
            warnings: &self.warnings,
        }
        .serialize(serializer)
    }
}

impl<T> Report<T> {
    /// A clean run.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self {
            ok: true,
            value,
            warnings: Vec::new(),
        }
    }

    /// A run where some items needed attention.
    #[must_use]
    pub fn with_warnings(value: T, warnings: Vec<Warning>) -> Self {
        Self {
            ok: true,
            value,
            warnings,
        }
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
            ok: true,
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
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] if `w` cannot be written to.
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()>;
}

/// The return type of every action.
pub type Outcome<T> = Result<Report<T>, Failure>;

#[cfg(test)]
#[path = "../tests/unit/error.rs"]
mod tests;
