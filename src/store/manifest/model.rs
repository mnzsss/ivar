//! The manifest model: the pure data types behind `ivar.json` and the value
//! invariants that make a hand-edited file refuse loudly instead of silently
//! doing nothing.
//!
//! Reading and writing the file is [`persistence`](super::persistence)'s job;
//! turning a violation into a user-facing `Failure` is
//! [`error`](super::error)'s. This module owns what the other two build on:
//! the `Manifest`, `Providers`, `Repo`, and `Graph` values, their accessors
//! and builders, and `Manifest::validate` — the one place the value invariants live.
//!
//! See `mod.rs` for the full schema, contract, and the "why explicit
//! validate" decision that keeps invariant checks out of `Deserialize`.

use std::borrow::Cow;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::domain::feature::IntegrationPolicy;
use crate::domain::mcp::McpServerDef;
#[cfg(test)]
use crate::domain::name::BranchName;
use crate::domain::name::{HallName, RepoName};
use crate::domain::provider::Provider;

use super::error::Error;

mod repo;
mod settings;

pub use repo::{Repo, local_ref_prefix};
pub use settings::Graph;
pub(crate) use settings::normalise_graph_path;

/// `ivar.json`'s schema version. v1 was the first public version; v2 adds the
/// hall integration defaults and each repo's ordered verification checks; v3
/// adds `McpServerDef.oauth`, an optional pre-provisioned OAuth client
/// registration; v4 adds `McpOauth.token_url` and `McpOauth.resource` so
/// refresh metadata survives across sessions; v5 drops `skills`, whose targets
/// are derived from `providers.available`.
pub(crate) const CURRENT_VERSION: u32 = 5;

/// The hall's identity, committed and team-shared. See the module doc comment
/// for the full JSON shape, the contract, and how the invariants are
/// enforced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    version: u32,
    /// The hall's name: a single, non-empty, non-whitespace, non-traversal
    /// path segment.
    #[schemars(example = &"valhalla")]
    name: HallName,
    providers: Providers,
    /// The repos this hall knows about.
    repos: Vec<Repo>,
    /// The hall's integration defaults: the via/strategy a feature inherits
    /// when neither the CLI nor the feature itself overrides a field.
    integration: IntegrationPolicy,
    #[serde(rename = "$schema", default)]
    schema: SchemaRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp: Option<Vec<McpServerDef>>,
    /// Hall-root directories the graph indexes on request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph: Option<Graph>,
}

/// The canonical URL of the document describing manifest version `version`.
///
/// One document per version, because the schema pins `version` with `const`:
/// a single URL could only ever be right for one version, and an editor that
/// fetched it for any other would report an error on a correct file.
#[must_use]
pub(super) fn manifest_schema_url(version: u32) -> String {
    format!("https://ivar.run/schema/{version}.json")
}

/// `ivar.json`'s `$schema` reference: a function of the version being
/// written, never state carried from the file.
///
/// Deserialising accepts the string on disk and drops it — it may be a legacy
/// unversioned URL, or something hand-typed — and serialising always emits
/// [`manifest_schema_url`] for [`CURRENT_VERSION`]. Correcting the value on
/// write would do the same thing later; holding no value at all is what makes
/// a stale one unrepresentable, and it keeps `Manifest::write`'s `&Self`
/// signature clone-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct SchemaRef;

impl Serialize for SchemaRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&manifest_schema_url(CURRENT_VERSION))
    }
}

impl<'de> Deserialize<'de> for SchemaRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Read so the key is consumed rather than rejected by
        // `deny_unknown_fields`, then dropped: see the type's doc comment.
        String::deserialize(deserializer)?;
        Ok(Self)
    }
}

impl schemars::JsonSchema for SchemaRef {
    fn schema_name() -> Cow<'static, str> {
        "SchemaRef".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::SchemaRef").into()
    }

    // Inlined rather than a `$defs` entry: `generate()` replaces the
    // `$schema` property wholesale, so a definition would be dead weight in
    // the published document — the same reason `McpServerDef` is removed
    // there.
    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string"
        })
    }
}

