//! AST symbol, call, and import extraction engine using Tree-sitter.
//!
//! Implements a 2-pass extraction pipeline:
//! - Pass 1: Extract definitions (symbols) and record their spans, signatures, docstrings, and visibility.
//! - Pass 2: Extract invocations (calls) and imports, attaching 5-tier confidence heuristic and provenance.

use std::collections::HashSet;
use thiserror::Error;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor};

use crate::domain::graph::{Edge, EdgeKind, Provenance, Span, Symbol, SymbolKind};
use crate::infra::graph::parser::{
    compile_rust_query, compile_typescript_query, ParserError, SupportedLanguage, TreeSitterEngine,
};

/// Errors produced during AST extraction.
#[derive(Debug, Error)]
pub enum ExtractorError {
    #[error("Parser error: {0}")]
    Parser(#[from] ParserError),
    #[error("Tree-sitter query error: {0}")]
    Query(#[from] tree_sitter::QueryError),
    #[error("Unsupported language for AST extraction: {0:?}")]
    UnsupportedLanguage(SupportedLanguage),
}

/// The extracted symbols and edges from a single source file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExtractedFile {
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
}

/// Extracts symbols, imports, and calls from a source file using Tree-sitter AST queries.
pub fn extract_file(
    repo: &str,
    _file_path: &str,
    content: &str,
    lang: SupportedLanguage,
) -> Result<ExtractedFile, ExtractorError> {
    let mut engine = TreeSitterEngine::new();
    let tree = engine.parse(lang, content)?;
    let root = tree.root_node();

    let query = match lang {
        SupportedLanguage::Rust => compile_rust_query()?,
        SupportedLanguage::TypeScript | SupportedLanguage::Tsx => compile_typescript_query()?,
        _ => return Err(ExtractorError::UnsupportedLanguage(lang)),
    };

    let source_bytes = content.as_bytes();

    // Pass 1: Extract definitions
    let symbols = extract_symbols(repo, root, &query, source_bytes, lang);
    let local_symbols: HashSet<String> = symbols.iter().map(|s| s.name.clone()).collect();

    // Pass 2: Extract imports and calls
    let edges = extract_edges(repo, root, &query, source_bytes, lang, &local_symbols);

    Ok(ExtractedFile { symbols, edges })
}

fn extract_symbols(
    repo: &str,
    root: Node,
    query: &Query,
    source_bytes: &[u8],
    lang: SupportedLanguage,
) -> Vec<Symbol> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source_bytes);

    let symbol_kind_idx = query.capture_index_for_name("symbol.kind");
    let symbol_name_idx = query.capture_index_for_name("symbol.name");

    let mut symbols = Vec::new();

    while let Some(m) = matches.next() {
        let mut kind_node = None;
        let mut name_node = None;

        for cap in m.captures {
            if Some(cap.index) == symbol_kind_idx {
                kind_node = Some(cap.node);
            } else if Some(cap.index) == symbol_name_idx {
                name_node = Some(cap.node);
            }
        }

        if let (Some(k_node), Some(n_node)) = (kind_node, name_node) {
            let name = n_node.utf8_text(source_bytes).unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }

            let kind = determine_symbol_kind(k_node.kind(), lang);
            let span = node_to_span(k_node);
            let is_exported = check_exported(k_node, source_bytes, lang);
            let signature = extract_signature(k_node, source_bytes);
            let docstring = extract_docstring(k_node, source_bytes, lang);

            symbols.push(Symbol {
                id: None,
                file_id: None,
                repo: repo.to_string(),
                name,
                kind,
                scope: None,
                signature,
                docstring,
                span,
                is_exported,
            });
        }
    }

    symbols
}

