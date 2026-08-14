//! The confirmation seam: whether a human said yes to a question.
//!
//! Two verbs delete or rewrite state — `ivar cleanup` and `ivar migrate` —
//! and both must *ask* before they act. The question used to be asked by a
//! `hall::ask` helper that checked `term::is_tty` itself; the problem with
//! that was the decision lived in the action layer, so an action could never
//! be told "this run is automated, do not ask" or "this test says yes".
//!
//! The seam is [`Confirm`], carried on [`Ctx`](crate::action::Ctx) like the
//! progress sink. `bin/ivar.rs` decides once, at startup, whether this run
//! may prompt at all (a `--json` run, a `$CI` run, or a non-tty run may not —
//! a pipe is not consent) and installs the answer; a test installs
//! [`fixed`] and gets a deterministic yes or no. Actions never decide whether
//! anyone is watching; they only ask.

use std::fmt;
use std::io::Write;
use std::sync::Arc;

use crate::error::Failure;

/// The confirmation seam. Implementations never decide *whether* to ask —
/// that is [`reporter`]'s job — they only ask and answer.
pub trait Confirm: fmt::Debug + Send + Sync {
    /// Ask `question` (with an optional `caveat` printed above it) and return
    /// whether the human answered yes. `true` only for an explicit `y`.
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure>;
}

/// Never asks and never consents. A pipe is not consent.
#[derive(Debug)]
struct NonInteractive;

impl Confirm for NonInteractive {
    fn confirm(&self, _question: &str, _caveat: Option<&str>) -> Result<bool, Failure> {
        Ok(false)
    }
}

/// A fixed answer, for tests and for callers that already decided.
#[derive(Debug)]
struct Fixed(bool);

impl Confirm for Fixed {
    fn confirm(&self, _question: &str, _caveat: Option<&str>) -> Result<bool, Failure> {
        Ok(self.0)
    }
}

/// The real interactive prompt: the question on stderr, the answer from
/// stdin, `true` only for an explicit `y`.
///
/// The prompt goes to stderr so that piping stdout — the machine surface —
/// never swallows the question, and `--json` output stays parseable.
#[derive(Debug)]
struct Interactive;

impl Confirm for Interactive {
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure> {
        let mut stderr = std::io::stderr().lock();
        if let Some(caveat) = caveat {
            writeln!(stderr, "{caveat}").map_err(|source| {
                Failure::failed(
                    "confirm.write_prompt",
                    format!("could not write the prompt: {source}"),
                )
            })?;
        }
        writeln!(stderr, "{question} [y/N] ").map_err(|source| {
            Failure::failed(
                "confirm.write_prompt",
                format!("could not write the prompt: {source}"),
            )
        })?;

        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|source| {
                Failure::failed(
                    "confirm.read_answer",
                    format!("could not read your answer: {source}"),
                )
            })?;
        Ok(answer.trim().eq_ignore_ascii_case("y"))
    }
}

/// Build the process's confirmer. `enabled` is the startup decision — a run
/// that may prompt. Disabled (the `--json`, `$CI`, and non-tty cases) installs
/// [`NonInteractive`], which answers `false` without reading anything.
#[must_use]
pub fn reporter(enabled: bool) -> Arc<dyn Confirm> {
    if enabled {
        Arc::new(Interactive)
    } else {
        Arc::new(NonInteractive)
    }
}

/// A confirmer that always answers `answer`, for tests and for callers that
/// already made the decision. This is the only way an action test can reach
/// the "yes" half of a prompt deterministically.
#[must_use]
pub(crate) fn fixed(answer: bool) -> Arc<dyn Confirm> {
    Arc::new(Fixed(answer))
}

#[cfg(test)]
#[path = "../../tests/unit/action/confirm.rs"]
mod tests;
