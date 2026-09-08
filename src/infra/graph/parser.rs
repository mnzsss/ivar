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
mod tests {
    use super::*;
    use streaming_iterator::StreamingIterator;

    #[test]
    fn test_from_extension() {
        assert_eq!(SupportedLanguage::from_extension("rs"), Some(SupportedLanguage::Rust));
        assert_eq!(SupportedLanguage::from_extension("RS"), Some(SupportedLanguage::Rust));
        assert_eq!(SupportedLanguage::from_extension("ts"), Some(SupportedLanguage::TypeScript));
        assert_eq!(SupportedLanguage::from_extension("js"), Some(SupportedLanguage::TypeScript));
        assert_eq!(SupportedLanguage::from_extension("tsx"), Some(SupportedLanguage::Tsx));
        assert_eq!(SupportedLanguage::from_extension("jsx"), Some(SupportedLanguage::Tsx));
        assert_eq!(SupportedLanguage::from_extension("json"), Some(SupportedLanguage::Json));
        assert_eq!(SupportedLanguage::from_extension("md"), Some(SupportedLanguage::Markdown));
        assert_eq!(SupportedLanguage::from_extension("markdown"), Some(SupportedLanguage::Markdown));
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
        let tree = engine.parse(SupportedLanguage::Rust, code).expect("parse rust");
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
        let tree = engine.parse(SupportedLanguage::TypeScript, code).expect("parse ts");
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
        let tree = engine.parse(SupportedLanguage::Tsx, code).expect("parse tsx");
        assert!(!TreeSitterEngine::has_error(&tree));
    }

    #[test]
    fn test_parse_json() {
        let mut engine = TreeSitterEngine::new();
        let code = r#"{"key": "value", "num": 123}"#;
        let tree = engine.parse(SupportedLanguage::Json, code).expect("parse json");
        assert!(!TreeSitterEngine::has_error(&tree));
    }

    #[test]
    fn test_parse_markdown() {
        let mut engine = TreeSitterEngine::new();
        let code = "# Title\n\n```rust\nfn main() {}\n```\n";
        let tree = engine.parse(SupportedLanguage::Markdown, code).expect("parse markdown");
        assert!(!TreeSitterEngine::has_error(&tree));
    }
}
