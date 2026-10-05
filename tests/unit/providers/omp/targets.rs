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
