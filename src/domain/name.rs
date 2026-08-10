//! Validated newtypes for the identifiers that become paths.
//!
//! # Why not `String`
//!
//! Two reasons, and the second is the load-bearing one.
//!
//! A `BranchName` in a signature cannot accidentally be a `RepoName`. That is
//! worth having and costs almost nothing.
//!
//! More importantly: **every one of these ends up in a path**, and this tool
//! creates directories and worktrees from them. `RepoName("../../etc")` is a
//! directory traversal in a program that runs `chmod` recursively and removes
//! trees. Validating at construction means the check happens once, at the edge,
//! instead of being remembered at each of the hundred places a name is joined
//! onto a path.
//!
//! The TypeScript predecessor solved this with `safeSingleSegmentSchema` and
//! `safeRelativePathSchema` in `lib/path-safety.ts`. Read those before writing
//! the equivalents — they encode cases learned in production. One case they
//! encode that is easy to miss from the rule list alone: `safeRelativePathSchema`
//! (what the predecessor used for a branch/default-branch value) rejects any
//! backslash, anywhere in the string, not only at the edges. `BranchName` here
//! carries that rule forward even though it also happens to be real
//! `git-check-ref-format` behaviour — the two reasons agree.
//!
//! # Contract
//!
//! Types: `HallName`, `RepoName`, `FeatureName`, `BranchName`, `SessionId`.
//!
//! Each has:
//!
//! - `new(impl Into<String>) -> Result<Self, InvalidName>` — the only constructor.
//!   There is no `from_unchecked`; if a caller needs one, the validation is wrong.
//! - `as_str(&self) -> &str`
//! - `Display`, `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Ord`
//! - `Serialize` transparently as a string, and `Deserialize` **through the
//!   validator** — a hand-edited `ivar.json` must not be able to smuggle
//!   `../` past the type. This is the part that is easy to get wrong: derive
//!   `Deserialize` and the validation is silently skipped.
//!
//! # Rules, per type
//!
//! `HallName`, `RepoName`, `FeatureName` are **single path
//! segments**: not empty (nor whitespace-only), no `/` or `\`, not `.` or `..`,
//! no leading or trailing whitespace, no control characters, no NUL. Reject a
//! leading `.` too — these become visible directories and a hidden one would be
//! invisible in every listing the user looks at.
//!
//! `BranchName` may contain `/` (`feat/auth-v2` is normal) but must satisfy git's
//! own rules: no leading or trailing `/`, no `//`, no leading `-`, no `..`
//! anywhere, must not end with `.`, none of `~^:?*[` or space or `\` anywhere,
//! not exactly `@`, and not containing `@{` anywhere. Two of git's rules are
//! **per slash-separated component**, not whole-string: no component may begin
//! with `.` (reuses `Hidden` — the same reason it exists for segment types: this
//! becomes a directory under `.ivar/repos/<repo>/<branch>/`, and a hidden one
//! would be invisible in every listing), and no component may end with `.lock`
//! (git's own worktree lock-file suffix). `fix/a.lockfile` is fine —
//! `.lockfile` is not `.lock`; `feat/x.lock/y` is not, because the middle
//! component ends with `.lock`. `BranchName` also rejects the same
//! emptiness/whitespace/control-character/NUL cases as the segment types.
//! Prior art hit a real libgit2 bug with slashes in worktree names — the sharp
//! edges here are known, not hypothetical.
//!
//! `SessionId` is a UUID string and is validated as one: anything
//! `uuid::Uuid::parse_str` refuses is refused here too.
//!
//! # Errors
//!
//! One `thiserror` enum, `InvalidName`, whose variants name the rule that was
//! broken — `Empty`, `NotASegment`, `Traversal`, `Hidden`, `ControlCharacter`,
//! `ReservedByGit`, … — never a single `Invalid(String)`. The variant is what
//! lets the `From<InvalidName> for Failure` conversion attach a fix action that
//! says what to do instead.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Failure, FixAction};

