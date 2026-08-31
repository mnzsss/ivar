//! `ivar discovery close <name> --outcome converted|abandoned` — end a
//! discovery.
//!
//! Two ways to end. **Converted** means the thinking became a feature;
//! `session convert` sets it (Task 10 calls this function). **Abandoned**
//! means it did not — and that doc is kept exactly as carefully as a
//! converted one. A dead end nobody recorded is a dead end the next person
//! walks into.
//!
//! Closing never deletes and never touches the body: only `status` and
//! `updated_at` change.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::action::Ctx;
use crate::domain::discovery::DiscoveryStatus;
use crate::domain::name::FeatureName;
use crate::domain::session::rfc3339_now;
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::fs;

use super::super::discover_hall;

/// What `ivar discovery close` needs.
#[derive(Debug, Clone)]
pub struct CloseInput {
    /// The unit of work's name.
    pub name: String,
    /// The terminal status: `Converted` or `Abandoned`.
    pub outcome: DiscoveryStatus,
}

/// What `ivar discovery close` did.
#[derive(Debug, Clone, Serialize)]
pub struct CloseOutcome {
    /// The doc's path — kept, never deleted.
    pub path: Utf8PathBuf,
    /// The status now recorded.
    pub status: DiscoveryStatus,
}

impl WriteHuman for CloseOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Closed discovery as `{}`. Doc kept: {}",
            self.status.as_str(),
            self.path
        )
    }
}

/// Close a discovery.
///
/// # Errors
///
/// When no hall is found, the name has no discovery doc, its front matter
/// is unreadable, or `outcome` is not a terminal status.
pub fn close(ctx: &Ctx, input: CloseInput) -> Outcome<CloseOutcome> {
    let layout = discover_hall(ctx)?;
    let name = FeatureName::new(input.name)?;

    if !matches!(
        input.outcome,
        DiscoveryStatus::Converted | DiscoveryStatus::Abandoned
    ) {
        return Err(Failure::blocked(
            "discovery.not_a_closure",
            format!(
                "`{}` is not a way to end a discovery",
                input.outcome.as_str()
            ),
        )
        .expected("`converted` or `abandoned`")
        .actual(format!("`{}`", input.outcome.as_str()))
        .fix(FixAction::safe(
            "discovery.choose_an_outcome",
            "Close with `--outcome converted` or `--outcome abandoned`.",
        )));
    }

    let mut doc = super::load(&layout, &name)?;
    super::ensure_writable(&doc, &name)?;

    doc.frontmatter.status = input.outcome;
    doc.frontmatter.updated_at = rfc3339_now();

    let path = layout.discovery_doc(&name);
    fs::write_text(&path, &crate::store::discovery::render(&doc)?)?;

    Ok(Report::new(CloseOutcome {
        path,
        status: input.outcome,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/discovery/close.rs"]
mod tests;
