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
//! Module error types live with their module, as `thiserror` enums, and convert
//! here via `From`. That conversion is where a mechanical error acquires a code
//! and a fix action, so it belongs to the module that knows what went wrong — not
//! to this one.

use std::fmt;
use std::io;

use serde::Serialize;

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

    /// Render the full human form: the summary, the mismatch, then the ordered
    /// fixes.
    ///
    /// Colour is not applied here — that belongs to the surface doing the
    /// writing, so this stays testable byte-for-byte.
    pub fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "{self}")?;
        if let Some(expected) = &self.expected {
            writeln!(w, "  expected: {expected}")?;
        }
        if let Some(actual) = &self.actual {
            writeln!(w, "  actual:   {actual}")?;
        }
        if !self.fix_actions.is_empty() {
            writeln!(w, "  try:")?;
            for (index, action) in self.fix_actions.iter().enumerate() {
                let needs_human = if action.safe { "" } else { " (needs you)" };
                writeln!(w, "    {}. {}{needs_human}", index + 1, action.what)?;
                if let Some(command) = &action.command {
                    writeln!(w, "       $ {command}")?;
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for Failure {
    /// The one-line summary. The full form is [`Failure::write_human`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self.status {
            Status::Blocked => "blocked",
            Status::Failed => "error",
        };
        write!(f, "{label}: {}", self.what)
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
