use camino::Utf8PathBuf;
use clap::{Args, Subcommand};

use crate::action::session::{
    connect as session_connect, conversion as session_conversion, env_cmd as session_env_cmd,
    relay as session_relay, start as session_start, stop as session_stop,
};

/// The `ivar session` surface.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Open a session: view dir over a feature's promoted repos, agent
    /// running in it, TUI on top.
    Start(SessionStartArgs),
    /// Re-bind to an existing live session: locate it, re-materialise its
    /// view dir, and emit the binding as `IVAR_*` env vars.
    Connect(SessionConnectArgs),
    /// Promote a discovery session to a feature session, keeping its name.
    Convert(SessionConvertArgs),
    /// Stop a session — tear down its view dir and end any running harness.
    /// Omitting the session stops *every* session in the hall.
    Stop(SessionStopArgs),
    /// Remove dead sessions: view dirs that exist but hold no readable
    /// `state.json`. A session with a readable record is never touched.
    Prune,
    /// Relay session info: four-line output contract for external consumers.
    Relay(SessionRelayArgs),
    /// Resolve and output the session environment by walking up from cwd.
    Env(SessionEnvArgs),
    /// Internal launcher: apply Landlock sandbox for the given session and exec child.
    #[command(hide = true)]
    Sandbox(SessionSandboxArgs),
}

/// Arguments for the internal hidden `ivar session sandbox` launcher.
#[derive(Debug, Args)]
pub struct SessionSandboxArgs {
    /// The session id whose writable set to enforce.
    #[arg(long)]
    pub session: String,

    /// Resume the provider's previous conversation, where it supports it.
    #[arg(long)]
    pub resume: bool,
    /// The command line to execute after applying the sandbox.
    #[arg(last = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

/// Arguments for `ivar session start`.
#[derive(Debug, Args)]
pub struct SessionStartArgs {
    /// The feature to open a session for. Omit for a discovery session: no
    /// feature bound, every repo read-only on its default branch.
    pub feature: Option<String>,
    /// Resume an existing session, where the harness supports it.
    #[arg(long)]
    pub resume: bool,
    /// The provider to run. Defaults to the hall's default provider.
    #[arg(long)]
    pub provider: Option<String>,
    /// Create the session without launching a provider. The view dir persists
    /// after this command returns, until an explicit stop.
    #[arg(long)]
    pub detached: bool,
    /// Relay: a fresh session on the same feature under a different provider
    /// than the feature's most recent session. Requires `--provider`.
    #[arg(long)]
    pub relay: bool,
}

/// Arguments for `ivar session connect`.
#[derive(Debug, Args)]
pub struct SessionConnectArgs {
    /// The session id, or a unique prefix of one.
    pub session_id: Option<String>,
    /// Narrow the search to sessions bound to this feature.
    #[arg(long)]
    pub feature: Option<String>,
    /// Attach or create: take the feature's most recent session that no
    /// harness is running in, and start a fresh detached one when every
    /// candidate is busy or none exist. Needs `--feature`.
    #[arg(long)]
    pub create: bool,
}

/// Arguments for `ivar session convert`.
#[derive(Debug, Args)]
pub struct SessionConvertArgs {
    /// The discovery session's id, or a unique prefix of one.
    pub session_id: String,
}

/// Arguments for `ivar session stop`.
#[derive(Debug, Args)]
pub struct SessionStopArgs {
    /// The session to stop — its id, or a unique prefix of one.
    ///
    /// Omitting it stops **every** session in the hall: every discovery
    /// session and every feature's sessions, not just this feature's and not
    /// just the most recent. Pass `$IVAR_SESSION_ID` to stop only your own.
    pub session: Option<String>,
}

/// Arguments for `ivar session relay`.
///
/// A thin alias over `session start --relay`: the same feature under a
/// different provider. It carries no logic of its own — see the session
/// relay action — so its surface mirrors start's relay flags.
#[derive(Debug, Args)]
pub struct SessionRelayArgs {
    /// The feature to relay a session for.
    pub feature: String,
    /// The provider to relay to. Required — relay must switch providers.
    #[arg(long)]
    pub provider: String,
}

/// Arguments for `ivar session env`.
#[derive(Debug, Args)]
pub struct SessionEnvArgs {
    /// Working directory to resolve from. Defaults to current working directory.
    #[arg(long)]
    pub cwd: Option<String>,
}

impl From<SessionEnvArgs> for session_env_cmd::EnvInput {
    fn from(args: SessionEnvArgs) -> Self {
        Self {
            cwd: args.cwd.map(Utf8PathBuf::from),
        }
    }
}

impl From<SessionStartArgs> for session_start::StartInput {
    fn from(args: SessionStartArgs) -> Self {
        let SessionStartArgs {
            feature,
            resume,
            provider,
            detached,
            relay,
        } = args;
        Self {
            feature,
            resume,
            provider,
            detached,
            relay,
        }
    }
}

impl From<SessionConnectArgs> for session_connect::ConnectInput {
    fn from(args: SessionConnectArgs) -> Self {
        let SessionConnectArgs {
            session_id,
            feature,
            create,
        } = args;
        Self {
            session_id,
            feature,
            create,
        }
    }
}

impl From<SessionConvertArgs> for session_conversion::ConvertInput {
    fn from(args: SessionConvertArgs) -> Self {
        let SessionConvertArgs { session_id } = args;
        Self { session_id }
    }
}

impl From<SessionStopArgs> for session_stop::StopInput {
    fn from(args: SessionStopArgs) -> Self {
        let SessionStopArgs { session } = args;
        Self { session }
    }
}

impl From<SessionRelayArgs> for session_relay::RelayInput {
    fn from(args: SessionRelayArgs) -> Self {
        let SessionRelayArgs { feature, provider } = args;
        Self { feature, provider }
    }
}
