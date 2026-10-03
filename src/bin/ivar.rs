//! Entrypoint. Parses argv, dispatches into `action`, renders the outcome,
//! sets the exit code. No logic beyond that plumbing lives here — see
//! ARCHITECTURE.md's module map.

use std::process::ExitCode;

fn main() -> ExitCode {
    ivar::app::run::run(ivar::app::run::parse(std::env::args_os().collect()))
}
