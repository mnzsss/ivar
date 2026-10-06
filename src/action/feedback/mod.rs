//! User feedback tracking: bugs, proposals, and GitHub issue submissions.

use crate::domain::feedback::{FeedbackEntry, Redactions, issue_body, issue_title, redact};
use crate::store::layout::Layout;
use camino::Utf8Path;

pub const DEFAULT_REPO: &str = "mnzsss/ivar";
pub const URL_LIMIT: usize = 8000;

pub mod add;
pub mod list;
pub mod show;

pub(crate) fn redactions(layout: &Layout) -> Redactions {
    let home = std::env::var("HOME").ok();
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok();
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .ok()
        .or_else(|| {
            crate::infra::fs::read_text(Utf8Path::new("/etc/hostname"))
                .ok()
                .flatten()
                .map(|h| h.trim().to_owned())
        });

    Redactions {
        hall: Some(layout.root().as_str().to_owned()),
        home,
        user,
        host,
    }
}

pub(crate) fn issue_preview(entry: &FeedbackEntry, r: &Redactions) -> (String, String) {
    (
        redact(&issue_title(entry), r),
        redact(&issue_body(entry), r),
    )
}
