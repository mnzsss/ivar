//! The `ivar guard` command: reads stdin, resolves the session, decides, and
//! shapes the output for the given provider.
//!
//! This is the command-level wrapper around the guard logic in [`super::guard`].
//! It owns stdin I/O and input construction so `bin/ivar.rs` stays thin.

use std::io::{self, Read};

use crate::domain::provider::Provider;
use crate::error::Failure;

pub use super::guard::GuardOutcome;

/// Input for the guard command.
#[derive(Debug)]
pub struct GuardInput {
    pub provider: Provider,
    /// Claude Code's extra hook entries: return this slice of the repository
    /// instructions and never decide.
    pub slice: Option<usize>,
}

impl GuardInput {
    /// Build the input, refusing `--slice` for any provider but Claude Code:
    /// only Claude's settings carry the slice entries.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] when `slice` is set for omp or opencode.
    pub fn new(provider: Provider, slice: Option<usize>) -> Result<Self, Failure> {
        if slice.is_some() && provider != Provider::ClaudeCode {
            return Err(Failure::blocked(
                "guard.invalid_slice",
                "`--slice` is only for claude-code",
            ));
        }
        Ok(Self { provider, slice })
    }
}

/// Run the guard: read stdin, delegate to the guard, and return the outcome.
///
/// # Errors
///
/// Returns [`Failure`] if stdin cannot be read or the guard refuses the
/// payload. A slice entry never fails: it reads unreadable stdin as empty.
pub fn run(input: &GuardInput) -> Result<GuardOutcome, Failure> {
    let mut stdin = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut stdin) {
        if input.slice.is_none() {
            return Err(Failure::blocked(
                "guard.stdin",
                format!("could not read stdin: {e}"),
            ));
        }
        stdin.clear();
    }

    super::guard::guard(input.provider, &stdin, input.slice)
}