fn extract_edges(
    repo: &str,
    root: Node,
    query: &Query,
    source_bytes: &[u8],
    _lang: SupportedLanguage,
    local_symbols: &HashSet<String>,
) -> Vec<Edge> {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source_bytes);

    let call_target_idx = query.capture_index_for_name("call.target");
    let call_receiver_idx = query.capture_index_for_name("call.receiver");
    let import_path_idx = query.capture_index_for_name("import.path");
    let import_source_idx = query.capture_index_for_name("import.source");
    let import_name_idx = query.capture_index_for_name("import.name");

    let mut edges = Vec::new();
    let mut imported_names: HashSet<String> = HashSet::new();

    // First collect imports and build edges + imported_names set
    while let Some(m) = matches.next() {
        for cap in m.captures {
            if Some(cap.index) == import_path_idx || Some(cap.index) == import_source_idx {
                let raw_text = cap.node.utf8_text(source_bytes).unwrap_or("").trim_matches(&['"', '\'', ';', ' '][..]);
                let span = node_to_span(cap.node);
                
                // For Rust use paths (e.g. `foo::bar` or `bar`), record leaf name as imported
                if let Some(leaf) = raw_text.split("::").last() {
                    if !leaf.contains('{') && !leaf.contains('*') && !leaf.is_empty() {
                        imported_names.insert(leaf.trim().to_string());
                    }
                }

                edges.push(Edge {
                    id: None,
                    repo: repo.to_string(),
                    file_id: None,
                    from_symbol_id: None,
                    to_symbol_id: None,
                    to_name: Some(raw_text.to_string()),
                    kind: EdgeKind::Imports,
                    provenance: Provenance::Extracted,
                    line: span.start_line,
                    col: span.start_col,
                    confidence: 0.95,
                });
            } else if Some(cap.index) == import_name_idx {
                let name = cap.node.utf8_text(source_bytes).unwrap_or("").trim();
                if !name.is_empty() {
                    imported_names.insert(name.to_string());
                }
            }
        }
    }

    // Now collect calls
    let mut cursor2 = QueryCursor::new();
    let mut matches2 = cursor2.matches(query, root, source_bytes);

    while let Some(m) = matches2.next() {
        let mut target_node = None;
        let mut receiver_node = None;

        for cap in m.captures {
            if Some(cap.index) == call_target_idx {
                target_node = Some(cap.node);
            } else if Some(cap.index) == call_receiver_idx {
                receiver_node = Some(cap.node);
            }
        }

        if let Some(t_node) = target_node {
            let target_name = t_node.utf8_text(source_bytes).unwrap_or("").to_string();
            if target_name.is_empty() {
                continue;
            }

            let span = node_to_span(t_node);
            let receiver_name = receiver_node.and_then(|r| r.utf8_text(source_bytes).ok());

            let (to_name, provenance, confidence) = if local_symbols.contains(&target_name) {
                // Tier 1: Local definition
                (Some(target_name), Provenance::Extracted, 1.0)
            } else if imported_names.contains(&target_name) {
                // Tier 2: Imported symbol
                (Some(target_name), Provenance::Extracted, 0.95)
            } else if let Some(receiver) = receiver_name {
                // Tier 3: Method call with receiver
                (Some(format!("{receiver}.{target_name}")), Provenance::Inferred, 0.85)
            } else {
                // Tier 4: General / dynamic / unresolved call
                (Some(target_name), Provenance::Inferred, 0.70)
            };

            edges.push(Edge {
                id: None,
                repo: repo.to_string(),
                file_id: None,
                from_symbol_id: None,
                to_symbol_id: None,
                to_name,
                kind: EdgeKind::Calls,
                provenance,
                line: span.start_line,
                col: span.start_col,
                confidence,
            });
        }
    }

    edges
}

fn determine_symbol_kind(node_kind: &str, _lang: SupportedLanguage) -> SymbolKind {
    match node_kind {
        "function_item" | "function_declaration" => SymbolKind::Fn,
        "method_definition" => SymbolKind::Method,
        "struct_item" => SymbolKind::Struct,
        "class_declaration" => SymbolKind::Class,
        "trait_item" => SymbolKind::Trait,
        "interface_declaration" => SymbolKind::Interface,
        "enum_item" | "enum_declaration" => SymbolKind::Enum,
        "type_alias_declaration" => SymbolKind::Other("type_alias".to_string()),
        "impl_item" => SymbolKind::Other("impl".to_string()),
        "mod_item" => SymbolKind::Mod,
        "const_item" | "static_item" => SymbolKind::Const,
        other => SymbolKind::Other(other.to_string()),
    }
}

