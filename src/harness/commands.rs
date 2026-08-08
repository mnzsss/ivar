//! The shipped workflow commands: the provider-neutral Markdown sources every
//! provider's `/ivar-<name>` commands are materialised from.
//!
//! # What lives here
//!
//! A **catalog** of 14 official workflow commands, embedded into the binary at
//! compile time with `include_str!` — there is no runtime asset directory, no
//! `build.rs`, and no directory scan. Each entry pairs the current provider-
//! neutral source with the SHA-256 of the legacy command file it supersedes,
//! so `ivar sync` can safely clean up the old, unprefixed command a previous
//! product wrote.
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
    fs::ensure_dir(commands_dir)
        .map_err(|source| Error::Fs {
            path: commands_dir.to_owned(),
            source,
        })?;

    let mut changes = Vec::new();

    for command in catalog() {
        let path = commands_dir.join(command.file_name());
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
            file_name: command.file_name(),
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
            // proves it is the official artifact, never a user's file.
            let digest = hash::file(&entry).map_err(|source| Error::Hash {
                path: entry.clone(),
                source,
            })?;
            if digest == command.legacy_sha256 {
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
            Some((_, path, bytes)) if !enabled => Some((path, Integrity::Stale)),
            Some((_, path, bytes)) if bytes == command.content.as_bytes() => {
                Some((path, Integrity::Current))
            }
            Some((_, path, _)) => Some((path, Integrity::Modified)),
            None if enabled => Some((
                &commands_dir.join(&file_name),
                Integrity::Missing,
            )),
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
            && hash::bytes(bytes) != command.legacy_sha256
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
    Ok(directory_entries(commands_dir)?.unwrap_or_default()
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

/// One shipped workflow command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShippedCommand {
    /// The command's id — the `<id>` in `ivar-<id>.md` and in `/ivar-<id>`.
    pub id: &'static str,
    /// The provider-neutral Markdown source, embedded at compile time.
    pub content: &'static str,
    /// SHA-256 of the legacy, unprefixed command file this id supersedes —
    /// the fingerprint that proves a Bifrost-era file is an official artifact
    /// safe to remove.
    pub legacy_sha256: &'static str,
}

impl ShippedCommand {
    /// The filename this command materialises as: `ivar-<id>.md`.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("ivar-{}.md", self.id)
    }

    /// The legacy, unprefixed filename this command supersedes: `<id>.md`.
    #[must_use]
    pub fn legacy_file_name(self) -> String {
        format!("{}.md", self.id)
    }
}

/// Every shipped workflow command, in a stable order. The catalog is explicit
/// and static — one `include_str!` per source — so adding a command is a
/// reviewable one-line change in a single file.
pub const fn catalog() -> &'static [ShippedCommand] {
    COMMANDS
}

