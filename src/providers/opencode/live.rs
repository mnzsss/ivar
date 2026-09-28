use crate::domain::mcp::CredentialState;
use std::collections::BTreeMap;

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_escape = false;
    for ch in input.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if ch == 'm' || ch == 'K' || ch == 'H' || ch == 'J' {
                in_escape = false;
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[allow(dead_code)]
pub(crate) fn parse_mcp_list(stdout: &str) -> BTreeMap<String, CredentialState> {
    let mut map = BTreeMap::new();
    let stripped = strip_ansi(stdout);
    for raw_line in stripped.lines() {
        let line = raw_line.trim();
        if !line.starts_with('●') {
            continue;
        }
        // Line format: ● <symbol> <name> <status text...>
        let rest = line.trim_start_matches('●').trim();
        if let Some(rest) = rest.strip_prefix('✓') {
            let rest = rest.trim();
            let name = if let Some((n, _)) = rest.split_once(" connected") {
                n.trim()
            } else {
                rest
            };
            if !name.is_empty() {
                map.insert(name.to_owned(), CredentialState::Authenticated);
            }
        } else if let Some(rest) = rest.strip_prefix('⚠') {
            let rest = rest.trim();
            let name = if let Some((n, _)) = rest.split_once(" needs authentication") {
                n.trim()
            } else {
                rest
            };
            if !name.is_empty() {
                map.insert(name.to_owned(), CredentialState::Missing);
            }
        } else {
            let rest = if let Some(stripped_prefix) = rest.strip_prefix('✗') {
                stripped_prefix.trim()
            } else if let Some(stripped_prefix) = rest.strip_prefix('?') {
                stripped_prefix.trim()
            } else {
                rest
            };
            let name = rest.split_whitespace().next().unwrap_or(rest);
            if !name.is_empty() {
                map.insert(name.to_owned(), CredentialState::Unknown);
            }
        }
    }
    map
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/opencode/live.rs"]
mod tests;
