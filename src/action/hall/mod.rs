//! `hall`: the verbs that act on the hall itself — `init · status · doctor ·
//! cleanup · migrate`. See ARCHITECTURE.md's module map.
//!
//! Each verb lives in its own file (`init.rs`, `status.rs`, `doctor.rs`,
//! `migrate.rs`, `cleanup.rs`); this facade owns what they share — the
//! discovery/read helpers and the interactive prompt — and reexports the
//! public command surface so `action::hall::{init, status, …}` keeps its
//! established names.

use std::io;
use std::io::Write;

use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::Ctx;

mod cleanup;
mod doctor;
mod init;
mod migrate;
mod status;

pub use cleanup::CleanupOutcome;
pub use cleanup::cleanup;
pub use doctor::Diagnosis;
pub use doctor::DoctorOutcome;
pub use doctor::doctor;
pub use init::InitInput;
pub use init::InitOutcome;
pub use init::init;
pub use migrate::MigrateOutcome;
pub use migrate::migrate;
pub use status::RepoStatusEntry;
pub use status::StatusOutcome;
pub use status::status;

/// The hall [`Ctx::cwd`] is inside, or a [`Failure`] saying there is none.
/// Shared with the other verbs that operate on the current hall.
fn discover_hall(ctx: &Ctx) -> Result<Layout, Failure> {
    super::discover_hall(ctx)
}

/// The manifest [`Layout::discover`] just proved exists.
fn read_manifest(layout: &Layout) -> Result<Manifest, Failure> {
    super::read_manifest(layout)
}

/// Ask `question` on stderr and read a yes/no from stdin. `true` only for an
/// explicit `y`.
///
/// **Non-tty runs answer `false` without asking.** That is the safety property
/// both callers depend on: neither a `cleanup` that deletes nor a `migrate`
/// that rewrites a committed file may act when there is nobody to read the
/// question. A pipe is not consent.
///
/// The prompt goes to stderr so that piping stdout — the machine surface —
/// never swallows the question, and `--json` output stays parseable.
///
/// `write_code` / `read_code` are the caller's own [`Failure::code`]s: these
/// are the stable identifiers a machine matches on, so each verb keeps its own
/// rather than inheriting a shared one from this helper.
pub(crate) fn ask(
    question: &str,
    write_code: &'static str,
    read_code: &'static str,
    caveat: Option<&str>,
) -> Result<bool, Failure> {
    if !crate::infra::term::is_tty(crate::infra::term::Stream::Stderr) {
        return Ok(false);
    }
    let mut stderr = io::stderr().lock();
    let write = |result: io::Result<()>| {
        result.map_err(|source| {
            Failure::failed(write_code, format!("could not write the prompt: {source}"))
        })
    };
    if let Some(caveat) = caveat {
        write(writeln!(stderr, "{caveat}"))?;
    }
    write(writeln!(stderr, "{question} [y/N] "))?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(|source| {
        Failure::failed(read_code, format!("could not read your answer: {source}"))
    })?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/hall.rs"]
mod tests;
