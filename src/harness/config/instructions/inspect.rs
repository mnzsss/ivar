use camino::Utf8Path;

use super::{
    Alias, CANONICAL_FILE, EntryKind, Error, Inspection, Integrity, entry_kind, io_error, locate,
    read,
};
use crate::infra::fs;

/// Report every root instruction entry's integrity in one pass: the canonical
/// file, then each alias. `block` is the expected managed-block bytes, used to
/// judge the canonical file's block current or stale.
///
/// # Errors
///
/// Returns [`Error`] if the canonical file or any alias cannot be read or
/// written.
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
