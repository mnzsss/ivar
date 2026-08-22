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
//! [`Harness`] — closed-enum dispatch for `ivar session start`: each variant
//! owns its interactive command construction. ARCHITECTURE.md, seam 5: the set
//! of harnesses is known at compile time, so dispatch is a match over a closed
//! enum, not a vtable, and capabilities are explicit flags rather than inferred.
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error`. Not `store` — so paths
//! arrive here already computed by [`crate::store::layout`], which stays the
//! one place that knows the on-disk tree.

pub mod commands;
pub mod config;

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

/// A harness adapter: knows how to start one provider's agent in a session.
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

    /// The invocation that starts (or resumes) an interactive session.
    ///
    /// `resume` is honoured only when [`Self::capabilities`] says
    /// [`Capabilities::supports_resume`]; callers check that before calling.
    pub fn start_command(self, resume: bool) -> proc::Command {
        let command = proc::Command::new(self.binary());
        match self {
            Self::ClaudeCode | Self::OpenCode if resume => command.arg("--continue"),
            Self::ClaudeCode | Self::OpenCode => command,
        }
    }
}

/// Refuse a resume request before spawning a harness that cannot honour it.
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
