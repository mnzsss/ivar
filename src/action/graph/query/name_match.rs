//! Rules for edges that name their target without resolving it to a symbol.
//!
//! An unresolved edge only carries `to_name`, so matching it to a definition is
//! a guess. Left unchecked, the guess links a Rust `mod` declaration to
//! TypeScript hooks, or one `new` to every other `new`. Queries apply two gates
//! to such edges: caller and target must share a language, and generic names
//! never match by name alone. Callers resolved to the symbol always pass.

use crate::infra::graph::SupportedLanguage;

/// Names shared by countless unrelated definitions (constructors, standard trait
/// methods, entry points). A bare-name match on one of these is noise.
pub(super) const GENERIC_NAMES: &[&str] = &[
    "new",
    "mod",
    "default",
    "from",
    "into",
    "try_from",
    "try_into",
    "fmt",
    "clone",
    "drop",
    "eq",
    "cmp",
    "hash",
    "deref",
    "constructor",
    "main",
    "init",
];

/// File extensions the indexer can parse, used to classify a file's language.
const EXTENSIONS: &[&str] = &[
    "rs", "ts", "js", "mjs", "cjs", "tsx", "jsx", "json", "md", "markdown",
];

/// SQL list literal of [`GENERIC_NAMES`], e.g. `'new','mod'`.
pub(super) fn generic_names_sql() -> String {
    GENERIC_NAMES
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// SQL expression that maps the file path in `path_expr` to a language family.
///
/// TypeScript, JavaScript and their JSX flavours form one family, since they
/// call each other freely. Unknown extensions yield `NULL`, which never equals
/// another language, so name-only matches involving them are dropped.
pub(super) fn language_sql(path_expr: &str) -> String {
    let branches = EXTENSIONS
        .iter()
        .filter_map(|ext| {
            let family = match SupportedLanguage::from_extension(ext)? {
                SupportedLanguage::Rust => "rust",
                SupportedLanguage::TypeScript | SupportedLanguage::Tsx => "script",
                SupportedLanguage::Json => "json",
                SupportedLanguage::Markdown => "markdown",
            };
            Some(format!("WHEN {path_expr} LIKE '%.{ext}' THEN '{family}'"))
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("(CASE {branches} END)")
}
