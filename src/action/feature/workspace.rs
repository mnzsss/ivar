//! `ivar feature workspace <feature> [repos...]` — generate a multi-root
//! `.code-workspace` file opening promoted repos writable and context repos read-only.

use std::collections::BTreeMap;
use std::io;

use camino::Utf8PathBuf;
use serde::Serialize;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;
use crate::domain::feature::Feature;
use crate::domain::name::{BranchName, FeatureName, RepoName};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::{json, proc};

/// What `ivar feature workspace` needs.
#[derive(Debug, Clone)]
pub struct WorkspaceInput {
    /// The feature to generate a workspace for.
    pub feature: String,
    /// Which declared repos to include; includes all when omitted.
    pub repos: Vec<String>,
    /// Whether to open the generated workspace in VS Code once it is written.
    ///
    /// Decided by `bin/ivar.rs`, which is the layer that knows whether this
    /// run is human or `--json`: a machine-shaped run has no editor to open
    /// into. Every other caller leaves it `false`.
    pub open: bool,
}

/// A single folder included in the generated `.code-workspace`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceFolderOutcome {
    /// The repo name.
    pub repo: RepoName,
    /// The branch checked out in the folder.
    pub branch: BranchName,
    /// Absolute path to the repo worktree.
    pub path: Utf8PathBuf,
    /// Whether the folder is marked read-only.
    pub readonly: bool,
}

/// What became of the attempt to open the generated workspace.
///
/// Deliberately not serialized: `--json` is a machine surface and its shape is
/// read by other tools, so a human-only detail must not appear in it. It is
/// not a [`Warning`](crate::error::Warning) either — a warning makes the
/// process exit 1, and a workspace that was written but not opened is a
/// success with a sentence attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAttempt {
    /// No open was asked for.
    NotRequested,
    /// The editor was started.
    Opened,
    /// The editor could not be started, and why.
    Failed {
        /// The spawn failure, as a human reads it back.
        reason: String,
    },
}

/// What `ivar feature workspace` computed and wrote.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The path to the written `.code-workspace` file.
    pub path: Utf8PathBuf,
    /// The feature.
    pub feature: FeatureName,
    /// The folders included in the workspace, in manifest declaration order.
    pub folders: Vec<WorkspaceFolderOutcome>,
    /// What became of the attempt to open the workspace. Human-surface only.
    #[serde(skip)]
    pub open: OpenAttempt,
}

impl WriteHuman for WorkspaceOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        writeln!(
            w,
            "Wrote workspace for `{}` to {}:",
            self.feature, self.path
        )?;
        for folder in &self.folders {
            let access = if folder.readonly {
                "read-only"
            } else {
                "writable"
            };
            writeln!(w, "  {} ({}, {})", folder.repo, folder.branch, access)?;
        }
        match &self.open {
            OpenAttempt::NotRequested => {}
            OpenAttempt::Opened => writeln!(w, "Opening it in VS Code.")?,
            OpenAttempt::Failed { reason } => {
                writeln!(w, "Wrote the workspace, but could not open it: {reason}")?;
            }
        }
        Ok(())
    }
}

/// On-disk VSCode `.code-workspace` schema.
#[derive(Serialize)]
struct CodeWorkspaceDoc<'a> {
    folders: Vec<CodeWorkspaceFolder<'a>>,
    settings: CodeWorkspaceSettings,
}

#[derive(Serialize)]
struct CodeWorkspaceFolder<'a> {
    name: &'a str,
    path: &'a str,
}

#[derive(Serialize)]
struct CodeWorkspaceSettings {
    #[serde(
        rename = "files.readonlyInclude",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    readonly_include: BTreeMap<String, bool>,
}

/// Generate and write a `.code-workspace` for `input.feature`.
pub fn workspace(ctx: &Ctx, input: WorkspaceInput) -> Outcome<WorkspaceOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let feature_name = FeatureName::new(input.feature)?;

    let feature = Feature::read(&layout, &feature_name)?.ok_or_else(|| {
        Failure::blocked(
            "feature.not_found",
            format!("feature `{feature_name}` does not exist"),
        )
        .expected("an existing feature")
        .actual(format!("`{feature_name}` has no feature.json"))
        .fix(FixAction::safe(
            "feature.create_first",
            format!("Create it first with `ivar feature create {feature_name}`."),
        ))
    })?;

    // Parse and validate explicit repos if provided
    let filter_set: Option<Vec<RepoName>> = if input.repos.is_empty() {
        None
    } else {
        let mut parsed = Vec::new();
        for raw in &input.repos {
            let repo_name = RepoName::new(raw)?;
            if !manifest.repos().iter().any(|r| r.name() == &repo_name) {
                return Err(Failure::blocked(
                    "repo.not_in_manifest",
                    format!("`{repo_name}` is not in ivar.json"),
                )
                .expected("a repo declared in the manifest")
                .actual(format!("`{repo_name}` does not appear in `repos`"))
                .fix(FixAction::safe(
                    "repo.add_first",
                    format!("Add `{repo_name}` with `ivar repo add {repo_name} <url>` first."),
                )));
            }
            parsed.push(repo_name);
        }
        Some(parsed)
    };

    let mut folders_outcome = Vec::new();
    let mut workspace_folders = Vec::new();
    let mut readonly_include = BTreeMap::new();

    // Iterate in manifest declaration order (R-WS-FILTER, R-WS-DETERMINISTIC)
    for repo in manifest.repos() {
        if let Some(ref filter) = filter_set
            && !filter.contains(repo.name())
        {
            continue;
        }

        let is_promoted = feature.is_promoted(repo.name());
        let branch = if is_promoted {
            feature.branch.clone()
        } else {
            repo.default_branch().clone()
        };
        let worktree_path = layout.repo_worktree(repo.name(), &branch);

        if !is_promoted {
            readonly_include.insert(format!("{worktree_path}/**"), true);
        }

        folders_outcome.push(WorkspaceFolderOutcome {
            repo: repo.name().clone(),
            branch,
            path: worktree_path,
            readonly: !is_promoted,
        });
    }

    for f in &folders_outcome {
        workspace_folders.push(CodeWorkspaceFolder {
            name: f.repo.as_str(),
            path: f.path.as_str(),
        });
    }

    let doc = CodeWorkspaceDoc {
        folders: workspace_folders,
        settings: CodeWorkspaceSettings { readonly_include },
    };

    let workspace_path = layout.feature_workspace(&feature_name);
    json::write_canonical(&workspace_path, &doc)?;

    // The file is on disk before anything is spawned: opening is a
    // convenience, and a missing editor must never change what was written.
    let open = if input.open {
        let command = proc::Command::new("code")
            .arg(workspace_path.as_str())
            .cwd(layout.root());
        match proc::detach(&command) {
            Ok(()) => OpenAttempt::Opened,
            Err(error) => OpenAttempt::Failed {
                reason: error.to_string(),
            },
        }
    } else {
        OpenAttempt::NotRequested
    };

    Ok(Report::new(WorkspaceOutcome {
        root: layout.root().to_path_buf(),
        path: workspace_path,
        feature: feature_name,
        folders: folders_outcome,
        open,
    }))
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/workspace.rs"]
mod tests;
