//! A transient line on stderr for work that takes long enough to look hung.
//!
//! # Contract
//!
//! - [`Progress::step`] replaces whatever transient line is on screen with a
//!   new one. It is the *only* thing a caller has to sequence correctly.
//! - [`Progress::clear`] erases it. Idempotent, and mandatory before the run's
//!   real output is written — a leftover progress line would collide with the
//!   [`crate::error::WriteHuman`] rendering of the outcome.
//! - [`Silent`] is the default everywhere, including every test. [`Stderr`] is
//!   built exactly once, by `bin/ivar.rs`, and only when there is a terminal to
//!   redraw. [`reporter`] is that decision, written down once.
//!
//! # Why this is not a `println!`
//!
//! ARCHITECTURE.md's first rule is that an action returns data and never
//! prints — one code path computes what to show, so `--json` and the human
//! surface cannot drift. A progress line is not part of the outcome: it is
//! *ephemeral*, it never appears in `--json`, and it is gone by the time the
//! outcome is rendered. Passing the sink in through [`crate::action::Ctx`]
//! keeps both facts true — the action still returns only data, and a test
//! still observes an action through its return value, because the sink it gets
//! is [`Silent`].
//!
//! # Why it writes to stderr
//!
//! Same reason `action::hall::ask` puts its prompt there: stdout is the
//! machine surface, and `ivar repo pull --json | jq` must not have a redraw
//! line in the middle of the document.
//!
//! # Design
//!
//! Indicatif draws a spinner on stderr with `{spinner} {wide_msg}` so the
//! message automatically truncates to the terminal width without wrapping.
//! The spinner is started lazily on the first [`Progress::step`]; a run that
//! finishes fast or never calls `step` draws nothing at all.
//!
//! [`Progress::clear`] finishes and clears the progress bar, leaving stderr
//! clean for subsequent output.
//!
//! Indicatif manages its own background tick thread to advance the spinner
//! smoothly. For that reason, `app::run` never holds a persistent lock on the
//! output streams so the tick thread can lock stderr when rendering ticks.
//!
//! Every write is best-effort: a failed write to a progress line must never
//! turn into a [`crate::error::Failure`]. If stderr is gone, the work still
//! ran, and the outcome is what the user came for.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressFinish, ProgressStyle};

use super::term::{self, Stream};

/// How often the spinner advances on its own while a step runs.
const TICK: Duration = Duration::from_millis(100);
/// Where a long-running verb reports what it is doing right now.
///
/// `Send + Sync` because [`crate::action::Ctx`] is `Clone` and nothing should
/// stop it crossing a thread boundary later; `Debug` because `Ctx` derives it.
pub trait Progress: fmt::Debug + Send + Sync {
    /// Show `message`, replacing the current transient line.
    fn step(&self, message: &str);

    /// Erase the transient line. Idempotent — calling it with nothing on
    /// screen does nothing.
    fn clear(&self);
}

/// The reporter that shows nothing. The default for every `Ctx`, and what
/// every test sees.
#[derive(Debug, Clone, Copy, Default)]
pub struct Silent;

impl Progress for Silent {
    fn step(&self, _message: &str) {}
    fn clear(&self) {}
}

/// A spinner on stderr, created on the first [`Progress::step`] and removed by
/// [`Progress::clear`].
///
/// Lazy on purpose: [`reporter`] builds one for every human run on a tty,
/// and a verb that never reports progress must draw nothing.
pub struct Stderr {
    bar: Mutex<Option<ProgressBar>>,
    target: fn() -> ProgressDrawTarget,
}

impl fmt::Debug for Stderr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stderr")
            .field("started", &self.is_started())
            .finish()
    }
}

impl Default for Stderr {
    fn default() -> Self {
        Self::new()
    }
}

impl Stderr {
    #[must_use]
    pub fn new() -> Self {
        Self::with_target(ProgressDrawTarget::stderr)
    }

    pub(crate) fn with_target(target: fn() -> ProgressDrawTarget) -> Self {
        Self {
            bar: Mutex::new(None),
            target,
        }
    }

    /// A poisoned lock is recovered, not propagated: a panic elsewhere must
    /// not take a run down over a cosmetic line.
    fn bar(&self) -> MutexGuard<'_, Option<ProgressBar>> {
        self.bar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn is_started(&self) -> bool {
        self.bar().is_some()
    }

    #[cfg(test)]
    pub(crate) fn message(&self) -> Option<String> {
        self.bar().as_ref().map(ProgressBar::message)
    }

    fn start(&self) -> ProgressBar {
        let bar = ProgressBar::with_draw_target(None, (self.target)())
            .with_finish(ProgressFinish::AndClear);
        if let Ok(style) = ProgressStyle::with_template("{spinner} {wide_msg}") {
            bar.set_style(style);
        }
        bar.enable_steady_tick(TICK);
        bar
    }
}

impl Progress for Stderr {
    fn step(&self, message: &str) {
        let mut slot = self.bar();
        let bar = slot.get_or_insert_with(|| self.start());
        bar.set_message(one_line(message));
    }

    fn clear(&self) {
        if let Some(bar) = self.bar().take() {
            bar.finish_and_clear();
        }
    }
}

/// The reporter a run should use.
///
/// `wanted` is the caller's own decision — `--json` does not want one, because
/// even on stderr a redraw line is noise for a machine-shaped run. The tty
/// half is asked here so no call site has to remember it: a redirected stderr
/// gets [`Silent`], since `\r` into a file writes a control character nobody
/// will ever erase.
#[must_use]
pub fn reporter(wanted: bool) -> Arc<dyn Progress> {
    if wanted && term::is_tty(Stream::Stderr) {
        Arc::new(Stderr::new())
    } else {
        Arc::new(Silent)
    }
}

/// `message` with control characters as spaces: a newline inside the
/// message would push the spinner off its line. Width is indicatif's job
/// (`{wide_msg}` truncates to the terminal).
pub(crate) fn one_line(message: &str) -> String {
    message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/infra/progress.rs"]
mod tests;
