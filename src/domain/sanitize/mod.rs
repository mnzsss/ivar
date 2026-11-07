//! Sanitization of captured events before they reach disk.
//!
//! `Sanitized<Observation>` can only be built by [`Sanitized::sanitize`], so
//! every writer that accepts one is guaranteed a redacted, confined, bounded
//! body.

mod patterns;

use crate::domain::observation::Observation;
use camino::Utf8PathBuf;

/// Bounds the regex work on the agent's synchronous path: a body is cut to
/// this many body limits before redaction, so a multi-megabyte tool input
/// costs the same as a small one.
const PRE_REDACTION_FACTOR: usize = 8;

#[derive(Debug)]
pub struct Sanitized<T>(T);

#[derive(Clone)]
pub struct SanitizeContext {
    pub hall_root: Utf8PathBuf,
    pub env_values: Vec<String>,
    pub body_limit: usize,
}

impl SanitizeContext {
    pub const DEFAULT_BODY_LIMIT: usize = 4096;

    #[must_use]
    pub const fn new(hall_root: Utf8PathBuf, env_values: Vec<String>) -> Self {
        Self {
            hall_root,
            env_values,
            body_limit: Self::DEFAULT_BODY_LIMIT,
        }
    }
}

impl std::fmt::Debug for SanitizeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SanitizeContext")
            .field("hall_root", &self.hall_root)
            .field(
                "env_values",
                &format_args!("[{} redacted]", self.env_values.len()),
            )
            .field("body_limit", &self.body_limit)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SanitizeError {
    ResidualSecret,
}

impl Sanitized<Observation> {
    /// # Errors
    ///
    /// [`SanitizeError::ResidualSecret`] when a secret pattern still matches
    /// after redaction; the event is dropped rather than written.
    pub fn sanitize(
        observation: Observation,
        ctx: &SanitizeContext,
    ) -> Result<Self, SanitizeError> {
        sanitize_with(observation, ctx, patterns::redact_secrets)
    }

    #[must_use]
    pub const fn as_observation(&self) -> &Observation {
        &self.0
    }

    #[must_use]
    pub fn into_observation(self) -> Observation {
        self.0
    }
}

fn clean(
    text: &str,
    ctx: &SanitizeContext,
    limit: usize,
    redact: impl Fn(&str) -> String,
) -> Result<String, SanitizeError> {
    let pre_cut = patterns::cut_before_redaction(text, limit.saturating_mul(PRE_REDACTION_FACTOR));
    let input_was_cut = pre_cut.len() < text.len();
    let redacted = redact(pre_cut);
    let confined = patterns::confine_paths(&redacted, &ctx.hall_root);
    let enveloped = patterns::redact_env_values(&confined, &ctx.env_values);
    let bounded = patterns::truncate(&enveloped, limit, input_was_cut);
    if patterns::contains_secret(&bounded) {
        return Err(SanitizeError::ResidualSecret);
    }
    Ok(bounded)
}

fn sanitize_with(
    mut observation: Observation,
    ctx: &SanitizeContext,
    redact: impl Fn(&str) -> String,
) -> Result<Sanitized<Observation>, SanitizeError> {
    observation.body = clean(&observation.body, ctx, ctx.body_limit, redact)?;
    Ok(Sanitized(observation))
}

/// Redacts free text through the same pipeline as an observation body, bounded by `limit` bytes.
///
/// # Errors
///
/// [`SanitizeError::ResidualSecret`] when a secret survives redaction.
pub fn redact_text(
    text: &str,
    ctx: &SanitizeContext,
    limit: usize,
) -> Result<String, SanitizeError> {
    clean(text, ctx, limit, patterns::redact_secrets)
}

/// One place a secret pattern matched in a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretFinding {
    /// The 1-based line where the match starts.
    pub line: usize,
    /// The pattern's name (`api_key`, `private_key`, …). Never the secret.
    pub pattern: &'static str,
}

/// Every place `text` matches one of the secret patterns [`redact_text`]
/// redacts, in line order. The whole text is searched (no bound), and the
/// matched text is never returned: callers refuse the content instead of
/// redacting it.
#[must_use]
pub fn find_secrets(text: &str) -> Vec<SecretFinding> {
    patterns::find_secrets(text)
        .into_iter()
        .map(|(line, pattern)| SecretFinding { line, pattern })
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/sanitize.rs"]
mod tests;
