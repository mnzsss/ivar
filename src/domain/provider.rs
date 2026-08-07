//! Which harnesses exist, and where each one keeps its config.
//!
//! The set is **closed and known at compile time** — Claude Code and OpenCode.
//! That is a deliberate modelling choice, not a limitation waiting to be fixed:
//! adding a harness means writing an adapter, a config materialiser and a log
//! parser, so it is a code change either way. A closed enum means the compiler
//! finds every place that has to learn about the new one.
//!
//! This module holds only what a harness *is* and *where its files live*. How to
//! spawn it, how to normalise its output, and what it can and cannot do belong to
//! `harness`, which sits a layer out and may do I/O. `domain` stays pure.
//!
//! # Contract
//!
//! ```text
//! Provider::ClaudeCode  →  id "claude-code"  config dir ".claude"    instructions "CLAUDE.md"
//! Provider::OpenCode    →  id "opencode"     config dir ".opencode"  instructions "AGENTS.md"
//! ```
//!
//! - `id()` — the stable wire string, as it appears in `ivar.json` and in
//!   `--provider`. Kebab-case, never reworded.
//! - `config_dir()` — the dotdir at the hall root the harness discovers by
//!   walk-up.
//! - `instruction_file()` — the Markdown file the harness reads for standing
//!   instructions.
//! - `commands_dir()` — where `ivar`'s own slash-commands are mirrored. Owned
//!   entirely by `ivar` and gitignored: files there that do not correspond to a
//!   shipped skill get deleted on sync, so user-authored commands must not live
//!   there.
//! - `skills_dir()` — where hall-scoped skills materialise for this harness.
//! - `mcp_config_path()` and `mcp_key()` — the file and the key the servers hang
//!   off. These differ per harness and are mapped here rather than at each call
//!   site.
//! - `ALL` — every variant, for iteration.
//! - `Display`, `FromStr` (so `clap` can parse `--provider`), `Serialize` /
//!   `Deserialize` as `id()`.
//!
//! # The mapping is not the identity function
//!
//! `claude-code` → `.claude`, **not** `.claude-code`. This is the whole reason the
//! module exists: deriving the dotdir from the id by string interpolation is
//! wrong for one of the two harnesses today, and would fail silently — `ivar`
//! would write config into a directory the harness never reads, and the user would
//! see an agent that simply does not know about their skills.
//!
//! # Where each fact came from
//!
//! `id()` comes from the manifest schema
//! (`packages/bifrost/src/manifest/schema.ts`):
//! `z.enum(['claude-code', 'opencode'])` — those two strings are the whole set.
//!
//! `config_dir()`, `instruction_file()`, `commands_dir()`, `skills_dir()` and
//! the Claude Code half of `mcp_config_path()` come from
//! `packages/bifrost/CONTEXT.md` prose, not from guesswork:
//!
//! - *Hall*: "per-provider config (`.claude/commands/` + `CLAUDE.md` for Claude
//!   Code, `.opencode/commands/` + `AGENTS.md` for OpenCode)".
//! - *Hall Skill Home*: "Skills materialise at the hall root
//!   (`<hall>/.claude/skills/`, `<hall>/.opencode/skills/`)".
//! - *MCP*: "Materialised at the hall root by `bifrost hall sync` (`.mcp.json`
//!   for Claude Code, the OpenCode equivalent) ... discovered by walk-up".
//!
//! `mcp_key()` is sourced too, just not from `bifrost`:
//! `docs/wayfinder/bifrost-open-source/BACKLOG.md`, item B19, surveys
//! `vibe-kanban` (Apache-2.0, nine working harness adapters including both
//! Claude Code and OpenCode) and records its MCP config keys verbatim —
//! `mcp_servers` for Codex, `amp.mcpServers` for Amp, `mcp` + `$schema` for
//! OpenCode, `mcpServers` as the default. `"mcpServers"` for Claude Code and
//! `"mcp"` for OpenCode both come from that survey of a shipping
//! multi-harness tool, which is a stronger source than the convention this
//! module leaned on before finding it.
//!
//! The one fact B19 does *not* settle is the OpenCode *filename* half of
//! `mcp_config_path()` — it documents OpenCode's MCP *key*, not the path of
//! the file that key lives in, and `CONTEXT.md`'s own wording ("the OpenCode
//! equivalent" of `.mcp.json`) is deliberately vague on the same point. That
//! filename (`opencode.json`) is still *inferred*, from OpenCode's own
//! convention of one config file at the project root — revisit it if
//! `bifrost` or a landed harness adapter ever names it explicitly.
//!
//! B19 also notes OpenCode's config carries a `$schema` key alongside `mcp`.
//! No accessor for it here — nothing in `ivar` needs it yet, and `domain`
//! should not grow surface speculatively — but it is worth knowing for
//! whoever writes the MCP materialiser in `harness::config`, so they do not
//! have to rediscover it.
//!
//! # This type replaces `name::ProviderName`
//!
//! `domain::name::ProviderName` was written as a validated single-segment string,
//! on the assumption that a provider is an arbitrary name that becomes a path.
//! It is not — the set is closed. A validated string that can hold `"claude-cod"`
//! is strictly worse than an enum that cannot.
//!
//! So: **delete `ProviderName` from `domain::name`** (and its tests), and change
//! `store::layout::harness_dir` to take `&Provider` and return
//! `root.join(provider.config_dir())`. That accessor currently does the identity
//! interpolation and is wrong for Claude Code today.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Failure, FixAction};

