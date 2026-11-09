use camino::Utf8PathBuf;

use crate::action::read_manifest;
use crate::domain::name::FeatureName;
use crate::domain::plan_commands::{CommandFinding, CommandKind, scan_shell_commands};
use crate::error::{Failure, FixAction};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Repo;

/// The manifest repos `plan.md` declares. Blocked when any declared name is
/// not in `ivar.json` — `session connect` would try to promote it.
pub(super) fn require_known_repos(
    layout: &Layout,
    feature: &FeatureName,
) -> Result<Vec<Repo>, Failure> {
    let manifest = read_manifest(layout)?;
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    for declared in crate::action::plan::plan_declared_repos(layout, feature)? {
        match manifest
            .repos()
            .iter()
            .find(|repo| repo.name() == &declared)
        {
            Some(repo) => known.push(repo.clone()),
            None => unknown.push(declared.to_string()),
        }
    }
    if unknown.is_empty() {
        return Ok(known);
    }
    Err(Failure::blocked(
        "plan.unknown_repo",
        format!(
            "`plan.md` for `{feature}` declares repos not in ivar.json: {}",
            unknown.join(", ")
        ),
    )
    .expected("every `repos:` entry to name a repo in ivar.json")
    .actual(format!("unknown: {}", unknown.join(", ")))
    .fix(FixAction::safe(
        "plan.fix_repos",
        format!(
            "Remove the unknown names from `repos:` in plan.md, or add them with `ivar repo add <name> <url>`, then run `ivar plan approve {feature} plan` again."
        ),
    )))
}

/// Blocked when a `cd` or `run` in `plan.md` or `tasks/*.md` cannot run in any
/// declared repo's base view — the worktree a feature session links the repo
/// at ([`base_view::resolve_on_disk`]), or the default worktree when the
/// feature has no record. Skipped when no such worktree is on disk: there is
/// nothing to resolve a path against.
pub(super) fn require_runnable_commands(
    layout: &Layout,
    feature: &FeatureName,
    repos: &[Repo],
) -> Result<(), Failure> {
    let roots: Vec<Utf8PathBuf> = crate::action::plan::base_view_roots(layout, feature, repos)?
        .into_iter()
        .map(|(_, root)| root)
        .collect();
    if roots.is_empty() {
        return Ok(());
    }

    let repo_names: Vec<String> = repos.iter().map(|repo| repo.name().to_string()).collect();
    let mut findings = Vec::new();
    for file in plan_command_files(layout, feature)? {
        let Some(source) = fs::read_text(&file)? else {
            continue;
        };
        let mut created: Vec<Utf8PathBuf> = Vec::new();
        let mut block = 0;
        for command in scan_shell_commands(&source, &repo_names) {
            if command.block != block {
                block = command.block;
                created.clear();
            }
            let dir = match &command.kind {
                CommandKind::MakeDir { dir } => {
                    created.push(dir.clone());
                    continue;
                }
                CommandKind::ChangeDir { dir } | CommandKind::RunScript { dir, .. } => dir,
            };
            if created.iter().any(|new_dir| dir.starts_with(new_dir)) {
                continue;
            }
            if let Some(reason) = command_problem(&command.kind, &roots)? {
                findings.push(CommandFinding {
                    file: file.clone(),
                    line: command.line,
                    command: command.command,
                    reason,
                });
            }
        }
    }
    if findings.is_empty() {
        return Ok(());
    }

    Err(Failure::blocked(
        "plan.invalid_commands",
        format!(
            "the plan for `{feature}` has {} command(s) that cannot run in its declared repos",
            findings.len()
        ),
    )
    .expected("every `cd` to reach a directory and every `run` to name a package.json script in a declared repo")
    .actual(
        findings
            .iter()
            .map(CommandFinding::render)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .fix(FixAction::safe(
        "plan.fix_commands",
        format!("Correct each listed command, then run `ivar plan approve {feature} plan` again."),
    )))
}

/// `plan.md`, then `tasks/*.md` in name order.
fn plan_command_files(layout: &Layout, feature: &FeatureName) -> Result<Vec<Utf8PathBuf>, Failure> {
    let plan_dir = layout.plan_dir(feature);
    let mut files = vec![plan_dir.join("plan.md")];
    let tasks = plan_dir.join("tasks");
    if fs::is_dir(&tasks)? {
        let mut packets: Vec<Utf8PathBuf> = fs::read_dir(&tasks)?
            .into_iter()
            .filter(|path| path.extension() == Some("md"))
            .collect();
        packets.sort();
        files.extend(packets);
    }
    Ok(files)
}

/// The reason `kind` cannot run, or `None` when it runs or there is not enough
/// on disk to tell.
fn command_problem(kind: &CommandKind, roots: &[Utf8PathBuf]) -> Result<Option<String>, Failure> {
    match kind {
        CommandKind::MakeDir { .. } => Ok(None),
        CommandKind::ChangeDir { dir } => {
            for root in roots {
                if fs::is_dir(&root.join(dir))? {
                    return Ok(None);
                }
            }
            Ok(Some(format!("`{dir}` does not exist in any declared repo")))
        }
        CommandKind::RunScript { dir, script } => {
            let mut saw_package = false;
            for root in roots {
                let Some(text) = fs::read_text(&root.join(dir).join("package.json"))? else {
                    continue;
                };
                let Ok(package) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                saw_package = true;
                if package
                    .get("scripts")
                    .and_then(|scripts| scripts.get(script))
                    .is_some()
                {
                    return Ok(None);
                }
            }
            Ok(saw_package
                .then(|| format!("no `{script}` script in `{}`", dir.join("package.json"))))
        }
    }
}
