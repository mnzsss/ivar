//! The `Environment contract` table in ARCHITECTURE.md is the source of truth
//! for the session environment; this test keeps the code honest to it.

use std::collections::BTreeSet;

use camino::Utf8PathBuf;

use super::*;
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::store::layout::Layout;

fn documented_session_hook_vars() -> BTreeSet<String> {
    let text = include_str!("../../../../ARCHITECTURE.md");
    let table = text
        .split_once("## Environment contract")
        .expect("contract heading")
        .1;
    let mut vars = BTreeSet::new();
    for line in table.lines() {
        let Some(rest) = line.strip_prefix("| `") else {
            continue;
        };
        let cells: Vec<&str> = rest.split('|').map(str::trim).collect();
        let var = cells[0].strip_suffix('`').expect("backticked var");
        if cells.get(2) == Some(&"✓") {
            vars.insert(var.to_owned());
        }
    }
    vars
}

#[test]
fn session_hook_column_matches_session_env_keys() {
    let hall = Utf8PathBuf::from("/tmp/acme");
    let layout = Layout::at(hall);
    let session_id = SessionId::new("6f0c9d5f-0000-4000-8000-000000000000").unwrap();
    let view_dir =
        Utf8PathBuf::from("/tmp/acme/.ivar/sessions/6f0c9d5f-0000-4000-8000-000000000000");

    let mut produced: BTreeSet<String> = BTreeSet::new();
    let discovery = SessionEnv::build(&layout, &session_id, &view_dir, Provider::ClaudeCode, None);
    produced.extend(discovery.keys().iter().map(|s| s.to_string()));

    let feature = FeatureName::new("checkout").unwrap();
    let feature_env = SessionEnv::build(
        &layout,
        &session_id,
        &view_dir,
        Provider::ClaudeCode,
        Some(&feature),
    );
    produced.extend(feature_env.keys().iter().map(|s| s.to_string()));

    assert_eq!(
        documented_session_hook_vars(),
        produced,
        "ARCHITECTURE.md's session-hook column and SessionEnv::keys() disagree"
    );
}
