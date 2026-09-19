use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

use super::RUN_CURRENT_VERSION;
use super::checkpoint::{CheckpointKind, CoordinatorEntry, RunCheckpoint, WaveProgress};
use super::coordinator::CoordinatorReport;
use super::evidence::{RunBaseline, RunDiff};
use super::id::RunId;
use super::legacy::LegacyEvidence;
use super::status::{RunOutcome, RunProvenance, RunStatus};
use crate::domain::name::{FeatureName, SessionId};
use crate::domain::provider::Provider;
use crate::error::{Failure, FixAction};

/// The durable record of one provider-coordinated execution of an approved
/// plan.
///
/// Every field is either authorisation (`plan_path`, `plan_fingerprint`),
/// identity (`id`, `feature`, `provenance`, `coordinators`), lifecycle
/// (`status`, `checkpoints`, timestamps), or evidence (`baseline`,
/// `final_diff`, `outcome`, `legacy`). There is no scheduling here and there
/// is no provider state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReceipt {
    /// The schema version — always [`RUN_CURRENT_VERSION`] for a value built
    /// here or read by `store::feature::run`.
    pub version: u32,
    /// This run's identity, stable across resume.
    pub id: RunId,
    /// The feature this run executes.
    pub feature: FeatureName,
    /// Where the receipt came from.
    pub provenance: RunProvenance,
    /// Where the run is in its lifecycle.
    pub status: RunStatus,
    /// The plan the run was authorised against, hall-relative as given.
    pub plan_path: Utf8PathBuf,
    /// SHA-256 of that plan's content at start — or at the last accepted
    /// revision.
    pub plan_fingerprint: String,
    /// When the run was created.
    pub started_at: String,
    /// When it last changed.
    pub updated_at: String,
    /// When it became terminal, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminated_at: Option<String>,
    /// Every coordinator that attached, in order. Never empty for a native
    /// run; empty for a legacy import.
    #[serde(default)]
    pub coordinators: Vec<CoordinatorEntry>,
    /// What the filesystem looked like at start.
    #[serde(default)]
    pub baseline: RunBaseline,
    /// Every lifecycle decision, in order.
    #[serde(default)]
    pub checkpoints: Vec<RunCheckpoint>,
    /// The evidence recorded at the terminal checkpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_diff: Option<RunDiff>,
    /// The outcome the coordinator reported, once accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RunOutcome>,
    /// What an imported board contributed, when `provenance` is
    /// `legacy-import`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyEvidence>,
}

impl RunReceipt {
    /// Create an active run.
    ///
    /// The caller supplies the id, the timestamp, and the baseline, because
    /// all three come from outside the domain — a uuid, a clock, and a git
    /// worktree respectively. That is what makes every test below a pure
    /// value comparison.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        id: RunId,
        feature: FeatureName,
        plan_path: impl Into<Utf8PathBuf>,
        plan_fingerprint: impl Into<String>,
        baseline: RunBaseline,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Self {
        let at = at.into();
        let fingerprint = plan_fingerprint.into();
        Self {
            version: RUN_CURRENT_VERSION,
            id,
            feature,
            provenance: RunProvenance::Native,
            status: RunStatus::Active,
            plan_path: plan_path.into(),
            plan_fingerprint: fingerprint.clone(),
            started_at: at.clone(),
            updated_at: at.clone(),
            terminated_at: None,
            coordinators: vec![CoordinatorEntry {
                session: session.clone(),
                provider,
                attached_at: at.clone(),
            }],
            baseline,
            checkpoints: vec![RunCheckpoint {
                at,
                kind: CheckpointKind::Started,
                status: RunStatus::Active,
                session: Some(session),
                provider: Some(provider),
                report: None,
                diff: None,
                plan_fingerprint_from: None,
                plan_fingerprint_to: Some(fingerprint),
                wave: None,
            }],
            final_diff: None,
            outcome: None,
            legacy: None,
        }
    }

    /// Whether this run holds the feature's single-run lock.
    #[must_use]
    pub const fn holds_lock(&self) -> bool {
        self.status.holds_lock()
    }

    /// The coordinator that attached most recently, if any.
    #[must_use]
    pub fn current_coordinator(&self) -> Option<&CoordinatorEntry> {
        self.coordinators.last()
    }

