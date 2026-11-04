use super::*;

// Shipped workflow commands: a missing or modified official command is a
// problem worth naming, but a *convenience* one — it never changes hall
// health and never blocks session start. `ivar sync` is the repair.
pub(super) fn diagnose_provider_commands(manifest: &Manifest, layout: &Layout) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
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
    findings
}

// Shipped workflow skills: a missing or modified official skill is a
// problem worth naming, but a *convenience* one — it never changes hall
// health and never blocks session start. `ivar sync` is the repair.
pub(super) fn diagnose_provider_skills(manifest: &Manifest, layout: &Layout) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    for provider in Provider::ALL {
        let enabled = manifest.providers().available().contains(&provider);
        let dir = layout.skills_dir(&provider);
        match skills::inspect(&dir, enabled) {
            Ok(inspections) => {
                for inspection in inspections {
                    if let Some(diagnosis) = skill_diagnosis(provider, &inspection, enabled) {
                        findings.push(diagnosis);
                    }
                }
            }
            Err(error) => findings.push(Diagnosis {
                code: "provider.skills_inspect_failed",
                what: format!("could not inspect {provider}'s workflow skills: {error}"),
                fix: "Run `ivar sync` to reconcile them.".to_owned(),
            }),
        }
    }
    findings
}

// Root instruction topology: `HALL.md` and every provider alias. Each
// non-current state is one finding — every applicable one in a single
// run. `ivar sync` repairs the automatic cases; an enabled regular alias
// is preserved by design, so its fix is the human adoption checklist.
pub(super) fn diagnose_instructions(manifest: &Manifest, layout: &Layout) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    let aliases = crate::action::collect_instruction_aliases(layout, manifest);
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
    findings
}

/// One diagnosis for a command file that is not in its target state.
///
/// Every finding says `ivar sync` repairs it — it does — except
/// `legacy_command_modified`, where sync preserves the customized file by
/// design, so the way out is the user reviewing it and renaming or removing
/// it themselves.
fn command_diagnosis(
    provider: Provider,
    inspection: &CommandInspection,
    enabled: bool,
) -> Option<Diagnosis> {
    let file_name = inspection.path.file_name().unwrap_or("file");
    match inspection.integrity {
        CommandIntegrity::Current => None,
        CommandIntegrity::Missing => Some(Diagnosis {
            code: "provider.command_missing",
            what: format!(
                "{provider}'s `/ivar-{}` command is missing (`{file_name}`)",
                inspection.id
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        CommandIntegrity::Modified => Some(Diagnosis {
            code: "provider.command_modified",
            what: format!(
                "{provider}'s `/ivar-{}` command has been modified (`{file_name}`)",
                inspection.id
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        CommandIntegrity::LegacyModified => Some(Diagnosis {
            code: "provider.legacy_command_modified",
            what: format!(
                "{provider}'s legacy `{file_name}` command was customised and is preserved"
            ),
            fix: "Review it, then rename or remove it — `ivar sync` keeps it by design.".to_owned(),
        }),
        CommandIntegrity::Stale => Some(Diagnosis {
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

/// One diagnosis for a skill directory or file that is not in its target state.
fn skill_diagnosis(
    provider: Provider,
    inspection: &SkillInspection,
    enabled: bool,
) -> Option<Diagnosis> {
    let file_name = inspection.path.file_name().unwrap_or("file");
    match inspection.integrity {
        SkillIntegrity::Current => None,
        SkillIntegrity::Missing => Some(Diagnosis {
            code: "provider.skill_missing",
            what: format!(
                "{provider}'s `ivar-{}` skill is missing (`{}`)",
                inspection.id, inspection.path
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        SkillIntegrity::Modified => Some(Diagnosis {
            code: "provider.skill_modified",
            what: format!(
                "{provider}'s `ivar-{}` skill has been modified (`{}`)",
                inspection.id, inspection.path
            ),
            fix: "Run `ivar sync` to restore it.".to_owned(),
        }),
        SkillIntegrity::Stale => Some(Diagnosis {
            code: "provider.skill_stale",
            what: if enabled {
                format!("{provider}'s `{file_name}` is not an ivar-shipped skill")
            } else {
                format!(
                    "{provider} is no longer listed, but its `ivar-{}` skill remains",
                    inspection.id
                )
            },
            fix: "Run `ivar sync` to remove it.".to_owned(),
        }),
        SkillIntegrity::Obsolete => Some(Diagnosis {
            code: "provider.skill_obsolete",
            what: format!(
                "{provider}'s `{file_name}` in the reserved `ivar-*` skill namespace is not recognised"
            ),
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

/// Runs `t3` only for a hall that lists it. The probe path is never a T3 project, so a
/// reachable server answers `ProjectNotFoundError`; the stopped-server answer is not
/// measured and is treated as any other non-answer.
pub(super) fn diagnose_t3_host(manifest: &Manifest, layout: &Layout) -> Vec<Diagnosis> {
    if !manifest.providers().available().contains(&Provider::T3) {
        return Vec::new();
    }
    let never_a_project = layout.ivar_dir().join("doctor-probe");
    match providers::t3::host::probe(&never_a_project) {
        providers::t3::host::Probe::Ready => Vec::new(),
        providers::t3::host::Probe::Missing => vec![Diagnosis {
            code: "t3.binary_missing",
            what: "`t3` is listed as a provider but is not installed or does not run".to_owned(),
            fix: "Install T3 Code and put `t3` on PATH, or run `ivar provider remove t3`."
                .to_owned(),
        }],
        providers::t3::host::Probe::Outdated(found) => vec![Diagnosis {
            code: "t3.version_unsupported",
            what: providers::t3::host::unsupported_summary(&found),
            fix: providers::t3::host::VERSION_ADVICE.to_owned(),
        }],
        providers::t3::host::Probe::Unreachable(reason) => vec![Diagnosis {
            code: "t3.server_unreachable",
            what: format!("T3 Code did not answer: {reason}"),
            fix: "Start the T3 Code desktop app, then run `ivar doctor` again.".to_owned(),
        }],
    }
}

pub fn diagnose_mcp_auth(
    manifest: &Manifest,
    state_of: &dyn Fn(Provider, &str, &str) -> CredentialState,
) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    let hall = manifest.name().as_str();

    for server in manifest.mcp_servers() {
        if server.auth_requirement() != AuthRequirement::Required {
            continue;
        }
        let url = server.url.as_deref().unwrap_or("");
        let materialised = format!("{hall}-{}", server.name);

        for provider in manifest.providers().config_owners() {
            let state = state_of(provider, &materialised, url);
            if state == CredentialState::Missing {
                findings.push(Diagnosis {
                    code: "mcp.auth_missing",
                    what: format!(
                        "`{}` is not authenticated for {}",
                        server.name,
                        provider.id()
                    ),
                    fix: format!(
                        "Run `ivar mcp auth {} --provider {}`.",
                        server.name,
                        provider.id()
                    ),
                });
            }
        }
    }

    findings
}
