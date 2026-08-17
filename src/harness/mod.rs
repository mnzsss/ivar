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

use camino::Utf8Path;

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
    /// Whether the harness can ask the human a question mid-run, in the
    /// headless mode [`Self::execute_command`](Harness::execute_command)
    /// builds.
    ///
    /// False for OpenCode, which refuses the `question` tool and emits no
    /// question envelope — see [`stream`]'s "OpenCode cannot ask" for why. A
    /// workstream on such a harness never reaches `blocked` waiting for `ivar
    /// feature execute reply`: it finishes, or it fails.
    pub supports_questions: bool,
    /// Whether the harness runs a long-lived interactive process (needs a
    /// PTY) or a one-shot command.
    pub interactive: bool,
}

impl Capabilities {
    const CLAUDE_CODE: Self = Self {
        supports_resume: true,
        supports_review: true,
        supports_questions: true,
        interactive: true,
    };
    const OPENCODE: Self = Self {
        supports_resume: true,
        supports_review: false,
        supports_questions: false,
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
    /// Claude Code: `claude -p <prompt> --output-format stream-json --verbose
    /// --permission-mode bypassPermissions`. Stream-json output *requires*
    /// `--verbose`, and there is no `--cwd` flag — the working directory is
    /// set on the spawn (via [`proc::Command::cwd`]), not the argv.
    ///
    /// **The permission mode is not optional.** Left unset, the child runs in
    /// the interactive default, where a tool call outside the pre-approved set
    /// raises a permission prompt — and `-p` has nobody to answer it, so the
    /// prompt is denied where it stands. An executor launched that way cannot
    /// write the files its own write contract grants it, and cannot even
    /// `Read` the plan to recover what its prompt failed to tell it. Three
    /// workstreams once ran to completion that way, two of them writing
    /// nothing at all.
    ///
    /// `bypassPermissions` is chosen over `acceptEdits` because an executor
    /// has to run its repo's tests, and `acceptEdits` still prompts (and so
    /// still denies) every `Bash` call. Bypassing the *harness's* permission
    /// layer does not leave the child unarbitrated: writes are arbitrated by
    /// ivar's own execution guard ([`guard`]), a `PreToolUse` hook that runs
    /// regardless of permission mode and refuses anything outside the
    /// workstream's write contract. The harness prompt was never the gate
    /// here — it was a gate with nobody behind it, in front of the gate that
    /// does the work.
    ///
    /// OpenCode: `opencode run --format json [flags]`, with the prompt fed on
    /// **stdin** rather than argv. Three things about that line are not
    /// interchangeable with Claude Code's, and each was got wrong before:
    ///
    /// - **`-p` is not the prompt.** On the `opencode` CLI `-p` is the short
    ///   form of `--password` (HTTP basic auth for `--attach`), and `run`'s
    ///   message is a positional array. `opencode run -p <prompt>` therefore
    ///   leaves the message empty and exits 1 with "You must provide a message
    ///   or a command" — never reaching the model at all.
    /// - **`--format json` is required.** The default format is `default`,
    ///   which prints prose for a human. [`stream::parse_opencode_line`]
    ///   parses JSON events, so without this flag every line it sees is noise
    ///   and the whole run produces no events.
    /// - **The prompt goes on stdin, not argv.** `run` reads stdin to EOF when
    ///   it is not a TTY and uses it as the message when argv carries none —
    ///   verbatim. The positional path does not: `run` re-renders its argv
    ///   message by wrapping any element containing a space in literal double
    ///   quotes and backslash-escaping the quotes inside it, so a rendered plan
    ///   prompt would reach the model dressed in punctuation ivar never wrote.
    ///   Stdin is also the channel that cannot be mistaken for a flag, which
    ///   the argv path would need a `--` separator to guarantee.
    ///
    /// - **`--dir` is not the same as the spawn's working directory.** Without
    ///   it, `opencode run` takes its project directory from `$PWD` rather
    ///   than from `getcwd`. [`proc::Command`] now sets `PWD` with every
    ///   `cwd`; this flag says the same thing again in the one channel a
    ///   child cannot inherit stale. An executor that resolves elsewhere
    ///   loads the *hall's* config and plugins — which is to say it runs with
    ///   no execution guard, on paths the workstream never asked for.
    ///
    /// Claude Code has no equivalent flag and relies on the spawn's working
    /// directory, so `view_dir` reaches its argv nowhere.
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
        view_dir: &Utf8Path,
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
                .arg("--verbose")
                .arg("--permission-mode")
                .arg("bypassPermissions"),
            Self::OpenCode => command
                .arg("run")
                .arg("--dir")
                .arg(view_dir.as_str())
                .arg("--format")
                .arg("json")
                .stdin(prompt),
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
