//! The `ivar`-managed block inside a harness's instruction file.
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
//! # Idempotence is checked, not assumed
//!
//! [`materialise`] compares before writing and reports [`Change::Unchanged`]
//! when the block already matches. That is not an optimisation. `ivar sync` is
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
//! different product surface.

use camino::Utf8Path;

use crate::domain::name::{HallName, RepoName};
use crate::error::{Failure, FixAction};
use crate::infra::fs;

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

/// Everything that can go wrong maintaining a managed block. There is one
/// thing: the file would not read or write.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not update the instruction file `{path}`")]
    Io {
        path: camino::Utf8PathBuf,
        #[source]
        source: fs::Error,
    },
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
}
