//! Everything that can go wrong reading or writing `ivar.json`, and how each
//! becomes a user-facing [`Failure`].
//!
//! [`Error`] is the manifest module's own error type; its `Failure`
//! conversion is the single place a manifest problem acquires a code, an
//! expected/actual pair, and a fix action.

use crate::domain::name::RepoName;
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::store::versioned;

use super::model::provider_ids;

/// Everything that can go wrong reading or writing `ivar.json`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Something failed at the [`Store`] layer: I/O, invalid JSON, an unknown
    /// key, a schema version newer than this binary understands, or the
    /// committed-file write policy. Delegates its `Failure` conversion
    /// entirely to the wrapped error, which already has its own code and fix
    /// action.
    #[error(transparent)]
    Store(#[from] versioned::Error),

    /// The file has no `version` field, or one that is not a JSON number. See
    /// the "Why v1 and not v2" section of the module doc comment: `ivar.json`
    /// has no v0, so unversioned data is not an `ivar.json` at all, not a file
    /// silently adopted as one.
    #[error("{path}: has no `version` field (or a non-numeric one); this is not an ivar.json")]
    MissingVersion { path: camino::Utf8PathBuf },

    /// `providers.default` does not appear in `providers.available`.
    #[error("default provider `{default}` is not in `providers.available`")]
    DefaultProviderNotAvailable {
        default: Provider,
        available: Vec<Provider>,
    },

    /// `providers.available` is empty.
    #[error("`providers.available` must not be empty")]
    NoAvailableProviders,

    /// Two repos share the same `name`.
    #[error("repo name `{name}` is used by more than one repo")]
    DuplicateRepoName { name: RepoName },

    /// A repo's `url` is empty or blank.
    ///
    /// Not a path-safety concern — a remote URL is never joined onto disk — but
    /// it is the difference between a `Failure` that names the offending repo
    /// and a raw `git clone` error the first time someone runs `ivar sync`.
    #[error("repo `{name}` has an empty `url`")]
    EmptyRepoUrl { name: RepoName },

    /// A repo's `checks` list contains a blank command. A blank entry would
    /// run nothing and be recorded as a silent pass — the difference between
    /// naming the offending entry and believing an integration was verified
    /// when no command actually ran.
    #[error("repo `{name}` has an empty check at index {index}")]
    EmptyRepoCheck { name: RepoName, index: usize },

    /// A repo named `name` is not in `self.repos` — `ivar repo remove` was
    /// asked to remove something the hall does not know about.
    #[error("repo `{name}` is not in ivar.json")]
    RepoNotFound { name: RepoName },

    /// Two MCP server definitions in `mcp` share the same `name`.
    #[error("MCP server name `{name}` is used by more than one definition")]
    DuplicateMcpServerName { name: String },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        // The `#[error(...)]` attribute is the single source of the sentence.
        // Re-typing it per arm is how the two drift — they already had.
        let what = error.to_string();

        match error {
            Error::Store(source) => source.into(),
            Error::MissingVersion { .. } => Failure::blocked("manifest.missing_version", what)
            .expected("a `version` field naming the schema version")
            .actual("no `version` field, or a non-numeric one")
            .fix(FixAction::safe(
                "manifest.add_version_field",
                "Add a `\"version\": 1` field to the manifest — every ivar.json has one.",
            )),
            Error::DefaultProviderNotAvailable { default, available } => {
                let ids = provider_ids(&available);
                Failure::blocked("manifest.default_provider_not_available", what)
                .expected(format!("`providers.default` to be one of: {ids}"))
                .actual(format!("`providers.default` is `{default}`, not in [{ids}]"))
                .fix(FixAction::safe(
                    "manifest.fix_default_provider",
                    format!(
                        "Add `{default}` to `providers.available`, or change `providers.default` to one of: {ids}."
                    ),
                ))
            }
            Error::NoAvailableProviders => {
                Failure::blocked("manifest.no_available_providers", what)
            }
            .expected("at least one provider id in `providers.available`")
            .actual("an empty `providers.available` list")
            .fix(FixAction::safe(
                "manifest.add_available_provider",
                "Add at least one provider id (`claude-code` or `opencode`) to `providers.available`.",
            )),
            Error::DuplicateRepoName { name } => Failure::blocked("manifest.duplicate_repo_name", what)
            .expected("every entry in `repos` to have a unique `name`")
            .actual(format!("`{name}` appears more than once in `repos`"))
            .fix(FixAction::safe(
                "manifest.rename_duplicate_repo",
                format!("Rename or remove the duplicate `{name}` entry in `repos` so the name appears once."),
            )),
            Error::EmptyRepoUrl { name } => Failure::blocked("manifest.empty_repo_url", what)
            .expected("a git remote URL")
            .actual("an empty string")
            .fix(FixAction::safe(
                "manifest.set_repo_url",
                format!("Set `url` on the `{name}` entry in `repos` to its git remote, or remove the entry."),
            )),
            Error::EmptyRepoCheck { name, index } => {
                Failure::blocked("manifest.empty_repo_check", what)
                .expected("an executable command in every `checks` entry")
                .actual(format!("`{name}`'s `checks[{index}]` is blank"))
                .fix(FixAction::safe(
                    "manifest.fix_repo_check",
                    format!(
                        "Remove the blank entry from `{name}`'s `checks` list, or give it an executable command."
                    ),
                ))
            }
            Error::RepoNotFound { name } => Failure::blocked("manifest.repo_not_found", what)
            .expected(format!("`{name}` to be listed in `repos`"))
            .actual(format!("`{name}` does not appear in ivar.json"))
            .fix(FixAction::safe(
                "manifest.add_repo_first",
                format!("Add `{name}` with `ivar repo add`, or check the spelling."),
            )),
            Error::DuplicateMcpServerName { name } => {
                Failure::blocked("manifest.duplicate_mcp_server_name", what)
                .expected("every definition in `mcp` to have a unique `name`")
                .actual(format!("`{name}` appears more than once in `mcp`"))
                .fix(FixAction::safe(
                    "manifest.rename_duplicate_mcp_server",
                    format!("Give one of the duplicate `{name}` definitions a different name."),
                ))
            }
        }
    }
}
