#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::graph::{EdgeKind, Provenance, SymbolKind};

#[test]
fn test_rust_symbols_and_local_calls() {
    let code = r#"
/// A helper function.
fn helper() {}

pub fn main() {
helper();
}
"#;
    let res = extract_file("my-repo", "src/main.rs", code, SupportedLanguage::Rust)
        .expect("extraction failed");

    assert_eq!(res.symbols.len(), 2);
    let helper = &res.symbols[0];
    assert_eq!(helper.name, "helper");
    assert_eq!(helper.kind, SymbolKind::Fn);
    assert!(!helper.is_exported);
    assert_eq!(helper.docstring.as_deref(), Some("/// A helper function."));

    let main_sym = &res.symbols[1];
    assert_eq!(main_sym.name, "main");
    assert_eq!(main_sym.kind, SymbolKind::Fn);
    assert!(main_sym.is_exported);

    // Check call edge
    let call_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call_edge.to_name.as_deref(), Some("helper"));
    assert_eq!(call_edge.provenance, Provenance::Extracted);
    assert!((call_edge.confidence - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_typescript_imports_and_calls() {
    let code = r#"
import { run } from './runner';

export function execute() {
run();
}
"#;
    let res = extract_file(
        "ts-repo",
        "src/index.ts",
        code,
        SupportedLanguage::TypeScript,
    )
    .expect("extraction failed");

    assert_eq!(res.symbols.len(), 1);
    let exec_sym = &res.symbols[0];
    assert_eq!(exec_sym.name, "execute");
    assert_eq!(exec_sym.kind, SymbolKind::Fn);
    assert!(exec_sym.is_exported);

    // Check import edge
    let import_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Imports)
        .expect("import edge");
    assert_eq!(import_edge.to_name.as_deref(), Some("./runner"));
    assert_eq!(import_edge.provenance, Provenance::Extracted);
    assert!((import_edge.confidence - 0.95).abs() < f64::EPSILON);

    // Check calls edge to imported function
    let call_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call_edge.to_name.as_deref(), Some("run"));
    assert_eq!(call_edge.provenance, Provenance::Extracted);
    assert!((call_edge.confidence - 0.95).abs() < f64::EPSILON);
}

#[test]
fn test_method_call_on_receiver() {
    let code = r#"
fn process(runner: &Runner) {
runner.execute();
}
"#;
    let res = extract_file("my-repo", "src/lib.rs", code, SupportedLanguage::Rust)
        .expect("extraction failed");

    let call_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(call_edge.to_name.as_deref(), Some("runner.execute"));
    assert_eq!(call_edge.provenance, Provenance::Inferred);
    assert!((call_edge.confidence - 0.85).abs() < f64::EPSILON);
}

#[test]
fn test_span_coordinates() {
    let code = "fn foo() {}\n";
    let res =
        extract_file("repo", "foo.rs", code, SupportedLanguage::Rust).expect("extraction failed");

    assert_eq!(res.symbols.len(), 1);
    let sym = &res.symbols[0];
    assert_eq!(sym.span.start_line, 1);
    assert_eq!(sym.span.start_col, 1);
    assert_eq!(sym.span.end_line, 1);
    assert_eq!(sym.span.end_col, 12);
}
