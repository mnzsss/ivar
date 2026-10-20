//! The whole environment of a `runtime: host` run: built from nothing, so a
//! credential the parent holds reaches the tool only when it is listed here.

use camino::Utf8Path;

use super::tool::Tool;

const KEPT: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "LANG"];

fn is_kept(key: &str) -> bool {
    KEPT.contains(&key) || key.starts_with("LC_")
}

pub fn host_env(
    parent: &[(String, String)],
    tmpdir: &str,
    secrets: &[(String, String)],
    tool: Tool,
    state: &Utf8Path,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = parent
        .iter()
        .filter(|(key, _)| is_kept(key))
        .cloned()
        .collect();
    env.extend(
        [
            ("TMPDIR", tmpdir),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_TERMINAL_PROMPT", "0"),
            // An empty helper resets the list, so a repo-level `credential.helper` never runs.
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "credential.helper"),
            ("GIT_CONFIG_VALUE_0", ""),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned())),
    );
    env.extend(secrets.iter().cloned());
    env.extend(tool.env());
    env.extend(tool.state_env(state));
    env
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/agent/host_env.rs"]
mod tests;
