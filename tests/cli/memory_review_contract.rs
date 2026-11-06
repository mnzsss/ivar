#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8Path;
use ivar::domain::capability::{Support, provider_support};
use ivar::domain::provider::Provider;
use serde_json::{Value, json};

use crate::common::{hall_root, ivar};

fn payloads(provider: Provider, page: &str) -> (Value, Value) {
    match provider {
        Provider::ClaudeCode | Provider::T3 => (
            json!({"hook_event_name": "PostToolUse", "session_id": "native-1", "tool_name": "mcp__acme-ivar__memory_query", "tool_input": {"query": "locks"}, "tool_response": {}}),
            json!({"hook_event_name": "PostToolUse", "session_id": "native-1", "tool_name": "Write", "tool_input": {"file_path": page, "content": "# Locks\n"}, "tool_response": {}}),
        ),
        Provider::OpenCode => (
            json!({"event": "tool", "session": "native-1", "tool": "acme-ivar_memory_query", "args": {"query": "locks"}, "failed": false, "result": ""}),
            json!({"event": "tool", "session": "native-1", "tool": "write", "args": {"filePath": page, "content": "# Locks\n"}, "failed": false, "result": ""}),
        ),
        Provider::Omp => (
            json!({"event": "tool", "session": "native-1", "tool": "acme-ivar_memory_query", "args": {"query": "locks"}, "failed": false, "result": ""}),
            json!({"event": "tool", "session": "native-1", "tool": "write", "args": {"path": page, "content": "# Locks\n"}, "failed": false, "result": ""}),
        ),
    }
}

fn run(root: &Utf8Path, args: &[&str]) {
    ivar().current_dir(root).args(args).assert().success();
}

fn capture(root: &Utf8Path, provider: Provider, payload: &Value) {
    ivar()
        .current_dir(root)
        .env("IVAR_HALL", root)
        .env("IVAR_SESSION_ID", "contract-session")
        .env("IVAR_FEATURE", "checkout")
        .args(["memory", "capture", "--provider", provider.id()])
        .write_stdin(payload.to_string())
        .assert()
        .success();
}

fn review(root: &Utf8Path) -> Value {
    let out = ivar()
        .current_dir(root)
        .env_remove("IVAR_SESSION_ID")
        .args(["--json", "memory", "review", "checkout"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn a_captured_write_and_query_reach_the_review_where_the_matrix_says_supported() {
    for provider in Provider::SELECTABLE {
        let (_guard, root) = hall_root();
        run(&root, &["init"]);
        run(&root, &["feature", "create", "checkout"]);
        run(&root, &["plan", "create", "checkout"]);
        run(&root, &["plan", "approve", "checkout", "requirements"]);
        let page = root.join("docs/memory/decision/locks.md");
        let (query, write) = payloads(provider, page.as_str());

        capture(&root, provider, &query);
        capture(&root, provider, &write);
        run(&root, &["plan", "approve", "checkout", "analysis"]);

        let report = review(&root);
        let written: Vec<&str> = report["written"]
            .as_array()
            .unwrap()
            .iter()
            .map(|page| page["path"].as_str().unwrap())
            .collect();
        let gaps: Vec<&str> = report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|gap| gap["kind"].as_str().unwrap())
            .collect();
        match provider_support(provider).memory_review {
            Support::Supported => {
                assert_eq!(
                    written,
                    ["docs/memory/decision/locks.md"],
                    "{provider}: {report}"
                );
                assert!(
                    !gaps.contains(&"analysis_without_query"),
                    "{provider}: {report}"
                );
            }
            Support::NotSupported(_) => {
                assert!(written.is_empty(), "{provider}: {report}");
            }
        }
    }
}
