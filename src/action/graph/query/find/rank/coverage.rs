//! Query-term coverage: how many distinct terms of an intent query a file
//! matches. A file matching one stray term of a long query is a coincidence,
//! not an answer, so coverage decides whether an explore answer is confident.

use std::collections::{BTreeSet, HashMap};

/// `(repo, path)` of an indexed file.
pub(super) type FileKey = (String, String);
/// Lowercased query terms matched by each file.
pub(super) type TermCoverage = HashMap<FileKey, BTreeSet<String>>;

/// Words that carry no evidence about what a file is about.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "did", "do", "does", "for", "from",
    "how", "if", "in", "into", "is", "it", "its", "no", "not", "of", "on", "or", "that", "the",
    "this", "to", "use", "used", "using", "was", "were", "what", "when", "where", "which", "why",
    "with",
];

/// Queries with at least this many distinct terms are held to a coverage floor.
const MIN_TERMS_FOR_FLOOR: usize = 3;

/// The lowercased form a term is deduplicated and matched by.
pub(super) fn term_key(term: &str) -> String {
    term.trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
        .to_lowercase()
}

/// The distinct, meaningful terms of a query, in query order.
#[derive(Debug, Clone, Default)]
pub(super) struct QueryTerms {
    /// `(key, as written in the query)`.
    entries: Vec<(String, String)>,
}

impl QueryTerms {
    pub(super) fn new(terms: &[String]) -> Self {
        let mut entries: Vec<(String, String)> = Vec::new();
        for term in terms {
            let key = term_key(term);
            if key.chars().count() < 2
                || STOPWORDS.contains(&key.as_str())
                || entries.iter().any(|(known, _)| known == &key)
            {
                continue;
            }
            let display = term
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_owned();
            entries.push((key, display));
        }
        Self { entries }
    }

    pub(super) fn total(&self) -> usize {
        self.entries.len()
    }

    /// Terms a file must match to be a confident answer; 0 means no floor.
    ///
    /// Two terms or fewer are plain lookups. From three up, a file needs at
    /// least two terms, and a third of them on longer queries.
    pub(super) fn required(&self) -> usize {
        if self.total() < MIN_TERMS_FOR_FLOOR {
            0
        } else {
            self.total().div_ceil(3).max(2)
        }
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(key, _)| key.as_str())
    }

    pub(super) fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(known, _)| known == key)
    }

    /// The term as the query wrote it.
    pub(super) fn display(&self, key: &str) -> String {
        self.entries
            .iter()
            .find(|(known, _)| known == key)
            .map_or_else(|| key.to_owned(), |(_, display)| display.clone())
    }
}

/// Whether a path names `key` as a whole word, ignoring case and a plural `s`.
pub(super) fn path_names_term(path: &str, key: &str) -> bool {
    path.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == key || word.strip_suffix('s') == Some(key))
}

/// Documentation and configuration files, where intent queries find policy and
/// workflow answers that code symbols only mention.
pub(super) fn is_doc_or_config_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if name.ends_with("lock.json") || name.ends_with(".lock") {
        return false;
    }
    if lower.starts_with(".github/") || lower.contains("/.github/") {
        return true;
    }
    if matches!(name, "dockerfile" | "makefile" | "justfile") {
        return true;
    }
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext,
            "md" | "mdx"
                | "markdown"
                | "rst"
                | "txt"
                | "yml"
                | "yaml"
                | "toml"
                | "json"
                | "ini"
                | "cfg"
        )
    })
}
