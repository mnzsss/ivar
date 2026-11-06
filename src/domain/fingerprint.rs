//! The normalized request fingerprint of a user prompt.
//!
//! The input is an observation body that already went through the sanitizer, so
//! a known secret is a `[REDACTED:...]` mark by now. Normalization only ever
//! shortens or replaces text; it never adds anything the body did not hold.
//! There is no version on a stored fingerprint: a change to these rules applies
//! to new prompts and to the retained observations a migration backfills, and
//! leaves older fingerprints as they were written.

use std::sync::LazyLock;

use regex::{Captures, Regex};

const PATH: &str = "<path>";
const OUTSIDE_HALL: &str = "<outside-hall>";
const ID: &str = "<id>";
const NUMBER: &str = "<n>";
const TOKEN: &str = "<token>";

/// A token this long is judged for entropy.
const TOKEN_MIN_LEN: usize = 20;
const SHA_MIN_LEN: usize = 7;
const SHA_MAX_LEN: usize = 40;

/// Portuguese and English, accent-free, lowercase. Deliberately small: a longer
/// list starts to merge prompts that ask different things.
const STOPWORDS: &[&str] = &[
    // English
    "a", "an", "the", "and", "or", "of", "to", "in", "on", "for", "with", "is", "are", "be", "it",
    "this", "that", "please", "can", "could", "you", "i", "me", "my", "we",
    "just", // Portuguese
    "o", "os", "as", "um", "uma", "uns", "umas", "de", "do", "da", "dos", "das", "em", "no", "na",
    "nos", "nas", "para", "por", "com", "e", "ou", "que", "se", "eu", "voce", "pode", "favor",
    "isso", "isto", "esse", "essa", "este", "esta",
];

#[allow(clippy::expect_used)]
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    // The leading group keeps the character before the path, so a URL such as
    // `https://host/a` (preceded by `:`) is not taken for one.
    Regex::new(concat!(
        r#"(?m)(^|[\s"'`=(\[<])"#,
        r#"(?:(?:~|\.{1,2})?(?:/[^\s"'`<>)\]]+)+"#,
        r"|[\w.@-]+(?:/[\w.@-]+){2,}",
        r"|[\w.@-]+/[\w@-]*\.[\w.@-]+)"
    ))
    .expect("valid regex")
});

#[allow(clippy::expect_used)]
static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
        .expect("valid regex")
});

#[allow(clippy::expect_used)]
static ISSUE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[a-z]{2,10}-\d+\b").expect("valid regex"));

#[allow(clippy::expect_used)]
static SHA_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"\b[0-9a-f]{{{SHA_MIN_LEN},{SHA_MAX_LEN}}}\b")).expect("valid regex")
});

// A number is a whole token: `v2` and `3d` keep their digits, so an identifier
// that mixes letters and digits is still there for the entropy step.
#[allow(clippy::expect_used)]
static NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|\W)\d+(?:[.,]\d+)*\b").expect("valid regex"));

#[allow(clippy::expect_used)]
static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"[a-z0-9+/=_-]{{{TOKEN_MIN_LEN},}}")).expect("valid regex")
});

/// The normalized form of a sanitized prompt body.
///
/// Runs, in order: lowercase and strip accents; `<path>`; `<id>` for UUIDs,
/// issue keys and 7-40 character hex SHAs; `<n>`; `<token>` for high-entropy
/// tokens; stopword removal; whitespace collapse.
#[must_use]
pub fn normalize(body: &str) -> String {
    let text = fold(body);
    let text = replace_paths(&text);
    let text = replace_ids(&text);
    let text = replace_numbers(&text);
    let text = replace_tokens(&text);
    remove_stopwords(&text)
}

/// Lowercases and drops accents. No normalization crate is available, so the
/// Latin letters with diacritics that Portuguese and its neighbours use are
/// mapped by hand and combining marks are dropped.
fn fold(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for c in body.chars().flat_map(char::to_lowercase) {
        match c {
            '\u{0300}'..='\u{036f}' => {}
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'ç' | 'ć' | 'č' => out.push('c'),
            'ď' | 'đ' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => out.push('i'),
            'ł' => out.push('l'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => out.push('o'),
            'ř' => out.push('r'),
            'ś' | 'š' | 'ş' => out.push('s'),
            'ť' => out.push('t'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            other => out.push(other),
        }
    }
    out
}

fn replace_paths(text: &str) -> String {
    // The sanitizer already replaced a path outside the hall with this mark.
    let text = text.replace(OUTSIDE_HALL, PATH);
    PATH_RE
        .replace_all(&text, |caps: &Captures<'_>| {
            // Punctuation that ends the sentence is not part of the path.
            let whole = &caps[0];
            let kept = whole.trim_end_matches(['.', ',', ';', ':', '!', '?']);
            let ending = whole.strip_prefix(kept).unwrap_or_default();
            format!("{}{PATH}{ending}", &caps[1])
        })
        .into_owned()
}

fn replace_ids(text: &str) -> String {
    let text = UUID_RE.replace_all(text, ID);
    let text = ISSUE_KEY_RE.replace_all(&text, ID);
    // A hex run of only digits is a number and only letters is a word such as
    // `decade`; a SHA has both.
    SHA_RE
        .replace_all(&text, |caps: &Captures<'_>| {
            let run = &caps[0];
            let digit = run.chars().any(|c| c.is_ascii_digit());
            let letter = run.chars().any(|c| c.is_ascii_lowercase());
            if digit && letter { ID } else { run }.to_owned()
        })
        .into_owned()
}

fn replace_numbers(text: &str) -> String {
    NUMBER_RE
        .replace_all(text, |caps: &Captures<'_>| format!("{}{NUMBER}", &caps[1]))
        .into_owned()
}

fn replace_tokens(text: &str) -> String {
    TOKEN_RE
        .replace_all(text, |caps: &Captures<'_>| {
            if is_high_entropy(&caps[0]) {
                TOKEN.to_owned()
            } else {
                caps[0].to_owned()
            }
        })
        .into_owned()
}

/// A run of 20 or more token characters that is hex, base64, or an
/// alphanumeric blob mixing letters and digits.
fn is_high_entropy(run: &str) -> bool {
    let hex = run.chars().all(|c| c.is_ascii_hexdigit());
    let base64 = run.contains(['+', '/']) || run.ends_with('=');
    let separated = run.contains(['-', '_']);
    let letter = run.chars().any(|c| c.is_ascii_lowercase());
    let digit = run.chars().any(|c| c.is_ascii_digit());
    hex || base64 || (!separated && letter && digit)
}

fn remove_stopwords(text: &str) -> String {
    text.split_whitespace()
        .filter(|word| {
            let core = word.trim_matches(|c: char| !c.is_alphanumeric());
            !STOPWORDS.contains(&core)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "../../tests/unit/domain/fingerprint.rs"]
mod tests;
