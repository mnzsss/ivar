//! The manifest model: the pure data types behind `ivar.json` and the value
//! invariants that make a hand-edited file refuse loudly instead of silently
//! doing nothing.
//!
//! Reading and writing the file is [`persistence`](super::persistence)'s job;
//! turning a violation into a user-facing `Failure` is
//! [`error`](super::error)'s. This module owns what the other two build on:
//! the `Manifest`, `Providers`, `Repo`, `Skills`, and `Targets` values, their
//! accessors and builders, and `Manifest::validate` — the one place the value
//! invariants live.
//!
//! See `mod.rs` for the full schema, contract, and the "why explicit
//! validate" decision that keeps invariant checks out of `Deserialize`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::domain::feature::IntegrationPolicy;
use crate::domain::mcp::McpServerDef;
use crate::domain::name::{BranchName, HallName, RepoName};
use crate::domain::provider::Provider;

use super::error::Error;

/// `ivar.json`'s schema version. v1 was the first public version; v2 adds the
/// hall integration defaults and each repo's ordered verification checks; v3
/// adds `McpServerDef.oauth`, an optional pre-provisioned OAuth client
/// registration; v4 adds `McpOauth.token_url` and `McpOauth.resource` so
/// refresh metadata survives across sessions.
pub(super) const CURRENT_VERSION: u32 = 4;

/// The hall's identity, committed and team-shared. See the module doc comment
/// for the full JSON shape, the contract, and how the invariants are
/// enforced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    version: u32,
    name: HallName,
    providers: Providers,
    repos: Vec<Repo>,
    /// The hall's integration defaults: the via/strategy a feature inherits
    /// when neither the CLI nor the feature itself overrides a field.
    integration: IntegrationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skills: Option<Skills>,
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp: Option<Vec<McpServerDef>>,
}

/// Canonical URL for the Ivar manifest schema.
pub(super) const MANIFEST_SCHEMA_URL: &str = "https://ivar.run/ivar.schema.json";

impl Manifest {
    /// Build a validated `Manifest`. Refuses exactly what [`Self::validate`]
    /// refuses — see the module doc comment for why this is the one type-level
    /// guarantee this module makes: any `Manifest` built through this
    /// constructor already satisfies every invariant.
    ///
    /// The v1 call shape is preserved: a manifest built through this
    /// constructor carries the embedded integration defaults
    /// ([`IntegrationPolicy::default`], `local`/`squash`) and repos with no
    /// checks, and the canonical manifest schema reference.
    pub fn new(
        name: HallName,
        providers: Providers,
        repos: Vec<Repo>,
        skills: Option<Skills>,
    ) -> Result<Self, Error> {
        let manifest = Self {
            version: CURRENT_VERSION,
            name,
            providers,
            repos,
            integration: IntegrationPolicy::default(),
            skills,
            schema: Some(MANIFEST_SCHEMA_URL.to_owned()),
            mcp: None,
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

    /// The hall's shared skill home, if it has one.
    #[must_use]
    pub fn skills(&self) -> Option<&Skills> {
        self.skills.as_ref()
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

    /// Return a new `Manifest` with `repo` appended to `repos`.
    ///
    /// Returns [`Error::DuplicateRepoName`] if a repo with `repo.name()`
    /// already appears. The original is untouched — `ivar.json` is rewritten
    /// from the returned value, never mutated in place.
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
    /// skills, and MCP definitions as `self`, with `providers` and `repos`
    /// replaced — so no mutation can silently drop a field it is not about.
    /// Validates, because the new combination may violate an invariant.
    fn rebuild(&self, providers: Providers, repos: Vec<Repo>) -> Result<Self, Error> {
        let manifest = Self {
            version: CURRENT_VERSION,
            name: self.name.clone(),
            providers,
            repos,
            integration: self.integration,
            skills: self.skills.clone(),
            schema: self.schema.clone(),
            mcp: self.mcp.clone(),
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
}

/// One repo a hall knows about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    name: RepoName,
    url: String,
    default_branch: BranchName,
    /// The repo's ordered verification checks, run via `bash -lc` in the
    /// relevant worktree when this repo is integrated or delivered. Empty
    /// means "no checks" — the v1 common case, and deliberately omitted from
    /// the on-disk shape. `#[serde(default)]` so a v1 repo still deserialises.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checks: Vec<String>,
}

impl Repo {
    /// Build a `Repo`. `url` stays a plain `String` rather than one of
    /// `domain::name`'s newtypes: it is a git remote, never joined onto a path,
    /// so none of the path-safety rules apply to it.
    ///
    /// It must still be non-empty, and that is checked by
    /// [`Manifest::validate`] alongside the other invariants rather than here —
    /// so it is enforced on **read** too, which is the case that matters, since
    /// this file is hand-edited. An empty `url` is the difference between a
    /// `Failure` naming the offending repo and a bare `git clone` error the
    /// first time someone runs `ivar sync`.
    ///
    /// The v1 call shape is preserved: a repo built through this constructor
    /// carries no checks.
    #[must_use]
    pub fn new(name: RepoName, url: impl Into<String>, default_branch: BranchName) -> Self {
        Self {
            name,
            url: url.into(),
            default_branch,
            checks: Vec::new(),
        }
    }

    /// This repo's name. Unique among a manifest's repos — see
    /// [`Manifest::validate`].
    #[must_use]
    pub fn name(&self) -> &RepoName {
        &self.name
    }

    /// The git remote URL this repo clones from.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The branch a fresh worktree of this repo defaults to.
    #[must_use]
    pub fn default_branch(&self) -> &BranchName {
        &self.default_branch
    }

    /// The repo's ordered verification checks, in execution order. Empty when
    /// the hall declared none for this repo.
    #[must_use]
    pub fn checks(&self) -> &[String] {
        &self.checks
    }

    /// Return this repo with `checks` as its verification commands, replacing
    /// whatever it carried before. The original is untouched.
    ///
    /// `cfg(test)`: a fixture builder — the CLI parses checks out of
    /// `ivar.json`, it never grafts them onto a `Repo` in memory.
    #[cfg(test)]
    #[must_use]
    pub fn with_checks(mut self, checks: Vec<String>) -> Self {
        self.checks = checks;
        self
    }
}

/// A hall's shared skill home: which harnesses skills materialise for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Skills {
    targets: Targets,
}

impl Skills {
    /// Build a `Skills` value.
    #[must_use]
    pub fn new(targets: Targets) -> Self {
        Self { targets }
    }

    /// Which harnesses this hall's shared skills materialise for.
    #[must_use]
    pub fn targets(&self) -> &Targets {
        &self.targets
    }
}

/// Which harnesses a hall's shared skills materialise for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Targets {
    claude: bool,
    opencode: bool,
}

impl Targets {
    /// Build a `Targets` value.
    #[must_use]
    pub fn new(claude: bool, opencode: bool) -> Self {
        Self { claude, opencode }
    }

    /// Whether skills materialise at `.claude/skills/`.
    #[must_use]
    pub fn claude(&self) -> bool {
        self.claude
    }

    /// Whether skills materialise at `.opencode/skills/`.
    #[must_use]
    pub fn opencode(&self) -> bool {
        self.opencode
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