    /// Attach a coordinator to a non-terminal run and make it active again.
    ///
    /// Accepts `Active` as well as `Blocked` so a coordinator whose session
    /// died mid-run can re-attach without first having to terminalize a run
    /// that was never finished. `Diverged` is refused on purpose: the plan
    /// moved, and adopting the new revision is `accept_revision`'s explicit
    /// decision, not a side effect of resuming.
    ///
    /// The lineage entry is always appended, even for the same session and
    /// provider — a receipt should record that a coordinator re-attached,
    /// and collapsing repeats would lose exactly that.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition`] if the run is neither `Active` nor
    /// `Blocked`.
    pub fn resume(
        &mut self,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active, RunStatus::Blocked], "resume")?;
        let at = at.into();
        self.coordinators.push(CoordinatorEntry {
            session: session.clone(),
            provider,
            attached_at: at.clone(),
        });
        self.status = RunStatus::Active;
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Resumed,
            status: RunStatus::Active,
            session: Some(session),
            provider: Some(provider),
            report: None,
            diff: None,
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
            wave: None,
        });
        Ok(())
    }

    /// Record a blocked finish: the coordinator stopped on a question.
    ///
    /// Keeps the baseline, the run id, and the lock, because the run is not
    /// over. The report and diff land on a checkpoint rather than on
    /// `final_diff`, which only a terminal checkpoint fills.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition`] if the run is not `Active`.
    pub fn block(
        &mut self,
        report: CoordinatorReport,
        diff: RunDiff,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active], "block")?;
        let at = at.into();
        self.status = RunStatus::Blocked;
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Blocked,
            status: RunStatus::Blocked,
            session: Some(session),
            provider: Some(provider),
            report: Some(report),
            diff: Some(diff),
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
            wave: None,
        });
        Ok(())
    }

    /// Record that the approved plan changed under a run in flight.
    ///
    /// The submitted report is preserved — the coordinator's work is evidence
    /// whether or not its authorisation still holds — but no outcome is
    /// accepted and the pinned fingerprint is *not* rewritten. Both
    /// fingerprints go on the checkpoint so the divergence is legible after
    /// the fact.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition`] if the run is not `Active`.
    pub fn diverge(
        &mut self,
        observed_fingerprint: impl Into<String>,
        report: Option<CoordinatorReport>,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active], "diverge")?;
        let at = at.into();
        self.status = RunStatus::Diverged;
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Diverged,
            status: RunStatus::Diverged,
            session: Some(session),
            provider: Some(provider),
            report,
            diff: None,
            plan_fingerprint_from: Some(self.plan_fingerprint.clone()),
            plan_fingerprint_to: Some(observed_fingerprint.into()),
            wave: None,
        });
        Ok(())
    }

    /// Adopt a newly approved plan revision for a diverged run.
    ///
    /// Lands on `Blocked`, never straight on `Active`: attaching a
    /// coordinator is `start --resume`'s job, and collapsing the two would
    /// mean a revision could be accepted by a session that then never picks
    /// the work up.
    ///
    /// A fingerprint identical to the pinned one is refused rather than
    /// accepted as a no-op — the receipt says the plan diverged, so "nothing
    /// changed" means the caller is looking at a different file than finish
    /// was.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition`] if the run is not `Diverged`, or
    /// [`RunTransition::RevisionUnchanged`] if `new_fingerprint` matches the
    /// fingerprint already pinned.
    pub fn accept_revision(
        &mut self,
        new_fingerprint: impl Into<String>,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Diverged], "accept-revision")?;
        let new_fingerprint = new_fingerprint.into();
        if new_fingerprint == self.plan_fingerprint {
            return Err(RunTransition::RevisionUnchanged {
                fingerprint: new_fingerprint,
            });
        }
        let previous = std::mem::replace(&mut self.plan_fingerprint, new_fingerprint.clone());
        self.status = RunStatus::Blocked;
        self.push(RunCheckpoint {
            at: at.into(),
            kind: CheckpointKind::RevisionAccepted,
            status: RunStatus::Blocked,
            session: Some(session),
            provider: Some(provider),
            report: None,
            diff: None,
            plan_fingerprint_from: Some(previous),
            plan_fingerprint_to: Some(new_fingerprint),
            wave: None,
        });
        Ok(())
    }

    /// Terminalize the run with a reported outcome and its final evidence.
    ///
    /// Only `Succeeded` and `Failed` reach here — [`RunOutcome::Blocked`] is
    /// [`Self::block`], which is recoverable and therefore not a
    /// termination. Passing it is a caller bug, and is refused rather than
    /// quietly redirected.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition::BlockedIsNotTerminal`] if `outcome` is
    /// [`RunOutcome::Blocked`], or [`RunTransition`] if the run is not
    /// `Active`.
    pub fn terminate(
        &mut self,
        outcome: RunOutcome,
        report: CoordinatorReport,
        diff: RunDiff,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        if outcome == RunOutcome::Blocked {
            return Err(RunTransition::BlockedIsNotTerminal);
        }
        self.require(&[RunStatus::Active], "finish")?;
        let at = at.into();
        self.status = outcome.status();
        self.outcome = Some(outcome);
        self.final_diff = Some(diff.clone());
        self.terminated_at = Some(at.clone());
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Terminated,
            status: outcome.status(),
            session: Some(session),
            provider: Some(provider),
            report: Some(report),
            diff: Some(diff),
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
            wave: None,
        });
        Ok(())
    }

    /// Record an approved wave on an active run.
    ///
    /// Progress lives here rather than in `plan.md`, because any prose edit to
    /// the plan moves its fingerprint and diverges the run.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition`] if the run is not `Active`.
    pub fn checkpoint_wave(
        &mut self,
        number: u32,
        summary: impl Into<String>,
        session: SessionId,
        provider: Provider,
        at: impl Into<String>,
    ) -> Result<(), RunTransition> {
        self.require(&[RunStatus::Active], "checkpoint")?;
        self.push(RunCheckpoint {
            at: at.into(),
            kind: CheckpointKind::Wave,
            status: RunStatus::Active,
            session: Some(session),
            provider: Some(provider),
            report: None,
            diff: None,
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
            wave: Some(WaveProgress {
                number,
                summary: summary.into(),
            }),
        });
        Ok(())
    }

    /// Abandon a non-terminal run, preserving everything collected so far.
    ///
    /// What `start --restart` does before creating a fresh run, and what a
    /// non-terminal legacy board becomes on import. No outcome is set: the
    /// run reported none, and inventing one would be the dishonesty this
    /// state exists to avoid.
    ///
    /// # Errors
    ///
    /// Returns [`RunTransition`] if the run is already terminal.
    pub fn interrupt(&mut self, at: impl Into<String>) -> Result<(), RunTransition> {
        if self.status.is_terminal() {
            return Err(RunTransition::AlreadyTerminal {
                status: self.status,
                operation: "restart",
            });
        }
        let at = at.into();
        self.status = RunStatus::Interrupted;
        self.terminated_at = Some(at.clone());
        let session = self
            .current_coordinator()
            .map(|entry| entry.session.clone());
        let provider = self.current_coordinator().map(|entry| entry.provider);
        self.push(RunCheckpoint {
            at,
            kind: CheckpointKind::Interrupted,
            status: RunStatus::Interrupted,
            session,
            provider,
            report: None,
            diff: None,
            plan_fingerprint_from: None,
            plan_fingerprint_to: None,
            wave: None,
        });
        Ok(())
    }

    /// Build the receipt an imported execution board becomes.
    ///
    /// Always terminal: a board that completed keeps its outcome, and every
    /// other board — running, blocked, paused, never started — becomes
    /// `interrupted`. Nothing here claims the old workstreams can be
    /// continued, because the provider-native coordinator has no faithful
    /// mapping to their dependency, session, and write-contract state.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_legacy(
        id: RunId,
        feature: FeatureName,
        plan_path: impl Into<Utf8PathBuf>,
        status: RunStatus,
        outcome: Option<RunOutcome>,
        evidence: LegacyEvidence,
        at: impl Into<String>,
    ) -> Self {
        let at = at.into();
        let fingerprint = evidence.plan_fingerprint.clone().unwrap_or_default();
        Self {
            version: RUN_CURRENT_VERSION,
            id,
            feature,
            provenance: RunProvenance::LegacyImport,
            status,
            plan_path: plan_path.into(),
            plan_fingerprint: fingerprint,
            started_at: at.clone(),
            updated_at: at.clone(),
            terminated_at: Some(at.clone()),
            coordinators: Vec::new(),
            baseline: RunBaseline::empty(),
            checkpoints: vec![RunCheckpoint {
                at,
                kind: CheckpointKind::LegacyImport,
                status,
                session: None,
                provider: None,
                report: None,
                diff: None,
                plan_fingerprint_from: None,
                plan_fingerprint_to: None,
                wave: None,
            }],
            final_diff: None,
            outcome,
            legacy: Some(evidence),
        }
    }

    /// Refuse `operation` unless the current status is one of `allowed`.
    ///
    /// One helper rather than a guard per method: every transition asks the
    /// same question, and the two refusals it can produce ("already over" and
    /// "wrong state") are the two sentences a caller can act on.
    fn require(&self, allowed: &[RunStatus], operation: &'static str) -> Result<(), RunTransition> {
        if allowed.contains(&self.status) {
            return Ok(());
        }
        if self.status.is_terminal() {
            return Err(RunTransition::AlreadyTerminal {
                status: self.status,
                operation,
            });
        }
        Err(RunTransition::WrongState {
            status: self.status,
            operation,
        })
    }

    /// Append a checkpoint and move `updated_at` with it. The one place
    /// `checkpoints` grows, so the timestamp cannot fall behind the history.
    fn push(&mut self, checkpoint: RunCheckpoint) {
        self.updated_at.clone_from(&checkpoint.at);
        self.checkpoints.push(checkpoint);
    }
}

