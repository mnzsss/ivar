//! The shipped workflow commands: the provider-neutral Markdown sources every
//! provider's `/ivar-<name>` commands are materialised from.
//!
//! # What lives here
//!
//! A **catalog** of 15 official workflow commands, embedded into the binary at
//! compile time with `include_str!` — there is no runtime asset directory, no
//! `build.rs`, and no directory scan. Each entry pairs the current provider-
//! neutral source with the SHA-256 of the legacy command file it supersedes,
//! so `ivar sync` can safely clean up the old, unprefixed command a previous
//! product wrote. A command with no legacy predecessor carries `None` and its
//! unprefixed file is never touched.
//!
//! The catalog is the *source*; the filesystem lives on the other side of
//! [`materialise`], [`remove`] and [`inspect`]. Paths never appear here —
//! callers compute them with [`Layout::commands_dir`] and hand this module a
//! `&Utf8Path`.
//!
//! # The namespace contract
//!
//! `ivar-*` is reserved for Ivar-owned commands. [`materialise`] deletes any
//! `ivar-*.md` file that is not in the catalog, and never touches any other
//! file in the directory — user commands are preserved byte for byte. The one
//! exception is the fingerprint-gated legacy cleanup: an unprefixed command
//! file is removed only when its SHA-256 matches the catalog constant for its
//! id, because that is how a known, official artifact is recognised without
//! ever risking a user's own file.
//!
//! # Why the sources are embedded, not read at runtime
//!
//! The binary must be able to initialise a hall anywhere — including a
//! machine where only the binary exists — and the command content is a
//! shipping artifact, the same way the help text is. `include_str!` makes the
//! release binary self-contained and the catalog impossible to forget to
//! rebuild.
//!
//! # Layering
//!
//! `harness` may import `domain`, `infra` and `error` — not `store`, so paths
//! arrive here already computed by [`crate::store::layout`].

use camino::{Utf8Path, Utf8PathBuf};

use crate::error::Failure;
use crate::infra::{fs, hash};

mod catalog;

pub use catalog::{ShippedCommand, catalog};

/// What happened to one command file during reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The file did not exist and now does.
    Created,
    /// The file existed and its content changed.
    Updated,
    /// The file was taken away — a leftover `ivar-*` file or a fingerprint-
    /// matched legacy artifact.
    Removed,
    /// The file was already exactly right.
    Unchanged,
}

/// One command file's fate during a reconciliation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandChange {
    /// The command's id — the catalog id for a shipped command, the stripped
    /// `ivar-<id>` stem for a removed file in the reserved namespace.
    pub id: String,
    /// The file this entry is about, e.g. `ivar-plan.md`.
    pub file_name: String,
    /// What happened to it.
    pub change: Change,
}

/// The state of one command file as inspection found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    /// The shipped content is in place, byte for byte.
    Current,
    /// A shipped command is absent.
    Missing,
    /// A shipped command exists but differs from its embedded source.
    Modified,
    /// A legacy, unprefixed command file exists whose content does not match
    /// the known official artifact — a user's customized file that sync must
    /// preserve, never delete.
    LegacyModified,
    /// A file in the reserved `ivar-*` namespace that sync would remove: not
    /// in the catalog, or left over for a provider the hall no longer lists.
    Stale,
}

/// One command file's state for `ivar doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// The command's id — a catalog id, or the stripped stem of a stale
    /// `ivar-*` file.
    pub id: String,
    /// The file this inspection is about.
    pub path: Utf8PathBuf,
    /// How the file's integrity compares with its target state.
    pub integrity: Integrity,
}

/// Everything that can go wrong reconciling a command directory.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The underlying filesystem operation failed. The path names the file or
    /// directory that was being read or written; the source is the
    /// [`fs::Error`] with its own structured detail.
    #[error("could not reconcile workflow commands at `{path}`: {source}")]
    Fs {
        /// The path being read or written when the operation failed.
        path: Utf8PathBuf,
        #[source]
        source: fs::Error,
    },
    /// Fingerprinting a legacy command file failed. The source is the
    /// [`hash::Error`] naming the underlying cause.
    #[error("could not fingerprint legacy command at `{path}`: {source}")]
    Hash {
        /// The legacy file that could not be hashed.
        path: Utf8PathBuf,
        #[source]
        source: hash::Error,
    },
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            // The module-local error adds the reconciliation context to the
            // message; the underlying error already maps to a Failure with the
            // right status and code.
            Error::Fs { source, .. } => source.into(),
            Error::Hash { source, .. } => source.into(),
        }
    }
}

