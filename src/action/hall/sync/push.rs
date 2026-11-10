//! The push half of a sync: sending the branch upstream, and catching up with
//! a remote that moved on.

use crate::action::skill::sync::reconcile;
use crate::git::{Git, PushOutcome, Upstream};

use super::super::read_manifest;
use super::super::resolve::{Resolution, SKILLS_STATE, resolve_conflicts};
use super::{MAX_REBASE_STOPS, Run, detail};

impl<G: Git> Run<'_, G> {
    /// Push the branch when it is ahead of its upstream.
    pub(super) fn push(&mut self) {
        let state = match self.git.repo_state(self.root()) {
            Ok(state) => state,
            Err(error) => {
                self.warn(
                    "hall.push_failed",
                    format!("could not read the hall's git state: {}", detail(&error)),
                );
                return;
            }
        };
        if let Some(reason) = state.skip_reason() {
            self.skipped(reason);
            return;
        }
        let branch = state.branch.unwrap_or_default();
        let Some(upstream) = state.upstream else {
            self.warn(
                "hall.push_skipped",
                format!("branch `{branch}` has no upstream, so nothing was pushed"),
            );
            return;
        };
        if state.ahead == 0 {
            return;
        }
        match self.git.push_upstream(self.layout.root(), &upstream) {
            Ok(PushOutcome::Pushed) => self.out.pushed = true,
            Ok(PushOutcome::NonFastForward(_)) => self.catch_up_and_push(&upstream),
            Err(error) => self.push_failed(&upstream, &detail(&error)),
        }
    }

    /// The remote has commits the hall lacks: replay the hall's on top of them
    /// and push again, once.
    fn catch_up_and_push(&mut self, upstream: &Upstream) {
        // `git pull --rebase` refuses to start over tracked changes, and
        // stashing a user's work to get around that would put it at risk. The
        // changes are there because a hook refused them.
        match self.git.tracked_changes(self.layout.root()) {
            Ok(false) => {}
            Ok(true) => {
                self.push_failed(
                    upstream,
                    "the remote has commits the hall lacks, but tracked files have uncommitted \
                     changes (a commit hook may have refused them), so nothing was pulled",
                );
                return;
            }
            Err(error) => {
                self.push_failed(upstream, &detail(&error));
                return;
            }
        }

        let Some(skills_state_taken) = self.pull(upstream) else {
            return;
        };
        if skills_state_taken {
            self.regenerate_skill_state();
            if !self.commit_pending() {
                return;
            }
        }

        let ahead = self
            .git
            .repo_state(self.root())
            .map(|state| state.ahead)
            .unwrap_or_default();
        if ahead == 0 {
            return;
        }
        match self.git.push_upstream(self.layout.root(), upstream) {
            Ok(PushOutcome::Pushed) => self.out.pushed = true,
            Ok(PushOutcome::NonFastForward(output)) => self.push_failed(upstream, &output),
            Err(error) => self.push_failed(upstream, &detail(&error)),
        }
    }

    /// `git pull --rebase`, settling any conflicts. `Some(true)` when it
    /// finished having taken the remote's `state.json`, `Some(false)` when it
    /// finished without, `None` when it did not — a warning has been recorded
    /// and the hall is as it was before.
    fn pull(&mut self, upstream: &Upstream) -> Option<bool> {
        let Err(error) = self.git.pull_rebase(self.layout.root()) else {
            return Some(false);
        };
        if !self.git.is_rebase_in_progress(self.root()).unwrap_or(false) {
            self.push_failed(upstream, &detail(&error));
            return None;
        }
        self.settle_rebase(upstream)
    }

    /// Resolve each commit the rebase stops on until it finishes, or abort it.
    fn settle_rebase(&mut self, upstream: &Upstream) -> Option<bool> {
        let root = self.layout.root();
        let mut skills_state_taken = false;
        for _ in 0..MAX_REBASE_STOPS {
            let conflicts = match self.git.conflicts(root) {
                Ok(conflicts) if !conflicts.is_empty() => conflicts,
                Ok(_) => {
                    return self
                        .abandon(upstream, "the rebase stopped without a conflict to resolve");
                }
                Err(error) => return self.abandon(upstream, &detail(&error)),
            };
            match resolve_conflicts(self.git, root, &conflicts) {
                Ok(Resolution::Resolved) => {}
                Ok(Resolution::Unresolved(files)) => {
                    let files: Vec<&str> = files.iter().map(|file| file.as_str()).collect();
                    return self.abandon(
                        upstream,
                        &format!(
                            "the rebase conflicts in {}, which the fixed rules do not cover",
                            files.join(", ")
                        ),
                    );
                }
                Err(error) => return self.abandon(upstream, &detail(&error)),
            }
            skills_state_taken |= conflicts
                .iter()
                .any(|conflict| conflict.path.as_str() == SKILLS_STATE);

            // A commit whose every change was settled in the remote's favour is
            // empty now; `--continue` would refuse it, `--skip` drops it.
            let step = match self.git.staged_changes(root) {
                Ok(true) => self.git.continue_rebase(root),
                Ok(false) => self.git.skip_rebase(root),
                Err(error) => Err(error),
            };
            match step {
                Ok(()) => return Some(skills_state_taken),
                Err(error) => {
                    if !self.git.is_rebase_in_progress(root).unwrap_or(false) {
                        self.push_failed(upstream, &detail(&error));
                        return None;
                    }
                }
            }
        }
        self.abandon(upstream, "the rebase kept stopping on new conflicts")
    }

    /// Abort the rebase, which puts the hall back as it was before the pull,
    /// and record why.
    fn abandon(&mut self, upstream: &Upstream, why: &str) -> Option<bool> {
        match self.git.abort_rebase(self.layout.root()) {
            Ok(()) => self.push_failed(
                upstream,
                &format!("{why}; the rebase was aborted and the hall is as it was"),
            ),
            Err(error) => self.push_failed(
                upstream,
                &format!(
                    "{why}; and `git rebase --abort` failed, so the rebase is still stopped: {}",
                    detail(&error)
                ),
            ),
        }
        None
    }

    /// Run the skill sync's plan again, so the remote's `state.json` is
    /// rewritten from what is actually on disk.
    fn regenerate_skill_state(&mut self) {
        let result = read_manifest(self.layout).and_then(|manifest| {
            let mut warnings = Vec::new();
            reconcile(self.layout, &manifest, &mut warnings).map(|_| warnings)
        });
        match result {
            Ok(warnings) => self.out.warnings.extend(warnings),
            Err(failure) => self.warn(
                "hall.commit_failed",
                format!(
                    "could not regenerate the skill state after taking the remote's: {}",
                    failure.what
                ),
            ),
        }
    }

    fn push_failed(&mut self, upstream: &Upstream, why: &str) {
        self.warn(
            "hall.push_failed",
            format!(
                "could not push to `{}`; the commits stay local: {why}",
                upstream.name
            ),
        );
    }
}