/// A transition the receipt's state machine refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunTransition {
    /// The run is over; nothing may change it.
    #[error("this run is already {status} — `{operation}` needs a run that is still in flight")]
    AlreadyTerminal {
        /// The terminal status the run holds.
        status: RunStatus,
        /// What was attempted.
        operation: &'static str,
    },

    /// The run is live but in the wrong state for this operation.
    #[error("this run is {status} — `{operation}` is not valid from there")]
    WrongState {
        /// The status the run holds.
        status: RunStatus,
        /// What was attempted.
        operation: &'static str,
    },

    /// `accept-revision` was handed the fingerprint already pinned.
    #[error("the plan fingerprint `{fingerprint}` is the one this run is already pinned to")]
    RevisionUnchanged {
        /// The fingerprint that did not change.
        fingerprint: String,
    },

    /// A blocked outcome was routed to the terminal path.
    #[error("a blocked outcome is recoverable and does not terminate a run")]
    BlockedIsNotTerminal,
}

impl From<RunTransition> for Failure {
    fn from(error: RunTransition) -> Self {
        let what = error.to_string();
        match error {
            RunTransition::AlreadyTerminal { .. } => Failure::blocked("execute.run_terminal", what)
                .fix(FixAction::safe(
                    "execute.start_new_run",
                    "Start a new run with `ivar feature execute start <feature> --plan <path>`.",
                )),
            RunTransition::WrongState { status, .. } => {
                Failure::blocked("execute.run_wrong_state", what).fix(match status {
                    RunStatus::Diverged => FixAction::safe(
                        "execute.accept_revision",
                        "Adopt the new plan revision with \
                         `ivar feature execute accept-revision <feature> --plan <path>`.",
                    ),
                    RunStatus::Blocked => FixAction::safe(
                        "execute.resume_run",
                        "Re-attach with \
                         `ivar feature execute start <feature> --plan <path> --resume`.",
                    ),
                    _ => FixAction::safe(
                        "execute.inspect_run",
                        "Inspect the run with `ivar feature execute status <feature>`.",
                    ),
                })
            }
            RunTransition::RevisionUnchanged { .. } => {
                Failure::blocked("execute.revision_unchanged", what).fix(FixAction::safe(
                    "execute.reapprove_plan",
                    "Re-approve the plan gate so the pinned fingerprint has something to move \
                     to, then run accept-revision again.",
                ))
            }
            RunTransition::BlockedIsNotTerminal => {
                Failure::blocked("execute.blocked_not_terminal", what).fix(FixAction::safe(
                    "execute.finish_outcome",
                    "Use `--outcome succeeded` or `--outcome failed` to end the run.",
                ))
            }
        }
    }
}
