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

#[test]
fn test_rust_cyclomatic_complexity() {
    let code = r#"
fn simple_fn() -> i32 {
    42
}

struct MyStruct {
    x: i32,
}

fn complex_fn(x: i32) -> i32 {
    if x > 10 && x < 20 {
        return 1;
    }
    match x {
        1 => 10,
        2 => 20,
        _ => 0,
    }
    while x > 0 {
        println!("{}", x);
    }
    for i in 0..5 {
        println!("{}", i);
    }
    x
}
"#;
    let res = extract_file("repo", "src/lib.rs", code, SupportedLanguage::Rust).expect("extract");

    let simple = res
        .symbols
        .iter()
        .find(|s| s.name == "simple_fn")
        .expect("simple_fn");
    assert_eq!(simple.complexity, Some(1));

    let struct_sym = res
        .symbols
        .iter()
        .find(|s| s.name == "MyStruct")
        .expect("MyStruct");
    assert_eq!(struct_sym.complexity, None);

    let complex = res
        .symbols
        .iter()
        .find(|s| s.name == "complex_fn")
        .expect("complex_fn");
    // Base: 1 + if(1) + &&(1) + match arms(3) + while(1) + for(1) = 8
    assert_eq!(complex.complexity, Some(8));
}

#[test]
fn test_typescript_cyclomatic_complexity() {
    let code = r#"
function simple(): number {
    return 1;
}

function branching(x: number, y: boolean): number {
    if (x > 0 || y) {
        for (let i = 0; i < 5; i++) {
            x += i;
        }
    }
    switch (x) {
        case 1:
            return 10;
        case 2:
            return 20;
        default:
            return x > 5 ? 100 : 0;
    }
}
"#;
    let res =
        extract_file("repo", "src/mod.ts", code, SupportedLanguage::TypeScript).expect("extract");

    let simple = res
        .symbols
        .iter()
        .find(|s| s.name == "simple")
        .expect("simple");
    assert_eq!(simple.complexity, Some(1));

    let branching = res
        .symbols
        .iter()
        .find(|s| s.name == "branching")
        .expect("branching");
    // Base: 1 + if(1) + ||(1) + for(1) + case(2) + ternary(1) = 7
    assert_eq!(branching.complexity, Some(7));
}

#[test]
fn test_rust_hierarchy_implements() {
    let code = r#"
pub struct Dog;
pub trait Animal {}

impl Animal for Dog {}
"#;
    let res = extract_file("repo", "src/dog.rs", code, SupportedLanguage::Rust).expect("extract");

    let impl_edge = res
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Implements)
        .expect("implements edge");
    assert_eq!(impl_edge.to_name.as_deref(), Some("Animal"));
    assert_eq!(impl_edge.provenance, Provenance::Extracted);
}

#[test]
fn test_typescript_hierarchy_inherits_and_implements() {
    let code = r#"
export class Dog extends Animal implements Runnable, Serializable {
    bark() {}
}

export interface Cat extends Animal, Pet {
    meow(): void;
}
"#;
    let res =
        extract_file("repo", "src/pets.ts", code, SupportedLanguage::TypeScript).expect("extract");

    let inherits_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Inherits)
        .collect();
    assert!(
        inherits_edges
            .iter()
            .any(|e| e.to_name.as_deref() == Some("Animal")),
        "inherits edge to Animal"
    );

    let implements_edges: Vec<_> = res
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Implements)
        .collect();
    assert!(
        implements_edges
            .iter()
            .any(|e| e.to_name.as_deref() == Some("Runnable")),
        "implements edge to Runnable"
    );
    assert!(
        implements_edges
            .iter()
            .any(|e| e.to_name.as_deref() == Some("Serializable")),
        "implements edge to Serializable"
    );
}
