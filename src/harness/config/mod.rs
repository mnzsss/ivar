//! The `ivar`-managed block inside a harness's instruction file, plus the
//! hall's MCP server config.
//!
//! Every harness reads a Markdown file at the hall root for standing
//! instructions — `CLAUDE.md` for Claude Code, `AGENTS.md` for OpenCode (see
//! [`Provider::instruction_file`](crate::domain::provider::Provider::instruction_file)).
//! `ivar sync` keeps a block in that file
//! describing what the hall contains, so an agent opened anywhere in the hall
//! knows what it is looking at without being told.
//!
//! # The file belongs to the user, not to `ivar`
//!
//! This is the whole design constraint. A hall's `CLAUDE.md` is where a team
//! writes its own standing instructions, and it is committed. So `ivar` owns
//! exactly the bytes between [`MANAGED_START`] and [`MANAGED_END`] and nothing
//! else:
//!
//! - file absent → create it holding only the block
//! - markers present → replace what is between them, byte for byte
//! - markers absent → prepend the block, keeping every existing byte after it
//! - provider dropped from the hall → strip the block, and delete the file only
//!   if the block was all it ever held
//!
//! Rewriting the file wholesale would be the same silent-overwrite bug `init`
//! refuses to commit against `ivar.json`, on a file people care about more.
//!
//! # MCP config materialisation: the same constraint, one key at a time
//!
//! The hall's MCP server definitions materialise at the hall root — `.mcp.json`
//! for Claude Code, `opencode.json` for OpenCode — discovered by walk-up from
//! every session's View Dir. [`materialise_mcp`] and [`remove_mcp`] apply the
//! "the file belongs to the user" rule with a JSON key standing in for the
//! marker pair:
//!
//! - `.mcp.json` is *exclusively* an MCP file, so `ivar` owns it wholesale.
//! - `opencode.json` is OpenCode's **general** config — model, permissions,
//!   MCP all live there. `ivar` owns exactly the `mcp` key: the materialiser
//!   merges, replacing that key and leaving every other key the user wrote
//!   untouched, and never clobbers a file it cannot parse as a JSON object.
//!
//! The two harnesses spell the same definition differently, and the
//! translation lives here: Claude Code's `mcpServers` entries keep
//! `command`/`args`/`env` as separate fields; OpenCode's `mcp` entries turn
//! `stdio` into `local` (with `command` as one array) and `sse`/`streamable-http`
//! into `remote`, and rename `env` to `environment`. The `$schema` key OpenCode
//! expects accompanies its `mcp` key.
//!
//! # Idempotence is checked, not assumed
//!
//! [`materialise`] and [`materialise_mcp`] compare before writing and report
//! [`Change::Unchanged`] when the content already matches. That is not an
//! optimisation. `ivar sync` is
//! what people run after every `git pull`; a version that rewrote the file each
//! time would put a spurious modification in `git status` on every run, and a
//! tool that dirties your working tree for no reason is a tool you stop
//! running.
//!
//! # Reference
//!
//! `packages/bifrost/src/lib/provider-config.ts` in the private monorepo, read
//! for the marker mechanic and the three placement cases, which it got right.
//! The block's *content* is not ported: it advertised commands that belong to a
//! different product surface. The OpenCode `$schema` URL and the `mcp` key
//! shape come from OpenCode's own docs (`opencode.ai/docs/config`,
//! `opencode.ai/docs/mcp-servers`), which are the same sources
//! [`Provider::mcp_config_path`](crate::domain::provider::Provider::mcp_config_path)
//! cites.

use camino::Utf8Path;

use crate::domain::name::{HallName, RepoName};
use crate::error::{Failure, FixAction};
use crate::infra::{fs, json};

mod mcp;

pub use mcp::{materialise_mcp, remove_mcp};

/// Opens the region of the instruction file `ivar` owns.
pub const MANAGED_START: &str = "<!-- ivar:managed:start -->";

/// Closes the region of the instruction file `ivar` owns.
pub const MANAGED_END: &str = "<!-- ivar:managed:end -->";

/// What [`materialise`] or [`remove`] did to a file.
///
/// Four total states, no failure state: both functions return `Result`, and a
/// failure is an `Err` carrying what broke. Callers that need a fifth "failed"
/// bucket for a report build it themselves — folding it in here would make
/// every match arm handle a value this module can never produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The file did not exist and now does.
    Created,
    /// The file existed and its managed block changed.
    Updated,
    /// The file existed and already said exactly this.
    Unchanged,
    /// The managed block was taken out (and the file with it, if the block was
    /// all it held).
    Removed,
}

/// Build the block for a hall named `hall` containing `repos`.
///
/// Pure: no I/O, no clock, no environment. Two calls with the same arguments
/// produce the same bytes, which is what lets [`materialise`] decide
/// "unchanged" by comparison instead of by bookkeeping.
///
/// Repos are listed in the order given — manifest order, which is the order the
/// user wrote them in and therefore the order they expect to read them back.
#[must_use]
pub fn build_block(hall: &HallName, repos: &[RepoName]) -> String {
    let mut block = String::new();

    block.push_str(MANAGED_START);
    block.push('\n');
    block.push_str(&format!("# {hall}\n\n"));
    block.push_str(
        "This directory is an `ivar` hall. Each repository below is a real git\n\
         worktree mounted under `.ivar/repos/`, so a change here is a change in\n\
         that repository — there is no copy and no sync step to remember.\n\n\
         After pulling this hall, run `ivar sync` to bring the local checkout\n\
         back in line with `ivar.json`.\n\n",
    );

    block.push_str("## Repositories\n\n");
    if repos.is_empty() {
        block.push_str(
            "None yet. Add one to the `repos` list in `ivar.json`, then run `ivar sync`.\n",
        );
    } else {
        for repo in repos {
            block.push_str("- `");
            block.push_str(repo.as_str());
            block.push_str("`\n");
        }
    }

    block.push('\n');
    block.push_str(MANAGED_END);
    block
}

