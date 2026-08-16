//! The write contract of a workstream: the globs its operations may touch.
//!
//! Split out of `execution.rs` — [`WriteContract::allows`] and its glob
//! matcher have their own semantics (relative-matches-at-any-depth,
//! trailing-slash-as-prefix, `..` denial, single-`*` only) and touch no
//! board, workstream, status or journal. The execution board carries a
//! contract per [`WorkstreamDef`](super::WorkstreamDef); this module is the
//! whole of what deciding "is this path allowed" means, independent of the
//! board that stores the answer.
//!
//! Pure — no filesystem. Matching is done against an in-memory list of globs,
//! with `..` never allowed to escape the hall view dir.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

/// The write contract of a workstream: the globs its operations may touch.
///
/// Pure — no filesystem. Matching is done against an in-memory list of globs,
/// with `..` never allowed to escape the hall view dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteContract(Vec<String>);

impl WriteContract {
    /// Build a contract from the raw glob list.
    #[must_use]
    pub fn new(globs: Vec<String>) -> Self {
        Self(globs)
    }

    /// Whether `path` is allowed by the contract. The default is to deny:
    /// an empty contract allows nothing.
    ///
    /// A glob may be relative to the hall view dir (the common case, e.g.
    /// `src/`) or absolute. A relative glob matches `path` at any depth —
    /// `/hall/src/main.rs` and `src/main.rs` both match `src/` — because the
    /// workstream never knows where the hall lives.
    #[must_use]
    pub fn allows(&self, path: &Utf8Path) -> bool {
        let path_str = path.as_str();
        // `..` never escapes the hall view dir.
        if path_str.split('/').any(|seg| seg == "..") {
            return false;
        }
        self.0.iter().any(|glob| {
            let absolute = glob.starts_with('/');
            if let Some(prefix) = glob.strip_suffix('/') {
                // A trailing `/` matches the directory and everything under it.
                let prefix = prefix.to_owned();
                if absolute {
                    path_str == prefix
                        || path_str.starts_with(&prefix)
                            && path_str[prefix.len()..].starts_with('/')
                } else {
                    // Relative: match the prefix at any depth.
                    let needle_dir = format!("/{prefix}/");
                    path_str == prefix
                        || path_str.ends_with(&format!("/{prefix}"))
                        || path_str.contains(&needle_dir)
                        || path_str.starts_with(&format!("{prefix}/"))
                }
            } else if glob.contains('*') {
                if absolute {
                    glob_match(glob, path_str)
                } else {
                    // Try the glob against every suffix so a relative glob
                    // matches at any depth.
                    let mut slice = path_str;
                    loop {
                        if glob_match(glob, slice) {
                            return true;
                        }
                        match slice.find('/') {
                            Some(idx) => slice = &slice[idx + 1..],
                            None => return false,
                        }
                    }
                }
            } else if absolute {
                path_str == glob
                    || path_str.starts_with(glob) && path_str[glob.len()..].starts_with('/')
            } else {
                // A bare relative name matches a path that ends with it.
                path_str == glob
                    || path_str.ends_with(&format!("/{glob}"))
                    || path_str.ends_with(&format!("/{glob}/"))
            }
        })
    }
}

/// Whether `path` matches a simple glob: `*` matches any run of characters,
/// and a trailing `/` matches the directory and everything under it.
fn glob_match(glob: &str, path: &str) -> bool {
    let glob = glob.trim_end_matches('/');
    if glob.is_empty() {
        return false;
    }
    // Split on the first `*` and match the literal head/tail around it.
    let Some(star) = glob.find('*') else {
        return path == glob;
    };
    let head = &glob[..star];
    let tail = &glob[star + 1..];
    if !path.starts_with(head) {
        return false;
    }
    if tail.is_empty() {
        return true;
    }
    path[head.len()..].contains(tail)
}

#[cfg(test)]
#[path = "../../../tests/unit/domain/feature/write_contract.rs"]
mod tests;
