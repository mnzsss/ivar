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
//! explicit flags rather than inferred.
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error`. Not `store` — so paths
//! arrive here already computed by [`crate::store::layout`], which stays the
//! one place that knows where anything under a hall lives.

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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn each_provider_maps_to_its_harness() {
        assert_eq!(
            Harness::for_provider(Provider::ClaudeCode).unwrap(),
            Harness::ClaudeCode
        );
        assert_eq!(
            Harness::for_provider(Provider::OpenCode).unwrap(),
            Harness::OpenCode
        );
    }

    #[test]
    fn claude_code_resumes_with_continue() {
        let command = Harness::ClaudeCode.start_command(true);
        let display = command.display();
        assert!(display.starts_with("claude"), "was: {display}");
        assert!(display.contains("--continue"), "was: {display}");
    }

    #[test]
    fn a_fresh_start_has_no_extra_flags() {
        let display = Harness::ClaudeCode.start_command(false).display();
        assert_eq!(display, "claude");
    }

    #[test]
    fn opencode_builds_its_own_command() {
        let display = Harness::OpenCode.start_command(false).display();
        assert!(display.starts_with("opencode"), "was: {display}");
    }

    #[test]
    fn resume_is_supported_for_both_harnesses_today() {
        assert!(check_resume_supported(Harness::ClaudeCode).is_ok());
        assert!(check_resume_supported(Harness::OpenCode).is_ok());
    }

    #[test]
    fn capabilities_are_explicit_not_inferred() {
        // The contract of seam 5: the flags say what the harness can do,
        // and nothing in this module ever guesses from the binary name.
        let caps = Harness::ClaudeCode.capabilities();
        assert!(caps.supports_resume);
        assert!(caps.interactive);
        let opencode = Harness::OpenCode.capabilities();
        assert!(!opencode.supports_review);
    }
}
