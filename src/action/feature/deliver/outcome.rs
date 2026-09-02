//! Outcome structs and human-readable output formatting for `ivar feature deliver`.

use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use crate::domain::feature::{
    DeliveryAction, DeliveryMode, DeliveryPreview, DraftAction, VerificationResult,
};
use crate::domain::name::RepoName;
use crate::error::WriteHuman;

/// One repo's push, in apply mode.
#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    /// The repo that was pushed (or not).
    pub repo: RepoName,
    /// Whether the push landed.
    pub ok: bool,
    /// Why it failed, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One repo's land result, in apply mode.
#[derive(Debug, Clone, Serialize)]
pub struct LandResult {
    /// The repo that landed.
    pub repo: RepoName,
    /// Whether the local default branch fast-forwarded to the feature tip.
    pub merged: bool,
    /// Whether pushing the default branch to remote succeeded.
    pub pushed: bool,
    /// Detail when a step was skipped or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One root repo's ordered checks, run in its worktree before the push.
#[derive(Debug, Clone, Serialize)]
pub struct RepoCheckResult {
    /// The repo whose checks ran.
    pub repo: RepoName,
    /// Whether every check passed.
    pub passed: bool,
    /// The ordered results, in execution order.
    pub results: Vec<VerificationResult>,
}

/// What `ivar feature deliver` produced.
///
/// One value for both modes, so `--json` and the human surface cannot drift:
/// preview mode returns the preview with an empty `pushes`; apply mode returns
/// the same preview (the state that was actually pushed) plus the per-repo
/// results.
#[derive(Debug, Clone, Serialize)]
pub struct DeliverOutcome {
    /// Root path of the hall.
    pub root: Utf8PathBuf,
    /// The preview summary, present for both preview and apply mode.
    pub preview: DeliveryPreview,
    /// Per-repo push results, present in apply mode for non-land delivery.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pushes: Vec<PushResult>,
    /// Per-repo land results, present in apply mode for land delivery.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub land: Vec<LandResult>,
    /// Verification checks run per root repo before delivery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<RepoCheckResult>,
}

pub(crate) fn action_word(action: DeliveryAction, draft: Option<DraftAction>) -> String {
    let action = match action {
        DeliveryAction::NewPr => "new pr",
        DeliveryAction::UpdatePr => "update pr",
        DeliveryAction::PushOnly => "push only",
        DeliveryAction::LandOnDefault => "land on default",
    };
    match draft {
        Some(DraftAction::CreateAsDraft) => format!("{action} (draft)"),
        Some(DraftAction::ConvertToDraft) => "convert pr to draft".to_owned(),
        None => action.to_owned(),
    }
}

impl WriteHuman for DeliverOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.pushes.is_empty() && self.land.is_empty() {
            match self.preview.mode {
                DeliveryMode::Push => {
                    writeln!(
                        w,
                        "Delivery preview for `{}` in {}:",
                        self.preview.feature, self.root
                    )?;
                    if self.preview.repos.is_empty() {
                        writeln!(w, "  no repos promoted")?;
                    }
                    for repo in &self.preview.repos {
                        writeln!(w, "  {}:", repo.repo)?;
                        writeln!(w, "    branch:  {}", repo.local_branch)?;
                        writeln!(w, "    remote:  {}", repo.remote)?;
                        writeln!(w, "    refspec: {}", repo.push_refspec)?;
                        writeln!(w, "    base:    {}", repo.base_branch)?;
                        if repo.draft == Some(DraftAction::ConvertToDraft) {
                            writeln!(w, "    action:  {}", action_word(repo.action, None))?;
                            writeln!(w, "    action:  {}", action_word(repo.action, repo.draft))?;
                        } else {
                            writeln!(w, "    action:  {}", action_word(repo.action, repo.draft))?;
                        }
                        if repo.blockers.is_empty() {
                            writeln!(w, "    blockers: none")?;
                        } else {
                            for blocker in &repo.blockers {
                                writeln!(w, "    blocker: {blocker}")?;
                            }
                        }
                    }
                }
                DeliveryMode::Land => {
                    writeln!(
                        w,
                        "Delivery preview (land on default) for `{}` in {}:",
                        self.preview.feature, self.root
                    )?;
                    if self.preview.repos.is_empty() {
                        writeln!(w, "  no repos promoted")?;
                    }
                    for repo in &self.preview.repos {
                        let target = repo.default_branch.as_ref().map_or("-", |b| b.as_str());
                        let ff_verdict = match repo.ff_possible {
                            Some(true) => "fast-forward",
                            Some(false) => "diverged",
                            None => "unknown",
                        };
                        writeln!(
                            w,
                            "  {}  {} -> {}  {}",
                            repo.repo, repo.local_branch, target, ff_verdict
                        )?;
                        for blocker in &repo.blockers {
                            writeln!(w, "    blocker: {blocker}")?;
                        }
                    }
                }
            }
            writeln!(w, "  plan gate:   {}", self.preview.plan_gate)?;
            writeln!(w, "  fingerprint: {}", self.preview.fingerprint)
        } else if !self.land.is_empty() {
            writeln!(
                w,
                "Landed `{}` in {} (fingerprint {}):",
                self.preview.feature, self.root, self.preview.fingerprint
            )?;
            for res in &self.land {
                if res.merged && res.pushed {
                    writeln!(w, "  {}: merged and pushed", res.repo)?;
                } else if res.merged {
                    if let Some(detail) = &res.detail {
                        writeln!(w, "  {}: merged, not pushed — {detail}", res.repo)?;
                    } else {
                        writeln!(w, "  {}: merged, not pushed", res.repo)?;
                    }
                } else if let Some(detail) = &res.detail {
                    writeln!(w, "  {}: not merged — {detail}", res.repo)?;
                } else {
                    writeln!(w, "  {}: not merged", res.repo)?;
                }
            }
            Ok(())
        } else {
            writeln!(
                w,
                "Delivered `{}` in {} (fingerprint {}):",
                self.preview.feature, self.root, self.preview.fingerprint
            )?;
            for push in &self.pushes {
                if push.ok {
                    writeln!(w, "  {}: pushed", push.repo)?;
                } else if let Some(detail) = &push.detail {
                    writeln!(w, "  {}: not pushed — {detail}", push.repo)?;
                } else {
                    writeln!(w, "  {}: not pushed", push.repo)?;
                }
            }
            Ok(())
        }
    }
}
