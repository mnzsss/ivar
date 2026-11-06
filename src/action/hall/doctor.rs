//! `ivar doctor` — diagnose the hall and suggest fixes.

// ---------------------------------------------------------------------------
// `ivar doctor` — diagnose the hall and suggest fixes.
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, HashSet};
use std::io;

use serde::Serialize;

use crate::domain::feature::{Feature, RunReceipt, RunStatus, WorktreeState};
use crate::domain::mcp::{AuthRequirement, CredentialState};
use crate::domain::name::{FeatureName, RepoName};
use crate::domain::provider::Provider;
use crate::error::{Failure, Outcome, Report, WriteHuman};
use crate::git::{self, Git, TargetState, WorktreeEntry};
use crate::harness::commands::{
    self, Inspection as CommandInspection, Integrity as CommandIntegrity,
};
use crate::harness::config::{build_block, instructions};
use crate::harness::skills::{self, Inspection as SkillInspection, Integrity as SkillIntegrity};
use crate::infra::fs;
use crate::providers;
use crate::store::graph::db::GraphDb;
use crate::store::graph::schema::SCHEMA_VERSION;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

use super::Ctx;
use super::{discover_hall, read_manifest};
use crate::action::graph::reconcile::HallFacts;
use crate::action::session::base_view::{self, BaseView};
use camino::{Utf8Path, Utf8PathBuf};

mod database;
mod features;
mod graph;
mod harness;
pub(crate) mod mcp;
mod memory;
mod pages;
mod repos;
mod scripts;

pub(crate) use features::feature_worktrees;
use features::{
    check_legacy_working_docs, diagnose_feature_state, diagnose_orphan_worktrees,
    diagnose_orphaned_runs,
};
use graph::graph_diagnoses;
pub use harness::diagnose_mcp_auth;
use harness::{
    diagnose_instructions, diagnose_provider_commands, diagnose_provider_skills, diagnose_t3_host,
};
use repos::diagnose_repos;

/// One diagnosed problem.
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
    /// PID of the live graph watch leader, if one is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_watch_leader: Option<u32>,
}

impl WriteHuman for DoctorOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        if self.findings.is_empty() {
            writeln!(w, "No problems found in {}.", self.root)?;
            if let Some(pid) = self.graph_watch_leader {
                writeln!(w, "Graph watcher: leader pid {pid}.")?;
            }
            return Ok(());
        }
        writeln!(w, "Problems in {}:", self.root)?;
        for finding in &self.findings {
            writeln!(w, "  - {} — {}", finding.code, finding.what)?;
            writeln!(w, "    fix: {}", finding.fix)?;
        }
        if let Some(pid) = self.graph_watch_leader {
            writeln!(w, "Graph watcher: leader pid {pid}.")?;
        }
        Ok(())
    }
}

/// Diagnose the hall. `status` says *how healthy*; `doctor` says *what is
/// wrong and what to do about it*. The one write is moving a legacy graph
/// out of `memory.db`; every other check is read-only.
pub fn doctor(ctx: &Ctx) -> Outcome<DoctorOutcome> {
    let layout = discover_hall(ctx)?;
    let manifest = read_manifest(&layout)?;
    let git = git::System;
    let hall_is_git = git::inside_work_tree(layout.root());

    let mut findings = diagnose_repos(&manifest, &layout, &git)?;
    findings.extend(diagnose_provider_commands(&manifest, &layout));
    findings.extend(diagnose_provider_skills(&manifest, &layout));
    findings.extend(diagnose_instructions(&manifest, &layout, hall_is_git));
    findings.extend(database::database_diagnoses(&layout, &manifest));
    findings.extend(memory::memory_capture_diagnoses(&layout, &manifest));
    findings.extend(pages::page_diagnoses(&layout, hall_is_git));
    findings.extend(scripts::script_diagnoses(&layout));
    findings.extend(graph_diagnoses(&layout, &manifest, &git));
    findings.extend(diagnose_orphaned_runs(&layout)?);
    findings.extend(diagnose_orphan_worktrees(&layout, &manifest, &git));
    findings.extend(diagnose_feature_state(&layout, &git));
    findings.extend(diagnose_mcp_auth(&manifest, &providers::credential_state));
    findings.extend(mcp::mcp_name_diagnoses(&layout, &manifest));
    findings.extend(diagnose_t3_host(&manifest, &layout));
    let agent_files = crate::store::agent::list_files(&layout)?;
    let agent_specs: Vec<_> = agent_files
        .iter()
        .filter_map(|file| crate::store::agent::parse(file).ok())
        .collect();
    findings.extend(crate::action::agent::requirements::diagnose_agents_present(
        &agent_specs,
        !agent_files.is_empty(),
        &crate::action::agent::requirements::system_probes(),
    ));
    check_legacy_working_docs(&layout, &mut findings)?;

    let graph_watch_leader = crate::action::graph::watch::lease::leader_pid(&layout);
    Ok(Report::new(DoctorOutcome {
        root: layout.root().to_path_buf(),
        findings,
        graph_watch_leader,
    }))
}
