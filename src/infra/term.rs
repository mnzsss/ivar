//! Terminal capabilities: colour, width, whether anyone is watching.
//!
//! # Contract
//!
//! - `colour()` — whether to emit colour on **stdout**, decided once and cached.
//!   Precedence, highest first: an explicit `--no-color` from the caller, then
//!   `NO_COLOR` (set to anything, per <https://no-color.org>), then
//!   `FORCE_COLOR`, then whether stdout is a tty. A pipe gets no colour.
//! - `colour_for(stream, …)` — the same question for a named stream, cached per
//!   stream. Failures and warnings go to stderr, and `ivar … 2>log` must put no
//!   escape codes in that file even while stdout is still a terminal — so the
//!   tty half of the decision has to be asked of the stream being written to.
//!   The flag and the environment variables are global and apply to both.
//! - `width()` — terminal columns, with a sane fallback when there is no tty.
//! - `is_tty()` — for stdout and stderr separately. They can differ, and the
//!   progress reporter cares.
//!
//! This module decides *whether* to colour. What the colours mean, and the
//! escape codes themselves, belong to [`crate::error::Palette`] — which is where
//! the layout of a failure already lives, so painting it needs no second
//! renderer. There is no colour crate: the vocabulary is five SGR constants.
//!
//! Values themselves are never coloured — that would put escape codes inside
//! data and break the `--json` contract. Only labels are painted.
//!
//! # Design
//!
//! The precedence rule is a pure function, [`decide_colour`], over plain values
//! — no env access, no tty probing — so it is exhaustively testable without
//! touching the process environment. [`colour`] is the thin, impure wrapper: it
//! reads the environment and the real tty state exactly once and caches the
//! result in a [`OnceLock`], per the house rule against process-wide mutable
//! globals.
//!
//! `FORCE_COLOR=0` is treated as "force off", mirroring the convention several
//! CLIs (npm, Node) already use; any other value, including empty, forces
//! colour on — the module doc only spells out that nuance for `NO_COLOR`, but
//! `FORCE_COLOR` existing at all is meaningless if `"0"` cannot turn it off.
//!
//! There is no `thiserror` enum in this module: every public function here
//! absorbs its own failure into a documented fallback (`width()`) or simply
//! cannot fail (`colour()`, `is_tty()`), so there is nothing to name.

use std::env;
use std::sync::OnceLock;

use crossterm::tty::IsTty as _;

/// A sane terminal width to fall back to when there is no tty to ask, or the
/// query fails.
const DEFAULT_WIDTH: u16 = 80;

/// Which stream to probe. Stdout and stderr can be redirected independently —
/// `2>&1` into a file while stdout stays a tty, or vice versa — so the answer
/// must be asked per-stream, never assumed to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// The precedence rule for whether to emit colour, as a pure function of its
/// inputs.
///
/// Highest precedence first:
/// 1. `override_` — an explicit caller decision (e.g. a `--no-color` flag).
///    `Some(true)` forces colour on, `Some(false)` forces it off.
/// 2. `no_color` — the value of `NO_COLOR`, if the variable is set at all.
///    Presence disables colour regardless of content, including an empty
///    string: <https://no-color.org> defines it that way on purpose.
/// 3. `force_color` — the value of `FORCE_COLOR`, if set. `"0"` forces colour
///    off; any other value, including empty, forces it on.
/// 4. `stdout_is_tty` — the fallback: colour only when stdout is a real
///    terminal. A pipe or a redirect gets no colour.
#[must_use]
pub fn decide_colour(
    override_: Option<bool>,
    no_color: Option<&str>,
    force_color: Option<&str>,
    stdout_is_tty: bool,
) -> bool {
    if let Some(explicit) = override_ {
        return explicit;
    }
    if no_color.is_some() {
        return false;
    }
    if let Some(force) = force_color {
        return force != "0";
    }
    stdout_is_tty
}

static COLOUR: OnceLock<bool> = OnceLock::new();
static COLOUR_STDERR: OnceLock<bool> = OnceLock::new();

/// Whether to emit colour. Decided once, from the real environment and the
/// real tty state, then cached for the lifetime of the process.
///
/// `override_` is the caller's explicit decision, if it has one (typically a
/// `--no-color` / `--color` flag parsed once at start-up). Only the first call
/// across the whole process has any effect on the cached value — see
/// [`decide_colour`] for the full precedence rule and a pure, per-call version
/// of this decision.
#[must_use]
pub fn colour(override_: Option<bool>) -> bool {
    *COLOUR.get_or_init(|| {
        decide_colour(
            override_,
            env::var("NO_COLOR").ok().as_deref(),
            env::var("FORCE_COLOR").ok().as_deref(),
            is_tty(Stream::Stdout),
        )
    })
}

/// Whether to emit colour on `stream`. Decided once per stream, then cached.
///
/// Identical to [`colour`] except for which stream's tty state serves as the
/// fallback. `Stream::Stdout` shares [`colour`]'s cache, so the two can never
/// disagree about stdout.
///
/// This exists because the two streams are redirected independently. A run of
/// `ivar sync 2>errors.log` has a tty on stdout and a file on stderr; asking
/// stdout's state would write SGR codes into `errors.log`, which is the thing
/// `NO_COLOR` exists to prevent.
#[must_use]
pub fn colour_for(stream: Stream, override_: Option<bool>) -> bool {
    match stream {
        Stream::Stdout => colour(override_),
        Stream::Stderr => *COLOUR_STDERR.get_or_init(|| {
            decide_colour(
                override_,
                env::var("NO_COLOR").ok().as_deref(),
                env::var("FORCE_COLOR").ok().as_deref(),
                is_tty(Stream::Stderr),
            )
        }),
    }
}

/// Terminal columns, or [`DEFAULT_WIDTH`] when there is no usable answer: no
/// tty to ask (a pipe, a redirect), the query failing, or the query
/// *succeeding with zero*.
///
/// The zero case is not hypothetical. A pty whose window size was never set —
/// `script -qec … | cat`, and several CI runners — answers `Ok((0, 0))` rather
/// than failing, so `unwrap_or` never fires and every caller ends up laying
/// out against a zero-column terminal. A width of zero is not a narrow
/// terminal; it is a missing answer, and it belongs in the same arm as the
/// error.
#[must_use]
pub fn width() -> u16 {
    decide_width(
        crossterm::terminal::size()
            .map(|(columns, _rows)| columns)
            .ok(),
    )
}

/// The width rule as a pure function of what the query answered — `None` for a
/// failure, `Some(0)` for the unset-winsize pty above. Split out for the same
/// reason [`decide_colour`] is: it is exhaustively testable without a terminal.
#[must_use]
fn decide_width(queried: Option<u16>) -> u16 {
    match queried {
        Some(columns) if columns > 0 => columns,
        _ => DEFAULT_WIDTH,
    }
}

/// Whether `stream` is a real terminal.
#[must_use]
pub fn is_tty(stream: Stream) -> bool {
    match stream {
        Stream::Stdout => std::io::stdout().is_tty(),
        Stream::Stderr => std::io::stderr().is_tty(),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/infra/term.rs"]
mod tests;
