//! Secret sanitization for shared memory documents and episodes.

use std::sync::LazyLock;

use regex::Regex;

/// Wrapper ensuring a string or serializable text has undergone secret scrubbing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sanitized<T>(T);

impl<T> Sanitized<T> {
    /// Wrap pre-sanitized value.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Unwrap the inner value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: AsRef<str>> Sanitized<T> {
    /// Reference to sanitized string.
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl<T: AsRef<str>> AsRef<str> for Sanitized<T> {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // GitHub Personal Access Tokens (classic and fine-grained)
        Regex::new(r"ghp_[A-Za-z0-9_]{36,255}").expect("valid regex"),
        Regex::new(r"gho_[A-Za-z0-9_]{36,255}").expect("valid regex"),
        Regex::new(r"ghu_[A-Za-z0-9_]{36,255}").expect("valid regex"),
        Regex::new(r"ghs_[A-Za-z0-9_]{36,255}").expect("valid regex"),
        Regex::new(r"ghr_[A-Za-z0-9_]{36,255}").expect("valid regex"),
        Regex::new(r"github_pat_[A-Za-z0-9_]{22,255}").expect("valid regex"),
        // Slack tokens (xoxb-, xoxp-, xoxa-, xoxr-)
        Regex::new(r"xox[baprs]-[A-Za-z0-9-]{10,255}").expect("valid regex"),
        // AWS Access Key ID
        Regex::new(r"(?-u:\b)(?:AKIA|ABIA|ACCA|ASIA)[0-9A-Z]{16}(?-u:\b)").expect("valid regex"),
        // Private Keys (PEM blocks)
        Regex::new(r"(?s)-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY(?: BLOCK)?-----.*?-----END (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY(?: BLOCK)?-----").expect("valid regex"),
        // Generic private key header lines if broken/single-line
        Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY[A-Z ]*-----").expect("valid regex"),
        Regex::new(r"-----END [A-Z ]*PRIVATE KEY[A-Z ]*-----").expect("valid regex"),
    ]
});

/// Redacts secrets such as API tokens, private keys, AWS/GCP keys, and credentials.
#[must_use]
pub fn sanitize_text(input: &str) -> Sanitized<String> {
    let mut result = input.to_string();
    for pattern in SECRET_PATTERNS.iter() {
        result = pattern.replace_all(&result, "[REDACTED]").into_owned();
    }
    Sanitized(result)
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/memory/sanitizer.rs"]
mod tests;