/// Bring `commands_dir` in line with the shipped catalog.
///
/// Creates the directory, writes each shipped command that is missing or
/// modified, deletes any `ivar-*.md` file that is not in the catalog (the
/// prefix is reserved), and removes an unprefixed legacy file only when its
/// SHA-256 matches the catalog constant for its id. Every other file is
/// preserved byte for byte.
///
/// Bytes are compared before every write — [`fs::write_atomic`] runs only when
/// the content differs, so a sync that changes nothing rewrites nothing.
pub fn materialise(commands_dir: &Utf8Path) -> Result<Vec<CommandChange>, Error> {
    fs::ensure_dir(commands_dir).map_err(|source| Error::Fs {
        path: commands_dir.to_owned(),
        source,
    })?;

    let mut changes = Vec::new();

    for command in catalog() {
        let file_name = command.file_name();
        let path = commands_dir.join(&file_name);
        let existing = fs::read_bytes(&path).map_err(|source| Error::Fs {
            path: path.clone(),
            source,
        })?;
        let change = match existing {
            Some(bytes) if bytes == command.content.as_bytes() => Change::Unchanged,
            Some(_) => {
                write_command(&path, command.content)?;
                Change::Updated
            }
            None => {
                write_command(&path, command.content)?;
                Change::Created
            }
        };
        changes.push(CommandChange {
            id: command.id.to_owned(),
            file_name,
            change,
        });
    }

    for entry in markdown_files(commands_dir)? {
        let file_name = entry.file_name().unwrap_or("file").to_owned();
        if let Some(id) = ivar_id(&file_name) {
            // The reserved namespace: anything not in the catalog is ours to
            // remove.
            if catalog().iter().any(|command| command.id == id) {
                continue;
            }
            fs::remove_file(&entry).map_err(|source| Error::Fs {
                path: entry.clone(),
                source,
            })?;
            changes.push(CommandChange {
                id: id.to_owned(),
                file_name,
                change: Change::Removed,
            });
        } else if let Some(command) = legacy_command(&file_name) {
            // The fingerprint gate: delete a legacy file only when its digest
            // proves it is the official artifact, never a user's file. A
            // command with no legacy predecessor (`legacy_sha256: None`) has
            // no unprefixed artifact to recognise, so its legacy file — if
            // any — is a user's file and is preserved.
            if let Some(expected) = command.legacy_sha256 {
                let digest = hash::file(&entry).map_err(|source| Error::Hash {
                    path: entry.clone(),
                    source,
                })?;
                if digest == expected {
                    fs::remove_file(&entry).map_err(|source| Error::Fs {
                        path: entry.clone(),
                        source,
                    })?;
                    changes.push(CommandChange {
                        id: command.id.to_owned(),
                        file_name,
                        change: Change::Removed,
                    });
                }
            }
        }
    }

    Ok(changes)
}

/// Strip every shipped command out of `commands_dir`.
///
/// Used when a provider leaves the hall: all `ivar-*.md` files go, every other
/// file survives, and the directory itself is removed only when it can be
/// proven empty afterwards.
pub fn remove(commands_dir: &Utf8Path) -> Result<Vec<CommandChange>, Error> {
    if !fs::is_dir(commands_dir).map_err(|source| Error::Fs {
        path: commands_dir.to_owned(),
        source,
    })? {
        return Ok(Vec::new());
    }

    let mut changes = Vec::new();
    for entry in markdown_files(commands_dir)? {
        let file_name = entry.file_name().unwrap_or("file").to_owned();
        let Some(id) = ivar_id(&file_name) else {
            continue;
        };
        fs::remove_file(&entry).map_err(|source| Error::Fs {
            path: entry.clone(),
            source,
        })?;
        changes.push(CommandChange {
            id: id.to_owned(),
            file_name,
            change: Change::Removed,
        });
    }

    // The directory is removed only when the read proves it empty — a
    // non-`.md` file a user dropped here keeps it.
    if fs::read_dir(commands_dir)
        .map_err(|source| Error::Fs {
            path: commands_dir.to_owned(),
            source,
        })?
        .is_empty()
    {
        fs::remove_path(commands_dir).map_err(|source| Error::Fs {
            path: commands_dir.to_owned(),
            source,
        })?;
    }

    Ok(changes)
}