/// The comma-separated list of valid ids, for error messages. Kept next to
/// `ALL` so the two are easy to keep in sync by eye.
const VALID_IDS: &str = "claude-code, opencode";

/// A harness `ivar` can open a session in.
///
/// The set is closed — see the module doc comment for why. Every accessor here
/// is a `match` over the two variants; there is no derivation from `id()`, which
/// is the bug this type exists to make impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Provider {
    /// Anthropic's Claude Code.
    ClaudeCode,
    /// OpenCode.
    OpenCode,
}

impl Provider {
    /// Every variant, for iteration. Kept exhaustive by
    /// `tests::all_is_exhaustive`, which fails to compile if a variant is added
    /// here without being added there too.
    pub const ALL: [Provider; 2] = [Provider::ClaudeCode, Provider::OpenCode];

    /// The stable wire string: as it appears in `ivar.json` and in
    /// `--provider`. Kebab-case, never reworded.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::OpenCode => "opencode",
        }
    }

    /// The dotdir at the hall root this harness discovers by walk-up.
    #[must_use]
    pub const fn config_dir(&self) -> &'static str {
        match self {
            Self::ClaudeCode => ".claude",
            Self::OpenCode => ".opencode",
        }
    }

    /// The Markdown file this harness reads for standing instructions.
    #[must_use]
    pub const fn instruction_file(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "CLAUDE.md",
            Self::OpenCode => "AGENTS.md",
        }
    }

    /// Where `ivar`'s own slash-commands are mirrored for this harness.
    ///
    /// Owned entirely by `ivar` and gitignored: files here that do not
    /// correspond to a shipped skill get deleted on sync, so user-authored
    /// commands must not live here.
    #[must_use]
    pub const fn commands_dir(&self) -> &'static str {
        match self {
            Self::ClaudeCode => ".claude/commands",
            Self::OpenCode => ".opencode/commands",
        }
    }

    /// Where hall-scoped skills materialise for this harness.
    #[must_use]
    pub const fn skills_dir(&self) -> &'static str {
        match self {
            Self::ClaudeCode => ".claude/skills",
            Self::OpenCode => ".opencode/skills",
        }
    }

    /// The file this harness's MCP server definitions live in, at the hall
    /// root.
    ///
    /// The OpenCode half of this mapping is inferred, not sourced from
    /// `bifrost` — see the module doc comment's "Where each fact came from"
    /// section.
    #[must_use]
    pub const fn mcp_config_path(&self) -> &'static str {
        match self {
            Self::ClaudeCode => ".mcp.json",
            Self::OpenCode => "opencode.json",
        }
    }

    /// The key the MCP servers hang off, inside [`Self::mcp_config_path`].
    ///
    /// The OpenCode half of this mapping is inferred, not sourced from
    /// `bifrost` — see the module doc comment's "Where each fact came from"
    /// section.
    #[must_use]
    pub const fn mcp_key(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "mcpServers",
            Self::OpenCode => "mcp",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

impl FromStr for Provider {
    type Err = InvalidProvider;

    /// Parses the wire string produced by [`Self::id`]. Used by `clap` for
    /// `--provider`, and shares its error (and therefore its message and fix
    /// action) with [`Deserialize`].
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.id() == value)
            .ok_or_else(|| InvalidProvider::UnknownId {
                given: value.to_owned(),
            })
    }
}

impl Serialize for Provider {
    /// As [`Self::id`] — the wire string, never the variant name.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for Provider {
    /// Routes through [`FromStr`], so a hand-edited `ivar.json` with an unknown
    /// id fails with the same clear, options-naming error `--provider` would
    /// give.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Why a provider id was refused. There is exactly one way: it does not name
/// one of the closed set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidProvider {
    /// The id did not match [`Provider::id`] for any variant.
    #[error("unknown provider id `{given}`; valid ids are: {VALID_IDS}")]
    UnknownId {
        /// The id that was rejected.
        given: String,
    },
}

impl InvalidProvider {
    /// Stable, machine-matchable identifier for [`Failure::code`].
    const fn code(&self) -> &'static str {
        match self {
            Self::UnknownId { .. } => "provider.unknown_id",
        }
    }

    /// The specific way out — naming the valid options, per this house's error
    /// model.
    fn fix_action(&self) -> FixAction {
        match self {
            Self::UnknownId { .. } => FixAction::safe(
                "provider.use_valid_id",
                format!("Use one of the valid provider ids: {VALID_IDS}."),
            ),
        }
    }
}