impl Manifest {
    /// Build a validated `Manifest`. Refuses exactly what [`Self::validate`]
    /// refuses — see the module doc comment for why this is the one type-level
    /// guarantee this module makes: any `Manifest` built through this
    /// constructor already satisfies every invariant.
    ///
    /// The constructor carries the embedded integration defaults
    /// ([`IntegrationPolicy::default`], `local`/`squash`) and the canonical
    /// manifest schema reference.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the built manifest fails validation.
    pub fn new(name: HallName, providers: Providers, repos: Vec<Repo>) -> Result<Self, Error> {
        let manifest = Self {
            version: CURRENT_VERSION,
            name,
            providers,
            repos,
            integration: IntegrationPolicy::default(),
            schema: SchemaRef,
            mcp: None,
            graph: None,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// The schema version. Always [`CURRENT_VERSION`] for a value obtained
    /// through [`Self::new`] or [`Self::read`].
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The hall's integration defaults — the via/strategy a feature inherits
    /// when neither the CLI nor the feature's own override sets a field.
    #[must_use]
    pub fn integration(&self) -> IntegrationPolicy {
        self.integration
    }

    /// The hall's name.
    #[must_use]
    pub fn name(&self) -> &HallName {
        &self.name
    }

    /// The hall's provider configuration.
    #[must_use]
    pub fn providers(&self) -> &Providers {
        &self.providers
    }

    /// The repos this hall knows about.
    #[must_use]
    pub fn repos(&self) -> &[Repo] {
        &self.repos
    }

    /// The hall-scoped MCP server definitions `ivar sync` materialises into
    /// each provider's config file at the hall root.
    ///
    /// Empty when the manifest carries none — the v1 common case. The materialiser
    /// still writes a valid (empty) config, so the file exists and the walk-up
    /// discovery contract holds.
    #[must_use]
    pub fn mcp_servers(&self) -> &[McpServerDef] {
        self.mcp.as_deref().unwrap_or_default()
    }

    /// Return a new `Manifest` carrying `servers` as its hall-scoped MCP
    /// definitions.
    ///
    /// Refuses (with [`Error::DuplicateMcpServerName`]) when two definitions
    /// share a `name` — duplicate names would silently collapse into one key
    /// in the generated config. An empty list is stored as *absent*, so a hall
    /// with no MCP servers round-trips byte-identical to one that never had
    /// the key.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if two `servers` share a `name`, or the
    /// rebuilt manifest fails validation.
    pub fn with_mcp_servers(&self, servers: Vec<McpServerDef>) -> Result<Self, Error> {
        let mut manifest = self.rebuild(self.providers.clone(), self.repos.clone())?;
        manifest.mcp = if servers.is_empty() {
            None
        } else {
            Some(servers)
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Hall-root directories the graph indexes on request; empty when none.
    #[must_use]
    pub fn graph_paths(&self) -> &[String] {
        self.graph.as_ref().map_or(&[], |g| g.paths.as_slice())
    }

    /// Return a new `Manifest` with `dir` in `graph.paths`, normalised and sorted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidGraphPath`] when `dir` is empty, the hall root,
    /// absolute, leaves the hall, or enters `.ivar` outside `.ivar/skills`; or any
    /// rebuild error.
    pub fn with_graph_path_added(&self, dir: &str) -> Result<Self, Error> {
        let dir = normalise_graph_path(dir)?;
        let mut paths: Vec<String> = self.graph_paths().to_vec();
        if !paths.contains(&dir) {
            paths.push(dir);
        }
        paths.sort();
        self.with_graph_paths(paths)
    }

    /// Return a new `Manifest` without `dir` in `graph.paths`; the key goes when the list empties.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidGraphPath`] for a malformed `dir`,
    /// [`Error::GraphPathNotFound`] when it is not configured, or any rebuild error.
    pub fn with_graph_path_removed(&self, dir: &str) -> Result<Self, Error> {
        let dir = normalise_graph_path(dir)?;
        let mut paths: Vec<String> = self.graph_paths().to_vec();
        let before = paths.len();
        paths.retain(|p| *p != dir);
        if paths.len() == before {
            return Err(Error::GraphPathNotFound { path: dir });
        }
        self.with_graph_paths(paths)
    }

    fn with_graph_paths(&self, paths: Vec<String>) -> Result<Self, Error> {
        let mut manifest = self.rebuild(self.providers.clone(), self.repos.clone())?;
        manifest.graph = if paths.is_empty() {
            None
        } else {
            Some(Graph { paths })
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Return a new `Manifest` with `repo` appended to `repos`.
    ///
    /// Returns [`Error::DuplicateRepoName`] if a repo with `repo.name()`
    /// already appears. The original is untouched — `ivar.json` is rewritten
    /// from the returned value, never mutated in place.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DuplicateRepoName`] if a repo named
    /// `repo.name()` already exists.
    pub fn with_repo_added(&self, repo: Repo) -> Result<Self, Error> {
        if self.repos.iter().any(|existing| existing.name == repo.name) {
            return Err(Error::DuplicateRepoName {
                name: repo.name().clone(),
            });
        }
        let mut repos = self.repos.clone();
        repos.push(repo);
        self.rebuild(self.providers.clone(), repos)
    }

    /// Return a new `Manifest` without the repo named `name`.
    ///
    /// Returns [`Error::RepoNotFound`] if no repo in `self.repos` carries
    /// that name. Removing never touches the filesystem — the repo's bare
    /// clone and worktrees stay until `ivar cleanup` (slice 8) is told to
    /// remove them.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RepoNotFound`] if no repo named `name` exists.
    pub fn with_repo_removed(&self, name: &RepoName) -> Result<Self, Error> {
        let repos: Vec<Repo> = self
            .repos
            .iter()
            .filter(|repo| repo.name != *name)
            .cloned()
            .collect();
        if repos.len() == self.repos.len() {
            return Err(Error::RepoNotFound { name: name.clone() });
        }
        self.rebuild(self.providers.clone(), repos)
    }

    /// Return a new `Manifest` carrying `providers` in place of the current
    /// provider configuration. Infallible beyond the usual manifest
    /// invariants — see [`Self::rebuild`].
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the rebuilt manifest fails validation.
    pub fn with_providers(&self, providers: Providers) -> Result<Self, Error> {
        self.rebuild(providers, self.repos.clone())
    }

    /// Return a new `Manifest` carrying `policy` as its hall integration
    /// defaults. Infallible: an [`IntegrationPolicy`] is a closed value, so
    /// nothing to validate beyond what the original already satisfied.
    ///
    /// `cfg(test)`: a fixture builder. The CLI writes a manifest's integration
    /// policy through `store::manifest`, never by rebuilding the value.
    #[cfg(test)]
    #[must_use]
    pub fn with_integration(&self, policy: IntegrationPolicy) -> Self {
        let mut manifest = self.clone();
        manifest.integration = policy;
        manifest
    }

    /// The common rebuild every `with_*` ends at: the same name, integration,
    /// MCP definitions, and graph paths as `self`, with `providers` and `repos`
    /// replaced — so no mutation can silently drop a field it is not about.
    /// Validates, because the new combination may violate an invariant.
    fn rebuild(&self, providers: Providers, repos: Vec<Repo>) -> Result<Self, Error> {
        let manifest = Self {
            version: CURRENT_VERSION,
            name: self.name.clone(),
            providers,
            repos,
            integration: self.integration,
            schema: self.schema,
            mcp: self.mcp.clone(),
            graph: self.graph.clone(),
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// The value invariants named in the module doc comment. See
    /// the "Invariant enforcement" section above for why this is an explicit
    /// step rather than folded into `Deserialize`.
    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.providers.available.is_empty() {
            return Err(Error::NoAvailableProviders);
        }
        if !self.providers.available.contains(&self.providers.default) {
            return Err(Error::DefaultProviderNotAvailable {
                default: self.providers.default,
                available: self.providers.available.clone(),
            });
        }
        for provider in &self.providers.available {
            let host = provider.config_owner();
            if host != *provider && !self.providers.available.contains(&host) {
                return Err(Error::HostedProviderNeedsHost {
                    provider: *provider,
                    host,
                });
            }
        }

        let mut seen: HashSet<&RepoName> = HashSet::new();
        for repo in &self.repos {
            if !seen.insert(&repo.name) {
                return Err(Error::DuplicateRepoName {
                    name: repo.name.clone(),
                });
            }
            if repo.url.trim().is_empty() {
                return Err(Error::EmptyRepoUrl {
                    name: repo.name.clone(),
                });
            }
            if let Some(prefix) = &repo.ref_prefix
                && *prefix != local_ref_prefix(&repo.name)
            {
                return Err(Error::InvalidRefPrefix {
                    name: repo.name.clone(),
                    prefix: prefix.clone(),
                });
            }
            for (index, check) in repo.checks.iter().enumerate() {
                if check.trim().is_empty() {
                    return Err(Error::EmptyRepoCheck {
                        name: repo.name.clone(),
                        index,
                    });
                }
            }
        }

        if let Some(servers) = &self.mcp {
            let mut seen: HashSet<&str> = HashSet::new();
            for server in servers {
                if !seen.insert(server.name.as_str()) {
                    return Err(Error::DuplicateMcpServerName {
                        name: server.name.clone(),
                    });
                }
                server
                    .transport()
                    .map_err(|transport| Error::InvalidMcpType {
                        name: server.name.clone(),
                        transport,
                    })?;
                server
                    .validate()
                    .map_err(|reason| Error::InvalidMcpServerDefinition {
                        name: server.name.clone(),
                        reason: format!("{reason}"),
                    })?;
            }
        }

        if let Some(graph) = &self.graph {
            let mut seen: HashSet<&str> = HashSet::new();
            for path in &graph.paths {
                if normalise_graph_path(path)? != *path {
                    return Err(Error::InvalidGraphPath {
                        path: path.clone(),
                        reason: "must be written in normalised form".to_owned(),
                    });
                }
                if !seen.insert(path.as_str()) {
                    return Err(Error::InvalidGraphPath {
                        path: path.clone(),
                        reason: "listed more than once".to_owned(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// A hall's provider configuration: which harnesses it knows about, and which
/// one `ivar session start` picks when none is named explicitly.
///
/// Unlike [`Manifest`], this type does not validate its own invariants at
/// construction — `providers.default` being a member of `providers.available`
/// is checked by [`Manifest::validate`], not here, because the check needs
/// nothing from `Providers` that `Manifest` cannot already see, and keeping
/// every invariant check in one place (rather than splitting it between this
/// type and `Manifest`) is what keeps [`Error`]'s variants exhaustive and easy
/// to audit against the module doc comment's list of three.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Providers {
    /// Every provider id this hall knows about.
    available: Vec<Provider>,
    default: Provider,
}

impl Providers {
    /// Build a `Providers` value. Not validated here — see the type doc
    /// comment for why; [`Manifest::new`] validates the whole `Manifest` this
    /// ends up inside.
    #[must_use]
    pub fn new(available: Vec<Provider>, default: Provider) -> Self {
        Self { available, default }
    }

    /// Every provider this hall knows about.
    #[must_use]
    pub fn available(&self) -> &[Provider] {
        &self.available
    }

    /// The provider `ivar session start` picks when none is named explicitly.
    #[must_use]
    pub fn default_provider(&self) -> Provider {
        self.default
    }

    /// The listed providers that own their harness config, hosted ones excluded.
    pub fn config_owners(&self) -> impl Iterator<Item = Provider> + '_ {
        self.available
            .iter()
            .copied()
            .filter(|provider| provider.config_owner() == *provider)
    }
}

/// The comma-joined provider ids in `providers`, for error messages.
pub(super) fn provider_ids(providers: &[Provider]) -> String {
    providers
        .iter()
        .map(Provider::id)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "../../../tests/unit/store/manifest/model.rs"]
mod tests;
