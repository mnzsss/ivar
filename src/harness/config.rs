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

use crate::domain::mcp::McpServerDef;
use crate::domain::name::{HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::{fs, json};

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

/// Materialise `provider`'s MCP config at `path` from `servers`.
///
/// This is the JSON half of the instruction-file block: idempotent by
/// comparison (nothing is written when the bytes already match), and respectful
/// of the user's own bytes — for a provider whose config file carries more than
/// MCP (OpenCode's `opencode.json`), only the provider's `mcp` key is replaced.
/// A file that exists but cannot be parsed as a JSON object is refused, never
/// clobbered.
pub fn materialise_mcp(
    path: &Utf8Path,
    provider: Provider,
    servers: &[McpServerDef],
) -> Result<Change, Error> {
    let servers_value = servers_doc(provider, servers);
    let (existing, raw) = read_doc(path)?;

    let Some(mut doc) = existing else {
        return write_doc(path, &mcp_doc(provider, servers_value)).map(|_| Change::Created);
    };

    let object = doc.as_object_mut().ok_or_else(|| Error::McpNotObject {
        path: path.to_path_buf(),
    })?;
    object.insert(provider.mcp_key().to_owned(), servers_value);
    // OpenCode's config carries a `$schema`; make sure one is there, without
    // clobbering one the user already wrote.
    if provider == Provider::OpenCode && !object.contains_key("$schema") {
        object.insert(
            "$schema".to_owned(),
            serde_json::json!("https://opencode.ai/config.json"),
        );
    }

    let rendered = json::to_canonical_string(&doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })?;
    if raw.as_deref() == Some(rendered.as_str()) {
        return Ok(Change::Unchanged);
    }

    write_doc(path, &doc)?;
    Ok(Change::Updated)
}

/// Take the MCP key out of the config at `path` — the JSON half of the
/// instruction-file strip.
///
/// The file is deleted only when the MCP key was its entire content (`.mcp.json`
/// with nothing but `mcpServers`); a file carrying other keys keeps them, minus
/// the MCP key. Absent file, or a file with no MCP key, is
/// [`Change::Unchanged`]. A file that cannot be parsed as a JSON object is
/// left alone — stripping a key out of something that is not an object has no
/// defined meaning, and deleting it would be the silent-overwrite bug again.
pub fn remove_mcp(path: &Utf8Path, provider: Provider) -> Result<Change, Error> {
    let (existing, _) = read_doc(path)?;
    let Some(mut doc) = existing else {
        return Ok(Change::Unchanged);
    };

    let Some(object) = doc.as_object_mut() else {
        return Ok(Change::Unchanged);
    };
    if object.remove(provider.mcp_key()).is_none() {
        return Ok(Change::Unchanged);
    }

    if object.is_empty() {
        fs::remove_file(path).map_err(|source| Error::Mcp {
            path: path.to_path_buf(),
            source: json::Error::Fs(source),
        })?;
        return Ok(Change::Removed);
    }

    write_doc(path, &doc)?;
    Ok(Change::Removed)
}

/// The full document `ivar` wants for `provider`: its `mcp` key holding
/// `servers`, plus OpenCode's `$schema`. Used only when the file is absent —
/// an existing file is merged key-by-key instead ([`materialise_mcp`]).
fn mcp_doc(provider: Provider, servers: serde_json::Value) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    if provider == Provider::OpenCode {
        root.insert(
            "$schema".to_owned(),
            serde_json::json!("https://opencode.ai/config.json"),
        );
    }
    root.insert(provider.mcp_key().to_owned(), servers);
    serde_json::Value::Object(root)
}

/// The `mcp` value itself: one entry per server, keyed by name.
fn servers_doc(provider: Provider, servers: &[McpServerDef]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for server in servers {
        map.insert(server.name.clone(), server_doc(provider, server));
    }
    serde_json::Value::Object(map)
}

