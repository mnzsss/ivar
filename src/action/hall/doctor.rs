//! `ivar doctor` — diagnose the hall and suggest fixes.

// ---------------------------------------------------------------------------
// `ivar doctor` — diagnose the hall and suggest fixes.
// ---------------------------------------------------------------------------

/// One diagnosed problem.
use std::io;

use serde::Serialize;

use crate::domain::provider::Provider;
use crate::error::{Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::harness::commands::{self, Inspection, Integrity};

use super::Ctx;
use super::{discover_hall, read_manifest};
use camino::Utf8PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    /// A stable code for the problem, e.g. `repo.bare_missing`.
    pub code: &'static str,
    /// What is wrong, in one sentence.
    pub what: String,
    /// The suggested fix.
    pub fix: String,
}

/// What `ivar doctor` found.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorOutcome {
    /// The hall root.
    pub root: Utf8PathBuf,
    /// Every diagnosed problem. Empty means a healthy hall.
    pub findings: Vec<Diagnosis>,
}

impl WriteHuman for DoctorOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.findings.is_empty() {
            writeln!(w, "No problems found in {}.", self.root)?;
            return Ok(());
        }
        writeln!(w, "Problems in {}:", self.root)?;
        for finding in &self.findings {
            writeln!(w, "  - {} — {}", finding.code, finding.what)?;
            writeln!(w, "    fix: {}", finding.fix)?;
        }
        Ok(())
    }
}

/// Diagnose the hall, read-only. `status` says *how healthy*; `doctor` says
/// *what is wrong and what to do about it*.
pub fn doctor(ctx: &Ctx) -> Outcome<DoctorOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;

    let mut findings = Vec::new();
    for repo in manifest.repos() {
        let bare = layout.repo_bare(repo.name());
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        let bare_state = git.target_state(&bare)?;

        match bare_state {
            TargetState::Repository => {}
            TargetState::Occupied => findings.push(Diagnosis {
                code: "repo.bare_occupied",
                what: format!("`{}` exists but is not a git repository", bare),
                fix: "Remove it and run `ivar sync` to clone afresh.".to_owned(),
            }),
            TargetState::Absent => findings.push(Diagnosis {
                code: "repo.bare_missing",
                what: format!("`{}` has not been cloned", repo.name()),
                fix: "Run `ivar sync` to clone it.".to_owned(),
            }),
        }

        // The worktree question only has an answer when the clone is there.
        if bare_state == TargetState::Repository {
            match git.target_state(&worktree)? {
                TargetState::Repository => {}
                TargetState::Occupied => findings.push(Diagnosis {
                    code: "repo.worktree_occupied",
                    what: format!("`{}` exists but is not a git worktree", worktree),
                    fix: "Remove it and run `ivar sync` to materialise it afresh.".to_owned(),
                }),
                TargetState::Absent => findings.push(Diagnosis {
                    code: "repo.worktree_missing",
                    what: format!("`{}` has no default-branch worktree", repo.name()),
                    fix: "Run `ivar sync` to materialise it.".to_owned(),
                }),
            }
        }
    }

    // Shipped workflow commands: a missing or modified official command is a
    // problem worth naming, but a *convenience* one — it never changes hall
    // health and never blocks session start. `ivar sync` is the repair.
    for provider in Provider::ALL {
        let enabled = manifest.providers().available().contains(&provider);
        let dir = layout.commands_dir(&provider);
        match commands::inspect(&dir, enabled) {
            Ok(inspections) => {
                for inspection in inspections {
                    if let Some(diagnosis) = command_diagnosis(provider, &inspection, enabled) {
                        findings.push(diagnosis);
                    }
                }
            }
            Err(error) => findings.push(Diagnosis {
                code: "provider.commands_inspect_failed",
                what: format!("could not inspect {provider}'s workflow commands: {error}"),
                fix: "Run `ivar sync` to reconcile them.".to_owned(),
            }),
        }
    }

    Ok(Report::new(DoctorOutcome {
        root: layout.root().to_path_buf(),
        findings,
    }))
}

/// One diagnosis for a command file that is not in its target state.
///
/// Every finding says `ivar sync` repairs it — it does — except
/// `legacy_command_modified`, where sync preserves the customized file by
/// design, so the way out is the user reviewing it and renaming or removing
/// it themselves.
fn command_diagnosis(
    provider: Provider,
    inspection: &Inspection,
    enabled: bool,
) -> Option<Diagnosis> {
    let file_name = inspection.path.file_name().unwrap_or("file");
    match inspection.integrity {
        Integrity::Current => None,
        Integrity::Missing => Some(Diagnosis {
            code: "provider.command_missing",
            what: format!(
                "{provider}'s `/ivar-{}` command is missing (`{file_name}`)",
                inspection.id
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        Integrity::Modified => Some(Diagnosis {
            code: "provider.command_modified",
            what: format!(
                "{provider}'s `/ivar-{}` command has been modified (`{file_name}`)",
                inspection.id
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        Integrity::LegacyModified => Some(Diagnosis {
            code: "provider.legacy_command_modified",
            what: format!(
                "{provider}'s legacy `{file_name}` command was customised and is preserved"
            ),
            fix: "Review it, then rename or remove it — `ivar sync` keeps it by design.".to_owned(),
        }),
        Integrity::Stale => Some(Diagnosis {
            code: "provider.command_stale",
            what: if enabled {
                format!("{provider}'s `{file_name}` is not an ivar-shipped command")
            } else {
                format!("{provider} is no longer listed, but its `{file_name}` command remains")
            },
            fix: "Run `ivar sync` to remove it.".to_owned(),
        }),
    }
}
