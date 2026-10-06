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
use std::fmt::Write as _;
use std::sync::Arc;

use crate::error::{Failure, FixAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    pub id: String,
    pub description: Option<String>,
    pub path_if_any: String,
}

impl SelectOption {
    #[must_use]
    pub fn new(id: impl Into<String>, description: Option<impl Into<String>>) -> Self {
        Self {
            id: id.into(),
            description: description.map(Into::into),
            path_if_any: String::new(),
        }
    }
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

    /// Prompt the human to choose exactly one option from `options`.
    /// Returns the chosen 0-based index, or `None` if non-interactive, cancelled, or empty.
    fn select_one(&self, prompt: &str, options: &[SelectOption]) -> Result<Option<usize>, Failure>;

    /// Whether this confirmation seam allows interactive user input.
    fn is_interactive(&self) -> bool {
        false
    }
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
                let _ = writeln!(opt_str, "  - {}{path_info} — {desc}", o.id);
            } else {
                let _ = writeln!(opt_str, "  - {}{path_info}", o.id);
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

    fn select_one(
        &self,
        _prompt: &str,
        _options: &[SelectOption],
    ) -> Result<Option<usize>, Failure> {
        Ok(None)
    }

    fn is_interactive(&self) -> bool {
        false
    }
}

/// A fixed answer, for tests and for callers that already decided.
#[cfg(test)]
#[derive(Debug)]
struct Fixed {
    answer: bool,
    selection: Option<Vec<usize>>,
    selection_one: Option<usize>,
}

#[cfg(test)]
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

    fn select_one(
        &self,
        _prompt: &str,
        _options: &[SelectOption],
    ) -> Result<Option<usize>, Failure> {
        Ok(self.selection_one)
    }

    fn is_interactive(&self) -> bool {
        self.answer
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct FixedInteractive {
    pub(crate) answer: bool,
}

#[cfg(test)]
impl Confirm for FixedInteractive {
    fn confirm(&self, _question: &str, _caveat: Option<&str>) -> Result<bool, Failure> {
        Ok(self.answer)
    }

    fn select_many(&self, _prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        Ok((0..options.len()).collect())
    }

    fn select_one(
        &self,
        _prompt: &str,
        _options: &[SelectOption],
    ) -> Result<Option<usize>, Failure> {
        Ok(None)
    }

    fn is_interactive(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[must_use]
pub(crate) fn fixed_interactive(answer: bool) -> std::sync::Arc<dyn Confirm> {
    std::sync::Arc::new(FixedInteractive { answer })
}

/// The real interactive prompt: the question on stderr, the answer from
/// stdin, `true` only for an explicit `y`.
///
/// The prompt goes to stderr so that piping stdout — the machine surface —
/// never swallows the question, and `--json` output stays parseable.
#[derive(Debug)]
struct Interactive;

/// Format a list of [`SelectOption`]s as item label strings for dialoguer menus,
/// matching the existing format: `"{id}"` or `"{id} — {description}"`.
fn select_items(options: &[SelectOption]) -> Vec<String> {
    options
        .iter()
        .map(|opt| match &opt.description {
            Some(desc) => format!("{} — {desc}", opt.id),
            None => opt.id.clone(),
        })
        .collect()
}

impl Confirm for Interactive {
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure> {
        let term = console::Term::stderr();
        let theme = dialoguer::theme::ColorfulTheme::default();
        if let Some(caveat) = caveat {
            let _ = term.write_line(caveat);
        }
        let result = dialoguer::Confirm::with_theme(&theme)
            .with_prompt(question)
            .default(false)
            .show_default(true)
            .interact_on_opt(&term)
            .map_err(|source| {
                Failure::failed(
                    "confirm.read_answer",
                    format!("could not read your answer: {source}"),
                )
            })?;
        Ok(result.unwrap_or(false))
    }

    fn select_many(&self, prompt: &str, options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        if options.is_empty() {
            return Ok(Vec::new());
        }
        let term = console::Term::stderr();
        let theme = dialoguer::theme::ColorfulTheme::default();
        let items = select_items(options);
        let result = dialoguer::MultiSelect::with_theme(&theme)
            .with_prompt(prompt)
            .items(&items)
            .interact_on_opt(&term)
            .map_err(|source| {
                Failure::failed(
                    "confirm.read_answer",
                    format!("could not read your answer: {source}"),
                )
            })?;
        match result {
            Some(selected) if !selected.is_empty() => Ok(selected),
            _ => Err(Failure::blocked(
                "confirm.no_answer",
                "no selection entered on stdin",
            )),
        }
    }

    fn select_one(&self, prompt: &str, options: &[SelectOption]) -> Result<Option<usize>, Failure> {
        if options.is_empty() {
            return Ok(None);
        }
        let term = console::Term::stderr();
        let theme = dialoguer::theme::ColorfulTheme::default();
        let items = select_items(options);
        let result = dialoguer::Select::with_theme(&theme)
            .with_prompt(prompt)
            .items(&items)
            .default(0)
            .interact_on_opt(&term)
            .map_err(|source| {
                Failure::failed(
                    "confirm.read_answer",
                    format!("could not read your answer: {source}"),
                )
            })?;
        Ok(result)
    }

    fn is_interactive(&self) -> bool {
        true
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
#[cfg(test)]
#[must_use]
pub(crate) fn fixed(answer: bool) -> Arc<dyn Confirm> {
    Arc::new(Fixed {
        answer,
        selection: None,
        selection_one: None,
    })
}

/// A confirmer that returns `selection` for multi-select, for tests.
///
/// `cfg(test)`: nothing in the CLI answers its own prompts, so this must not
/// reach the shipped binary.
#[cfg(test)]
#[must_use]
pub(crate) fn fixed_select(answer: bool, selection: Vec<usize>) -> Arc<dyn Confirm> {
    Arc::new(Fixed {
        answer,
        selection: Some(selection),
        selection_one: None,
    })
}

/// A confirmer that returns `selection` for single-select, for tests.
#[cfg(test)]
#[must_use]
pub(crate) fn fixed_select_one(_answer: bool, selection: Option<usize>) -> Arc<dyn Confirm> {
    Arc::new(Fixed {
        answer: _answer,
        selection: None,
        selection_one: selection,
    })
}

#[cfg(test)]
#[path = "../../tests/unit/action/confirm.rs"]
mod tests;
