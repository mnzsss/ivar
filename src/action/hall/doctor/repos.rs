use super::*;

pub(super) fn diagnose_repos(
    manifest: &Manifest,
    layout: &Layout,
    git: &impl Git,
) -> Result<Vec<Diagnosis>, Failure> {
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
    Ok(findings)
}