/// One server's entry, in `provider`'s spelling of it.
///
/// Claude Code's shape is the canonical one (`command`/`args`/`env`). OpenCode
/// spells the same facts differently — `stdio` → `local` with `command` as one
/// array, `sse`/`streamable-http` → `remote` with a `url`, `env` →
/// `environment` — and that translation is exactly what this function is for.
fn server_doc(provider: Provider, server: &McpServerDef) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    match provider {
        Provider::ClaudeCode => {
            object.insert("type".to_owned(), serde_json::json!(server.type_));
            if let Some(command) = &server.command {
                object.insert("command".to_owned(), serde_json::json!(command));
            }
            if let Some(args) = &server.args {
                object.insert("args".to_owned(), serde_json::json!(args));
            }
            if let Some(url) = &server.url {
                object.insert("url".to_owned(), serde_json::json!(url));
            }
            if let Some(env) = &server.env {
                object.insert("env".to_owned(), serde_json::json!(env));
            }
        }
        Provider::OpenCode => {
            let transport = if server.type_ == "stdio" {
                "local"
            } else {
                "remote"
            };
            object.insert("type".to_owned(), serde_json::json!(transport));
            if server.type_ == "stdio" {
                let mut command: Vec<&str> = Vec::new();
                if let Some(binary) = &server.command {
                    command.push(binary);
                }
                if let Some(args) = &server.args {
                    command.extend(args.iter().map(String::as_str));
                }
                if !command.is_empty() {
                    object.insert("command".to_owned(), serde_json::json!(command));
                }
            } else if let Some(url) = &server.url {
                object.insert("url".to_owned(), serde_json::json!(url));
            }
            if let Some(env) = &server.env {
                object.insert("environment".to_owned(), serde_json::json!(env));
            }
        }
    }
    serde_json::Value::Object(object)
}

/// Read `path` as JSON, returning the parsed document and its raw bytes.
///
/// `Ok((None, None))` when the file is absent. A file that exists but is not
/// valid JSON is an error — never a silent clobber of user config.
fn read_doc(path: &Utf8Path) -> Result<(Option<serde_json::Value>, Option<String>), Error> {
    let Some(text) = fs::read_text(path).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Fs(source),
    })?
    else {
        return Ok((None, None));
    };
    let value = serde_json::from_str(&text).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source: json::Error::Parse {
            path: path.to_path_buf(),
            source,
        },
    })?;
    Ok((Some(value), Some(text)))
}

