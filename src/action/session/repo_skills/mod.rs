//! Repo skills in session view dirs: find the skills a hall repo ships in
//! its provider skill dirs, decide the name each one gets in a session so a
//! hall or user skill is never shadowed, and project them into the session's
//! skills dir.

mod apply;
mod naming;
mod scan;
mod skill_md;

use camino::Utf8Path;

use crate::domain::provider::Provider;
use crate::error::{Failure, Warning};
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

pub(crate) use naming::plan;
pub(crate) use scan::{reserved_dirs, reserved_names, scan_repo};

pub(crate) fn materialise(
    layout: &Layout,
    manifest: &Manifest,
    provider: Provider,
    view_dir: &Utf8Path,
    home: Option<&Utf8Path>,
) -> Result<Vec<Warning>, Failure> {
    let mut warnings = Vec::new();
    let mut skills = Vec::new();
    for repo in manifest.repos() {
        let linked = view_dir.join(repo.name().as_str());
        // Only repos `view::materialise` actually linked (a missing worktree is skipped there).
        if !matches!(fs::read_symlink(&linked)?, fs::SymlinkTarget::Target(_)) {
            continue;
        }
        let scan = scan_repo(repo.name().as_str(), &linked);
        warnings.extend(scan.warnings);
        skills.extend(scan.skills);
    }
    let reserved = reserved_names(&reserved_dirs(layout.root(), home));
    let plan = plan(skills, &reserved);
    warnings.extend(plan.warnings.iter().cloned());
    warnings.extend(apply::apply(
        view_dir,
        &view_dir.join(provider.skills_dir()),
        &plan,
    )?);
    Ok(warnings)
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/session/repo_skills.rs"]
mod tests;
