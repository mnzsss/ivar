//! The delivery milestones a feature records as Observations, and the fixed
//! rules that stamp their importance and bound their body.

use serde_json::{Value, json};

use crate::domain::feature::Feature;
use crate::domain::observation::{Importance, SourceEvent};

const MAX_LISTED: usize = 10;
const MAX_TEXT_CHARS: usize = 200;
const MAX_BODY_BYTES: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedBy {
    Cleanup,
    Integrate,
}

impl ObservedBy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cleanup => "cleanup",
            Self::Integrate => "integrate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryScope {
    pub feature: String,
    pub parent_feature: String,
    pub repo: String,
    pub branch: String,
    pub head_sha: String,
}

impl DeliveryScope {
    #[must_use]
    pub fn for_repo(feature: &Feature, repo: &str, head_sha: &str) -> Self {
        Self {
            feature: feature.name.to_string(),
            parent_feature: feature
                .parent
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            repo: repo.to_owned(),
            branch: feature.branch.to_string(),
            head_sha: head_sha.to_owned(),
        }
    }

    #[must_use]
    pub fn for_feature(feature: &Feature) -> Self {
        Self::for_repo(feature, "", "")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryEvent {
    Refused {
        code: String,
        blockers: Vec<String>,
        fingerprint: String,
    },
    Pushed {
        ok: bool,
        detail: Option<String>,
        base: String,
    },
    Pr {
        action: String,
        number: u64,
        url: String,
        draft: bool,
    },
    Landed {
        merged: bool,
        pushed: bool,
        detail: Option<String>,
    },
    Checks {
        passed: bool,
        failed: Vec<String>,
    },
    Rebased {
        conflicted: bool,
        onto: Option<String>,
        base_source: Option<String>,
        sha_before: Option<String>,
        sha_after: Option<String>,
    },
    Integrated {
        ok: bool,
        via: String,
        source_sha: String,
        merge_commit: Option<String>,
    },
    Merged {
        number: u64,
        url: String,
        merge_commit: Option<String>,
        merged_at: Option<String>,
        observed_by: ObservedBy,
    },
    Cleaned {
        record_path: String,
        docs: String,
    },
    Closed {
        outcome: String,
    },
    Deleted {
        repos: Vec<String>,
    },
}

fn by_success(ok: bool) -> Importance {
    if ok {
        Importance::delivery_routine()
    } else {
        Importance::delivery_failure()
    }
}

fn bounded(value: Value, max_chars: usize, max_items: usize) -> Value {
    match value {
        Value::String(text) => Value::String(text.chars().take(max_chars).collect()),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(max_items)
                .map(|item| bounded(item, max_chars, max_items))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, field)| (key, bounded(field, max_chars, max_items)))
                .collect(),
        ),
        other => other,
    }
}

fn within_byte_bound(body: &Value) -> String {
    let mut text = body.to_string();
    let (mut max_chars, mut max_items) = (MAX_TEXT_CHARS, MAX_LISTED);
    while text.len() > MAX_BODY_BYTES && max_chars > 0 {
        max_chars /= 2;
        text = bounded(body.clone(), max_chars, max_items).to_string();
    }
    while text.len() > MAX_BODY_BYTES && max_items > 0 {
        max_items /= 2;
        text = bounded(body.clone(), max_chars, max_items).to_string();
    }
    text
}

impl DeliveryEvent {
    #[must_use]
    pub const fn source_event(&self) -> SourceEvent {
        match self {
            Self::Refused { .. } => SourceEvent::DeliveryRefused,
            Self::Pushed { .. } => SourceEvent::DeliveryPushed,
            Self::Pr { .. } => SourceEvent::DeliveryPr,
            Self::Landed { .. } => SourceEvent::DeliveryLanded,
            Self::Checks { .. } => SourceEvent::DeliveryChecks,
            Self::Rebased { .. } => SourceEvent::DeliveryRebased,
            Self::Integrated { .. } => SourceEvent::DeliveryIntegrated,
            Self::Merged { .. } => SourceEvent::DeliveryMerged,
            Self::Cleaned { .. } => SourceEvent::DeliveryCleaned,
            Self::Closed { .. } => SourceEvent::DeliveryClosed,
            Self::Deleted { .. } => SourceEvent::DeliveryDeleted,
        }
    }

    #[must_use]
    pub fn importance(&self) -> Importance {
        match self {
            Self::Refused { .. } => Importance::delivery_failure(),
            Self::Pushed { ok, .. } | Self::Integrated { ok, .. } => by_success(*ok),
            Self::Checks { passed, .. } => by_success(*passed),
            Self::Landed { merged, pushed, .. } => by_success(*merged && *pushed),
            Self::Rebased { conflicted, .. } => by_success(!*conflicted),
            Self::Merged { .. } => Importance::delivery_merged(),
            Self::Pr { .. } | Self::Cleaned { .. } | Self::Closed { .. } | Self::Deleted { .. } => {
                Importance::delivery_routine()
            }
        }
    }

    #[must_use]
    pub fn body(&self, scope: &DeliveryScope) -> String {
        let mut body = json!({ "branch": scope.branch, "head_sha": scope.head_sha });
        if let (Value::Object(target), Value::Object(fields)) = (
            &mut body,
            bounded(self.fields(), MAX_TEXT_CHARS, MAX_LISTED),
        ) {
            target.extend(fields);
        }
        within_byte_bound(&body)
    }

    fn fields(&self) -> Value {
        match self {
            Self::Refused {
                code,
                blockers,
                fingerprint,
            } => json!({ "code": code, "blockers": blockers, "fingerprint": fingerprint }),
            Self::Pushed { ok, detail, base } => {
                json!({ "ok": ok, "detail": detail, "base": base })
            }
            Self::Pr {
                action,
                number,
                url,
                draft,
            } => json!({ "action": action, "number": number, "url": url, "draft": draft }),
            Self::Landed {
                merged,
                pushed,
                detail,
            } => json!({ "merged": merged, "pushed": pushed, "detail": detail }),
            Self::Checks { passed, failed } => json!({ "passed": passed, "failed": failed }),
            Self::Rebased {
                conflicted,
                onto,
                base_source,
                sha_before,
                sha_after,
            } => json!({
                "conflicted": conflicted,
                "onto": onto,
                "base_source": base_source,
                "sha_before": sha_before,
                "sha_after": sha_after,
            }),
            Self::Integrated {
                ok,
                via,
                source_sha,
                merge_commit,
            } => json!({
                "ok": ok,
                "via": via,
                "source_sha": source_sha,
                "merge_commit": merge_commit,
            }),
            Self::Merged {
                number,
                url,
                merge_commit,
                merged_at,
                observed_by,
            } => json!({
                "number": number,
                "url": url,
                "merge_commit": merge_commit,
                "merged_at": merged_at,
                "observed_by": observed_by.as_str(),
            }),
            Self::Cleaned { record_path, docs } => {
                json!({ "record_path": record_path, "docs": docs })
            }
            Self::Closed { outcome } => json!({ "outcome": outcome }),
            Self::Deleted { repos } => json!({ "repos": repos }),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain/delivery_event.rs"]
mod tests;