/// The 14 official workflow commands, paired with the legacy fingerprint of
/// the command each one supersedes. The `legacy_sha256` values are the exact
/// SHA-256 digests of the Bifrost-era command files; do not change them
/// without regenerating the digest of the artifact they describe.
const COMMANDS: &[ShippedCommand] = &[
    ShippedCommand {
        id: "deliver",
        content: include_str!("commands/deliver.md"),
        legacy_sha256: "b8402403fba034c85355def2f40ca9cec0e5572f4e67b130ebeac14ceda64c8b",
    },
    ShippedCommand {
        id: "discovery",
        content: include_str!("commands/discovery.md"),
        legacy_sha256: "97fba325393f6eba415a62bb6120d7bdc4cd813872e15d6f6669c910e32c0120",
    },
    ShippedCommand {
        id: "execute",
        content: include_str!("commands/execute.md"),
        legacy_sha256: "94c2aa9d9617de45cc5d985e752a99d4c6f5899654967d618542f270a5e18a72",
    },
    ShippedCommand {
        id: "feature-create",
        content: include_str!("commands/feature-create.md"),
        legacy_sha256: "062a359e6ecf9fa8313d65f478737ee0018ef1c4c17868e2dff3e7abbc3dfe16",
    },
    ShippedCommand {
        id: "feature-status",
        content: include_str!("commands/feature-status.md"),
        legacy_sha256: "67d092c2ecf3469a96c17fd8971dd6caa2e0ea97ca404361fea59617d681129c",
    },
    ShippedCommand {
        id: "plan",
        content: include_str!("commands/plan.md"),
        legacy_sha256: "5b1e361e11d342c022901a41f89de1a8b2463eb63c42e15d4e8fee9498fa188e",
    },
    ShippedCommand {
        id: "promote",
        content: include_str!("commands/promote.md"),
        legacy_sha256: "eae89c066ce3526b5e7cb3d4cd76f822faec9b3430965d4fdf83ae97e40c084f",
    },
    ShippedCommand {
        id: "repo-list",
        content: include_str!("commands/repo-list.md"),
        legacy_sha256: "cd8705d0e972c339ca55607c89e5cf4702123677e1a1c02ea4cf5502d105a8e1",
    },
    ShippedCommand {
        id: "repo-setup",
        content: include_str!("commands/repo-setup.md"),
        legacy_sha256: "255554048fcf58d7f6d396acc1713bc888d185e00794db47be2965a849bc4068",
    },
    ShippedCommand {
        id: "review",
        content: include_str!("commands/review.md"),
        legacy_sha256: "da6d0ad313c366246d0b15fac0e04340af65786486dfaed5f5128770537d4b2d",
    },
    ShippedCommand {
        id: "session-connect",
        content: include_str!("commands/session-connect.md"),
        legacy_sha256: "c81e99ac2bbfcea31381e61ead8e2a51cf91c46781e4466025ab11f23bee7b24",
    },
    ShippedCommand {
        id: "session-start",
        content: include_str!("commands/session-start.md"),
        legacy_sha256: "43affb5874c67b0aa2e904c7bca48499401f8d04667cbe6500add74d2c6508e4",
    },
    ShippedCommand {
        id: "session-stop",
        content: include_str!("commands/session-stop.md"),
        legacy_sha256: "2e2c6fc76618a19f77dec801dd59d52b6a5b6446f8f048943750534701aa4bbd",
    },
    ShippedCommand {
        id: "sync",
        content: include_str!("commands/sync.md"),
        legacy_sha256: "e663a6534823dcc7a0699e126d4e32619277e08ea48e657de8f74da0806bf15d",
    },
];

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
    use std::collections::BTreeSet;

    #[test]
    fn catalog_is_complete_unique_and_current() {
        let commands = catalog();
        assert_eq!(commands.len(), 14);

        let ids = commands
            .iter()
            .map(|command| command.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), commands.len());

        for command in commands {
            assert_eq!(command.file_name(), format!("ivar-{}.md", command.id));
            assert!(command.content.starts_with("---\n"));
            assert!(command.content.contains("description:"));
            assert!(command.content.contains("`ivar "));
            assert!(!command.content.contains("bifrost"));
            assert!(!command.content.contains("BIFROST_"));
        }
    }

    /// Every catalog legacy fingerprint is a real SHA-256 of the artifact it
    /// claims to recognise — a typo would make the digest match nothing and
    /// legacy cleanup would silently never fire.
    #[test]
    fn legacy_fingerprints_are_well_formed_hex_sha256() {
        for command in catalog() {
            assert_eq!(command.legacy_sha256.len(), 64, "{}", command.id);
            assert!(
                command
                    .legacy_sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{}: `{}` is not lowercase hex",
                command.id,
                command.legacy_sha256
            );
        }
    }

    /// The one checked-in legacy fixture: the exact bytes of the Bifrost-era
    /// `repo-list` command, whose digest must equal the catalog constant. This
    /// is what the reconciliation tests use as a real legacy artifact.
    const LEGACY_REPO_LIST: &str = "# Repo List\n\
        \n\
        List all repositories registered in the hall manifest, along with active sessions\n\
        and promoted repos.\n\
        \n\
        ## Usage\n\
        \n\
        ```bash\n\
        bifrost hall status\n\
        ```\n\
        \n\
        ## Output\n\
        \n\
        Shows all repos with their name, default branch, and URL. Also shows features,\n\
        sessions, lifecycle state, and promoted repos per feature.\n";

    #[test]
    fn the_legacy_fixture_digests_to_its_catalog_constant() {
        let command = catalog()
            .iter()
            .find(|c| c.id == "repo-list")
            .expect("repo-list is in the catalog");
        assert_eq!(hash::text(LEGACY_REPO_LIST), command.legacy_sha256);
    }

    #[test]
    fn the_legacy_fixture_writes_repo_list_md() {
        let command = catalog()
            .iter()
            .find(|c| c.id == "repo-list")
            .expect("repo-list is in the catalog");
        assert_eq!(command.legacy_file_name(), "repo-list.md");
        assert_eq!(command.file_name(), "ivar-repo-list.md");
    }

    // -- materialise ----------------------------------------------------------

    fn commands_dir() -> (tempfile::TempDir, Utf8PathBuf) {
        let (guard, root) = utf8_temp_dir();
        (guard, root.join("commands"))
    }

    fn change<'a>(changes: &'a [CommandChange], file_name: &str) -> &'a CommandChange {
        changes
            .iter()
            .find(|change| change.file_name == file_name)
            .unwrap_or_else(|| panic!("no `{file_name}` change in {changes:?}"))
    }

    fn inspection<'a>(inspections: &'a [Inspection], file_name: &str) -> &'a Inspection {
        inspections
            .iter()
            .find(|inspection| {
                inspection
                    .path
                    .file_name()
                    .is_some_and(|name| name == file_name)
            })
            .unwrap_or_else(|| panic!("no inspection for `{file_name}` in {inspections:?}"))
    }

    #[test]
    fn materialise_creates_repairs_and_then_becomes_idempotent() {
        let (_guard, dir) = commands_dir();

        let first = materialise(&dir).unwrap();
        assert_eq!(first.len(), 14);
        assert!(first.iter().all(|change| change.change == Change::Created));

        fs::write_text(&dir.join("ivar-plan.md"), "changed").unwrap();
        let repaired = materialise(&dir).unwrap();
        assert_eq!(change(&repaired, "ivar-plan.md").change, Change::Updated);
        assert_eq!(
            fs::read_text(&dir.join("ivar-plan.md")).unwrap().unwrap(),
            catalog()
                .iter()
                .find(|c| c.id == "plan")
                .unwrap()
                .content
        );

        let third = materialise(&dir).unwrap();
        assert_eq!(third.len(), 14);
        assert!(
            third.iter().all(|change| change.change == Change::Unchanged),
            "expected everything unchanged, got {third:?}"
        );
        assert_eq!(
            fs::read_text(&dir.join("ivar-plan.md")).unwrap().unwrap(),
            catalog()
                .iter()
                .find(|c| c.id == "plan")
                .unwrap()
                .content
        );
    }

    #[test]
    fn materialise_preserves_unrelated_user_commands() {
        let (_guard, dir) = commands_dir();
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(&dir.join("custom.md"), "my own command\n").unwrap();

        materialise(&dir).unwrap();

        assert_eq!(
            fs::read_text(&dir.join("custom.md")).unwrap().unwrap(),
            "my own command\n"
        );
    }

    #[test]
    fn materialise_removes_unknown_files_in_reserved_ivar_namespace() {
        let (_guard, dir) = commands_dir();
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(&dir.join("ivar-retired.md"), "old\n").unwrap();

        let changes = materialise(&dir).unwrap();

        assert!(!fs::exists(&dir.join("ivar-retired.md")).unwrap());
        let removed = change(&changes, "ivar-retired.md");
        assert_eq!(removed.change, Change::Removed);
    }

    #[test]
    fn remove_deletes_only_reserved_ivar_commands() {
        let (_guard, dir) = commands_dir();
        materialise(&dir).unwrap();
        fs::write_text(&dir.join("custom.md"), "my own command\n").unwrap();

        let changes = remove(&dir).unwrap();

        assert_eq!(changes.len(), 14);
        assert!(changes.iter().all(|change| change.change == Change::Removed));
        for command in catalog() {
            assert!(
                !fs::exists(&dir.join(command.file_name())).unwrap(),
                "{} should be gone",
                command.file_name()
            );
        }
        assert_eq!(
            fs::read_text(&dir.join("custom.md")).unwrap().unwrap(),
            "my own command\n"
        );
    }

    #[test]
    fn matching_legacy_command_is_removed() {
        let (_guard, dir) = commands_dir();
        fs::ensure_dir(&dir).unwrap();
        fs::write_text(&dir.join("repo-list.md"), LEGACY_REPO_LIST).unwrap();

        let changes = materialise(&dir).unwrap();

        assert!(!fs::exists(&dir.join("repo-list.md")).unwrap());
        let removed = change(&changes, "repo-list.md");
        assert_eq!(removed.change, Change::Removed);
        // The shipped command now sits in its place.
        assert!(fs::exists(&dir.join("ivar-repo-list.md")).unwrap());
    }

    #[test]
    fn modified_legacy_command_is_preserved_and_reported() {
        let (_guard, dir) = commands_dir();
        fs::ensure_dir(&dir).unwrap();
        let customized = format!("{LEGACY_REPO_LIST}x");
        fs::write_text(&dir.join("repo-list.md"), &customized).unwrap();

        let changes = materialise(&dir).unwrap();
        assert!(
            !changes.iter().any(|change| change.change == Change::Removed),
            "a modified legacy file must never be deleted: {changes:?}"
        );
        assert_eq!(
            fs::read_text(&dir.join("repo-list.md")).unwrap().unwrap(),
            customized,
            "the user's customized file must survive byte for byte"
        );

        let inspections = inspect(&dir, true).unwrap();
        assert_eq!(
            inspection(&inspections, "repo-list.md").integrity,
            Integrity::LegacyModified
        );
    }

    // -- inspect --------------------------------------------------------------

    #[test]
    fn inspect_sees_a_healthy_directory_as_current() {
        let (_guard, dir) = commands_dir();
        materialise(&dir).unwrap();

        let inspections = inspect(&dir, true).unwrap();

        assert_eq!(inspections.len(), 14);
        assert!(
            inspections
                .iter()
                .all(|inspection| inspection.integrity == Integrity::Current)
        );
    }

    #[test]
    fn inspect_reports_missing_and_modified_shipped_commands() {
        let (_guard, dir) = commands_dir();
        materialise(&dir).unwrap();
        fs::remove_file(&dir.join("ivar-plan.md")).unwrap();
        fs::write_text(&dir.join("ivar-sync.md"), "tampered\n").unwrap();

        let inspections = inspect(&dir, true).unwrap();

        let plan = inspections
            .iter()
            .find(|inspection| inspection.id == "plan")
            .unwrap();
        assert_eq!(plan.integrity, Integrity::Missing);
        let sync = inspections
            .iter()
            .find(|inspection| inspection.id == "sync")
            .unwrap();
        assert_eq!(sync.integrity, Integrity::Modified);
    }

    #[test]
    fn inspect_marks_leftover_files_stale_for_a_disabled_provider() {
        let (_guard, dir) = commands_dir();
        materialise(&dir).unwrap();

        let inspections = inspect(&dir, false).unwrap();

        assert_eq!(inspections.len(), 14);
        assert!(
            inspections
                .iter()
                .all(|inspection| inspection.integrity == Integrity::Stale),
            "a disabled provider's leftovers are all stale: {inspections:?}"
        );
    }
}
