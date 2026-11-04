//! Sanitization of captured events before they reach disk.
//!
//! `Sanitized<Observation>` can only be built by [`Sanitized::sanitize`], so
//! every writer that accepts one is guaranteed a redacted, confined, bounded
//! body.

mod patterns;

use crate::domain::observation::Observation;
use camino::Utf8PathBuf;

#[derive(Debug)]
pub struct Sanitized<T>(T);

#[derive(Debug, Clone)]
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

fn sanitize_with(
    mut observation: Observation,
    ctx: &SanitizeContext,
    redact: impl Fn(&str) -> String,
) -> Result<Sanitized<Observation>, SanitizeError> {
    let redacted = redact(&observation.body);
    let confined = patterns::confine_paths(&redacted, &ctx.hall_root);
    let enveloped = patterns::redact_env_values(&confined, &ctx.env_values);
    let bounded = patterns::truncate(&enveloped, ctx.body_limit);
    if patterns::contains_secret(&bounded) {
        return Err(SanitizeError::ResidualSecret);
    }
    observation.body = bounded;
    Ok(Sanitized(observation))
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/sanitize.rs"]
mod tests;
