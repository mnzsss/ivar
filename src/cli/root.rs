//! The root command surface.
//!
//! The settled surface (ARCHITECTURE.md's module map):
//! `ivar init · sync · status · doctor · cleanup · repo · feature · session ·
//! provider · plan · skill`. Only `init` carries real arguments this slice —
//! every other verb is a bare placeholder `bin/ivar.rs` turns into a
//! `Failure` naming it as not implemented yet, never a silent success and
//! never `todo!()`. See ARCHITECTURE.md's build order: those verbs land in
//! later slices, not stubbed 40-deep now.

use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::action::hall::InitInput;

/// Mount the repos a feature spans into one directory, on one branch, for
/// one agent session.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Emit machine-readable output. `--json` and the human surface render
    /// the exact same value the action returned — see ARCHITECTURE.md,
    /// "1. `action` is the unit, and it has one output shape".
    #[arg(long, global = true)]
    pub json: bool,

    /// Colour control. `auto` (the default) follows `NO_COLOR` /
    /// `FORCE_COLOR` / tty detection; `always` and `never` are an explicit
    /// override fed to `infra::term::colour`.
    #[arg(long = "color", global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,

    #[command(subcommand)]
    pub command: Command,
}

/// The root verbs. See the module doc comment for which ones do anything in
/// this slice.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a hall: `ivar.json`, `.ivar/`, and the hall's `.gitignore`
    /// lines.
    Init(InitArgs),
    /// Materialise harness config, setup scripts, and clone repos. Not
    /// implemented yet — the next slice.
    Sync,
    /// Report hall health. Not implemented yet.
    Status,
    /// Diagnose and suggest fixes. Not implemented yet.
    Doctor,
    /// Reconcile stale state. Not implemented yet.
    Cleanup,
    /// Manage repos. Not implemented yet.
    Repo,
    /// Manage features. Not implemented yet.
    Feature,
    /// Manage sessions. Not implemented yet.
    Session,
    /// Manage providers. Not implemented yet.
    Provider,
    /// Manage SPDD plans. Not implemented yet.
    Plan,
    /// Manage skills. Not implemented yet.
    Skill,
}

/// Arguments for `ivar init`.
///
/// `name` and `provider` stay plain strings here — validating them into
/// `HallName` / `Provider` needs `domain`, which `cli` may not import (see
/// the layering table in ARCHITECTURE.md). That validation is
/// `action::hall::init`'s job; this type only carries what clap parsed.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Directory to create the hall in. Defaults to the current directory.
    #[arg(default_value = ".")]
    pub path: Utf8PathBuf,

    /// The hall's name. Defaults to the target directory's name.
    #[arg(long)]
    pub name: Option<String>,

    /// The provider to record as the hall's sole available (and default)
    /// provider. Defaults to `claude-code`.
    #[arg(long)]
    pub provider: Option<String>,
}

impl From<InitArgs> for InitInput {
    /// A straight field copy — no validation. See the type doc comment.
    fn from(args: InitArgs) -> Self {
        Self {
            path: args.path,
            name: args.name,
            provider: args.provider,
        }
    }
}

/// Colour control for the root command. See [`Cli::color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorMode {
    /// Follow `NO_COLOR` / `FORCE_COLOR` / tty detection.
    Auto,
    /// Force colour on.
    Always,
    /// Force colour off.
    Never,
}

impl ColorMode {
    /// The `Option<bool>` override `infra::term::colour` expects. `cli`
    /// cannot import `infra` itself (see the layering table) — this is a
    /// plain value conversion, applied by `bin/ivar.rs`, which can reach
    /// both `cli` and `infra`.
    #[must_use]
    pub const fn as_override(self) -> Option<bool> {
        match self {
            Self::Auto => None,
            Self::Always => Some(true),
            Self::Never => Some(false),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        // `debug_assert` panics on a malformed clap definition (duplicate
        // ids, conflicting args, ...) — the cheapest test that the derive
        // actually produced a usable `Command`.
        Cli::command().debug_assert();
    }

    #[test]
    fn init_args_convert_into_init_input_without_change() {
        let args = InitArgs {
            path: Utf8PathBuf::from("some/dir"),
            name: Some("acme".to_owned()),
            provider: Some("opencode".to_owned()),
        };

        let input: InitInput = args.into();

        assert_eq!(input.path, Utf8PathBuf::from("some/dir"));
        assert_eq!(input.name, Some("acme".to_owned()));
        assert_eq!(input.provider, Some("opencode".to_owned()));
    }

    #[test]
    fn color_mode_maps_to_the_override_colour_expects() {
        assert_eq!(ColorMode::Auto.as_override(), None);
        assert_eq!(ColorMode::Always.as_override(), Some(true));
        assert_eq!(ColorMode::Never.as_override(), Some(false));
    }
}
