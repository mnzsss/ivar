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