/// Put `block` into the instruction file at `path`, touching nothing else.
///
/// See the module doc comment for the three placement cases and why the file's
/// other bytes are never rewritten.
pub fn materialise(path: &Utf8Path, block: &str) -> Result<Change, Error> {
    let Some(existing) = read(path)? else {
        write(path, &format!("{block}\n"))?;
        return Ok(Change::Created);
    };

    match locate(&existing) {
        Some(span) => {
            let current = existing.get(span.clone()).unwrap_or_default();
            if current == block {
                return Ok(Change::Unchanged);
            }
            let before = existing.get(..span.start).unwrap_or_default();
            let after = existing.get(span.end..).unwrap_or_default();
            write(path, &format!("{before}{block}{after}"))?;
            Ok(Change::Updated)
        }
        None => {
            // No markers: the user's file predates this hall, or someone
            // deleted the block. Prepend rather than append — an instruction
            // file is read top-down, and what the directory *is* belongs before
            // what to do in it.
            let rest = existing.trim_start();
            write(path, &format!("{block}\n\n{rest}"))?;
            Ok(Change::Updated)
        }
    }
}

/// Take the managed block out of the instruction file at `path`.
///
/// Deletes the file only when the block was the entire content — a file the
/// user has written in is left in place, minus the block. Absent file, or a
/// file with no block, is [`Change::Unchanged`].
pub fn remove(path: &Utf8Path) -> Result<Change, Error> {
    let Some(existing) = read(path)? else {
        return Ok(Change::Unchanged);
    };

    let Some(span) = locate(&existing) else {
        return Ok(Change::Unchanged);
    };

    let before = existing.get(..span.start).unwrap_or_default();
    let after = existing.get(span.end..).unwrap_or_default();
    let stripped = format!("{before}{after}");

    if stripped.trim().is_empty() {
        fs::remove_file(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        return Ok(Change::Removed);
    }

    write(path, &format!("{}\n", stripped.trim()))?;
    Ok(Change::Removed)
}

/// The byte range the managed block occupies in `content`, markers included.
///
/// `None` when the markers are absent, or when the end marker precedes the
/// start — a half-truncated block is treated as no block rather than as a
/// region to splice, because splicing on a reversed range would eat whatever
/// sits between them, which is the user's text.
fn locate(content: &str) -> Option<std::ops::Range<usize>> {
    let start = content.find(MANAGED_START)?;
    let end = content.find(MANAGED_END)?;
    if end < start {
        return None;
    }
    Some(start..end + MANAGED_END.len())
}

fn read(path: &Utf8Path) -> Result<Option<String>, Error> {
    fs::read_text(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write(path: &Utf8Path, contents: &str) -> Result<(), Error> {
    fs::write_atomic(path, contents.as_bytes()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Everything that can go wrong maintaining a managed block or an MCP config.
/// There are three things: the file would not read or write, the MCP config
/// could not be parsed, or it parsed as something the `mcp` key cannot merge
/// into.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not update the instruction file `{path}`")]
    Io {
        path: camino::Utf8PathBuf,
        #[source]
        source: fs::Error,
    },
    /// Something failed reading, writing, or serialising an MCP config — the
    /// wrapped error is `infra::json`'s own, which already distinguishes the
    /// mechanical cause.
    #[error("could not maintain the MCP config `{path}`: {source}")]
    Mcp {
        path: camino::Utf8PathBuf,
        #[source]
        source: json::Error,
    },
    /// The MCP config parsed as JSON but is not an object, so there is no safe
    /// way to merge the `mcp` key into it — and `ivar` will not invent one.
    #[error("`{path}` is not a JSON object; ivar will not overwrite it")]
    McpNotObject { path: camino::Utf8PathBuf },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::Io { path, source } => {
                // The wrapped `fs::Error` already carries a code and a fix
                // action for the mechanical cause; this only adds which file it
                // was about, which the fs layer cannot know.
                let failure: Failure = source.into();
                failure.fix(FixAction::safe(
                    "harness.check_instruction_file",
                    format!("Check that `{path}` is writable, then run `ivar sync` again."),
                ))
            }
            Error::Mcp { path, source } => {
                let failure: Failure = source.into();
                failure.fix(FixAction::safe(
                    "harness.check_mcp_config",
                    format!(
                        "Check that `{path}` is valid JSON and writable, then run `ivar sync` again."
                    ),
                ))
            }
            Error::McpNotObject { path } => Failure::blocked(
                "harness.mcp_not_an_object",
                format!("`{path}` is not a JSON object"),
            )
            .expected("a JSON object at the hall root")
            .actual("some other JSON shape")
            .fix(FixAction::safe(
                "harness.fix_mcp_config",
                format!("Make `{path}` a JSON object (or remove it), then run `ivar sync` again."),
            )),
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config.rs"]
mod tests;
