//! `ivar session env` — resolve the session environment by cwd walk-up.

use camino::Utf8PathBuf;

use super::env::SessionEnv;
use crate::action::Ctx;
use crate::error::{Failure, FixAction, Outcome, Report};

/// Input for `ivar session env`.
#[derive(Debug, Clone, Default)]
pub struct EnvInput {
    /// Start path for walk-up resolution. If `None`, uses `ctx.cwd()`.
    pub cwd: Option<Utf8PathBuf>,
}

/// Execute `ivar session env`.
pub fn run(ctx: &Ctx, input: EnvInput) -> Outcome<SessionEnv> {
    let start = input.cwd.as_deref().unwrap_or(&ctx.cwd);
    let env = SessionEnv::resolve_by_cwd(start)?.ok_or_else(|| {
        Failure::blocked(
            "session.not_in_session",
            "not inside an active session view directory",
        )
        .expected("to be inside a session view directory or pass --cwd")
        .actual(format!(
            "`{start}` has no state.json in its directory hierarchy"
        ))
        .fix(FixAction::safe(
            "session.start_or_connect",
            "Start a session with `ivar session start` or connect with `ivar session connect`.",
        ))
    })?;

    Ok(Report::new(env))
}
