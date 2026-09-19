use clap::{Args, Subcommand};

use crate::action::discovery::create as discovery_create;
use crate::action::discovery::show as discovery_show;

/// `ivar discovery …`.
#[derive(Debug, Args)]
pub struct DiscoveryArgs {
    #[command(subcommand)]
    pub command: DiscoveryCommand,
}

#[derive(Debug, Subcommand)]
pub enum DiscoveryCommand {
    /// Start a unit of work's discovery brief. When run in a discovery session,
    /// writes `discovery.md` into the session view dir.
    Create(DiscoveryCreateArgs),
    /// List every unit of work with a discovery doc.
    List(DiscoveryListArgs),
    /// Print one unit of work's memory.
    Show(DiscoveryShowArgs),
    /// Add to a unit of work's memory. Appends a dated block by default;
    /// `--merge` replaces the whole document and requires `--expected-hash`.
    Amend(DiscoveryAmendArgs),
    /// End a discovery: `converted` when it became a feature, `abandoned`
    /// when it did not. The doc is kept either way.
    Close(DiscoveryCloseArgs),
}

/// `ivar discovery create <name> [--title …]`.
#[derive(Debug, Args)]
pub struct DiscoveryCreateArgs {
    /// The unit of work's name. Lowercase kebab-case.
    pub name: String,
    /// A human-readable title. Defaults to the name.
    #[arg(long)]
    pub title: Option<String>,
}

/// `ivar discovery list [--status …]`.
#[derive(Debug, Args)]
pub struct DiscoveryListArgs {
    /// Show only discoveries in this status.
    #[arg(long, value_parser = ["exploring", "converted", "abandoned", "unknown"])]
    pub status: Option<String>,
}

/// `ivar discovery show <name> [--path]`.
#[derive(Debug, Args)]
pub struct DiscoveryShowArgs {
    /// The unit of work's name.
    pub name: String,
    /// Print only the path, not the content.
    #[arg(long)]
    pub path: bool,
}

/// `ivar discovery amend <name> [--file <path>|-] [--merge --expected-hash <sha256>]`.
#[derive(Debug, Args)]
pub struct DiscoveryAmendArgs {
    /// The unit of work's name.
    pub name: String,
    /// Read the content from this file, or from stdin with `-`.
    #[arg(long)]
    pub file: Option<String>,
    /// Replace the whole document instead of appending to it.
    #[arg(long, requires = "expected_hash")]
    pub merge: bool,
    /// The document's current SHA-256. Required with `--merge`.
    #[arg(long)]
    pub expected_hash: Option<String>,
}

/// `ivar discovery close <name> --outcome converted|abandoned`.
#[derive(Debug, Args)]
pub struct DiscoveryCloseArgs {
    /// The unit of work's name.
    pub name: String,
    /// How it ended.
    #[arg(long, value_parser = ["converted", "abandoned"])]
    pub outcome: String,
}

impl From<DiscoveryCreateArgs> for discovery_create::CreateInput {
    fn from(args: DiscoveryCreateArgs) -> Self {
        let DiscoveryCreateArgs { name, title } = args;
        Self { name, title }
    }
}

impl From<DiscoveryShowArgs> for discovery_show::ShowInput {
    fn from(args: DiscoveryShowArgs) -> Self {
        let DiscoveryShowArgs { name, path } = args;
        Self {
            name,
            path_only: path,
        }
    }
}
