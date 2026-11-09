//! `ivar plan lint <feature>` — the mechanical checks on a feature's task
//! packets, run before a reviewer reads them.
//!
//! The rules live in [`crate::domain::plan_lint`]; this verb gathers what they
//! read — `plan.md`, `tasks/*.md`, and the declared repos' base views — and
//! reports each problem as a warning with its own code, so a clean run exits
//! `0` and a run with problems exits `1`.

use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::domain::name::{FeatureName, RepoName};
use crate::domain::plan_lint::{self, Finding, PacketSource, RepoFiles};
use crate::error::{Failure, FixAction, Outcome, Report, Warning, WriteHuman};
use crate::infra::fs;

use super::super::{discover_hall, read_manifest};
use crate::action::Ctx;

/// What `ivar plan lint` needs.
#[derive(Debug, Clone)]
pub struct LintInput {
    /// The feature whose task packets to check.
    pub feature: String,
}

/// What `ivar plan lint` checked. Each problem is a warning on the report.
#[derive(Debug, Clone, Serialize)]
pub struct LintOutcome {
    /// The hall root this ran against.
    pub root: Utf8PathBuf,
    /// The feature.
    pub feature: FeatureName,
    /// The packet files checked, in name order.
    pub packets: Vec<String>,
    /// How many problems were found.
    pub problems: usize,
}

impl WriteHuman for LintOutcome {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let count = self.packets.len();
        if self.problems == 0 {
            writeln!(
                w,
                "Linted {count} task packet(s) of `{}`: no problems",
                self.feature
            )
        } else {
            writeln!(
                w,
                "Linted {count} task packet(s) of `{}`: {} problem(s)",
                self.feature, self.problems
            )
        }
    }
}

/// The declared repos' base views, asked about a path a packet names.
struct BaseViews(Vec<(RepoName, Utf8PathBuf)>);

impl RepoFiles for BaseViews {
    fn len(&self, path: &str) -> Result<Option<u64>, Failure> {
        for (repo, root) in &self.0 {
            let rooted = path
                .strip_prefix(repo.as_str())
                .and_then(|rest| rest.strip_prefix('/'));
            for candidate in std::iter::once(path).chain(rooted) {
                let full = root.join(candidate);
                if let Some(meta) = fs::stat(&full)?
                    && meta.is_file()
                {
                    return Ok(Some(meta.len()));
                }
            }
        }
        Ok(None)
    }
}

/// Lint `input.feature`'s task packets.
///
/// Blocked when the feature has no `plan.md` or no packets under `tasks/`:
/// there is nothing to check, and a clean exit would say otherwise.
pub fn lint(ctx: &Ctx, input: LintInput) -> Outcome<LintOutcome> {
    let layout = discover_hall(ctx)?;
    let feature = FeatureName::new(input.feature)?;
    super::approve::require_feature(&layout, &feature)?;

    let dir = layout.plan_dir(&feature);
    let plan = fs::read_text(&dir.join("plan.md"))?.ok_or_else(|| {
        Failure::blocked("plan.artifact_missing", "`plan.md` does not exist")
            .expected("a plan whose task packets can be checked")
            .actual("`plan.md` is absent")
            .fix(FixAction::safe(
                "plan.create_first",
                format!("Scaffold the feature's plans with `ivar plan create {feature}`."),
            ))
    })?;
    let packets = read_packets(&dir.join("tasks"))?;
    if packets.is_empty() {
        return Err(Failure::blocked(
            "plan.no_packets",
            format!("`{feature}` has no task packets under `tasks/`"),
        )
        .expected("task packets at `tasks/NN-<semantic-task-name>.md`")
        .actual("`tasks/` is absent or holds no `.md` file")
        .fix(FixAction::safe(
            "plan.write_packets",
            "Generate the task packets, then lint again.",
        )));
    }

    let declared = super::plan_declared_repos(&layout, &feature)?;
    let repos: Vec<_> = read_manifest(&layout)?
        .repos()
        .iter()
        .filter(|repo| declared.contains(repo.name()))
        .cloned()
        .collect();
    let roots = super::base_view_roots(&layout, &feature, &repos)?;
    let views = BaseViews(roots);
    let files: Option<&dyn RepoFiles> = (!views.0.is_empty()).then_some(&views);

    let findings = plan_lint::lint(&plan, &packets, files)?;
    let outcome = LintOutcome {
        root: layout.root().to_path_buf(),
        feature,
        packets: packets.into_iter().map(|packet| packet.file).collect(),
        problems: findings.len(),
    };
    Ok(Report::with_warnings(
        outcome,
        findings.iter().map(warning).collect(),
    ))
}

fn warning(finding: &Finding) -> Warning {
    Warning::new(
        finding.code,
        finding.subject(),
        format!("{}: {}", finding.code, finding.what),
    )
}

/// `tasks/*.md` in name order.
fn read_packets(tasks: &Utf8Path) -> Result<Vec<PacketSource>, Failure> {
    if !fs::is_dir(tasks)? {
        return Ok(Vec::new());
    }
    let mut paths: Vec<Utf8PathBuf> = fs::read_dir(tasks)?
        .into_iter()
        .filter(|path| path.extension() == Some("md"))
        .collect();
    paths.sort();
    let mut packets = Vec::new();
    for path in paths {
        let (Some(file), Some(text)) = (path.file_name(), fs::read_text(&path)?) else {
            continue;
        };
        packets.push(PacketSource {
            file: file.to_owned(),
            text,
        });
    }
    Ok(packets)
}

#[cfg(test)]
#[path = "../../../tests/unit/action/plan/lint.rs"]
mod tests;
