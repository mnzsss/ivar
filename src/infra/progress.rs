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
//! No escape codes. The line is erased by returning the carriage and writing
//! spaces over the previous one, which is why [`Stderr`] has to remember how
//! long that was. `\x1b[K` would be shorter and is universally supported, but
//! this module owning zero terminal vocabulary is worth more than the bytes —
//! [`super::term`] is where "what can this terminal do" lives.
//!
//! A line longer than the terminal wraps, and a wrapped line cannot be erased
//! by one `\r`, so [`fit`] truncates first. It is a pure function over
//! `(message, width)` and therefore testable without a terminal, which is the
//! same split [`super::term::decide_colour`] uses.
//!
//! Every write is best-effort: a failed write to a progress line must never
//! turn into a [`crate::error::Failure`]. If stderr is gone, the work still
//! ran, and the outcome is what the user came for.

use std::fmt;
use std::io::{self, Write as _};
use std::sync::{Arc, Mutex};

use super::term::{self, Stream};

/// The ellipsis appended to a message [`fit`] had to cut.
const ELLIPSIS: char = '…';

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

/// A single line redrawn in place on stderr.
///
/// The `usize` is the printed length of the line currently on screen — how
/// many spaces it takes to erase it. `0` means there is nothing to erase.
#[derive(Debug, Default)]
pub struct Stderr {
    live: Mutex<usize>,
}

impl Stderr {
    /// A reporter with nothing on screen yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The length of the line on screen, recovering the value from a poisoned
    /// lock rather than propagating the panic.
    ///
    /// A panic in another thread must not take down a run over the bookkeeping
    /// of a cosmetic line: the worst a stale count can do is leave a few
    /// characters on screen.
    fn live(&self) -> std::sync::MutexGuard<'_, usize> {
        self.live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Progress for Stderr {
    fn step(&self, message: &str) {
        let line = fit(message, usize::from(term::width()));
        let length = line.chars().count();
        let mut live = self.live();
        // Spaces over whatever the last line left uncovered, so a shorter line
        // does not leave the tail of a longer one behind it.
        let padding = live.saturating_sub(length);
        let mut stderr = io::stderr().lock();
        let _ = write!(stderr, "\r{line}{:padding$}", "");
        let _ = stderr.flush();
        *live = length;
    }

    fn clear(&self) {
        let mut live = self.live();
        if *live == 0 {
            return;
        }
        let mut stderr = io::stderr().lock();
        let _ = write!(stderr, "\r{:width$}\r", "", width = *live);
        let _ = stderr.flush();
        *live = 0;
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

/// `message` as one line that fits in `width` columns.
///
/// Control characters — a newline above all — become spaces: they would move
/// the cursor off the line the redraw is about to return to, and the erase
/// would then blank the wrong row. A message too long is cut and given an
/// [`ELLIPSIS`], which is what keeps the line from wrapping.
///
/// Truncation counts `char`s, not columns. A repo name is a validated
/// [`crate::domain::name::RepoName`] and the rest of the message is ASCII, so
/// the two agree here; a CJK-wide message would cut short, never long, which
/// is the harmless direction.
fn fit(message: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let flattened: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flattened.chars().count() <= width {
        return flattened;
    }
    // `width` is at least 1 here, so the ellipsis always has room.
    let mut line: String = flattened.chars().take(width - 1).collect();
    line.push(ELLIPSIS);
    line
}

#[cfg(test)]
#[path = "../../tests/unit/infra/progress.rs"]
mod tests;
