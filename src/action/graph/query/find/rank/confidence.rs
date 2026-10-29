//! Explore relevance floor: how many of the words a query asks about a candidate
//! file covers in its path, its symbol names and its content.
//!
//! A candidate that matches one generic term of a long intent query is a
//! plausible-looking answer to nothing. Coverage of the distinct query words is
//! the signal that tells a file about the topic from a file that shares a word.

use std::collections::HashSet;

use crate::action::graph::query::find::intent::is_identifier_shaped;

/// Lines of content that count as "the same place". A workflow or a doc section
/// that is about the topic holds its words close together; a large file holds
/// the same words by chance, thousands of lines apart.
pub(super) const CONTENT_WINDOW_LINES: usize = 40;

/// A candidate covers the query when it holds at least this share (percent) of
/// the query's distinct words. Two words of three is a match; one of three is
/// not; one of two is, so a short intent still finds the file it names.
pub(super) const MIN_COVERAGE_PERCENT: usize = 50;

/// Words that carry no topic in a query, such as articles, question words and
/// the vocabulary of asking for code.
const GENERIC_WORDS: &[&str] = &[
    "a",
    "all",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "by",
    "can",
    "class",
    "code",
    "do",
    "does",
    "file",
    "find",
    "for",
    "from",
    "function",
    "get",
    "how",
    "if",
    "implementation",
    "implemented",
    "implements",
    "in",
    "is",
    "it",
    "logic",
    "method",
    "of",
    "on",
    "or",
    "show",
    "that",
    "the",
    "their",
    "there",
    "this",
    "to",
    "use",
    "used",
    "uses",
    "using",
    "what",
    "when",
    "where",
    "which",
    "with",
    "work",
    "works",
];

/// File extensions that trail a file name typed in a query (`reply-comment.ts`).
const EXTENSIONS: &[&str] = &[
    "c", "cjs", "cpp", "css", "go", "h", "hpp", "html", "java", "js", "json", "jsx", "kt", "lock",
    "md", "mdx", "mjs", "proto", "py", "rs", "scss", "sh", "sql", "swift", "toml", "ts", "tsx",
    "txt", "xml", "yaml", "yml",
];

/// The distinct, stemmed words of a query that a file can cover.
#[derive(Debug, Clone, Default)]
pub(super) struct QueryWords {
    words: Vec<String>,
}

