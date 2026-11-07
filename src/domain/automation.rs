//! Automation decisions: what the human answered to a proposal to automate a
//! recurring request, and how an answer applies to a group.
//!
//! Pure. A decision holds the normalized texts of the group it was made on. A
//! group matches a decision when **any** of its texts is in that set, so a
//! decision survives the group gaining new texts. Where the decisions are kept
//! and how the groups are found is the store's business.

use std::collections::BTreeSet;
use std::fmt;

use camino::{Utf8Component, Utf8Path};

use crate::domain::session::{epoch_secs_from_rfc3339, rfc3339_from_epoch_secs};

/// How long a deferral lasts when `--until` is not given.
pub const DEFAULT_DEFERRAL_DAYS: u64 = 30;
/// The longest an intent name may be.
pub const MAX_INTENT_CHARS: usize = 100;

/// The human's answer to a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionKind {
    /// The automation exists; its group is no longer a candidate.
    Accepted,
    /// No automation, and do not ask again while the request keeps coming.
    Declined,
    /// Not now: do not ask again until a date.
    Deferred,
}

impl DecisionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Deferred => "deferred",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "accepted" => Some(Self::Accepted),
            "declined" => Some(Self::Declined),
            "deferred" => Some(Self::Deferred),
            _ => None,
        }
    }
}

impl fmt::Display for DecisionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What an accepted decision turned the request into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Form {
    Script,
    Skill,
    Agent,
    Procedure,
}

impl Form {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Procedure => "procedure",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "script" => Some(Self::Script),
            "skill" => Some(Self::Skill),
            "agent" => Some(Self::Agent),
            "procedure" => Some(Self::Procedure),
            _ => None,
        }
    }
}

impl fmt::Display for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stored decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub id: i64,
    pub kind: DecisionKind,
    pub intent: String,
    /// Set only for `accepted`.
    pub form: Option<Form>,
    /// Hall-relative path; set only for `accepted`.
    pub artifact: Option<String>,
    /// The last day of a deferral, `YYYY-MM-DD`; set only for `deferred`.
    pub until: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// The normalized texts of the group the decision covers.
    pub texts: BTreeSet<String>,
}

impl Decision {
    /// Whether the decision is in force on `today` (`YYYY-MM-DD`): a deferral
    /// past its `until` is not. `artifact_exists` is whether an accepted
    /// decision's artifact is still there; one whose artifact is gone is
    /// ignored, so its group is a candidate again.
    #[must_use]
    pub fn is_in_force(&self, today: &str, artifact_exists: bool) -> bool {
        match self.kind {
            DecisionKind::Accepted => artifact_exists,
            DecisionKind::Declined => true,
            DecisionKind::Deferred => self.until.as_deref().is_none_or(|until| until >= today),
        }
    }

    /// Whether a group with these distinct normalized `texts` is covered.
    #[must_use]
    pub fn matches<'a>(&self, texts: impl IntoIterator<Item = &'a str>) -> bool {
        texts.into_iter().any(|text| self.texts.contains(text))
    }
}

/// What the sweep does to the decisions.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepPlan {
    /// `(decision id, texts to add)`, only where there is something to add.
    pub extend: Vec<(i64, BTreeSet<String>)>,
    /// Decisions to delete.
    pub delete: Vec<i64>,
}

/// Whether any decision still depends on the groups of the window: only a
/// declined decision, or a deferral not yet due, reads them.
#[must_use]
pub fn needs_groups(decisions: &[Decision], today: &str) -> bool {
    decisions.iter().any(|decision| match decision.kind {
        DecisionKind::Accepted => false,
        DecisionKind::Declined => true,
        DecisionKind::Deferred => decision.is_in_force(today, true),
    })
}

/// What the sweep does with `decisions` on `today`, given `groups`, the
/// distinct normalized texts of each group of the 90-day window.
///
/// * a declined or deferred decision takes the texts of every group that
///   currently matches it;
/// * a declined decision that no group matches any more is deleted, since no
///   fingerprint of the window carries one of its texts;
/// * a deferral past its `until` is deleted;
/// * an accepted decision never expires.
#[must_use]
pub fn plan_sweep(decisions: &[Decision], groups: &[BTreeSet<String>], today: &str) -> SweepPlan {
    let mut plan = SweepPlan::default();
    for decision in decisions {
        match decision.kind {
            DecisionKind::Accepted => {}
            DecisionKind::Deferred if !decision.is_in_force(today, true) => {
                plan.delete.push(decision.id);
            }
            DecisionKind::Declined | DecisionKind::Deferred => {
                let mut matched = false;
                let mut added = BTreeSet::new();
                for group in groups
                    .iter()
                    .filter(|group| decision.matches(group.iter().map(String::as_str)))
                {
                    matched = true;
                    added.extend(group.difference(&decision.texts).cloned());
                }
                if !matched && decision.kind == DecisionKind::Declined {
                    plan.delete.push(decision.id);
                } else if !added.is_empty() {
                    plan.extend.push((decision.id, added));
                }
            }
        }
    }
    plan
}

/// The UTC date, `YYYY-MM-DD`, of `epoch_secs`.
#[must_use]
pub fn utc_day(epoch_secs: u64) -> String {
    rfc3339_from_epoch_secs(epoch_secs as f64)
        .chars()
        .take(10)
        .collect()
}

/// Whether `text` is a real calendar date written `YYYY-MM-DD`.
#[must_use]
pub fn is_day(text: &str) -> bool {
    let shaped = text.len() == 10
        && text.char_indices().all(|(at, c)| {
            if at == 4 || at == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    shaped
        && epoch_secs_from_rfc3339(&format!("{text}T00:00:00Z"))
            .and_then(|secs| u64::try_from(secs).ok())
            .is_some_and(|secs| utc_day(secs) == text)
}

/// Why an intent was refused.
#[must_use]
pub fn intent_problem(intent: &str) -> Option<&'static str> {
    if intent.trim().is_empty() {
        Some("the intent is empty")
    } else if intent.chars().count() > MAX_INTENT_CHARS {
        Some("the intent is longer than 100 characters")
    } else if intent.chars().any(char::is_control) {
        Some("the intent has a control character")
    } else {
        None
    }
}

/// The artifact as a clean hall-relative path with `/` separators, or why it
/// is refused: it must stay inside the hall.
///
/// # Errors
///
/// When the path is empty, absolute, or climbs out of the hall with `..`.
pub fn clean_artifact(artifact: &str) -> Result<String, &'static str> {
    let mut parts = Vec::new();
    for component in Utf8Path::new(artifact).components() {
        match component {
            Utf8Component::Normal(part) => parts.push(part),
            Utf8Component::CurDir => {}
            Utf8Component::ParentDir => return Err("the artifact path must not contain `..`"),
            Utf8Component::RootDir | Utf8Component::Prefix(_) => {
                return Err("the artifact path must be relative to the hall");
            }
        }
    }
    if parts.is_empty() {
        return Err("the artifact path is empty");
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
#[path = "../../tests/unit/domain/automation.rs"]
mod tests;
