//! The hall's sync, read from git on the spot: is something stopping it from
//! reaching the remote? No network, no marker file; once the user fixes the
//! hall the finding is gone.

use crate::action::hall::SyncHealth;

use super::*;

pub(super) fn hall_sync_diagnoses(layout: &Layout, git: &impl Git) -> Vec<Diagnosis> {
    match crate::action::hall::sync_health(layout, git) {
        SyncHealth::Healthy => Vec::new(),
        SyncHealth::InProgress(operation) => vec![Diagnosis {
            code: "hall.sync_in_progress",
            what: format!("a {operation} is stopped in the hall, so its sync cannot run"),
            fix: format!(
                "In the hall, finish it with `git {operation} --continue` or drop it with `git {operation} --abort`, then run `ivar hall sync`."
            ),
        }],
        SyncHealth::Diverged {
            branch,
            upstream,
            ahead,
            behind,
        } => vec![Diagnosis {
            code: "hall.sync_diverged",
            what: format!(
                "`{branch}` diverged from `{upstream}` ({}, {})",
                counted(ahead, "local commit"),
                counted(behind, "remote commit"),
            ),
            fix: "In the hall, run `git pull --rebase`, resolve the conflicts and run `git push`."
                .to_owned(),
        }],
        SyncHealth::Unpushed {
            branch,
            upstream,
            ahead,
        } => vec![Diagnosis {
            code: "hall.sync_unpushed",
            what: format!(
                "`{branch}` has {} not pushed to `{upstream}`",
                counted(ahead, "local commit"),
            ),
            fix: "Run `ivar hall sync`, or `git push` in the hall.".to_owned(),
        }],
    }
}

fn counted(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}
