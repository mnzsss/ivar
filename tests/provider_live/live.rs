use std::time::Duration;

use camino::Utf8Path;
use serde_json::{Value, json};

pub(crate) const REPS: usize = 2;
pub(crate) const TURN_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const READ_PROMPT_1: &str =
    "Read web/src/index.ts. What is the command to run web locally? One line.";
pub(crate) const READ_PROMPT_2: &str =
    "Read web/src/index.ts again. What is the command to run web locally now? One line.";
pub(crate) const BASH_PROMPT: &str = "Use only your shell/bash tool. Run `cat app/packages/api/src/x.ts` \
    and tell me what it exports. Then tell me the api test command for the app repo. Say \
    UNKNOWN if you cannot find it.";
pub(crate) const DEEP_PROMPT: &str = "Read backend/src/index.ts. Then tell me the exact command to \
    restart the backend local services. Say UNKNOWN if you cannot find it.";
pub(crate) const WRITE_PROMPT: &str = "Use the Write tool to create web/src/new.ts containing \
    `export const x = 1;`. If the tool refuses, quote its error message.";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Live {
    Claude,
    Omp,
    OpenCode,
}

impl Live {
    /// The name `IVAR_LIVE_PROVIDERS` lists.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    /// The value of `ivar … --provider`.
    pub(crate) fn provider(self) -> &'static str {
        match self {
            Self::Claude => "claude-code",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    pub(crate) fn binary(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Omp => "omp",
            Self::OpenCode => "opencode",
        }
    }

    pub(crate) fn config_dir(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Omp => ".omp",
            Self::OpenCode => ".opencode",
        }
    }

    /// Tool allow-list flag values; opencode's `run` takes none, so its prompts name the tool.
    pub(crate) fn read_tools(self) -> &'static str {
        match self {
            Self::Claude => "Read,Glob,Grep",
            Self::Omp => "read,grep,glob",
            Self::OpenCode => "",
        }
    }

    pub(crate) fn bash_tools(self) -> &'static str {
        match self {
            Self::Claude => "Bash",
            Self::Omp => "bash",
            Self::OpenCode => "",
        }
    }

    pub(crate) fn enabled(self) -> bool {
        let on = std::env::var("IVAR_LIVE_PROVIDERS")
            .is_ok_and(|list| list.split(',').any(|name| name.trim() == self.key()));
        if !on {
            eprintln!(
                "skipped: IVAR_LIVE_PROVIDERS does not list `{}`",
                self.key()
            );
        }
        on
    }

    /// opencode `run` has no turn-end event: its turn ends when the process closes stdout.
    pub(crate) fn is_turn_end(self, event: &Value) -> bool {
        match self {
            Self::Claude => event["type"] == "result",
            Self::Omp => event["type"] == "agent_end",
            Self::OpenCode => false,
        }
    }

    /// Arguments for a one-shot run of `prompt` with `tools`.
    pub(crate) fn one_shot_args(self, view: &Utf8Path, prompt: &str, tools: &str) -> Vec<String> {
        let args: Vec<&str> = match self {
            Self::Claude => vec![
                "-p",
                prompt,
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku",
                "--strict-mcp-config",
                "--tools",
                tools,
                "--allowedTools",
                tools,
            ],
            Self::Omp => vec![
                "-p",
                "--mode",
                "json",
                "--no-session",
                "--no-title",
                "--model",
                "haiku",
                "--cwd",
                view.as_str(),
                "--tools",
                tools,
                prompt,
            ],
            Self::OpenCode => vec!["run", "--format", "json", prompt],
        };
        args.into_iter().map(str::to_owned).collect()
    }

    /// Arguments for a live process fed one prompt per turn on stdin (claude, omp only).
    pub(crate) fn multi_turn_args(self, view: &Utf8Path) -> Vec<String> {
        let tools = self.read_tools();
        let args: Vec<&str> = match self {
            Self::Claude => vec![
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "haiku",
                "--strict-mcp-config",
                "--tools",
                tools,
                "--allowedTools",
                tools,
            ],
            Self::Omp => vec![
                "--mode",
                "rpc",
                "--no-session",
                "--no-title",
                "--model",
                "haiku",
                "--cwd",
                view.as_str(),
                "--tools",
                tools,
            ],
            Self::OpenCode => panic!("the opencode leg is one-shot only"),
        };
        args.into_iter().map(str::to_owned).collect()
    }

    /// One user turn, as the provider's stdin protocol wants it.
    pub(crate) fn turn_message(self, turn: usize, text: &str) -> Value {
        match self {
            Self::Claude => {
                json!({ "type": "user", "message": { "role": "user", "content": text } })
            }
            Self::Omp => json!({ "id": format!("turn-{turn}"), "type": "prompt", "message": text }),
            Self::OpenCode => panic!("the opencode leg is one-shot only"),
        }
    }
}
