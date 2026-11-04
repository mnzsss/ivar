//! Secret patterns ported from ai-memory.
//!
//! Upstream: <https://github.com/akitaonrails/ai-memory>, file
//! `crates/ai-memory-core/src/sanitize.rs` (`BUILTIN_PATTERNS`), release
//! v3.3.3, commit 8c39e4e6778b17aa103b02eb0a7ae327d63ba689.
//! Copyright (c) 2026 Fabio Akita. Licensed under the MIT License, whose text
//! is reproduced in `NOTICE`.
//!
//! Only the built-in pattern table is ported; the upstream allowlist, JSON
//! escape view and unicode shadow handling are not.

#![allow(clippy::expect_used)]

use camino::Utf8Path;
use regex::{Captures, Regex, RegexSet};
use std::sync::LazyLock;

const BUILTIN_PATTERNS: &[(&str, &str)] = &[
    (
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----.*?-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----",
        "private_key",
    ),
    (
        r#"(?P<secret>-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----(?:(?:\r?\n|(?:\\r)?\\n)[ \t]*(?:(?:Proc-Type|DEK-Info|Version|Comment|Hash):[^\r\n\\]*)?)*?(?:(?:\r?\n|(?:\\r)?\\n)[ \t]*[A-Za-z0-9+/=]{16,}[ \t]*)+)(?:\r?(?m:$)|["'\\…]|\.\.\.)"#,
        "private_key",
    ),
    (r#"(?i)bearer\s+[A-Za-z0-9._\-+/=]{16,}"#, "bearer_token"),
    (r"sk-[A-Za-z0-9_\-]{16,}", "api_key"),
    (r"(?:sk|rk)_live_[A-Za-z0-9_\-]{16,}", "stripe_key"),
    (r"gh[pousr]_[A-Za-z0-9]{20,}", "github_token"),
    (r"github_pat_[A-Za-z0-9_]{20,}", "github_token"),
    (r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", "aws_key"),
    (r"AIza[A-Za-z0-9_\-]{30,}", "google_api_key"),
    (r"AQ\.Ab[A-Za-z0-9_\-]{30,}", "google_api_key"),
    (r"1//[0-9A-Za-z_\-]{20,}", "google_oauth"),
    (r"EAA[A-Za-z0-9]{20,}", "meta_token"),
    (
        r"\b\d{6,10}:(?:AA[A-Za-z0-9_\-]{30,}|[A-Za-z0-9_\-]{34,35})\b",
        "telegram_token",
    ),
    (
        r"pit-[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}",
        "ghl_token",
    ),
    (r"xox[abprs]-[A-Za-z0-9\-]{10,}", "slack_token"),
    (r"xapp-[A-Za-z0-9\-]{10,}", "slack_token"),
    (
        r"eyJ[A-Za-z0-9_\-]{16,}\.[A-Za-z0-9_\-]{16,}\.[A-Za-z0-9_\-]{16,}",
        "jwt",
    ),
    (r"eyJ[A-Za-z0-9_\-+/]{40,}={0,2}", "base64_json_token"),
    (
        r"[a-zA-Z][a-zA-Z0-9+\-.]*://[^:/\s]+:[^@\s]+@[^\s]+",
        "url_credentials",
    ),
    (
        r#"(?i)\b[A-Za-z0-9_-]*(?:authentication|authorization|credentials?|password|passwd|apikey|accountkey|authtoken|[a-z0-9]*(?:api|auth|access|secret|security|private|session|refresh|client|consumer|subscription|app|bearer)-(?:key|token))(?:\\*")?\s*[=:]\s*(?:\\*")?(?:(?:basic|bearer|digest|token|apikey)\s+)?"?[A-Za-z0-9._~+/=-]{8,}"#,
        "auth_header",
    ),
    (
        r#"(?i)(ANTHROPIC_API_KEY|OPENAI_API_KEY|OPENROUTER_API_KEY|VOYAGE_API_KEY|MISTRAL_API_KEY|GROQ_API_KEY|HF_TOKEN|HUGGINGFACE_TOKEN|AWS_(SECRET_)?ACCESS_KEY[A-Z_]*|GITHUB_TOKEN|GH_TOKEN|GITLAB_TOKEN|GOOGLE_API_KEY|GEMINI_API_KEY|OLLAMA_API_KEY)(?:\\*")?\s*[=:]\s*(?:\\*"(?:[^"\\\r\n]|\\.)+(?:"[^\s"\\,;}\]]+)*|\S+)"#,
        "env_secret",
    ),
    (
        r#"(?i)\b[A-Z][A-Z0-9_]*_(KEY|TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|CREDENTIALS|PRIVATE_KEY)(?:\\*")?\s*[=:]\s*(?:\\*"(?:[^"\\\r\n]|\\.)+(?:"[^\s"\\,;}\]]+)*|\S+)"#,
        "env_secret",
    ),
    (
        r#"\b[A-Z][A-Z0-9_]*_(KEY_ID|PASSPHRASE|SIGNING_KEY|PEPPER|SALT)(?:\\*")?\s*[=:]\s*(?:\\*"(?:[^"\\\r\n]|\\.)+(?:"[^\s"\\,;}\]]+)*|\S+)"#,
        "env_secret",
    ),
    (
        r"(?i)(?:[A-Za-z]:)?(?:[\\/][^\\/\s]+)*[\\/]\.ssh(?:[\\/][^\s]+)?",
        "credential_path",
    ),
    (
        r"(?i)(?:[A-Za-z]:)?(?:[\\/][^\\/\s]+)*[\\/]\.aws(?:[\\/][^\s]+)?",
        "credential_path",
    ),
    (
        r"(?i)(?:[A-Za-z]:)?(?:[\\/][^\\/\s]+)*[\\/]\.kube(?:[\\/][^\s]+)?",
        "credential_path",
    ),
    (
        r"(?i)(?:[A-Za-z]:)?(?:[\\/][^\\/\s]+)*[\\/]\.config[\\/]gcloud(?:[\\/][^\s]+)?",
        "credential_path",
    ),
    (
        r"(?i)(?:[A-Za-z]:)?(?:[\\/][^\\/\s]+)*[\\/]\.gnupg(?:[\\/][^\s]+)?",
        "credential_path",
    ),
];

const SECRET_GROUP: &str = "secret";
const MIN_ENV_VALUE_LEN: usize = 6;
const OUTSIDE_HALL: &str = "<outside-hall>";

struct Compiled {
    set: RegexSet,
    each: Vec<(Regex, &'static str)>,
}

static COMPILED: LazyLock<Compiled> = LazyLock::new(|| Compiled {
    set: RegexSet::new(BUILTIN_PATTERNS.iter().map(|(pattern, _)| *pattern))
        .expect("built-in secret patterns are valid"),
    each: BUILTIN_PATTERNS
        .iter()
        .map(|(pattern, label)| {
            (
                Regex::new(pattern).expect("built-in secret patterns are valid"),
                *label,
            )
        })
        .collect(),
});

static REDACTION_MARK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[REDACTED:[a-z_]+\]").expect("valid regex"));

static ABSOLUTE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)(^|[\s"'=(\[<])(/[^\s"'<>)\]]+)"#).expect("valid regex"));

fn redaction(label: &str) -> String {
    format!("[REDACTED:{label}]")
}

pub(super) fn redact_secrets(text: &str) -> String {
    COMPILED
        .each
        .iter()
        .fold(text.to_owned(), |text, (re, label)| {
            re.replace_all(&text, |caps: &Captures<'_>| {
                let whole = caps.get(0).expect("group 0 always matches");
                match caps.name(SECRET_GROUP) {
                    Some(secret) => format!(
                        "{}{}{}",
                        &text[whole.start()..secret.start()],
                        redaction(label),
                        &text[secret.end()..whole.end()],
                    ),
                    None => redaction(label),
                }
            })
            .into_owned()
        })
}

pub(super) fn contains_secret(text: &str) -> bool {
    COMPILED
        .set
        .is_match(&REDACTION_MARK.replace_all(text, " "))
}

pub(super) fn redact_env_values(text: &str, values: &[String]) -> String {
    values
        .iter()
        .filter(|value| value.chars().count() >= MIN_ENV_VALUE_LEN)
        .fold(text.to_owned(), |text, value| {
            text.replace(value.as_str(), &redaction("env_value"))
        })
}

pub(super) fn confine_paths(text: &str, hall_root: &Utf8Path) -> String {
    ABSOLUTE_PATH
        .replace_all(text, |caps: &Captures<'_>| {
            let prefix = &caps[1];
            let path = &caps[2];
            if Utf8Path::new(path).starts_with(hall_root) {
                format!("{prefix}{path}")
            } else {
                format!("{prefix}{OUTSIDE_HALL}")
            }
        })
        .into_owned()
}

pub(super) fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let cut = text.floor_char_boundary(limit);
    format!("{}…", &text[..cut])
}

/// Cuts at a char boundary, then back to the last delimiter so a secret split
/// by the cut leaves no usable prefix behind once redaction shrinks the text.
pub(super) fn cut_before_redaction(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let head = &text[..text.floor_char_boundary(limit)];
    head.rfind(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | ':' | '='))
        .map_or(head, |end| &head[..end])
}
