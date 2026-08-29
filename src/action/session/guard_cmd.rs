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
}

/// Run the guard: read stdin, delegate to the guard, and return the outcome.
pub fn run(input: GuardInput) -> Result<GuardOutcome, Failure> {
    let mut stdin = String::new();
    io::stdin()
        .read_to_string(&mut stdin)
        .map_err(|e| Failure::blocked("guard.stdin", format!("could not read stdin: {e}")))?;

    super::guard::guard(input.provider, &stdin)
}
