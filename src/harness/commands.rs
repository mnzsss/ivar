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
    &COMMANDS
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
    use crate::infra::hash;
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
}
