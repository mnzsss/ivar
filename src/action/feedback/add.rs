use camino::Utf8PathBuf;
use serde::Serialize;
use std::io;

use crate::action::session::env::SessionEnv;
use crate::action::{Ctx, discover_hall};
use crate::domain::feedback::{FeedbackEntry, FeedbackKind, FeedbackStatus, Frontmatter};
use crate::domain::session::rfc3339_now;
use crate::error::{Outcome, Report, WriteHuman};
use crate::store::feedback as feedback_store;

const BUILD_VERSION: &str = env!("IVAR_BUILD_VERSION");

#[derive(Debug, Clone)]
pub struct AddInput {
    pub title: String,
    pub kind: FeedbackKind,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackEntryView {
    pub entry: FeedbackEntry,
    pub path: Utf8PathBuf,
}

impl WriteHuman for FeedbackEntryView {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Created feedback entry `{}` at {}",
            self.entry.id, self.path
        )
    }
}

pub fn add(ctx: &Ctx, input: AddInput) -> Outcome<FeedbackEntryView> {
    let layout = discover_hall(ctx)?;
    let session_env = SessionEnv::resolve_by_cwd(&ctx.cwd).ok().flatten();
    let title = input.title;
    let kind = input.kind;
    let body = input.body.unwrap_or_default();

    let created = feedback_store::create(&layout, &title, |id| FeedbackEntry {
        id: id.to_owned(),
        frontmatter: Frontmatter {
            title: title.clone(),
            kind,
            status: FeedbackStatus::Open,
            created_at: rfc3339_now(),
            ivar_version: BUILD_VERSION.to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            provider: session_env.as_ref().map(|e| e.provider.id().to_owned()),
            session: session_env.as_ref().map(|e| e.session_id.clone()),
            feature: session_env
                .as_ref()
                .and_then(|e| e.feature.as_ref().map(|f| f.as_str().to_owned())),
            published_url: None,
            extra: Default::default(),
        },
        body: body.clone(),
    })?;

    let path = layout.feedback_doc(&created.id);
    Ok(Report::new(FeedbackEntryView {
        entry: created,
        path,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feedback/add.rs"]
mod tests;
