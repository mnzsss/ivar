//! The canonical hall instructions and every provider's root alias: the sole
//! owner of the managed block in `HALL.md` and of the alias symlinks at the
//! hall root.
//!
//! # The canonical file
//!
//! `HALL.md` is the only editable, committed source of standing hall
//! instructions. It belongs to the user; `ivar` owns exactly the bytes between
//! [`MANAGED_START`] and [`MANAGED_END`] and nothing else:
//!
//! - file absent → create it holding only the block
//! - markers present → replace what is between them, byte for byte
//! - markers absent → prepend the block, keeping every existing byte after it
//! - file present but not a regular file → a typed [`Change::Conflict`], never
//!   a rewrite
//!
//! Rewriting the file wholesale would be the same silent-overwrite bug `init`
//! refuses to commit against `ivar.json`, on a file people care about more.
//!
//! # The aliases
//!
//! Each enabled provider's root alias — `CLAUDE.md` for Claude Code,
//! `AGENTS.md` for OpenCode — must be a **relative symlink to `HALL.md`**.
//! Aliases are never sources and never workflow edit targets; sessions read
//! `HALL.md` directly.
//!
//! For an **enabled** provider, a regular alias file is never moved,
//! overwritten, or deleted automatically — even when it is the sole legacy
//! file. It is preserved byte for byte and reported as a conflict so the human
//! can consolidate it. A broken or wrong-target symlink is atomically replaced.
//!
//! For a **disabled** provider (absent from `providers.available`), the alias
//! path is entirely Ivar-managed: any entry there — including a regular file —
//! is removed. This is a deliberately destructive exception to adoption
//! safety; it never removes `HALL.md` itself.
//!
//! # Layering
//!
//! This module is in `harness`, which may not import `store` — paths arrive
//! here already computed by [`crate::store::layout`]. Callers build
//! [`Alias`]es from `Layout::instruction_alias` and hand in the canonical path
//! from `Layout::hall_instructions`; `infra::fs` performs the I/O.
//!
//! # Reference
//!
//! `packages/bifrost/src/lib/provider-config.ts` in the private monorepo, read
//! for the marker mechanic and the three placement cases, which it got right.
//! The block's *content* is not ported: it advertised commands that belong to a
//! different product surface.

use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::name::{HallName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};
use crate::infra::fs;

/// Opens the region of the canonical instruction file `ivar` owns.
pub const MANAGED_START: &str = "<!-- ivar:managed:start -->";

/// Closes the region of the canonical instruction file `ivar` owns.
pub const MANAGED_END: &str = "<!-- ivar:managed:end -->";

/// The canonical root instruction filename and the relative target every
/// enabled provider alias must point at.
pub const CANONICAL_FILE: &str = "HALL.md";

/// What [`reconcile`] did to one root instruction entry.
///
/// Four of the states are what [`materialise`] reports; [`Change::Conflict`] is
/// the fifth, the case where the entry exists in a shape `ivar` must not
/// touch — a non-regular `HALL.md`, or an enabled provider's regular alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The file did not exist and now does.
    Created,
    /// The file existed and its managed block (or symlink target) changed.
    Updated,
    /// The file was taken away.
    Removed,
    /// The file was already exactly right.
    Unchanged,
    /// The entry exists in a shape `ivar` must not touch; nothing was written.
    Conflict,
}

/// One root instruction entry's fate during a reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path the entry is about — `HALL.md` or a provider alias.
    pub path: Utf8PathBuf,
    /// What happened to it.
    pub change: Change,
    /// Anything worth saying beyond the change — why an entry conflicted.
    pub detail: Option<String>,
}

/// One root alias's place in the reconciliation: which provider it belongs to,
/// where it lives, and whether that provider is still listed by the hall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    /// The provider this alias belongs to.
    pub provider: Provider,
    /// The alias path — `Layout::instruction_alias`'s answer.
    pub path: Utf8PathBuf,
    /// Whether the provider is in `providers.available`.
    pub enabled: bool,
}

/// The state of one root instruction entry as inspection found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    /// The entry is exactly as it should be.
    Current,
    /// The entry is absent.
    Missing,
    /// The canonical file exists but is not a regular file.
    NotRegular,
    /// The canonical file has no managed block.
    ManagedBlockMissing,
    /// The canonical file's managed block differs from the manifest.
    ManagedBlockStale,
    /// An enabled provider's alias is a regular file.
    AliasIsRegular,
    /// An enabled provider's alias is a symlink whose target does not exist.
    AliasBroken,
    /// An enabled provider's alias points somewhere other than `HALL.md`.
    AliasWrongTarget,
    /// A disabled provider's alias still has an entry at its path.
    DisabledAliasPresent,
}

