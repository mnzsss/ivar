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

use crate::error::{Failure, FixAction};

/// An option for multi-selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub id: String,
    pub description: Option<String>,
    pub path_if_any: String,
}

/// The confirmation seam. Implementations never decide *whether* to ask —
/// that is [`reporter`]'s job — they only ask and answer.
pub trait Confirm: fmt::Debug + Send + Sync {
    /// Ask `question` (with an optional `caveat` printed above it) and return
    /// whether the human answered yes. `true` only for an explicit `y`.
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure>;

    /// Prompt the human to choose zero or more options from `options`.
    /// Returns the chosen 0-based indices.
    fn select_many(&self, prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure>;
}

/// Never asks and never consents. A pipe is not consent.
#[derive(Debug)]
struct NonInteractive;

impl Confirm for NonInteractive {
    fn confirm(&self, _question: &str, _caveat: Option<&str>) -> Result<bool, Failure> {
        Ok(false)
    }

    fn select_many(&self, _prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        let mut opt_str = String::new();
        for o in options {
            let path_info = if o.path_if_any.is_empty() {
                String::new()
            } else {
                format!(" (--path {})", o.path_if_any)
            };
            if let Some(desc) = &o.description {
                opt_str.push_str(&format!("  - {}{path_info} — {desc}\n", o.id));
            } else {
                opt_str.push_str(&format!("  - {}{path_info}\n", o.id));
            }
        }
        Err(Failure::blocked(
            "skill.add.multiple_choices",
            format!(
                "repository contains multiple skills; select one by passing --path:\n{}",
                opt_str.trim_end()
            ),
        )
        .expected("a --path argument specifying which skill to install")
        .actual(format!("found {} skills", options.len()))
        .fix(FixAction::safe(
            "skill.add.specify_path",
            "Pass --path <path> to select a skill to install.",
        )))
    }
}

/// A fixed answer, for tests and for callers that already decided.
#[derive(Debug)]
struct Fixed {
    answer: bool,
    selection: Option<Vec<usize>>,
}

impl Confirm for Fixed {
    fn confirm(&self, _question: &str, _caveat: Option<&str>) -> Result<bool, Failure> {
        Ok(self.answer)
    }

    fn select_many(&self, _prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        match &self.selection {
            Some(indices) => Ok(indices.clone()),
            None => Ok((0..options.len()).collect()),
        }
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
        std::io::stdin().read_line(&mut answer).map_err(|source| {
            Failure::failed(
                "confirm.read_answer",
                format!("could not read your answer: {source}"),
            )
        })?;
        Ok(answer.trim().eq_ignore_ascii_case("y"))
    }

    fn select_many(&self, prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        let mut stderr = std::io::stderr().lock();
        writeln!(stderr, "{prompt}").map_err(|source| {
            Failure::failed(
                "confirm.write_prompt",
                format!("could not write the prompt: {source}"),
            )
        })?;
        for (i, opt) in options.iter().enumerate() {
            let desc_str = match &opt.description {
                Some(d) => format!(" — {d}"),
                None => String::new(),
            };
            writeln!(stderr, "  [{}] {}{desc_str}", i + 1, opt.id).map_err(|source| {
                Failure::failed(
                    "confirm.write_prompt",
                    format!("could not write options: {source}"),
                )
            })?;
        }
        write!(stderr, "Enter numbers (comma-separated) or \"all\": ").map_err(|source| {
            Failure::failed(
                "confirm.write_prompt",
                format!("could not write prompt line: {source}"),
            )
        })?;
        let _ = stderr.flush();

        let mut answer = String::new();
        let bytes_read = std::io::stdin().read_line(&mut answer).map_err(|source| {
            Failure::failed(
                "confirm.read_answer",
                format!("could not read your answer: {source}"),
            )
        })?;

        let trimmed = answer.trim();
        if trimmed.eq_ignore_ascii_case("all") {
            return Ok((0..options.len()).collect());
        }

        if bytes_read == 0 || trimmed.is_empty() {
            return Err(Failure::blocked(
                "confirm.no_answer",
                "no selection entered on stdin",
            ));
        }

        let mut selected = Vec::new();
        for part in trimmed.split(',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            let idx: usize = p.parse().map_err(|_| {
                Failure::blocked("confirm.invalid_selection", format!("invalid selection `{p}`"))
            })?;
            if idx == 0 || idx > options.len() {
                return Err(Failure::blocked(
                    "confirm.invalid_selection",
                    format!("selection index `{idx}` out of range (1..{})", options.len()),
                ));
            }
            selected.push(idx - 1);
        }
        Ok(selected)
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
/// the "yes" half of a prompt deterministically. Only test code constructs
/// it, so the library build sees it as dead.
#[must_use]
#[allow(dead_code)]
pub(crate) fn fixed(answer: bool) -> Arc<dyn Confirm> {
    Arc::new(Fixed {
        answer,
        selection: None,
    })
}

/// A confirmer that returns `selection` for multi-select, for tests.
#[must_use]
#[allow(dead_code)]
pub(crate) fn fixed_select(answer: bool, selection: Vec<usize>) -> Arc<dyn Confirm> {
    Arc::new(Fixed {
        answer,
        selection: Some(selection),
    })
}

#[cfg(test)]
#[path = "../../tests/unit/action/confirm.rs"]
mod tests;
