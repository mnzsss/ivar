use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::graph::watch::scopes::Scope;
use crate::action::session::lookup;
use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetKind {
    Base,
    Layer { promotion_base: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub scope: Scope,
    pub worktree: Utf8PathBuf,
    pub git_meta: Vec<(Utf8PathBuf, Vec<String>)>,
    pub kind: TargetKind,
}

pub(crate) type Discover = Box<dyn FnMut(&Layout, &GraphDb) -> Vec<Target> + Send>;

/// Constructs base targets from declared repos in the manifest whose worktrees exist.
#[must_use]
pub(crate) fn base_targets(layout: &Layout, manifest: &Manifest) -> Vec<Target> {
    let mut targets = Vec::new();
    for repo in manifest.repos() {
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        if !worktree.as_std_path().exists() {
            continue;
        }

        let dot_git = worktree.join(".git");
        let gitdir = if dot_git.is_file() {
            let Ok(content) = std::fs::read_to_string(dot_git.as_std_path()) else {
                continue;
            };
            let Some(rest) = content.strip_prefix("gitdir: ") else {
                continue;
            };
            let trimmed = rest.trim();
            let p = Utf8Path::new(trimmed);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                worktree.join(p)
            }
        } else if dot_git.is_dir() {
            dot_git
        } else {
            continue;
        };

        let branch = repo.default_branch().as_str();
        let branch_path = Utf8Path::new(branch);
        let branch_parent = branch_path.parent();
        let branch_leaf = branch_path.file_name().unwrap_or(branch).to_owned();

        let branch_ref_dir = match branch_parent {
            Some(p) if !p.as_str().is_empty() => gitdir.join("refs/heads").join(p),
            _ => gitdir.join("refs/heads"),
        };

        let git_meta = vec![
            (
                gitdir.clone(),
                vec!["HEAD".to_owned(), "packed-refs".to_owned()],
            ),
            (branch_ref_dir, vec![branch_leaf]),
        ];

        targets.push(Target {
            scope: Scope::Base {
                repo: repo.name().as_str().to_owned(),
            },
            worktree,
            git_meta,
            kind: TargetKind::Base,
        });
    }
    targets
}

/// Returns the base commit for a repo's layer indexing, checking `db.get_repo_last_commit` first
/// and falling back to `promotion_base`.
#[must_use]
pub(crate) fn base_commit_for(
    db: &GraphDb,
    repo: &str,
    promotion_base: Option<&str>,
) -> Option<String> {
    db.get_repo_last_commit(repo)
        .ok()
        .flatten()
        .or_else(|| promotion_base.map(str::to_owned))
}

/// Constructs layer targets for all features with live sessions whose worktrees exist.
#[must_use]
pub(crate) fn layer_targets(layout: &Layout) -> Vec<Target> {
    let mut targets = Vec::new();
    let features: BTreeSet<FeatureName> = lookup::list_all(layout)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| s.feature)
        .collect();

    for name in features {
        let Ok(Some(feature)) = Feature::read(layout, &name) else {
            continue;
        };
        for (repo, promotion) in &feature.promotions {
            let worktree = layout.repo_worktree(repo, &feature.branch);
            if !worktree.as_std_path().exists() {
                continue;
            }
            let dot_git = worktree.join(".git");
            let gitdir = if dot_git.is_file() {
                let Ok(content) = std::fs::read_to_string(dot_git.as_std_path()) else {
                    continue;
                };
                let Some(rest) = content.strip_prefix("gitdir: ") else {
                    continue;
                };
                let trimmed = rest.trim();
                let p = Utf8Path::new(trimmed);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    worktree.join(p)
                }
            } else if dot_git.is_dir() {
                dot_git
            } else {
                continue;
            };

            let branch = feature.branch.as_str();
            let branch_path = Utf8Path::new(branch);
            let branch_parent = branch_path.parent();
            let branch_leaf = branch_path.file_name().unwrap_or(branch).to_owned();

            let branch_ref_dir = match branch_parent {
                Some(p) if !p.as_str().is_empty() => gitdir.join("refs/heads").join(p),
                _ => gitdir.join("refs/heads"),
            };

            let git_meta = vec![
                (
                    gitdir.clone(),
                    vec!["HEAD".to_owned(), "packed-refs".to_owned()],
                ),
                (branch_ref_dir, vec![branch_leaf]),
            ];

            targets.push(Target {
                scope: Scope::Layer {
                    feature: name.as_str().to_owned(),
                    repo: repo.as_str().to_owned(),
                },
                worktree,
                git_meta,
                kind: TargetKind::Layer {
                    promotion_base: promotion.base.as_ref().map(|b| b.as_str().to_owned()),
                },
            });
        }
    }
    targets
}
