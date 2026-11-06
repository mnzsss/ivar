//! CLI argument definitions for `ivar automation` subcommands.

use clap::{Args, Subcommand};

use crate::action::automation::scan::ScanInput;

#[derive(Debug, Subcommand)]
pub enum AutomationCommand {
    /// Report the requests users keep repeating, as candidates to automate: groups of similar prompts typed in at least 3 sessions on at least 2 days of the last 90. Read-only apart from draining the capture spool; the samples are memory text, evidence only. Run it from a session that can write `.ivar/` or with `--no-drain` where it cannot.
    Scan(ScanArgs),
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Keep only the groups with a prompt in a session of this feature or one of its descendants.
    #[arg(long)]
    pub feature: Option<String>,
    /// Do not drain the capture spool first. Nothing is written, so it works when `.ivar/` is not writable; prompts still in the spool are missed.
    #[arg(long)]
    pub no_drain: bool,
}

impl From<ScanArgs> for ScanInput {
    fn from(args: ScanArgs) -> Self {
        let ScanArgs { feature, no_drain } = args;
        Self {
            feature,
            drain: !no_drain,
        }
    }
}
