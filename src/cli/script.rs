use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

/// The `ivar script` surface: the hall's scripts, under `.ivar/scripts/` and
/// `.ivar/scripts-local/`. `ivar` never writes, moves or deletes a script; it
/// only records which content a human approved to run.
#[derive(Debug, Subcommand)]
pub enum ScriptCommand {
    /// List the hall's scripts: name, scope (personal or team), description,
    /// usage and env.
    List,
    /// Approve a script's exact content, by sha256, so an agent may run it
    /// without asking. Refuses (and records nothing) when the content matches
    /// a secret pattern.
    Approve(ScriptPathArgs),
    /// Report whether a script's current content is approved. Exits 0 when
    /// approved and 3 when unreviewed (never approved, or changed since).
    Check(ScriptPathArgs),
}

/// The script a verb acts on.
#[derive(Debug, Args)]
pub struct ScriptPathArgs {
    /// The script, relative to the working directory or absolute; it must lie
    /// inside the hall (e.g. `.ivar/scripts/deploy.sh`).
    pub path: Utf8PathBuf,
}
