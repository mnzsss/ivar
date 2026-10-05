//! Unit tests for OMP target extraction (path keys only in Task 01).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use camino::Utf8PathBuf;

#[test]
fn extract_reads_path_keys_and_sets_writes_flag() {
    let payload_filepath = serde_json::json!({
        "filePath": "src/main.rs"
    });
    let extracted = extract("write", &payload_filepath);
    assert_eq!(extracted.targets, vec![Utf8PathBuf::from("src/main.rs")]);
    assert!(extracted.writes);

    let payload_path = serde_json::json!({
        "path": "src/lib.rs"
    });
    let extracted_read = extract("read", &payload_path);
    assert_eq!(
        extracted_read.targets,
        vec![Utf8PathBuf::from("src/lib.rs")]
    );
    assert!(!extracted_read.writes);
}

#[test]
fn extract_reads_hashline_headers_mv_and_apply_patch_lines() {
    // Hashline with multiple file sections and body line containing fake header
    let hashline_input = serde_json::json!({
        "input": "[src/main.rs#ABCD]\nPUT 1:\n+[other/file.rs#1234]\n[src/lib.rs#5678]\nPUT 2:\n+let x = 1;\n"
    });
    let extracted = extract("edit", &hashline_input);
    assert!(extracted.writes);
    assert_eq!(
        extracted.targets,
        vec![
            Utf8PathBuf::from("src/main.rs"),
            Utf8PathBuf::from("src/lib.rs")
        ]
    );

    // Hashline with MV command
    let mv_input = serde_json::json!({
        "input": "[src/old.rs#A1B2]\nMV \"src/nested/new.rs\"\n"
    });
    let extracted_mv = extract("edit", &mv_input);
    assert_eq!(
        extracted_mv.targets,
        vec![
            Utf8PathBuf::from("src/old.rs"),
            Utf8PathBuf::from("src/nested/new.rs")
        ]
    );

    // Apply patch headers
    let patch_input = serde_json::json!({
        "input": "*** Add File: src/added.rs\n*** Update File: src/updated.rs\n*** Delete File: src/deleted.rs\n*** Move to: src/moved.rs\n"
    });
    let extracted_patch = extract("apply_patch", &patch_input);
    assert_eq!(
        extracted_patch.targets,
        vec![
            Utf8PathBuf::from("src/added.rs"),
            Utf8PathBuf::from("src/updated.rs"),
            Utf8PathBuf::from("src/deleted.rs"),
            Utf8PathBuf::from("src/moved.rs")
        ]
    );
}

#[test]
fn extract_xd_ast_edit_and_direct_ast_edit_targets_and_handles_globs() {
    // xd://ast_edit with glob paths
    let write_ast = serde_json::json!({
        "path": "xd://ast_edit",
        "content": "{\"paths\": [\"src/**/*.rs\", \"tests/unit/foo.rs\", \"*.ts\"]}"
    });
    let extracted = extract("write", &write_ast);
    assert!(extracted.writes);
    assert_eq!(
        extracted.targets,
        vec![
            Utf8PathBuf::from("src"),
            Utf8PathBuf::from("tests/unit/foo.rs"),
            Utf8PathBuf::from(".")
        ]
    );

    // Direct ast_edit tool
    let direct_ast = serde_json::json!({
        "paths": ["crates/core/src/lib.rs"]
    });
    let extracted_direct = extract("ast_edit", &direct_ast);
    assert!(extracted_direct.writes);
    assert_eq!(
        extracted_direct.targets,
        vec![Utf8PathBuf::from("crates/core/src/lib.rs")]
    );

    // Unparseable JSON content on xd://ast_edit fails closed (writes=true, targets=[])
    let corrupt_ast = serde_json::json!({
        "path": "xd://ast_edit",
        "content": "not-json"
    });
    let extracted_corrupt = extract("write", &corrupt_ast);
    assert!(extracted_corrupt.writes);
    assert!(extracted_corrupt.targets.is_empty());
}

#[test]
fn extract_xd_lsp_and_direct_lsp_mutating_actions_and_read_only_actions() {
    // Mutating rename
    let lsp_rename = serde_json::json!({
        "path": "xd://lsp",
        "content": "{\"action\": \"rename\", \"file\": \"src/lib.rs\", \"new_name\": \"foo\"}"
    });
    let extracted = extract("write", &lsp_rename);
    assert!(extracted.writes);
    assert_eq!(extracted.targets, vec![Utf8PathBuf::from("src/lib.rs")]);

    // Mutating rename with file == "*"
    let lsp_wildcard = serde_json::json!({
        "path": "xd://lsp",
        "content": "{\"action\": \"rename\", \"file\": \"*\", \"new_name\": \"foo\"}"
    });
    let extracted_wild = extract("write", &lsp_wildcard);
    assert!(extracted_wild.writes);
    assert_eq!(extracted_wild.targets, vec![Utf8PathBuf::from(".")]);

    // Rename_file has two targets: file and new_name
    let lsp_mv = serde_json::json!({
        "path": "xd://lsp",
        "content": "{\"action\": \"rename_file\", \"file\": \"src/old.rs\", \"new_name\": \"src/new.rs\"}"
    });
    let extracted_mv = extract("write", &lsp_mv);
    assert!(extracted_mv.writes);
    assert_eq!(
        extracted_mv.targets,
        vec![
            Utf8PathBuf::from("src/old.rs"),
            Utf8PathBuf::from("src/new.rs")
        ]
    );

    // Read-only rename with apply: false
    let lsp_dry_rename = serde_json::json!({
        "path": "xd://lsp",
        "content": "{\"action\": \"rename\", \"file\": \"src/lib.rs\", \"apply\": false}"
    });
    let extracted_dry = extract("write", &lsp_dry_rename);
    assert!(!extracted_dry.writes);

    // Read-only hover
    let lsp_hover = serde_json::json!({
        "path": "xd://lsp",
        "content": "{\"action\": \"hover\", \"file\": \"src/lib.rs\"}"
    });
    let extracted_hover = extract("write", &lsp_hover);
    assert!(!extracted_hover.writes);

    // Direct lsp code_actions with apply: true is a write
    let direct_lsp = serde_json::json!({
        "action": "code_actions",
        "file": "src/lib.rs",
        "apply": true
    });
    let extracted_code_actions = extract("lsp", &direct_lsp);
    assert!(extracted_code_actions.writes);
    assert_eq!(
        extracted_code_actions.targets,
        vec![Utf8PathBuf::from("src/lib.rs")]
    );
}
