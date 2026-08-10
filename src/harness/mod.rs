//! Provider adapters: the code that knows what each harness wants on disk.
//!
//! `domain::provider` holds what a harness *is* — its id, its dotdir, its
//! instruction file. This layer holds what `ivar` *does* about it, which needs
//! I/O and therefore cannot live in `domain`.
//!
//! # What is here now
//!
//! [`config`] — the managed block in each harness's instruction file. That is
//! slice 2's harness work: `ivar sync` materialises it, and removes it for a
//! provider the hall no longer lists.
//!
//! [`commands`] — the embedded catalog of shipped workflow commands, and the
//! reconciliation that materialises them into each provider's command
//! directory. The catalog and every Markdown source are compiled into the
//! binary; this module owns only the command files `ivar-*.md` and leaves every
//! other file in the command directory to the user.
//!
//! [`Harness`] — the trait plus closed-enum dispatch that slice 5
//! (`ivar session start`) needs: each variant owns its command construction.
//! ARCHITECTURE.md, seam 5: the set of harnesses is known at compile time, so
//! dispatch is a match over a closed enum, not a vtable, and capabilities are
//! explicit flags rather than inferred. [`Harness::start_command`] builds the
//! *interactive* invocation; [`Harness::execute_command`] builds the
//! *headless, parsed* one that `ivar feature execute tick` spawns.
//!
//! [`stream`] — provider JSON in, [`stream::ExecutorEvent`] out. What
//! `execute_command`'s stdout means never leaves that module as raw JSON.
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error`. Not `store` — so paths
//! arrive here already computed by [`crate::store::layout`], which stays the
//! one place that knows where anything under a hall lives.

pub mod commands;
pub mod config;
pub mod guard;
pub mod stream;

use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::proc;

/// What a harness can and cannot do, stated explicitly rather than inferred.
///
/// A flag that is false means the harness does not pretend: `ivar session
/// start --resume` refuses for a harness whose `supports_resume` is false,
/// naming the gap instead of failing at spawn time with an option the harness
/// never reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether the harness can resume an existing session (`--resume`).
    pub supports_resume: bool,
    /// Whether the harness accepts review-style subcommands.
    pub supports_review: bool,
    /// Whether the harness runs a long-lived interactive process (needs a
    /// PTY) or a one-shot command.
    pub interactive: bool,
}

impl Capabilities {
    const CLAUDE_CODE: Self = Self {
        supports_resume: true,
        supports_review: true,
        interactive: true,
    };
    const OPENCODE: Self = Self {
        supports_resume: true,
        supports_review: false,
        interactive: true,
    };
}

/// A harness adapter: knows how to *run* one provider's agent in a session.
///
/// Closed-enum dispatch (ARCHITECTURE.md, seam 5) — `Provider` is a closed
/// set, so this is a match, not a vtable. Each variant owns its command
/// construction; `config.rs` owns the file shapes, this module owns the
/// process shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    /// Claude Code (`claude` CLI).
    ClaudeCode,
    /// OpenCode (`opencode` CLI).
    OpenCode,
}

impl Harness {
    /// The harness for a provider, or a `Blocked` failure naming the
    /// provider id. `Provider` is closed today, so this is exhaustive — the
    /// failure arm exists for when the set grows before this match does.
    pub fn for_provider(provider: Provider) -> Result<Self, Failure> {
        match provider {
            Provider::ClaudeCode => Ok(Self::ClaudeCode),
            Provider::OpenCode => Ok(Self::OpenCode),
        }
    }

    /// This harness's declared capabilities.
    #[must_use]
    pub fn capabilities(self) -> Capabilities {
        match self {
            Self::ClaudeCode => Capabilities::CLAUDE_CODE,
            Self::OpenCode => Capabilities::OPENCODE,
        }
    }

    /// The CLI binary this harness runs.
    #[must_use]
    pub fn binary(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::OpenCode => "opencode",
        }
    }

    /// The invocation that starts (or resumes) an interactive session in
    /// `worktree`, with the view dir as the working directory.
    ///
    /// `resume` is honoured only when [`Self::capabilities`] says
    /// [`Capabilities::supports_resume`]; callers check that before calling.
    pub fn start_command(self, resume: bool) -> proc::Command {
        let command = proc::Command::new(self.binary());
        match self {
            Self::ClaudeCode => {
                if resume {
                    command.arg("--continue")
                } else {
                    command
                }
            }
            Self::OpenCode => {
                if resume {
                    command.arg("--continue")
                } else {
                    command
                }
            }
        }
    }

    /// The invocation that runs a headless, output-parsed execution of
    /// `prompt` — what `ivar feature execute tick` spawns, as opposed to the
    /// interactive session [`Self::start_command`] builds.
    ///
    /// Claude Code: `claude -p <prompt> --output-format stream-json
    /// --verbose`. Stream-json output *requires* `--verbose`, and there is no
    /// `--cwd` flag — the working directory is set on the spawn (via
    /// [`proc::Command::cwd`]), not the argv.
    ///
    /// OpenCode: `opencode run -p <prompt>`.
    ///
    /// `model` and `agent` are appended only when the caller supplies them,
    /// and each is its own flag on both CLIs: `--model` selects the model,
    /// `--agent` selects the agent. They are never collapsed into one another
    /// — conflating them (the old `tick.rs` rendered `agent` as `--model
    /// <agent>`) is precisely the bug this builder exists to undo.
    #[must_use]
    pub fn execute_command(
        self,
        prompt: &str,
        model: Option<&str>,
        agent: Option<&str>,
    ) -> proc::Command {
        let command = proc::Command::new(self.binary());
        let command = match self {
            Self::ClaudeCode => command
                .arg("-p")
                .arg(prompt)
                .arg("--output-format")
                .arg("stream-json")
                .arg("--verbose"),
            Self::OpenCode => command.arg("run").arg("-p").arg(prompt),
        };
        let command = match model {
            Some(model) => command.arg("--model").arg(model),
            None => command,
        };
        match agent {
            Some(agent) => command.arg("--agent").arg(agent),
            None => command,
        }
    }
}

/// Whether `resume` is possible for `harness`, as a `Blocked` failure with a
/// fix action when it is not. Separated from [`Harness::start_command`] so
/// the check can run before any work is done (a session dir is not created
/// for a resume the harness cannot perform).
pub fn check_resume_supported(harness: Harness) -> Result<(), Failure> {
    if harness.capabilities().supports_resume {
        return Ok(());
    }
    Err(Failure::blocked(
        "harness.no_resume",
        format!("`{}` cannot resume a session", harness.binary()),
    )
    .expected("a harness whose capabilities include resume")
    .actual("this harness's `supports_resume` is false")
    .fix(FixAction::safe(
        "session.start_fresh",
        "Start a fresh session instead of resuming.",
    )))
}

#[cfg(test)]
#[path = "../../tests/unit/harness/mod.rs"]
mod tests;
