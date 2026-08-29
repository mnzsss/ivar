//! The shipped command catalog: every official workflow command, embedded at
//! compile time, paired with the legacy fingerprint of the command it
//! supersedes.
//!
//! The catalog is the *source*; reconciliation lives in
//! [`super`]'s `materialise` / `remove` / `inspect`. This file owns only the
//! declarative data and its accessors.

/// One shipped workflow command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShippedCommand {
    /// The command's id — the `<id>` in `ivar-<id>.md` and in `/ivar-<id>`.
    pub id: &'static str,
    /// The provider-neutral Markdown source, embedded at compile time.
    pub content: &'static str,
    /// SHA-256 of the legacy, unprefixed command file this id supersedes —
    /// the fingerprint that proves a Bifrost-era file is an official artifact
    /// safe to remove. `None` for a command with no legacy predecessor (its
    /// unprefixed file is never touched).
    pub legacy_sha256: Option<&'static str>,
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

/// The 16 official workflow commands, paired with the legacy fingerprint of
/// the command each one supersedes. The `legacy_sha256` values are the exact
/// SHA-256 digests of the Bifrost-era command files; do not change them
/// without regenerating the digest of the artifact they describe. A command
/// with no legacy predecessor carries `None`.
const COMMANDS: &[ShippedCommand] = &[
    ShippedCommand {
        id: "deliver",
        content: include_str!("../commands/deliver.md"),
        legacy_sha256: Some("b8402403fba034c85355def2f40ca9cec0e5572f4e67b130ebeac14ceda64c8b"),
    },
    ShippedCommand {
        id: "discovery",
        content: include_str!("../commands/discovery.md"),
        legacy_sha256: Some("97fba325393f6eba415a62bb6120d7bdc4cd813872e15d6f6669c910e32c0120"),
    },
    ShippedCommand {
        id: "execute",
        content: include_str!("../commands/execute.md"),
        legacy_sha256: Some("94c2aa9d9617de45cc5d985e752a99d4c6f5899654967d618542f270a5e18a72"),
    },
    ShippedCommand {
        id: "feature-cleanup",
        content: include_str!("../commands/feature-cleanup.md"),
        legacy_sha256: None,
    },
    ShippedCommand {
        id: "feature-create",
        content: include_str!("../commands/feature-create.md"),
        legacy_sha256: Some("062a359e6ecf9fa8313d65f478737ee0018ef1c4c17868e2dff3e7abbc3dfe16"),
    },
    ShippedCommand {
        id: "feature-status",
        content: include_str!("../commands/feature-status.md"),
        legacy_sha256: Some("67d092c2ecf3469a96c17fd8971dd6caa2e0ea97ca404361fea59617d681129c"),
    },
    ShippedCommand {
        id: "plan",
        content: include_str!("../commands/plan.md"),
        legacy_sha256: Some("5b1e361e11d342c022901a41f89de1a8b2463eb63c42e15d4e8fee9498fa188e"),
    },
    ShippedCommand {
        id: "promote",
        content: include_str!("../commands/promote.md"),
        legacy_sha256: Some("eae89c066ce3526b5e7cb3d4cd76f822faec9b3430965d4fdf83ae97e40c084f"),
    },
    ShippedCommand {
        id: "relations",
        content: include_str!("../commands/relations.md"),
        legacy_sha256: None,
    },
    ShippedCommand {
        id: "repo-list",
        content: include_str!("../commands/repo-list.md"),
        legacy_sha256: Some("cd8705d0e972c339ca55607c89e5cf4702123677e1a1c02ea4cf5502d105a8e1"),
    },
    ShippedCommand {
        id: "repo-setup",
        content: include_str!("../commands/repo-setup.md"),
        legacy_sha256: Some("255554048fcf58d7f6d396acc1713bc888d185e00794db47be2965a849bc4068"),
    },
    ShippedCommand {
        id: "review",
        content: include_str!("../commands/review.md"),
        legacy_sha256: Some("da6d0ad313c366246d0b15fac0e04340af65786486dfaed5f5128770537d4b2d"),
    },
    ShippedCommand {
        id: "session-connect",
        content: include_str!("../commands/session-connect.md"),
        legacy_sha256: Some("c81e99ac2bbfcea31381e61ead8e2a51cf91c46781e4466025ab11f23bee7b24"),
    },
    ShippedCommand {
        id: "session-start",
        content: include_str!("../commands/session-start.md"),
        legacy_sha256: Some("43affb5874c67b0aa2e904c7bca48499401f8d04667cbe6500add74d2c6508e4"),
    },
    ShippedCommand {
        id: "session-stop",
        content: include_str!("../commands/session-stop.md"),
        legacy_sha256: Some("2e2c6fc76618a19f77dec801dd59d52b6a5b6446f8f048943750534701aa4bbd"),
    },
    ShippedCommand {
        id: "sync",
        content: include_str!("../commands/sync.md"),
        legacy_sha256: Some("e663a6534823dcc7a0699e126d4e32619277e08ea48e657de8f74da0806bf15d"),
    },
];
