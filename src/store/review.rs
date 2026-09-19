//! Review comments on disk: `features/<name>/review/comments.json`.

use serde::{Deserialize, Serialize};

use crate::domain::name::{FeatureName, RepoName};
use crate::error::Failure;
use crate::store::layout::Layout;
use crate::store::versioned::{Policy, Store};

const REVIEW_COMMENTS_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentStatus {
    Open,
    Resolved,
}

impl CommentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: String,
    pub repo: RepoName,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub body: String,
    pub status: CommentStatus,
    pub created_at: u64,
    pub resolved_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComments {
    pub next_id: u64,
    pub comments: Vec<ReviewComment>,
}

impl Default for ReviewComments {
    fn default() -> Self {
        Self {
            next_id: 1,
            comments: Vec::new(),
        }
    }
}

// ponytail: no file lock, last rename wins; add an flock if concurrent harnesses write comments.
fn review_store(layout: &Layout, name: &FeatureName) -> Store<ReviewComments> {
    Store::new(
        layout.review_comments_file(name),
        vec![],
        REVIEW_COMMENTS_VERSION,
        Policy::Local,
    )
}

impl ReviewComments {
    /// Read the feature's review comments; empty when none were ever written.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if the file exists but cannot be read.
    pub fn read(layout: &Layout, name: &FeatureName) -> Result<Self, Failure> {
        Ok(review_store(layout, name)
            .read()
            .map_err(Failure::from)?
            .unwrap_or_default())
    }

    /// Write the feature's review comments; the store creates parent directories.
    ///
    /// # Errors
    ///
    /// Returns [`Failure`] if the parent directory or the file cannot
    /// be created/written.
    pub fn write(&self, layout: &Layout, name: &FeatureName) -> Result<(), Failure> {
        review_store(layout, name)
            .write(self)
            .map_err(Failure::from)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/store/review.rs"]
mod tests;