/// Report every command file's integrity in `commands_dir`.
///
/// `enabled` is whether the provider is still listed by the hall. An enabled
/// provider's shipped commands are expected to be present and current; a
/// disabled one's leftover `ivar-*` files are all stale (sync will remove
/// them). Legacy files are judged only for an enabled provider, and only a
/// modified one is reported — a fingerprint-matching legacy file is not a
/// problem, sync removes it.
pub fn inspect(commands_dir: &Utf8Path, enabled: bool) -> Result<Vec<Inspection>, Error> {
    let mut inspections = Vec::new();

    let Some(entries) = directory_entries(commands_dir)? else {
        // No command directory: an enabled provider is missing every shipped
        // command; a disabled one has nothing to reconcile.
        if enabled {
            for command in catalog() {
                inspections.push(Inspection {
                    id: command.id.to_owned(),
                    path: commands_dir.join(command.file_name()),
                    integrity: Integrity::Missing,
                });
            }
        }
        return Ok(inspections);
    };

    let mut present: Vec<(String, Utf8PathBuf, Vec<u8>)> = Vec::new();
    for entry in entries {
        let name = entry.file_name().unwrap_or("file").to_owned();
        if !name.ends_with(".md") {
            continue;
        }
        let bytes = fs::read_bytes(&entry).map_err(|source| Error::Fs {
            path: entry.clone(),
            source,
        })?;
        if let Some(bytes) = bytes {
            present.push((name, entry, bytes));
        }
    }

    for command in catalog() {
        let file_name = command.file_name();
        let integrity = match present.iter().find(|(name, _, _)| name == &file_name) {
            Some((_, path, _)) if !enabled => Some((path, Integrity::Stale)),
            Some((_, path, bytes)) if bytes == command.content.as_bytes() => {
                Some((path, Integrity::Current))
            }
            Some((_, path, _)) => Some((path, Integrity::Modified)),
            None if enabled => Some((&commands_dir.join(&file_name), Integrity::Missing)),
            None => None,
        };
        if let Some((path, integrity)) = integrity {
            inspections.push(Inspection {
                id: command.id.to_owned(),
                path: path.clone(),
                integrity,
            });
        }
    }

    for (name, path, bytes) in &present {
        if let Some(id) = ivar_id(name)
            && catalog().iter().all(|command| command.id != id)
        {
            // In the reserved namespace but not in the catalog: sync removes
            // it, whether the provider is still listed or not.
            inspections.push(Inspection {
                id: id.to_owned(),
                path: path.clone(),
                integrity: Integrity::Stale,
            });
        } else if enabled
            && let Some(command) = legacy_command(name)
            && let Some(expected) = command.legacy_sha256
            && hash::bytes(bytes) != expected
        {
            inspections.push(Inspection {
                id: command.id.to_owned(),
                path: path.clone(),
                integrity: Integrity::LegacyModified,
            });
        }
    }

    Ok(inspections)
}

/// Write one shipped command's embedded source to `path`, atomically.
fn write_command(path: &Utf8Path, content: &str) -> Result<(), Error> {
    fs::write_atomic(path, content.as_bytes()).map_err(|source| Error::Fs {
        path: path.to_owned(),
        source,
    })
}

/// Every `.md` file in `commands_dir`, sorted (the underlying
/// [`fs::read_dir`] sorts), for a directory that exists.
fn markdown_files(commands_dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>, Error> {
    Ok(directory_entries(commands_dir)?
        .unwrap_or_default()
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name.ends_with(".md")))
        .collect())
}

/// The entries of `commands_dir`, or `None` when it is not a directory.
fn directory_entries(commands_dir: &Utf8Path) -> Result<Option<Vec<Utf8PathBuf>>, Error> {
    if !fs::is_dir(commands_dir).map_err(|source| Error::Fs {
        path: commands_dir.to_owned(),
        source,
    })? {
        return Ok(None);
    }
    fs::read_dir(commands_dir)
        .map(Some)
        .map_err(|source| Error::Fs {
            path: commands_dir.to_owned(),
            source,
        })
}

/// The catalog id a file name carries, when it is in the reserved `ivar-*`
/// namespace: `ivar-plan.md` → `Some("plan")`. `ivar.md` (no hyphen) is not
/// reserved.
fn ivar_id(file_name: &str) -> Option<&str> {
    file_name.strip_prefix("ivar-")?.strip_suffix(".md")
}

/// The catalog command an unprefixed file name would supersede, if any:
/// `plan.md` → the `plan` command.
fn legacy_command(file_name: &str) -> Option<&ShippedCommand> {
    catalog()
        .iter()
        .find(|command| command.legacy_file_name() == file_name)
}

#[cfg(test)]
#[path = "../../tests/unit/harness/commands.rs"]
mod tests;
