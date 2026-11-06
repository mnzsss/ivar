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

fn payloads(provider: Provider, root: &Utf8Path, page: &str) -> (Value, Value) {
    match provider {
        Provider::ClaudeCode | Provider::T3 => (
            json!({"tool_name": "mcp__acme-ivar__memory_query", "tool_input": {"query": "teardown"}, "cwd": root.as_str(), "session_id": "n1"}),
            json!({"tool_name": "Read", "tool_input": {"file_path": page}, "cwd": root.as_str(), "session_id": "n1"}),
        ),
        Provider::OpenCode | Provider::Omp => (
            json!({"tool": "acme-ivar_memory_query", "args": {"query": "teardown"}, "cwd": root.as_str()}),
            json!({"tool": "read", "args": {"filePath": page}, "cwd": root.as_str()}),
        ),
    }
}

fn guard(root: &Utf8Path, provider: Provider, payload: &Value) {
    ivar()
        .current_dir(root)
        .env("IVAR_SESSION_ID", "contract-session")
        .args(["guard", "--provider", provider.id()])
        .write_stdin(payload.to_string())
        .assert()
        .success();
}

fn usage(root: &Utf8Path) -> Value {
    let out = ivar()
        .current_dir(root)
        .args(["--json", "memory", "status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<Value>(&out).unwrap()["usage"].clone()
}

#[test]
fn a_memory_read_after_memory_query_is_a_followup_where_the_matrix_says_supported() {
    for provider in Provider::SELECTABLE {
        let (_guard, root) = hall_root();
        ivar().current_dir(&root).arg("init").assert().success();
        let page = root.join("docs/memory/gotcha/a.md");
        std::fs::create_dir_all(page.parent().unwrap()).unwrap();
        std::fs::write(&page, "# A\nteardown\n").unwrap();
        ivar()
            .current_dir(&root)
            .args(["memory", "status"])
            .assert()
            .success();
        let (query, read) = payloads(provider, &root, page.as_str());

        guard(&root, provider, &query);
        guard(&root, provider, &read);

        let usage = usage(&root);
        match provider_support(provider).memory_misses {
            Support::Supported => {
                assert_eq!(usage["followup"], 1, "{provider}: {usage}");
                assert_eq!(usage["skipped"], 0, "{provider}: {usage}");
            }
            Support::NotSupported(_) => {
                assert_eq!(usage["followup"], 0, "{provider}: {usage}");
            }
        }
    }
}
