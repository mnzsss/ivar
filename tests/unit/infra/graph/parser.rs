#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use streaming_iterator::StreamingIterator;
use tree_sitter::QueryCursor;

#[test]
fn test_from_extension() {
    assert_eq!(
        SupportedLanguage::from_extension("rs"),
        Some(SupportedLanguage::Rust)
    );
    assert_eq!(
        SupportedLanguage::from_extension("RS"),
        Some(SupportedLanguage::Rust)
    );
    assert_eq!(
        SupportedLanguage::from_extension("ts"),
        Some(SupportedLanguage::TypeScript)
    );
    assert_eq!(
        SupportedLanguage::from_extension("js"),
        Some(SupportedLanguage::TypeScript)
    );
    assert_eq!(
        SupportedLanguage::from_extension("tsx"),
        Some(SupportedLanguage::Tsx)
    );
    assert_eq!(
        SupportedLanguage::from_extension("jsx"),
        Some(SupportedLanguage::Tsx)
    );
    assert_eq!(
        SupportedLanguage::from_extension("json"),
        Some(SupportedLanguage::Json)
    );
    assert_eq!(
        SupportedLanguage::from_extension("md"),
        Some(SupportedLanguage::Markdown)
    );
    assert_eq!(
        SupportedLanguage::from_extension("markdown"),
        Some(SupportedLanguage::Markdown)
    );
    assert_eq!(SupportedLanguage::from_extension("unknown"), None);
}

#[test]
fn test_parse_rust_and_query() {
    let mut engine = TreeSitterEngine::new();
    let code = r#"
        pub fn test_fn(a: u32) -> bool {
            true
        }

        fn main() {
            test_fn(42);
        }
    "#;
    let tree = engine
        .parse(SupportedLanguage::Rust, code)
        .expect("parse rust");
    assert!(!TreeSitterEngine::has_error(&tree));

    let query = compile_rust_query().expect("compile rust query");
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), code.as_bytes());
    let mut count = 0;
    while let Some(_m) = matches.next() {
        count += 1;
    }
    assert!(count > 0, "rust query matches should not be empty");
}

#[test]
fn test_parse_typescript_and_query() {
    let mut engine = TreeSitterEngine::new();
    let code = r#"
        import { something } from "./module";

        export class Foo {
            bar() {
                this.bar();
            }
        }
    "#;
    let tree = engine
        .parse(SupportedLanguage::TypeScript, code)
        .expect("parse ts");
    assert!(!TreeSitterEngine::has_error(&tree));

    let query = compile_typescript_query().expect("compile ts query");
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), code.as_bytes());
    let mut count = 0;
    while let Some(_m) = matches.next() {
        count += 1;
    }
    assert!(count > 0, "ts query matches should not be empty");
}

#[test]
fn test_parse_tsx() {
    let mut engine = TreeSitterEngine::new();
    let code = r#"
        export const Component = () => <div>Hello</div>;
    "#;
    let tree = engine
        .parse(SupportedLanguage::Tsx, code)
        .expect("parse tsx");
    assert!(!TreeSitterEngine::has_error(&tree));
}

#[test]
fn test_parse_json() {
    let mut engine = TreeSitterEngine::new();
    let code = r#"{"key": "value", "num": 123}"#;
    let tree = engine
        .parse(SupportedLanguage::Json, code)
        .expect("parse json");
    assert!(!TreeSitterEngine::has_error(&tree));
}

#[test]
fn test_parse_markdown() {
    let mut engine = TreeSitterEngine::new();
    let code = "# Title\n\n```rust\nfn main() {}\n```\n";
    let tree = engine
        .parse(SupportedLanguage::Markdown, code)
        .expect("parse markdown");
    assert!(!TreeSitterEngine::has_error(&tree));
}
