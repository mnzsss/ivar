//! Reads a file shipped from `src/harness/`.

#![allow(clippy::expect_used)]

pub(crate) fn shipped(path: &str) -> String {
    std::fs::read_to_string(format!("{}/src/harness/{path}", env!("CARGO_MANIFEST_DIR")))
        .expect("shipped file is readable")
}