/// One root instruction entry's state for `ivar doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// The path this inspection is about.
    pub path: Utf8PathBuf,
    /// How the entry's integrity compares with its target state.
    pub integrity: Integrity,
}

/// Build the managed block for a hall named `hall` containing `repos`.
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
/// other bytes are never rewritten. A file that is not regular (a directory,
/// a symlink) is *not* this function's decision — [`reconcile`] refuses those
/// as conflicts before materialisation is ever reached.
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
            // what to do in it. Every existing byte survives.
            write(path, &format!("{block}\n\n{existing}"))?;
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

/// Reconcile the canonical `HALL.md` and every provider root alias in one
/// pass: process the canonical file once, then each [`Alias`] in order.
///
/// `canonical` is `Layout::hall_instructions`'s answer; `block` is the
/// manifest-derived managed block; `aliases` carries each provider's alias
/// path and enabled state. The canonical file is never removed and never
/// rewritten wholesale — only its managed block is ever replaced — and an
/// enabled provider's regular alias is preserved byte for byte.
pub fn reconcile(
    canonical: &Utf8Path,
    block: &str,
    aliases: &[Alias],
) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    entries.push(reconcile_canonical(canonical, block)?);
    for alias in aliases {
        entries.push(reconcile_alias(alias)?);
    }
    Ok(entries)
}

/// Report every root instruction entry's integrity in one pass: the canonical
/// file, then each alias. `block` is the expected managed-block bytes, used to
/// judge the canonical file's block current or stale.
pub fn inspect(
    canonical: &Utf8Path,
    block: &str,
    aliases: &[Alias],
) -> Result<Vec<Inspection>, Error> {
    let mut inspections = Vec::new();
    inspections.push(inspect_canonical(canonical, block)?);
    for alias in aliases {
        inspections.push(inspect_alias(alias)?);
    }
    Ok(inspections)
}

/// Reconcile `HALL.md` itself. See [`reconcile`] for the placement rules.
fn reconcile_canonical(path: &Utf8Path, block: &str) -> Result<Entry, Error> {
    match entry_kind(path)? {
        // A symlink or any non-regular entry at the canonical path is a typed
        // conflict: rewriting through it — or over it — could clobber a
        // directory or another file the user pointed at.
        EntryKind::Symlink(_) => Ok(conflict(
            path,
            "`HALL.md` is a symlink; the canonical instructions must be a \
             regular file — replace it with one, then run `ivar sync`",
        )),
        EntryKind::NonRegular => Ok(conflict(
            path,
            "`HALL.md` exists but is not a regular file; make it a regular \
                 file, then run `ivar sync`",
        )),
        _ => {
            let change = materialise(path, block)?;
            Ok(Entry {
                path: path.to_path_buf(),
                change,
                detail: None,
            })
        }
    }
}

/// Reconcile one provider alias. See the module doc for the enabled/disabled
/// rules; the destructive disabled-provider case never touches `HALL.md`.
fn reconcile_alias(alias: &Alias) -> Result<Entry, Error> {
    let path = &alias.path;
    let name = path.file_name().unwrap_or("alias").to_owned();
    let target = Utf8Path::new(CANONICAL_FILE);

    match (entry_kind(path)?, alias.enabled) {
        (EntryKind::Absent, true) => {
            fs::create_symlink(target, path).map_err(|source| io_error(path, source))?;
            Ok(Entry {
                path: path.clone(),
                change: Change::Created,
                detail: None,
            })
        }
        (EntryKind::Absent, false) => Ok(Entry {
            path: path.clone(),
            change: Change::Unchanged,
            detail: None,
        }),
        (EntryKind::Regular | EntryKind::NonRegular, true) => Ok(conflict(
            path,
            &format!(
                "`{name}` is a regular file and was preserved; consolidate its \
                 instructions into `HALL.md`, remove it, run `ivar sync`, and \
                 review the git diff"
            ),
        )),
        (EntryKind::Regular | EntryKind::NonRegular, false) => remove_alias_entry(alias),
        (EntryKind::Symlink(current), true) => {
            if current.as_str() == CANONICAL_FILE {
                Ok(Entry {
                    path: path.clone(),
                    change: Change::Unchanged,
                    detail: None,
                })
            } else {
                fs::replace_symlink_if_changed(target, path)
                    .map_err(|source| io_error(path, source))?;
                Ok(Entry {
                    path: path.clone(),
                    change: Change::Updated,
                    detail: Some(format!(
                        "`{name}` pointed at `{current}`; now a relative symlink to `HALL.md`"
                    )),
                })
            }
        }
        (EntryKind::Symlink(_), false) => remove_alias_entry(alias),
    }
}