impl From<InvalidProvider> for Failure {
    fn from(error: InvalidProvider) -> Self {
        Failure::blocked(error.code(), error.to_string()).fix(error.fix_action())
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;

    // -- accessors, per variant: literals, not computed ---------------------

    #[test]
    fn claude_code_accessors() {
        let provider = Provider::ClaudeCode;
        assert_eq!(provider.id(), "claude-code");
        assert_eq!(provider.config_dir(), ".claude");
        assert_eq!(provider.instruction_file(), "CLAUDE.md");
        assert_eq!(provider.commands_dir(), ".claude/commands");
        assert_eq!(provider.skills_dir(), ".claude/skills");
        assert_eq!(provider.mcp_config_path(), ".mcp.json");
        assert_eq!(provider.mcp_key(), "mcpServers");
    }

    #[test]
    fn opencode_accessors() {
        let provider = Provider::OpenCode;
        assert_eq!(provider.id(), "opencode");
        assert_eq!(provider.config_dir(), ".opencode");
        assert_eq!(provider.instruction_file(), "AGENTS.md");
        assert_eq!(provider.commands_dir(), ".opencode/commands");
        assert_eq!(provider.skills_dir(), ".opencode/skills");
        assert_eq!(provider.mcp_config_path(), "opencode.json");
        assert_eq!(provider.mcp_key(), "mcp");
    }

    // -- Display --------------------------------------------------------------

    #[test]
    fn display_is_the_id() {
        assert_eq!(Provider::ClaudeCode.to_string(), "claude-code");
        assert_eq!(Provider::OpenCode.to_string(), "opencode");
    }

    // -- serde: Serialize is a bare string, never the variant name -----------

    #[test]
    fn serialize_is_the_wire_string_not_the_variant_name() {
        assert_eq!(
            serde_json::to_string(&Provider::ClaudeCode).unwrap(),
            r#""claude-code""#
        );
        assert_eq!(
            serde_json::to_string(&Provider::OpenCode).unwrap(),
            r#""opencode""#
        );
    }

    #[test]
    fn deserializing_a_well_formed_id_round_trips() {
        let provider: Provider = serde_json::from_str(r#""claude-code""#).unwrap();
        assert_eq!(provider, Provider::ClaudeCode);

        let provider: Provider = serde_json::from_str(r#""opencode""#).unwrap();
        assert_eq!(provider, Provider::OpenCode);
    }

    /// A hand-edited `ivar.json` naming a harness that does not exist must fail
    /// clearly, naming the valid options — not panic, and not silently pick a
    /// default.
    #[test]
    fn deserializing_an_unknown_id_fails_naming_the_valid_options() {
        let result: Result<Provider, _> = serde_json::from_str(r#""claude""#);
        let error = result.expect_err("unknown id must be rejected");
        let message = error.to_string();
        assert!(message.contains("claude-code"));
        assert!(message.contains("opencode"));
    }

    // -- FromStr agrees with Deserialize --------------------------------------

    #[test]
    fn from_str_accepts_every_id() {
        assert_eq!(
            "claude-code".parse::<Provider>().unwrap(),
            Provider::ClaudeCode
        );
        assert_eq!("opencode".parse::<Provider>().unwrap(), Provider::OpenCode);
    }

    #[test]
    fn from_str_and_deserialize_agree_on_an_unknown_id() {
        let from_str_error = "claude".parse::<Provider>().unwrap_err();
        let deserialize_error = serde_json::from_str::<Provider>(r#""claude""#).unwrap_err();
        assert_eq!(from_str_error.to_string(), deserialize_error.to_string());
    }

    #[test]
    fn from_str_error_names_the_rejected_value() {
        let error = "not-a-provider".parse::<Provider>().unwrap_err();
        assert_eq!(
            error,
            InvalidProvider::UnknownId {
                given: "not-a-provider".to_owned()
            }
        );
    }

    // -- InvalidProvider -> Failure -------------------------------------------

    #[test]
    fn invalid_provider_converts_to_a_blocked_failure_with_a_fix_naming_the_options() {
        let failure: Failure = InvalidProvider::UnknownId {
            given: "claude".to_owned(),
        }
        .into();
        assert_eq!(failure.code, "provider.unknown_id");
        assert_eq!(failure.fix_actions.len(), 1);
        assert!(failure.fix_actions[0].safe);
        assert!(failure.fix_actions[0].what.contains("claude-code"));
        assert!(failure.fix_actions[0].what.contains("opencode"));
    }

    // -- ALL is exhaustive -----------------------------------------------------

    /// The cheap way to make forgetting a new variant in `ALL` a compile
    /// error: this `match` has no wildcard arm, so adding a variant to
    /// `Provider` without adding it here fails to compile, forcing the two to
    /// stay in lockstep.
    #[test]
    fn all_is_exhaustive() {
        fn covers_every_variant(provider: Provider) {
            match provider {
                Provider::ClaudeCode | Provider::OpenCode => {}
            }
        }

        for provider in Provider::ALL {
            covers_every_variant(provider);
        }
        assert_eq!(Provider::ALL.len(), 2);
    }
}
