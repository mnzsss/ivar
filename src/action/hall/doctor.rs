//! `ivar doctor` — diagnose the hall and suggest fixes.

// ---------------------------------------------------------------------------
// `ivar doctor` — diagnose the hall and suggest fixes.
// ---------------------------------------------------------------------------

/// One diagnosed problem.
use std::io;

use serde::Serialize;

use crate::domain::feature::{RunReceipt, RunStatus};
use crate::domain::name::FeatureName;
use crate::domain::provider::Provider;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState};
use crate::harness::commands::{self, Inspection, Integrity};
use crate::harness::config::{build_block, instructions};
use crate::infra::fs;
use crate::store::layout::Layout;

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

    // Root instruction topology: `HALL.md` and every provider alias. Each
    // non-current state is one finding — every applicable one in a single
    // run. `ivar sync` repairs the automatic cases; an enabled regular alias
    // is preserved by design, so its fix is the human adoption checklist.
    let mut aliases: Vec<instructions::Alias> = Vec::new();
    for provider in Provider::ALL {
        let path = layout.instruction_alias(&provider);
        let enabled = manifest.providers().available().contains(&provider);
        match aliases.iter_mut().find(|alias| alias.path == path) {
            Some(existing) => {
                if !existing.owners.contains(&provider) {
                    existing.owners.push(provider);
                }
                existing.enabled |= enabled;
            }
            None => aliases.push(instructions::Alias {
                path,
                owners: vec![provider],
                enabled,
            }),
        }
    }
    let block = build_block(
        manifest.name(),
        &manifest
            .repos()
            .iter()
            .map(|repo| repo.name().clone())
            .collect::<Vec<_>>(),
    );
    match instructions::inspect(&layout.hall_instructions(), &block, &aliases) {
        Ok(inspections) => {
            for inspection in inspections {
                if let Some(diagnosis) = instruction_diagnosis(&inspection) {
                    findings.push(diagnosis);
                }
            }
        }
        Err(error) => findings.push(Diagnosis {
            code: "instructions.inspect_failed",
            what: format!("could not inspect the hall instructions: {error}"),
            fix: "Run `ivar sync` to reconcile them.".to_owned(),
        }),
    }

    // In-flight run receipts: `active` means a coordinator is attached and
    // work is in flight, so the coordinating session must be alive. When its
    // View Dir is gone the run is stranded — it still holds the feature's
    // single-run lock, so no competing run can start and nothing can finish
    // it. `blocked` and `diverged` deliberately wait on a human with no live
    // coordinator, so only `active` is orphan-checked.
    for (feature, receipt) in in_flight_receipts(&layout)? {
        let plan = receipt.plan_path.to_string();
        if !receipt.coordinators.iter().any(|entry| {
            fs::is_dir(&layout.feature_session(&feature, &entry.session)).unwrap_or(false)
        }) {
            findings.push(Diagnosis {
                code: "execute.run_orphaned",
                what: format!(
                    "run {} for feature `{feature}` is {} but its coordinating session is gone",
                    receipt.id, receipt.status
                ),
                fix: format!(
                    "Attach a feature session and resume it with \
                     `ivar feature execute start {feature} --plan {plan} --resume`, or abandon \
                     the run with `--restart`."
                ),
            });
        }
    }

    Ok(Report::new(DoctorOutcome {
        root: layout.root().to_path_buf(),
        findings,
    }))
}

/// Every feature's current receipt that is still in flight — non-terminal.
///
/// Iterates the hall's features directly rather than through the validated
/// parent tree: `doctor` should name a broken feature, not refuse to run
/// because one feature's parent reference is dangling. A receipt is skipped
/// when it is terminal or when reading it fails.
fn in_flight_receipts(layout: &Layout) -> Result<Vec<(FeatureName, RunReceipt)>, Failure> {
    let mut receipts = Vec::new();
    let features_dir = layout.features_dir();
    if fs::is_dir(&features_dir)? {
        for entry in fs::read_dir(&features_dir)? {
            let Some(name) = entry.file_name() else {
                continue;
            };
            let Ok(feature) = FeatureName::new(name.to_owned()) else {
                continue;
            };
            let Some(receipt) = RunReceipt::read(layout, &feature).ok().flatten() else {
                continue;
            };
            if receipt.status == RunStatus::Active {
                receipts.push((feature, receipt));
            }
        }
    }
    Ok(receipts)
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

/// One diagnosis for a root instruction entry that is not in its target
/// state.
///
/// Automatic cases all say `ivar sync` repairs them. An enabled provider's
/// regular alias is the exception — sync preserves it by design, so the way
/// out is the human adoption checklist. A disabled provider's leftover entry
/// is removed by sync, and the finding says so explicitly, including the
/// regular-file case.
fn instruction_diagnosis(inspection: &instructions::Inspection) -> Option<Diagnosis> {
    let name = inspection.path.file_name().unwrap_or("instructions");
    match inspection.integrity {
        instructions::Integrity::Current => None,
        instructions::Integrity::Missing => Some(Diagnosis {
            code: if name == instructions::CANONICAL_FILE {
                "instructions.canonical_missing"
            } else {
                "instructions.alias_missing"
            },
            what: format!("`{name}` is missing"),
            fix: "Run `ivar sync` to create it.".to_owned(),
        }),
        instructions::Integrity::NotRegular => Some(Diagnosis {
            code: "instructions.canonical_not_regular",
            what: format!("`{name}` exists but is not a regular file"),
            fix: "Replace it with a regular file, then run `ivar sync`.".to_owned(),
        }),
        instructions::Integrity::ManagedBlockMissing => Some(Diagnosis {
            code: "instructions.managed_block_missing",
            what: format!("`{name}` has no ivar-managed block"),
            fix: "Run `ivar sync` to add it.".to_owned(),
        }),
        instructions::Integrity::ManagedBlockStale => Some(Diagnosis {
            code: "instructions.managed_block_stale",
            what: format!("`{name}`'s managed block does not match ivar.json"),
            fix: "Run `ivar sync` to update it.".to_owned(),
        }),
        instructions::Integrity::AliasIsRegular => Some(Diagnosis {
            code: "instructions.alias_regular",
            what: format!("`{name}` is a regular file; ivar preserves it by design"),
            fix: "Consolidate its instructions into `HALL.md`, remove it, run `ivar sync`, and review the git diff."
                .to_owned(),
        }),
        instructions::Integrity::AliasBroken => Some(Diagnosis {
            code: "instructions.alias_broken",
            what: format!("`{name}` is a symlink whose target is missing"),
            fix: "Run `ivar sync` to point it at `HALL.md`.".to_owned(),
        }),
        instructions::Integrity::AliasWrongTarget => Some(Diagnosis {
            code: "instructions.alias_wrong_target",
            what: format!("`{name}` points somewhere other than `HALL.md`"),
            fix: "Run `ivar sync` to point it at `HALL.md`.".to_owned(),
        }),
        instructions::Integrity::DisabledAliasPresent => Some(Diagnosis {
            code: "instructions.disabled_alias_present",
            what: format!("`{name}` remains but its provider is no longer available"),
            fix: "Run `ivar sync` — it removes the entry, including a regular file.".to_owned(),
        }),
    }
}