/// The disabled-provider rule: the alias path is entirely Ivar-managed, so any
/// entry there — symlink or regular file — is removed.
fn remove_alias_entry(alias: &Alias) -> Result<Entry, Error> {
    let path = &alias.path;
    let name = path.file_name().unwrap_or("alias").to_owned();
    fs::remove_file(path).map_err(|source| io_error(path, source))?;
    Ok(Entry {
        path: path.clone(),
        change: Change::Removed,
        detail: Some(format!(
            "removed `{name}` because {} is no longer available",
            alias.provider
        )),
    })
}

/// Inspect the canonical file. `Current` means a regular file whose managed
/// block matches `block` exactly.
fn inspect_canonical(path: &Utf8Path, block: &str) -> Result<Inspection, Error> {
    let integrity = match entry_kind(path)? {
        EntryKind::Absent => Integrity::Missing,
        EntryKind::Symlink(_) | EntryKind::NonRegular => Integrity::NotRegular,
        EntryKind::Regular => match read(path)? {
            None => Integrity::Missing,
            Some(content) => match locate(&content) {
                None => Integrity::ManagedBlockMissing,
                Some(span) => {
                    let current = content.get(span).unwrap_or_default();
                    if current == block {
                        Integrity::Current
                    } else {
                        Integrity::ManagedBlockStale
                    }
                }
            },
        },
    };
    Ok(Inspection {
        path: path.to_path_buf(),
        integrity,
    })
}

/// Inspect one provider alias. A disabled provider's remaining entry —
/// whatever it is — is [`Integrity::DisabledAliasPresent`].
fn inspect_alias(alias: &Alias) -> Result<Inspection, Error> {
    let path = &alias.path;
    let integrity = match (entry_kind(path)?, alias.enabled) {
        (EntryKind::Absent, true) => Integrity::Missing,
        (EntryKind::Absent, false) => Integrity::Current,
        (EntryKind::Regular | EntryKind::NonRegular, true) => Integrity::AliasIsRegular,
        (EntryKind::Regular | EntryKind::NonRegular, false) => Integrity::DisabledAliasPresent,
        (EntryKind::Symlink(current), true) => {
            if current.as_str() == CANONICAL_FILE {
                Integrity::Current
            } else {
                // Broken vs wrong target: does the target resolve to anything?
                let resolved = path
                    .parent()
                    .map(|parent| parent.join(&current))
                    .unwrap_or(current);
                match fs::exists(&resolved).map_err(|source| io_error(path, source))? {
                    true => Integrity::AliasWrongTarget,
                    false => Integrity::AliasBroken,
                }
            }
        }
        (EntryKind::Symlink(_), false) => Integrity::DisabledAliasPresent,
    };
    Ok(Inspection {
        path: path.clone(),
        integrity,
    })
}

/// A conflict entry: nothing was written to `path`, and `detail` says why.
fn conflict(path: &Utf8Path, detail: &str) -> Entry {
    Entry {
        path: path.to_path_buf(),
        change: Change::Conflict,
        detail: Some(detail.to_owned()),
    }
}

/// The filesystem shape each reconciliation branch needs to distinguish. This
/// centralises the `read_symlink`/regular-file interpretation so reconciliation
/// and inspection share one definition of a root instruction entry.
enum EntryKind {
    Absent,
    Symlink(Utf8PathBuf),
    Regular,
    NonRegular,
}

fn entry_kind(path: &Utf8Path) -> Result<EntryKind, Error> {
    match fs::read_symlink(path).map_err(|source| io_error(path, source))? {
        fs::SymlinkTarget::Absent => Ok(EntryKind::Absent),
        fs::SymlinkTarget::Target(target) => Ok(EntryKind::Symlink(target)),
        fs::SymlinkTarget::NotASymlink => {
            if fs::is_file(path).map_err(|source| io_error(path, source))? {
                Ok(EntryKind::Regular)
            } else {
                Ok(EntryKind::NonRegular)
            }
        }
    }
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

fn io_error(path: &Utf8Path, source: fs::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Everything that can go wrong maintaining the canonical instructions or an
/// alias: the file would not read or write.
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
        let Error::Io { path, source } = error;
        // The wrapped `fs::Error` already carries a code and a fix action for
        // the mechanical cause; this only adds which file it was about, which
        // the fs layer cannot know.
        let failure: Failure = source.into();
        failure.fix(FixAction::safe(
            "harness.check_instruction_file",
            format!("Check that `{path}` is writable, then run `ivar sync` again."),
        ))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/harness/config/instructions.rs"]
mod tests;
