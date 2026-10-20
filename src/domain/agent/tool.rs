//! How each coding tool is launched non-interactively, and what its output means.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

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
    // permission prompt, which is safe only because the container is the sandbox.
    pub fn env(self) -> Vec<(String, String)> {
        match self {
            Self::Opencode => vec![(
                "OPENCODE_CONFIG_CONTENT".to_owned(),
                r#"{"permission":{"*":"allow"}}"#.to_owned(),
            )],
            Self::Codex | Self::Claude => Vec::new(),
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
