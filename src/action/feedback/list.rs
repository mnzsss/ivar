use serde::Serialize;
use std::io;

use crate::action::{Ctx, discover_hall};
use crate::domain::feedback::{FeedbackEntry, FeedbackStatus};
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::feedback as feedback_store;

#[derive(Debug, Clone, Copy, Default)]
pub struct ListInput {
    pub status: Option<FeedbackStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackList {
    pub entries: Vec<FeedbackEntry>,
}

impl WriteHuman for FeedbackList {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.entries.is_empty() {
            writeln!(w, "No feedback entries found.")?;
            return Ok(());
        }
        for e in &self.entries {
            writeln!(
                w,
                "{:<20} [{}] {:<10} {}",
                e.id,
                e.frontmatter.kind.as_str(),
                e.frontmatter.status.as_str(),
                e.frontmatter.title
            )?;
        }
        Ok(())
    }
}

pub fn list(ctx: &Ctx, input: ListInput) -> Outcome<FeedbackList> {
    let layout = discover_hall(ctx)?;
    let all = feedback_store::list(&layout)?;
    let filtered = match input.status {
        Some(s) => all
            .into_iter()
            .filter(|e| e.frontmatter.status == s)
            .collect(),
        None => all,
    };

    Ok(Report::new(FeedbackList { entries: filtered }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feedback/list.rs"]
mod tests;
