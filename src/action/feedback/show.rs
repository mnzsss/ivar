use serde::Serialize;
use std::io;

use super::{issue_preview, redactions};
use crate::action::{Ctx, discover_hall};
use crate::domain::feedback::FeedbackEntry;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::store::feedback as feedback_store;

#[derive(Debug, Clone)]
pub struct ShowInput {
    pub id: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackShow {
    pub entry: FeedbackEntry,
}

impl WriteHuman for FeedbackShow {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(w, "ID:     {}", self.entry.id)?;
        writeln!(w, "Title:  {}", self.entry.frontmatter.title)?;
        writeln!(w, "Kind:   {}", self.entry.frontmatter.kind.as_str())?;
        writeln!(w, "Status: {}", self.entry.frontmatter.status.as_str())?;
        if let Some(url) = &self.entry.frontmatter.published_url {
            writeln!(w, "URL:    {url}")?;
        }
        writeln!(w, "\n{}", self.entry.body)?;
        Ok(())
    }
}

pub fn show(ctx: &Ctx, input: ShowInput) -> Outcome<FeedbackShow> {
    let layout = discover_hall(ctx)?;
    let id = input.id;
    let entry = feedback_store::read(&layout, &id)?.ok_or_else(|| {
        Failure::failed(
            "feedback.not_found",
            format!("feedback entry `{id}` not found"),
        )
    })?;

    let final_entry = if input.redacted {
        let r = redactions(&layout);
        let (title, body) = issue_preview(&entry, &r);
        let mut f = entry.frontmatter;
        f.title = title;
        FeedbackEntry {
            id: entry.id,
            frontmatter: f,
            body,
        }
    } else {
        entry
    };

    Ok(Report::new(FeedbackShow { entry: final_entry }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feedback/show.rs"]
mod tests;
