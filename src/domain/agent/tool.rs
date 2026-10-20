//! How each coding tool is launched non-interactively, and what its output means.

use std::str::FromStr;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

/// One login file a host run copies into its private state dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub real: Utf8PathBuf,
    pub private: &'static str,
    /// Holds a rotating OAuth token, so a refresh during the run must reach the user's copy.
    pub write_back: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Codex,
    Claude,
    Opencode,
}

impl FromStr for Tool {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "opencode" => Ok(Self::Opencode),
            other => Err(format!(
                "unknown tool `{other}`; expected one of codex, claude, opencode"
            )),
        }
    }
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Opencode => "opencode",
        }
    }

    pub fn argv(self, model: Option<&str>, prompt: &str) -> Vec<String> {
        let (head, model_flag): (&[&str], &str) = match self {
            Self::Codex => (
                &[
                    "codex",
                    "exec",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "--skip-git-repo-check",
                ],
                "-m",
            ),
            Self::Claude => (
                &["claude", "-p", "--permission-mode", "bypassPermissions"],
                "--model",
            ),
            Self::Opencode => (&["opencode", "run"], "-m"),
        };
        let mut argv: Vec<String> = head.iter().map(|s| (*s).to_owned()).collect();
        if let Some(model) = model {
            argv.extend([model_flag.to_owned(), model.to_owned()]);
        }
        argv.push(prompt.to_owned());
        argv
    }

    // opencode reads inline config from OPENCODE_CONFIG_CONTENT; `"*": "allow"` answers every
    // permission prompt, which is safe only because the container or the Landlock sandbox bounds writes.
    pub fn env(self) -> Vec<(String, String)> {
        match self {
            Self::Opencode => vec![(
                "OPENCODE_CONFIG_CONTENT".to_owned(),
                r#"{"permission":{"*":"allow"}}"#.to_owned(),
            )],
            Self::Codex | Self::Claude => Vec::new(),
        }
    }

    /// The login files a host run copies into its private state dir:
    /// `real` is where the user's own login lives, `private` is relative to the state dir.
    pub fn credentials(
        self,
        home: &Utf8Path,
        var: &dyn Fn(&str) -> Option<String>,
    ) -> Vec<Credential> {
        let or_home = |key: &str, fallback: &str| {
            var(key).map_or_else(|| home.join(fallback), Utf8PathBuf::from)
        };
        let credential = |real: Utf8PathBuf, private, write_back| Credential {
            real,
            private,
            write_back,
        };
        match self {
            Self::Claude => {
                let dir = or_home("CLAUDE_CONFIG_DIR", ".claude");
                let config = var("CLAUDE_CONFIG_DIR")
                    .map_or_else(|| home.join(".claude.json"), |_| dir.join(".claude.json"));
                vec![
                    credential(dir.join(".credentials.json"), ".credentials.json", true),
                    credential(config, ".claude.json", false),
                ]
            }
            Self::Codex => vec![credential(
                or_home("CODEX_HOME", ".codex").join("auth.json"),
                "auth.json",
                true,
            )],
            Self::Opencode => vec![credential(
                or_home("XDG_DATA_HOME", ".local/share").join("opencode/auth.json"),
                "data/opencode/auth.json",
                true,
            )],
        }
    }

    /// Variables that make the tool keep all its state under `state`.
    pub fn state_env(self, state: &Utf8Path) -> Vec<(String, String)> {
        let pair = |key: &str, dir: Utf8PathBuf| (key.to_owned(), dir.into_string());
        match self {
            Self::Claude => vec![pair("CLAUDE_CONFIG_DIR", state.to_path_buf())],
            Self::Codex => vec![pair("CODEX_HOME", state.to_path_buf())],
            Self::Opencode => [
                ("XDG_DATA_HOME", "data"),
                ("XDG_STATE_HOME", "state"),
                ("XDG_CONFIG_HOME", "config"),
                ("XDG_CACHE_HOME", "cache"),
            ]
            .into_iter()
            .map(|(key, sub)| pair(key, state.join(sub)))
            .collect(),
        }
    }

    pub fn blocked_markers(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &["approval required", "waiting for approval"],
            Self::Claude => &["requires approval", "permission to use"],
            Self::Opencode => &["permission requested", "requires permission"],
        }
    }

    pub fn is_blocked(self, output: &str) -> bool {
        let lower = output.to_lowercase();
        self.blocked_markers().iter().any(|m| lower.contains(m))
    }

    pub fn cost_usd(self, output: &str) -> Option<f64> {
        let key = match self {
            Self::Claude => "total_cost_usd",
            Self::Codex | Self::Opencode => "cost_usd",
        };
        output
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
            .find_map(|value| value.get(key).and_then(serde_json::Value::as_f64))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/tool.rs"]
mod tests;
