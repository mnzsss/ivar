//! `ivar review comment add|list|resolve` — line-range comments on a
//! feature's repos, stored at `features/<feature>/review/comments.json`.

use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::action::{Ctx, discover_hall};
use crate::domain::feature::Feature;
use crate::domain::name::{FeatureName, RepoName};
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::layout::Layout;
use crate::store::review::ReviewComments;
pub use crate::store::review::{CommentStatus, ReviewComment};

/// What `ivar review comment add` needs.
#[derive(Debug, Clone)]
pub struct AddInput {
    pub feature: String,
    pub repo: String,
    pub file: String,
    /// `<n>` or `<n>-<m>`, 1-based and inclusive.
    pub lines: String,
    pub body: String,
}

/// What `ivar review comment list` needs.
#[derive(Debug, Clone)]
pub struct ListInput {
    pub feature: String,
    pub repo: Option<String>,
    pub status: Option<CommentStatus>,
}

/// What `ivar review comment resolve` needs.
#[derive(Debug, Clone)]
pub struct ResolveInput {
    pub feature: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListOutcome {
    pub comments: Vec<ReviewComment>,
}

impl WriteHuman for ReviewComment {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let status = match self.status {
            CommentStatus::Open => "open",
            CommentStatus::Resolved => "resolved",
        };
        writeln!(
            w,
            "{}  {}:{}:{}-{}  {status}  {}",
            self.id, self.repo, self.file, self.line_start, self.line_end, self.body
        )
    }
}

impl WriteHuman for ListOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.comments.is_empty() {
            return writeln!(w, "No review comments.");
        }
        self.comments.iter().try_for_each(|c| c.write_human(w))
    }
}

pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<ReviewComment> {
    let (layout, name) = feature_in_hall(ctx, &input.feature)?;
    let (line_start, line_end) = parse_lines(&input.lines)?;
    let mut stored = ReviewComments::read(&layout, &name)?;
    let next = stored.next_id.max(1);
    let comment = ReviewComment {
        id: format!("c{next}"),
        repo: RepoName::new(input.repo)?,
        file: input.file,
        line_start,
        line_end,
        body: input.body,
        status: CommentStatus::Open,
        created_at: unix_now(),
        resolved_at: None,
    };
    stored.next_id = next + 1;
    stored.comments.push(comment.clone());
    stored.write(&layout, &name)?;
    Ok(Report::new(comment))
}

pub fn list(ctx: &Ctx, input: ListInput) -> Outcome<ListOutcome> {
    let (layout, name) = feature_in_hall(ctx, &input.feature)?;
    let comments = ReviewComments::read(&layout, &name)?
        .comments
        .into_iter()
        .filter(|c| input.repo.as_deref().is_none_or(|r| c.repo.as_str() == r))
        .filter(|c| input.status.is_none_or(|s| c.status == s))
        .collect();
    Ok(Report::new(ListOutcome { comments }))
}

pub fn resolve(ctx: &Ctx, input: ResolveInput) -> Outcome<ReviewComment> {
    let (layout, name) = feature_in_hall(ctx, &input.feature)?;
    let mut stored = ReviewComments::read(&layout, &name)?;
    let comment = stored
        .comments
        .iter_mut()
        .find(|c| c.id == input.id)
        .ok_or_else(|| {
            Failure::blocked(
                "review.comment_not_found",
                format!("no comment `{}` on feature `{name}`", input.id),
            )
        })?;
    comment.status = CommentStatus::Resolved;
    comment.resolved_at = Some(unix_now());
    let resolved = comment.clone();
    stored.write(&layout, &name)?;
    Ok(Report::new(resolved))
}

fn feature_in_hall(ctx: &Ctx, feature: &str) -> Result<(Layout, FeatureName), Failure> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(feature.to_owned())?;
    Feature::read(&layout, &name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{name}` does not exist"),
        )
    })?;
    Ok((layout, name))
}

fn parse_lines(lines: &str) -> Result<(u32, u32), Failure> {
    let invalid = || {
        Failure::blocked(
            "review.invalid_lines",
            format!("`{lines}` is not <n> or <n>-<m> with n <= m"),
        )
    };
    let (start, end) = match lines.split_once('-') {
        Some((a, b)) => (
            a.parse().map_err(|_| invalid())?,
            b.parse().map_err(|_| invalid())?,
        ),
        None => {
            let n = lines.parse().map_err(|_| invalid())?;
            (n, n)
        }
    };
    if start == 0 || start > end {
        return Err(invalid());
    }
    Ok((start, end))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[path = "../../../tests/unit/action/review/comment.rs"]
mod tests;
