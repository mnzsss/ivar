use crate::domain::mcp::CredentialState;
use std::collections::BTreeMap;

pub(crate) fn parse_mcp_list(stdout: &str) -> BTreeMap<String, CredentialState> {
    let mut map = BTreeMap::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Checking MCP server health") {
            continue;
        }
        // Split on the last " - " to isolate status
        let Some((server_part, status_part)) = line.rsplit_once(" - ") else {
            continue;
        };
        // Split on the first ": " to isolate name (names may contain colons or spaces)
        let Some((name, _target)) = server_part.split_once(": ") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let status = status_part.trim();
        let state = if status.contains("✔ Connected") || status == "Connected" {
            CredentialState::Authenticated
        } else if status.contains("! Needs authentication") || status == "Needs authentication" {
            CredentialState::Missing
        } else {
            CredentialState::Unknown
        };
        map.insert(name.to_owned(), state);
    }
    map
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/claude_code/live.rs"]
mod tests;
