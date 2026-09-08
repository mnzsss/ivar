//! Tree-sitter parser wrapper and query execution engine for multi-language AST extraction.

use thiserror::Error;
use tree_sitter::{Language, Parser, Query, QueryCursor, QueryError, Tree};

/// Supported languages for AST extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupportedLanguage {
    Rust,
    TypeScript,
    Tsx,
    Json,
    Markdown,
}

impl SupportedLanguage {
    /// Detects supported language from a file extension.
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "ts" | "js" | "mjs" | "cjs" => Some(Self::TypeScript),
            "tsx" | "jsx" => Some(Self::Tsx),
            "json" => Some(Self::Json),
            "md" | "markdown" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// Returns the corresponding `tree_sitter::Language`.
    #[must_use]
    pub fn tree_sitter_language(&self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Markdown => tree_sitter_md::LANGUAGE.into(),
        }
    }
}

/// Errors produced during parsing or tree-sitter operations.
#[derive(Debug, Error)]
pub enum ParserError {
    #[error("Failed to set tree-sitter language for {0:?}")]
    LanguageSetError(SupportedLanguage),
    #[error("Failed to parse source text")]
    ParseFailure,
    #[error("Query compilation failed: {0}")]
    QueryCompilation(#[from] QueryError),
}

/// Synchronous Tree-sitter parsing engine maintaining a reusable parser instance.
pub struct TreeSitterEngine {
    parser: Parser,
    current_lang: Option<SupportedLanguage>,
}

impl std::fmt::Debug for TreeSitterEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeSitterEngine")
            .field("current_lang", &self.current_lang)
            .finish_non_exhaustive()
    }
}

impl Default for TreeSitterEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeSitterEngine {
    /// Creates a new `TreeSitterEngine`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            current_lang: None,
        }
    }

    /// Parses the given source code for the specified language.
    pub fn parse(&mut self, lang: SupportedLanguage, source: &str) -> Result<Tree, ParserError> {
        if self.current_lang != Some(lang) {
            self.parser
                .set_language(&lang.tree_sitter_language())
                .map_err(|_| ParserError::LanguageSetError(lang))?;
            self.current_lang = Some(lang);
        }

        self.parser
            .parse(source, None)
            .ok_or(ParserError::ParseFailure)
    }

    /// Checks if a syntax tree contains any parse errors.
    #[must_use]
    pub fn has_error(tree: &Tree) -> bool {
        tree.root_node().has_error()
    }
}

/// Compiles a Tree-sitter query for a given language.
pub fn compile_query(lang: SupportedLanguage, query_str: &str) -> Result<Query, QueryError> {
    Query::new(&lang.tree_sitter_language(), query_str)
}

/// Returns the vendored Rust query string.
#[must_use]
pub fn rust_query_str() -> &'static str {
    include_str!("queries/rust.scm")
}

/// Returns the vendored TypeScript query string.
#[must_use]
pub fn typescript_query_str() -> &'static str {
    include_str!("queries/typescript.scm")
}

/// Compiles the vendored Rust query.
pub fn compile_rust_query() -> Result<Query, QueryError> {
    compile_query(SupportedLanguage::Rust, rust_query_str())
}

/// Compiles the vendored TypeScript query.
pub fn compile_typescript_query() -> Result<Query, QueryError> {
    compile_query(SupportedLanguage::TypeScript, typescript_query_str())
}

#[cfg(test)]
#[path = "../../../tests/unit/infra/graph/parser.rs"]
mod tests;