/// Write `doc` to `path` in the canonical byte format.
fn write_doc(path: &Utf8Path, doc: &serde_json::Value) -> Result<(), Error> {
    json::write_canonical(path, doc).map_err(|source| Error::Mcp {
        path: path.to_path_buf(),
        source,
    })
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
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use crate::test_support::utf8_temp_dir;

    fn hall() -> HallName {
        HallName::new("acme").unwrap()
    }

    fn repo(name: &str) -> RepoName {
        RepoName::new(name).unwrap()
    }

    // -- build_block ----------------------------------------------------------

    #[test]
    fn the_block_is_delimited_by_the_markers_and_names_the_hall() {
        let block = build_block(&hall(), &[repo("api")]);

        assert!(block.starts_with(MANAGED_START));
        assert!(block.ends_with(MANAGED_END));
        assert!(block.contains("# acme"));
    }

    #[test]
    fn repos_are_listed_in_the_order_given() {
        let block = build_block(&hall(), &[repo("web"), repo("api")]);

        let web = block.find("`web`").unwrap();
        let api = block.find("`api`").unwrap();
        assert!(web < api, "manifest order must survive into the block");
    }

    #[test]
    fn a_hall_with_no_repos_says_how_to_add_one() {
        let block = build_block(&hall(), &[]);

        assert!(block.contains("ivar.json"));
        assert!(block.contains("ivar sync"));
    }

    /// [`materialise`] decides "unchanged" by comparing bytes, so the builder
    /// has to be a function of its arguments and nothing else.
    #[test]
    fn building_the_same_block_twice_produces_identical_bytes() {
        let first = build_block(&hall(), &[repo("api")]);
        let second = build_block(&hall(), &[repo("api")]);

        assert_eq!(first, second);
    }

    // -- materialise: the three placement cases -------------------------------

    #[test]
    fn an_absent_file_is_created_holding_only_the_block() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("CLAUDE.md");
        let block = build_block(&hall(), &[repo("api")]);

        assert_eq!(materialise(&path, &block).unwrap(), Change::Created);

        assert_eq!(fs::read_text(&path).unwrap().unwrap(), format!("{block}\n"));
    }

    #[test]
    fn an_existing_block_is_replaced_in_place_leaving_the_users_text_alone() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("CLAUDE.md");
        let first = build_block(&hall(), &[repo("api")]);
        fs::write_text(
            &path,
            &format!("# House rules\n\n{first}\n\nNever force-push.\n"),
        )
        .unwrap();

        let second = build_block(&hall(), &[repo("api"), repo("web")]);
        assert_eq!(materialise(&path, &second).unwrap(), Change::Updated);

        let content = fs::read_text(&path).unwrap().unwrap();
        assert!(content.starts_with("# House rules\n"));
        assert!(content.ends_with("Never force-push.\n"));
        assert!(content.contains("`web`"));
        assert_eq!(content.matches(MANAGED_START).count(), 1);
    }

    #[test]
    fn a_file_with_no_markers_keeps_every_byte_and_gains_the_block_on_top() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("CLAUDE.md");
        fs::write_text(&path, "# House rules\n\nNever force-push.\n").unwrap();
        let block = build_block(&hall(), &[repo("api")]);

        assert_eq!(materialise(&path, &block).unwrap(), Change::Updated);

        let content = fs::read_text(&path).unwrap().unwrap();
        assert!(content.starts_with(MANAGED_START));
        assert!(content.contains("# House rules"));
        assert!(content.contains("Never force-push."));
    }

    /// `ivar sync` runs after every `git pull`. A version that rewrote the file
    /// each time would put a spurious modification in `git status` on every
    /// run.
    #[test]
    fn materialising_the_same_block_twice_reports_unchanged_and_does_not_rewrite() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("CLAUDE.md");
        let block = build_block(&hall(), &[repo("api")]);

        assert_eq!(materialise(&path, &block).unwrap(), Change::Created);
        let after_first = fs::read_bytes(&path).unwrap().unwrap();

        assert_eq!(materialise(&path, &block).unwrap(), Change::Unchanged);
        assert_eq!(fs::read_bytes(&path).unwrap().unwrap(), after_first);
    }

    /// An end marker before a start marker is not a block to splice — treating
    /// it as one would replace the region *between* them, which is the user's
    /// text, with the block.
    #[test]
    fn reversed_markers_are_treated_as_no_block_rather_than_spliced() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("CLAUDE.md");
        fs::write_text(
            &path,
            &format!("{MANAGED_END}\nprecious user text\n{MANAGED_START}\n"),
        )
        .unwrap();
        let block = build_block(&hall(), &[repo("api")]);

        assert_eq!(materialise(&path, &block).unwrap(), Change::Updated);

        let content = fs::read_text(&path).unwrap().unwrap();
        assert!(
            content.contains("precious user text"),
            "the user's text must survive: {content}"
        );
    }

    // -- remove ---------------------------------------------------------------

    #[test]
    fn removing_from_a_file_that_held_only_the_block_deletes_the_file() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("AGENTS.md");
        let block = build_block(&hall(), &[repo("api")]);
        materialise(&path, &block).unwrap();

        assert_eq!(remove(&path).unwrap(), Change::Removed);
        assert!(!fs::exists(&path).unwrap());
    }

    #[test]
    fn removing_from_a_file_the_user_wrote_in_keeps_the_file() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("AGENTS.md");
        let block = build_block(&hall(), &[repo("api")]);
        fs::write_text(&path, &format!("{block}\n\n# House rules\n")).unwrap();

        assert_eq!(remove(&path).unwrap(), Change::Removed);

        let content = fs::read_text(&path).unwrap().unwrap();
        assert_eq!(content, "# House rules\n");
    }

    #[test]
    fn removing_when_there_is_nothing_to_remove_is_unchanged() {
        let (_guard, dir) = utf8_temp_dir();
        let absent = dir.join("AGENTS.md");
        assert_eq!(remove(&absent).unwrap(), Change::Unchanged);

        let untouched = dir.join("CLAUDE.md");
        fs::write_text(&untouched, "# House rules\n").unwrap();
        assert_eq!(remove(&untouched).unwrap(), Change::Unchanged);
        assert_eq!(
            fs::read_text(&untouched).unwrap().unwrap(),
            "# House rules\n"
        );
    }

    // -- Error -> Failure ------------------------------------------------------

    #[test]
    fn an_io_error_keeps_the_fs_layers_code_and_names_the_file() {
        let (_guard, dir) = utf8_temp_dir();
        // A directory where a file is expected: reading it fails at the fs
        // layer, which is the mechanical cause this module wraps.
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(&path).unwrap();

        let error = materialise(&path, "block").expect_err("cannot read a directory as text");
        let failure: Failure = error.into();

        assert!(
            failure
                .fix_actions
                .iter()
                .any(|fix| fix.code == "harness.check_instruction_file"),
            "expected the file-naming fix action, got {:?}",
            failure.fix_actions
        );
    }

    // -- materialise_mcp: Claude Code ----------------------------------------

    /// The v1 case the sync step starts from: no servers declared, and still a
    /// valid — empty — config at the hall root, so walk-up discovery finds it.
    #[test]
    fn an_empty_server_list_materialises_a_valid_empty_config() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join(".mcp.json");

        assert_eq!(
            materialise_mcp(&path, Provider::ClaudeCode, &[]).unwrap(),
            Change::Created
        );

        let expected = json::to_canonical_string(&serde_json::json!({ "mcpServers": {} })).unwrap();
        assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
    }

    #[test]
    fn a_stdio_server_is_serialised_with_command_args_and_env() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join(".mcp.json");
        let servers = vec![
            McpServerDef::new("docs", "stdio")
                .command("npx")
                .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
                .env(std::collections::BTreeMap::from([(
                    "TOKEN".to_owned(),
                    "{env:TOKEN}".to_owned(),
                )])),
        ];

        materialise_mcp(&path, Provider::ClaudeCode, &servers).unwrap();

        let expected = json::to_canonical_string(&serde_json::json!({
            "mcpServers": {
                "docs": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "@acme/docs-mcp"],
                    "env": { "TOKEN": "{env:TOKEN}" },
                }
            }
        }))
        .unwrap();
        assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
    }

    /// `ivar sync` runs after every `git pull`; a second run must touch nothing.
    #[test]
    fn materialising_mcp_twice_is_unchanged_and_does_not_rewrite() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join(".mcp.json");
        let servers = vec![McpServerDef::new("docs", "stdio").command("npx")];

        assert_eq!(
            materialise_mcp(&path, Provider::ClaudeCode, &servers).unwrap(),
            Change::Created
        );
        let after_first = fs::read_bytes(&path).unwrap().unwrap();

        assert_eq!(
            materialise_mcp(&path, Provider::ClaudeCode, &servers).unwrap(),
            Change::Unchanged
        );
        assert_eq!(fs::read_bytes(&path).unwrap().unwrap(), after_first);
    }

    #[test]
    fn changed_servers_rewrite_the_config() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join(".mcp.json");
        materialise_mcp(&path, Provider::ClaudeCode, &[]).unwrap();

        let with_server = vec![McpServerDef::new("docs", "stdio").command("npx")];
        assert_eq!(
            materialise_mcp(&path, Provider::ClaudeCode, &with_server).unwrap(),
            Change::Updated
        );

        let on_disk = fs::read_text(&path).unwrap().unwrap();
        assert!(on_disk.contains("\"docs\""), "was: {on_disk}");
    }

    // -- materialise_mcp: OpenCode -------------------------------------------

    #[test]
    fn opencode_materialises_the_schema_key_next_to_the_mcp_key() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("opencode.json");

        materialise_mcp(&path, Provider::OpenCode, &[]).unwrap();

        let expected = json::to_canonical_string(&serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "mcp": {},
        }))
        .unwrap();
        assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
    }

    /// The same definition, spelled the way OpenCode reads it: `stdio` →
    /// `local`, one `command` array, `environment` for the env map.
    #[test]
    fn opencode_translates_a_stdio_definition_into_its_own_spelling() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("opencode.json");
        let servers = vec![
            McpServerDef::new("docs", "stdio")
                .command("npx")
                .args(vec!["-y".to_owned(), "@acme/docs-mcp".to_owned()])
                .env(std::collections::BTreeMap::from([(
                    "TOKEN".to_owned(),
                    "{env:TOKEN}".to_owned(),
                )])),
        ];

        materialise_mcp(&path, Provider::OpenCode, &servers).unwrap();

        let expected = json::to_canonical_string(&serde_json::json!({
            "$schema": "https://opencode.ai/config.json",
            "mcp": {
                "docs": {
                    "type": "local",
                    "command": ["npx", "-y", "@acme/docs-mcp"],
                    "environment": { "TOKEN": "{env:TOKEN}" },
                }
            }
        }))
        .unwrap();
        assert_eq!(fs::read_text(&path).unwrap().unwrap(), expected);
    }

    #[test]
    fn opencode_spells_a_remote_definition_with_type_remote_and_a_url() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("opencode.json");
        let servers = vec![McpServerDef::new("sentry", "sse").url("https://mcp.example.com/mcp")];

        materialise_mcp(&path, Provider::OpenCode, &servers).unwrap();

        let on_disk = fs::read_text(&path).unwrap().unwrap();
        assert!(on_disk.contains("\"type\": \"remote\""), "was: {on_disk}");
        assert!(
            on_disk.contains("\"url\": \"https://mcp.example.com/mcp\""),
            "was: {on_disk}"
        );
    }

    /// `opencode.json` is OpenCode's *general* config. The user's other keys
    /// must survive a sync that replaces the `mcp` key.
    #[test]
    fn an_existing_opencode_config_keeps_its_other_keys() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("opencode.json");
        fs::write_text(
            &path,
            &json::to_canonical_string(&serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "model": "anthropic/claude-sonnet-4-5",
                "mcp": { "stale": { "type": "local", "command": ["old"] } },
            }))
            .unwrap(),
        )
        .unwrap();

        let servers = vec![McpServerDef::new("docs", "stdio").command("npx")];
        assert_eq!(
            materialise_mcp(&path, Provider::OpenCode, &servers).unwrap(),
            Change::Updated
        );

        let on_disk = fs::read_text(&path).unwrap().unwrap();
        assert!(
            on_disk.contains("claude-sonnet-4-5"),
            "the user's model must survive: {on_disk}"
        );
        assert!(
            !on_disk.contains("\"stale\""),
            "the mcp key must be replaced: {on_disk}"
        );
        assert!(
            on_disk.contains("\"docs\""),
            "the manifest's servers must land: {on_disk}"
        );

        // And the next sync touches nothing.
        assert_eq!(
            materialise_mcp(&path, Provider::OpenCode, &servers).unwrap(),
            Change::Unchanged
        );
    }

    // -- materialise_mcp: never clobber what cannot be parsed ----------------

    #[test]
    fn a_config_that_is_not_an_object_is_refused_not_clobbered() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("opencode.json");
        fs::write_text(&path, "[1, 2, 3]").unwrap();

        let error = materialise_mcp(&path, Provider::OpenCode, &[])
            .expect_err("an array cannot take an mcp key");

        let failure: Failure = error.into();
        assert_eq!(failure.code, "harness.mcp_not_an_object");
        assert_eq!(
            fs::read_text(&path).unwrap().unwrap(),
            "[1, 2, 3]",
            "the file must be left exactly as it was"
        );
    }

    #[test]
    fn a_config_that_is_not_valid_json_is_refused_not_clobbered() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join(".mcp.json");
        fs::write_text(&path, "{ not json").unwrap();

        let error = materialise_mcp(&path, Provider::ClaudeCode, &[]).expect_err("unparseable");

        let failure: Failure = error.into();
        assert!(
            failure
                .fix_actions
                .iter()
                .any(|fix| fix.code == "harness.check_mcp_config"),
            "expected the mcp fix action, got {:?}",
            failure.fix_actions
        );
        assert_eq!(
            fs::read_text(&path).unwrap().unwrap(),
            "{ not json",
            "the file must be left exactly as it was"
        );
    }

    // -- remove_mcp ----------------------------------------------------------

    #[test]
    fn removing_an_exclusively_mcp_file_deletes_it() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join(".mcp.json");
        materialise_mcp(&path, Provider::ClaudeCode, &[]).unwrap();

        assert_eq!(
            remove_mcp(&path, Provider::ClaudeCode).unwrap(),
            Change::Removed
        );
        assert!(!fs::exists(&path).unwrap());
    }

    #[test]
    fn removing_the_mcp_key_keeps_a_file_with_other_keys() {
        let (_guard, dir) = utf8_temp_dir();
        let path = dir.join("opencode.json");
        fs::write_text(
            &path,
            &json::to_canonical_string(&serde_json::json!({
                "$schema": "https://opencode.ai/config.json",
                "model": "anthropic/claude-sonnet-4-5",
                "mcp": {},
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            remove_mcp(&path, Provider::OpenCode).unwrap(),
            Change::Removed
        );

        let on_disk = fs::read_text(&path).unwrap().unwrap();
        assert!(
            on_disk.contains("claude-sonnet-4-5"),
            "the user's keys must survive: {on_disk}"
        );
        assert!(
            !on_disk.contains("\"mcp\""),
            "the mcp key must be gone: {on_disk}"
        );
    }

    #[test]
    fn removing_mcp_when_there_is_nothing_to_remove_is_unchanged() {
        let (_guard, dir) = utf8_temp_dir();

        let absent = dir.join(".mcp.json");
        assert_eq!(
            remove_mcp(&absent, Provider::ClaudeCode).unwrap(),
            Change::Unchanged
        );

        let without_mcp = dir.join("opencode.json");
        fs::write_text(&without_mcp, "{\"model\": \"x\"}").unwrap();
        assert_eq!(
            remove_mcp(&without_mcp, Provider::OpenCode).unwrap(),
            Change::Unchanged
        );
        assert_eq!(
            fs::read_text(&without_mcp).unwrap().unwrap(),
            "{\"model\": \"x\"}",
            "a file with no mcp key is not rewritten"
        );
    }
}
