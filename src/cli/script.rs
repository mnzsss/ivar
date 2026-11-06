use clap::Subcommand;

/// The `ivar script` surface: the hall's scripts, under `.ivar/scripts/` and
/// `.ivar/scripts-local/`. Read-only: `ivar` never writes, moves or deletes a
/// script.
#[derive(Debug, Subcommand)]
pub enum ScriptCommand {
    /// List the hall's scripts: name, scope (personal or team), description,
    /// usage and env.
    List,
}
