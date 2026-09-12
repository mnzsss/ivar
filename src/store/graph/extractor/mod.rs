//! AST symbol, call, and import extraction engine using Tree-sitter.
//!
//! Implements a 2-pass extraction pipeline:
//! - Pass 1: Extract definitions (symbols) and record their spans, signatures, docstrings, and visibility.
//! - Pass 2: Extract invocations (calls) and imports, attaching 5-tier confidence heuristic and provenance.

mod edges;
mod http;
mod symbols;

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::OnceLock;

use thiserror::Error;
use tree_sitter::Query;

use self::edges::extract_edges;
use self::http::extract_http;
use self::symbols::extract_symbols;
use crate::domain::graph::{Edge, Symbol};
use crate::infra::graph::parser::{
    ParserError, SupportedLanguage, TreeSitterEngine, compile_rust_query, compile_tsx_query,
    compile_typescript_query,
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

thread_local! {
    static PARSER: RefCell<TreeSitterEngine> = RefCell::new(TreeSitterEngine::new());
}

/// Extracts symbols, imports, and calls from a source file using Tree-sitter AST queries.
pub fn extract_file(
    repo: &str,
    _file_path: &str,
    content: &str,
    lang: SupportedLanguage,
) -> Result<ExtractedFile, ExtractorError> {
    let Some(query) = cached_query(lang)? else {
        return Ok(ExtractedFile::default());
    };
    let tree = PARSER.with_borrow_mut(|engine| engine.parse(lang, content))?;
    let root = tree.root_node();

    let source_bytes = content.as_bytes();

    // Pass 1: Extract definitions
    let mut symbols = extract_symbols(repo, root, query, source_bytes, lang);
    let (route_symbols, http_edges) = extract_http(repo, root, source_bytes, lang);
    symbols.extend(route_symbols);
    let local_symbols: HashSet<String> = symbols.iter().map(|s| s.name.clone()).collect();

    // Pass 2: Extract imports and calls
    let mut edges = extract_edges(repo, root, query, source_bytes, lang, &local_symbols);
    edges.extend(http_edges);

    Ok(ExtractedFile { symbols, edges })
}

type QueryCompiler = fn() -> Result<Query, tree_sitter::QueryError>;

/// Compiles each language's query once per process instead of once per file.
fn cached_query(lang: SupportedLanguage) -> Result<Option<&'static Query>, ExtractorError> {
    static RUST: OnceLock<Query> = OnceLock::new();
    static TYPESCRIPT: OnceLock<Query> = OnceLock::new();
    static TSX: OnceLock<Query> = OnceLock::new();

    let (cell, compile): (&'static OnceLock<Query>, QueryCompiler) = match lang {
        SupportedLanguage::Rust => (&RUST, compile_rust_query),
        SupportedLanguage::TypeScript => (&TYPESCRIPT, compile_typescript_query),
        SupportedLanguage::Tsx => (&TSX, compile_tsx_query),
        _ => return Ok(None),
    };
    if let Some(query) = cell.get() {
        return Ok(Some(query));
    }
    let query = compile()?;
    Ok(Some(cell.get_or_init(|| query)))
}

#[cfg(test)]
#[path = "../../../../tests/unit/store/graph/extractor.rs"]
mod tests;
