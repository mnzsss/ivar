//! Which runs may print the update notice. The decision itself is
//! `domain::upgrade::notice_enabled`; this reads the process state it needs.

use crate::action::upgrade::notice::current_version;
use crate::cli::GraphCommand;
use crate::cli::root::{Command, SessionCommand};
use crate::domain::upgrade::NoticeContext;
use crate::infra::term;

/// Verbs whose stderr is read by a program: provider hooks (`guard`,
/// `session env`), the MCP stdio server, git's credential protocol, the
/// relay contract — and `upgrade`, which reports versions itself.
pub(super) fn is_machine_verb(command: &Command) -> bool {
    matches!(
        command,
        Command::Guard(_)
            | Command::GitCredential(_)
            | Command::Upgrade(_)
            | Command::Session(SessionCommand::Env(_) | SessionCommand::Relay(_))
            | Command::Graph(GraphCommand::Mcp(_))
    )
}

pub(super) fn notice_context(json: bool, command: &Command) -> NoticeContext {
    NoticeContext {
        opted_out: std::env::var_os("IVAR_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty()),
        ci: std::env::var_os("CI").is_some(),
        stderr_tty: term::is_tty(term::Stream::Stderr),
        machine_output: json,
        machine_verb: is_machine_verb(command),
        release_build: !cfg!(debug_assertions)
            && current_version().is_some_and(|v| v.to_string() != "0.0.0"),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/app/notice.rs"]
mod tests;