/// Why a name was refused. The variant names the rule that was broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidName {
    /// Empty, or made of nothing but whitespace.
    #[error("must not be empty")]
    Empty,
    /// Non-empty, but with whitespace at either edge.
    #[error("must not have leading or trailing whitespace")]
    Whitespace,
    /// Contains a `/` or `\` where a single path segment is required.
    #[error("must not contain '/' or '\\'")]
    NotASegment,
    /// Exactly `.` or `..`, or (for `BranchName`) contains `..` anywhere.
    #[error("must not be, or contain, '.' or '..'")]
    Traversal,
    /// Starts with `.`, which would make the resulting directory hidden.
    #[error("must not start with '.'")]
    Hidden,
    /// Contains an ASCII/Unicode control character, including NUL.
    #[error("must not contain control characters")]
    ControlCharacter,
    /// Starts or ends with `/`.
    #[error("must not start or end with '/'")]
    LeadingOrTrailingSlash,
    /// Contains `//`.
    #[error("must not contain '//'")]
    DoubleSlash,
    /// Starts with `-`, which a shell could mistake for a flag.
    #[error("must not start with '-'")]
    LeadingDash,
    /// A slash-separated component ends with `.lock`, git's own worktree
    /// lock-file suffix.
    #[error("must not have a '/'-separated component ending in '.lock'")]
    TrailingDotLock,
    /// Ends with `.`.
    #[error("must not end with '.'")]
    TrailingDot,
    /// Contains one of the characters git's ref format forbids: `~^:?*[` or a
    /// literal space.
    #[error("must not contain '{0}'")]
    ForbiddenCharacter(char),
    /// Exactly `@`, which git treats as shorthand for `HEAD`.
    #[error("must not be exactly '@', which git reserves for HEAD")]
    ReservedByGit,
    /// Contains `@{` anywhere, which git reserves for reflog/revision syntax
    /// (e.g. `@{upstream}`).
    #[error("must not contain '@{{', which git reserves for revision syntax")]
    RevisionSyntax,
    /// Not parseable as a UUID.
    #[error("must be a valid UUID")]
    NotUuid,
}

impl InvalidName {
    /// Stable, machine-matchable identifier for [`Failure::code`].
    const fn code(&self) -> &'static str {
        match self {
            Self::Empty => "name.empty",
            Self::Whitespace => "name.whitespace",
            Self::NotASegment => "name.not_a_segment",
            Self::Traversal => "name.traversal",
            Self::Hidden => "name.hidden",
            Self::ControlCharacter => "name.control_character",
            Self::LeadingOrTrailingSlash => "branch.leading_or_trailing_slash",
            Self::DoubleSlash => "branch.double_slash",
            Self::LeadingDash => "branch.leading_dash",
            Self::TrailingDotLock => "branch.trailing_dot_lock",
            Self::TrailingDot => "branch.trailing_dot",
            Self::ForbiddenCharacter(_) => "branch.forbidden_character",
            Self::ReservedByGit => "branch.reserved_by_git",
            Self::RevisionSyntax => "branch.revision_syntax",
            Self::NotUuid => "session.not_uuid",
        }
    }

    /// The specific way out. Every variant gets its own — a generic "fix your
    /// name" fix action would not tell an agent anything it does not already
    /// know from the error message.
    fn fix_action(&self) -> FixAction {
        match self {
            Self::Empty => FixAction::safe("name.non_empty", "Provide a non-empty name."),
            Self::Whitespace => FixAction::safe(
                "name.trim",
                "Remove the leading or trailing whitespace and try again.",
            ),
            Self::NotASegment => FixAction::safe(
                "name.single_segment",
                "Use a single path segment, with no '/' or '\\' in it.",
            ),
            Self::Traversal => FixAction::safe(
                "name.no_traversal",
                "Choose a name that is not, and does not contain, '.' or '..'.",
            ),
            Self::Hidden => FixAction::safe(
                "name.not_hidden",
                "Choose a name that does not start with '.'.",
            ),
            Self::ControlCharacter => FixAction::safe(
                "name.no_control_characters",
                "Remove control characters (including NUL) from the name.",
            ),
            Self::LeadingOrTrailingSlash => FixAction::safe(
                "branch.trim_slashes",
                "Remove the leading or trailing '/' from the branch name.",
            ),
            Self::DoubleSlash => FixAction::safe(
                "branch.collapse_slashes",
                "Collapse the repeated '/' into a single '/'.",
            ),
            Self::LeadingDash => FixAction::safe(
                "branch.no_leading_dash",
                "Choose a branch name that does not start with '-'.",
            ),
            Self::TrailingDotLock => FixAction::safe(
                "branch.no_dot_lock_suffix",
                "Choose a branch name where no '/'-separated component ends with '.lock'.",
            ),
            Self::TrailingDot => FixAction::safe(
                "branch.no_trailing_dot",
                "Choose a branch name that does not end with '.'.",
            ),
            Self::ForbiddenCharacter(char) => FixAction::safe(
                "branch.remove_forbidden_character",
                format!("Remove '{char}' from the branch name; git's ref format forbids it."),
            ),
            Self::ReservedByGit => FixAction::safe(
                "branch.not_bare_at",
                "Choose a branch name other than the reserved '@'.",
            ),
            Self::RevisionSyntax => FixAction::safe(
                "branch.remove_at_brace",
                "Remove the '@{' sequence from the branch name; git reserves it for revision syntax like '@{upstream}'.",
            ),
            Self::NotUuid => FixAction::safe(
                "session.valid_uuid",
                "Provide a valid UUID, e.g. one generated by `uuid::Uuid::new_v4`.",
            ),
        }
    }
}