fn node_to_span(node: Node) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span::new(
        start.row + 1,
        start.column + 1,
        end.row + 1,
        end.column + 1,
    )
}

fn check_exported(node: Node, source_bytes: &[u8], lang: SupportedLanguage) -> bool {
    match lang {
        SupportedLanguage::Rust => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "visibility_modifier" {
                    return true;
                }
            }
            false
        }
        SupportedLanguage::TypeScript | SupportedLanguage::Tsx => {
            // Check if parent is export_statement or node has export modifier
            if let Some(parent) = node.parent() {
                if parent.kind() == "export_statement" {
                    return true;
                }
            }
            let text = node.utf8_text(source_bytes).unwrap_or("");
            text.starts_with("export ")
        }
        _ => false,
    }
}

fn extract_signature(node: Node, source_bytes: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameters" || child.kind() == "formal_parameters" {
            let start = node.start_position().row;
            let p_end = child.end_position().row;
            // If signature spans multiple lines or single line, extract from node start to parameters end
            let node_text = node.utf8_text(source_bytes).ok()?;
            if let Some(brace_pos) = node_text.find('{') {
                return Some(node_text[..brace_pos].trim().to_string());
            } else if let Some(semi_pos) = node_text.find(';') {
                return Some(node_text[..semi_pos].trim().to_string());
            }
            let _ = (start, p_end);
        }
    }
    None
}

fn extract_docstring(node: Node, source_bytes: &[u8], _lang: SupportedLanguage) -> Option<String> {
    let mut prev = node.prev_sibling();
    let mut comments = Vec::new();

    while let Some(sibling) = prev {
        if sibling.kind() == "line_comment" || sibling.kind() == "comment" {
            if let Ok(comment_text) = sibling.utf8_text(source_bytes) {
                if comment_text.starts_with("///") || comment_text.starts_with("/**") || comment_text.starts_with("//") {
                    comments.push(comment_text.trim().to_string());
                }
            }
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

#[cfg(test)]
mod tests {
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
        let call_edge = res.edges.iter().find(|e| e.kind == EdgeKind::Calls).expect("call edge");
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
        let res = extract_file("ts-repo", "src/index.ts", code, SupportedLanguage::TypeScript)
            .expect("extraction failed");

        assert_eq!(res.symbols.len(), 1);
        let exec_sym = &res.symbols[0];
        assert_eq!(exec_sym.name, "execute");
        assert_eq!(exec_sym.kind, SymbolKind::Fn);
        assert!(exec_sym.is_exported);

        // Check import edge
        let import_edge = res.edges.iter().find(|e| e.kind == EdgeKind::Imports).expect("import edge");
        assert_eq!(import_edge.to_name.as_deref(), Some("./runner"));
        assert_eq!(import_edge.provenance, Provenance::Extracted);
        assert!((import_edge.confidence - 0.95).abs() < f64::EPSILON);

        // Check calls edge to imported function
        let call_edge = res.edges.iter().find(|e| e.kind == EdgeKind::Calls).expect("call edge");
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

        let call_edge = res.edges.iter().find(|e| e.kind == EdgeKind::Calls).expect("call edge");
        assert_eq!(call_edge.to_name.as_deref(), Some("runner.execute"));
        assert_eq!(call_edge.provenance, Provenance::Inferred);
        assert!((call_edge.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_span_coordinates() {
        let code = "fn foo() {}\n";
        let res = extract_file("repo", "foo.rs", code, SupportedLanguage::Rust)
            .expect("extraction failed");

        assert_eq!(res.symbols.len(), 1);
        let sym = &res.symbols[0];
        assert_eq!(sym.span.start_line, 1);
        assert_eq!(sym.span.start_col, 1);
        assert_eq!(sym.span.end_line, 1);
        assert_eq!(sym.span.end_col, 12);
    }
}
