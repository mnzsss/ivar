//! The hidden `ivar agent sandbox` launcher: a `runtime: host` run applies
//! its Landlock write ruleset here and becomes the tool, so the parent that
//! writes the receipt stays unrestricted (ADR-0006 D1).

use camino::Utf8PathBuf;

use crate::action::session::sandbox::{Sandbox, SandboxStatus};
use crate::error::Failure;
use crate::infra::proc::{self, Command};

/// # Errors
///
/// `agent.sandbox_empty_command`, `agent.landlock_unavailable` when the kernel
/// enforces nothing, `agent.sandbox_process_failed`, or a spawn failure.
pub fn launch(writes: &[String], command: &[String]) -> Result<(), Failure> {
    let (program, args) = command.split_first().ok_or_else(|| {
        Failure::failed(
            "agent.sandbox_empty_command",
            "`ivar agent sandbox` needs a command after `--`",
        )
    })?;
    let roots = writes.iter().map(Utf8PathBuf::from).collect();
    if let SandboxStatus::Unavailable { reason } = Sandbox::from_roots(roots).apply()? {
        return Err(Failure::blocked(
            "agent.landlock_unavailable",
            format!("a `runtime: host` run needs Landlock to confine its writes: {reason}"),
        ));
    }
    match proc::exec(&Command::new(program.as_str()).args(args.iter().cloned()))? {
        Some(0) | None => Ok(()),
        Some(code) => Err(Failure::failed(
            "agent.sandbox_process_failed",
            format!("`{program}` exited with {code}"),
        )),
    }
}