impl QueryWords {
    /// Words of the query terms, minus generic words and file extensions.
    /// A query made only of generic words keeps them, so it still has a floor.
    pub(super) fn from_terms(terms: &[String]) -> Self {
        let mut all = Vec::new();
        let mut significant = Vec::new();
        for term in terms {
            let words = words_of(term);
            let last = words.len().saturating_sub(1);
            for (index, word) in words.into_iter().enumerate() {
                if word.len() < 2 || all.contains(&word) {
                    continue;
                }
                let is_extension =
                    index == last && index > 0 && EXTENSIONS.contains(&word.as_str());
                if is_extension {
                    continue;
                }
                all.push(word.clone());
                if !is_generic(&word) {
                    significant.push(word);
                }
            }
        }
        Self {
            words: if significant.is_empty() {
                all
            } else {
                significant
            },
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The query words found among the words of `text`.
    pub(super) fn covered_by(&self, text: &str) -> HashSet<String> {
        if self.is_empty() {
            return HashSet::new();
        }
        words_of(text)
            .into_iter()
            .filter(|word| self.words.contains(word))
            .collect()
    }

    /// The query words found together in `text`: the most distinct words that
    /// any [`CONTENT_WINDOW_LINES`] consecutive lines hold.
    pub(super) fn covered_together(&self, text: &str) -> HashSet<String> {
        // One bit per word; a longer query is judged on its first 64 words.
        let words: Vec<&String> = self.words.iter().take(64).collect();
        if words.is_empty() {
            return HashSet::new();
        }
        let line_masks: Vec<u64> = text
            .lines()
            .map(|line| {
                words_of(line)
                    .iter()
                    .filter_map(|word| words.iter().position(|w| *w == word))
                    .fold(0_u64, |mask, bit| mask | (1 << bit))
            })
            .collect();

        // Slide the window, counting how many of its lines hold each word.
        let mut holders = [0_u32; 64];
        let mut window = 0_u64;
        let mut best = 0_u64;
        for (index, &entering) in line_masks.iter().enumerate() {
            for bit in set_bits(entering) {
                if let Some(count) = holders.get_mut(bit) {
                    *count += 1;
                    window |= 1 << bit;
                }
            }
            if let Some(&leaving) = index
                .checked_sub(CONTENT_WINDOW_LINES)
                .and_then(|old| line_masks.get(old))
            {
                for bit in set_bits(leaving) {
                    if let Some(count) = holders.get_mut(bit) {
                        *count -= 1;
                        if *count == 0 {
                            window &= !(1 << bit);
                        }
                    }
                }
            }
            if window.count_ones() > best.count_ones() {
                best = window;
            }
        }
        words
            .iter()
            .enumerate()
            .filter(|(bit, _)| best & (1 << bit) != 0)
            .map(|(_, word)| (*word).clone())
            .collect()
    }

    /// Whether `covered` holds enough of the query's words.
    pub(super) fn is_covered(&self, covered: &HashSet<String>) -> bool {
        self.is_empty() || covered.len() * 100 >= self.words.len() * MIN_COVERAGE_PERCENT
    }

    /// The words of the route intent (`route`, `endpoint`, `api`) of the query.
    pub(super) fn route_words(&self) -> HashSet<String> {
        self.words
            .iter()
            .filter(|word| matches!(word.as_str(), "route" | "endpoint" | "api"))
            .cloned()
            .collect()
    }
}

/// Positions of the set bits of `mask`.
fn set_bits(mask: u64) -> impl Iterator<Item = usize> {
    (0..64).filter(move |bit| mask & (1 << bit) != 0)
}

/// Whether a stemmed word is one of [`GENERIC_WORDS`].
fn is_generic(word: &str) -> bool {
    GENERIC_WORDS.iter().any(|generic| stem(generic) == word)
}

/// Whether the query is plain language: no path, no identifier-shaped term.
pub(super) fn is_natural_language(terms: &[String], has_path_tokens: bool) -> bool {
    !has_path_tokens && !terms.is_empty() && !terms.iter().any(|term| is_identifier_shaped(term))
}

/// Lowercase, stemmed words of `text`, split at non-alphanumerics and at
/// camelCase boundaries.
pub(super) fn words_of(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !c.is_alphanumeric() {
            push_word(&mut words, &mut current);
            previous = None;
            continue;
        }
        // `session0` is the word `session` and a number.
        let digit_edge = previous.is_some_and(|p| p.is_ascii_digit() != c.is_ascii_digit());
        let camel_edge = c.is_uppercase()
            && previous.is_some_and(|p| {
                p.is_lowercase()
                    || (p.is_uppercase() && chars.peek().is_some_and(|n| n.is_lowercase()))
            });
        let starts_word = digit_edge || camel_edge;
        if starts_word {
            push_word(&mut words, &mut current);
        }
        current.extend(c.to_lowercase());
        previous = Some(c);
    }
    push_word(&mut words, &mut current);
    words
}

fn push_word(words: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        words.push(stem(current));
        current.clear();
    }
}

/// Folds plurals together: `changesets` and `changeset`, `entries` and `entry`.
fn stem(word: &str) -> String {
    if word.len() <= 3 || !word.is_ascii() {
        return word.to_owned();
    }
    if let Some(base) = word.strip_suffix("ies") {
        return format!("{base}y");
    }
    for suffix in ["sses", "xes", "ches", "shes"] {
        if word.ends_with(suffix) {
            return word[..word.len() - 2].to_owned();
        }
    }
    if word.ends_with('s') && !word.ends_with("ss") && !word.ends_with("us") {
        return word[..word.len() - 1].to_owned();
    }
    word.to_owned()
}

/// Whether a path is documentation or configuration: the files that answer a
/// question about how a repo works, rather than how its code is written.
pub(super) fn is_doc_or_config_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map_or("", |(_, ext)| ext);
    matches!(
        extension,
        "md" | "mdx" | "rst" | "adoc" | "txt" | "yml" | "yaml" | "toml" | "json" | "ini" | "cfg"
    ) || lower
        .split('/')
        .any(|segment| matches!(segment, ".github" | "workflows" | "docs" | "doc"))
}

#[cfg(test)]
#[path = "../../../../../../tests/unit/action/graph/rank_confidence.rs"]
mod tests;