impl From<InvalidName> for Failure {
    fn from(error: InvalidName) -> Self {
        Failure::blocked(error.code(), error.to_string()).fix(error.fix_action())
    }
}

/// True if `value` is empty or made of nothing but whitespace.
fn is_blank(value: &str) -> bool {
    value.chars().all(char::is_whitespace)
}

/// True if `value` has whitespace at either edge (and is not blank).
fn has_edge_whitespace(value: &str) -> bool {
    value.trim() != value
}

/// True if `value` contains a control character, including NUL.
fn has_control_character(value: &str) -> bool {
    value.chars().any(|c| c.is_control())
}

/// Shared prefix of every rule set: reject blank, edge-whitespace, and control
/// characters before any type-specific rule runs.
fn validate_common(value: &str) -> Result<(), InvalidName> {
    if is_blank(value) {
        return Err(InvalidName::Empty);
    }
    if has_edge_whitespace(value) {
        return Err(InvalidName::Whitespace);
    }
    if has_control_character(value) {
        return Err(InvalidName::ControlCharacter);
    }
    Ok(())
}

/// Rules for `HallName`, `RepoName`, `FeatureName`: exactly one
/// path segment.
fn validate_segment(value: &str) -> Result<(), InvalidName> {
    validate_common(value)?;
    if value == "." || value == ".." {
        return Err(InvalidName::Traversal);
    }
    if value.contains('/') || value.contains('\\') {
        return Err(InvalidName::NotASegment);
    }
    if value.starts_with('.') {
        return Err(InvalidName::Hidden);
    }
    Ok(())
}

/// Characters git's ref format forbids anywhere in a branch name, beyond the
/// structural rules checked separately.
const BRANCH_FORBIDDEN_CHARACTERS: [char; 8] = ['~', '^', ':', '?', '*', '[', ' ', '\\'];

/// Rules for `BranchName`: git's own `check-ref-format` rules, as named in the
/// module doc comment.
fn validate_branch(value: &str) -> Result<(), InvalidName> {
    validate_common(value)?;
    if value == "." || value == ".." || value.contains("..") {
        return Err(InvalidName::Traversal);
    }
    if value == "@" {
        return Err(InvalidName::ReservedByGit);
    }
    if value.contains("@{") {
        return Err(InvalidName::RevisionSyntax);
    }
    if value.starts_with('/') || value.ends_with('/') {
        return Err(InvalidName::LeadingOrTrailingSlash);
    }
    if value.contains("//") {
        return Err(InvalidName::DoubleSlash);
    }
    if value.starts_with('-') {
        return Err(InvalidName::LeadingDash);
    }
    if value.ends_with('.') {
        return Err(InvalidName::TrailingDot);
    }
    // Two of git's rules apply per slash-separated component, not to the
    // string as a whole: a component beginning with `.` would create a
    // hidden directory (worktree path `.ivar/repos/<repo>/<branch>/`), and a
    // component ending in `.lock` collides with git's own lock-file suffix
    // even when it is not the last component (`feat/x.lock/y`).
    for component in value.split('/') {
        if component.starts_with('.') {
            return Err(InvalidName::Hidden);
        }
        if component.ends_with(".lock") {
            return Err(InvalidName::TrailingDotLock);
        }
    }
    if let Some(char) = value
        .chars()
        .find(|c| BRANCH_FORBIDDEN_CHARACTERS.contains(c))
    {
        return Err(InvalidName::ForbiddenCharacter(char));
    }
    Ok(())
}

/// Rules for `SessionId`: must parse as a UUID. `uuid::Uuid::parse_str` already
/// rejects everything the segment rules would (empty, control characters,
/// traversal-shaped garbage, …), so there is nothing to add.
fn validate_session_id(value: &str) -> Result<(), InvalidName> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| InvalidName::NotUuid)
}

/// Defines one validated newtype: the struct, its constructor, the trait
/// impls the module doc comment promises, and a `Deserialize` that always
/// routes through `new`. Keeping this as one macro is what makes that last
/// part impossible to forget on any one type — the whole point of the module.
macro_rules! validated_name {
    ($name:ident, $validate:path) => {
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validates `value` against this type's rules. The only
            /// constructor — there is no unchecked path in or out.
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidName> {
                let value = value.into();
                $validate(&value)?;
                Ok(Self(value))
            }

            /// The validated value, borrowed.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            /// Routes through [`Self::new`]. This is the whole point of the
            /// module: a derived `Deserialize` would skip validation and let a
            /// hand-edited manifest smuggle a traversal straight past the type.
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let raw = String::deserialize(deserializer)?;
                Self::new(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

validated_name!(HallName, validate_segment);
validated_name!(RepoName, validate_segment);
validated_name!(FeatureName, validate_segment);
validated_name!(BranchName, validate_branch);
validated_name!(SessionId, validate_session_id);

#[cfg(test)]
#[path = "../../tests/unit/domain/name.rs"]
mod tests;
