//! The session write guard: determines which files a session may write.
//!
//! Provider-neutral: `decide` classifies tool requests as Allow or Deny.
//! Provider-specific adapters shape the output for Claude Code's hook
//! protocol or OpenCode's hook protocol.

use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::domain::feature::Feature;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;

/// The set of paths a session is allowed to write into: its view dir plus
/// the worktrees of promoted repos.
#[derive(Debug, Clone)]
pub(crate) struct WritableSet {
    view_dir: Utf8PathBuf,
    worktrees: Vec<Utf8PathBuf>,
}

impl WritableSet {
    /// Build the writable set from the session's view dir and the feature's
    /// promoted repos. Both the view dir and each worktree are canonicalised to
    /// prevent symlink escapes.
    pub(crate) fn from_session(
        layout: &Layout,
        feature: &Feature,
        view_dir: &Utf8Path,
    ) -> Result<Self, Failure> {
        let view_dir = view_dir
            .canonicalize_utf8()
            .map_err(|source| Failure::failed(
                "guard.unresolvable_view_dir",
                format!("could not canonicalise view dir `{view_dir}`: {source}"),
            ))?;
        let worktrees = feature
            .promotions
            .keys()
            .map(|repo| {
                let wt = layout.repo_worktree(repo, &feature.branch);
                wt.canonicalize_utf8().map_err(|source| Failure::failed(
                    "guard.unresolvable_worktree",
                    format!("could not canonicalise worktree `{wt}`: {source}"),
                ))
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        Ok(Self {
            view_dir,
            worktrees,
        })
    }

    /// Whether `path` is inside the view dir or one of the promoted worktrees.
    /// The input path is canonicalised (falling back to the raw path for
    /// not-yet-existing files) so symlinks cannot escape the set.
    pub(crate) fn allows(&self, path: &Utf8Path) -> bool {
        let canonical = path
            .canonicalize_utf8()
            .unwrap_or_else(|_| path.to_path_buf());
        canonical.starts_with(&self.view_dir)
            || self.worktrees.iter().any(|wt| canonical.starts_with(wt))
    }

    /// The view dir — the canonical root of this set.
    pub(crate) fn view_dir(&self) -> &Utf8Path {
        &self.view_dir
    }

    /// Build a `WritableSet` from explicit parts. Test-only.
    #[cfg(test)]
    pub(crate) fn from_parts(view_dir: Utf8PathBuf, worktrees: Vec<Utf8PathBuf>) -> Self {
        Self { view_dir, worktrees }
    }
}

/// A tool invocation the guard is asked to evaluate.
#[derive(Debug)]
pub(crate) struct ToolRequest {
    pub tool: String,
    pub file_path: Option<Utf8PathBuf>,
}

/// The guard's decision for a tool request.
#[derive(Debug)]
pub(crate) enum GuardDecision {
    Allow,
    Deny { reason: String },
}

/// Decide whether a tool request is allowed inside the session.
///
/// Write and Edit tools are checked against the writable set; everything
/// else is allowed.
pub(crate) fn decide(set: Option<&WritableSet>, req: &ToolRequest) -> GuardDecision {
    match req.tool.to_ascii_lowercase().as_str() {
        "write" | "edit" => match (set, &req.file_path) {
            (Some(set), Some(path)) if set.allows(path) => GuardDecision::Allow,
            (Some(set), _) => GuardDecision::Deny {
                reason: format!(
                    "writable set: {}",
                    std::iter::once(set.view_dir().to_string())
                        .chain(set.worktrees.iter().map(|w| w.to_string()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            (None, _) => GuardDecision::Deny {
                reason: "no ivar session resolves from the cwd".into(),
            },
        },
        _ => GuardDecision::Allow,
    }
}

// ---------------------------------------------------------------------------
// Provider-specific adapters
// ---------------------------------------------------------------------------

/// Claude Code hook input: `tool_name`, `tool_input.file_path`, `cwd`.
#[derive(Debug, Deserialize)]
struct ClaudeHookInput {
    tool_name: String,
    tool_input: serde_json::Value,
    cwd: Option<Utf8PathBuf>,
}

/// OpenCode hook input: `tool`, `args.filePath`, `cwd`.
#[derive(Debug, Deserialize)]
struct OpenCodeHookInput {
    tool: String,
    args: serde_json::Value,
    cwd: Option<Utf8PathBuf>,
}

/// The outcome of a guard evaluation: stdout body and whether the process
/// exits 0.
#[derive(Debug)]
pub struct GuardOutcome {
    pub body: String,
    pub exit_zero: bool,
}

/// Run the guard: parse stdin JSON, resolve the session, decide, and
/// shape the output for the given provider.
pub fn guard(provider: Provider, stdin_json: &str) -> Result<GuardOutcome, Failure> {
    let (tool_request, cwd) = match provider {
        Provider::ClaudeCode => {
            let input: ClaudeHookInput = serde_json::from_str(stdin_json)
                .map_err(|e| Failure::blocked("guard.parse", format!("invalid Claude hook JSON: {e}")))?;
            let req = ToolRequest {
                tool: input.tool_name,
                file_path: input
                    .tool_input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(Utf8PathBuf::from),
            };
            (req, input.cwd)
        }
        Provider::OpenCode => {
            let input: OpenCodeHookInput = serde_json::from_str(stdin_json)
                .map_err(|e| Failure::blocked("guard.parse", format!("invalid OpenCode hook JSON: {e}")))?;
            let req = ToolRequest {
                tool: input.tool,
                file_path: input
                    .args
                    .get("filePath")
                    .and_then(|v| v.as_str())
                    .map(Utf8PathBuf::from),
            };
            (req, input.cwd)
        }
    };

    let set = cwd
        .as_deref()
        .and_then(|cwd| crate::action::session::env::SessionEnv::resolve_by_cwd(cwd).ok())
        .flatten()
        .and_then(|env| resolve_writable_set(&env));

    let decision = decide(set.as_ref(), &tool_request);

    match provider {
        Provider::ClaudeCode => {
            let (perm, reason): (String, String) = match &decision {
                GuardDecision::Allow => ("allow".into(), String::new()),
                GuardDecision::Deny { reason } => ("deny".into(), reason.clone()),
            };
            let body = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": perm,
                    "permissionDecisionReason": reason,
                }
            });
            Ok(GuardOutcome {
                body: body.to_string(),
                exit_zero: true,
            })
        }
        Provider::OpenCode => match decision {
            GuardDecision::Allow => Ok(GuardOutcome {
                body: String::new(),
                exit_zero: true,
            }),
            GuardDecision::Deny { reason } => Ok(GuardOutcome {
                body: reason,
                exit_zero: false,
            }),
        },
    }
}

/// Try to build a `WritableSet` from a resolved session env.
fn resolve_writable_set(env: &crate::action::session::env::SessionEnv) -> Option<WritableSet> {
    let feature_name = env.feature.as_ref()?;
    let layout = Layout::discover(&env.view_dir).ok()??;
    let feature = Feature::read(&layout, feature_name).ok()??;
    WritableSet::from_session(&layout, &feature, &env.view_dir).ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/guard.rs"]
mod tests;
