use super::*;

// In-flight run receipts: `active` means a coordinator is attached and
// work is in flight, so the coordinating session must be alive. When its
// View Dir is gone the run is stranded — it still holds the feature's
// single-run lock, so no competing run can start and nothing can finish
// it. `blocked` and `diverged` deliberately wait on a human with no live
// coordinator, so only `active` is orphan-checked.
pub(super) fn diagnose_orphaned_runs(layout: &Layout) -> Result<Vec<Diagnosis>, Failure> {
    let mut findings = Vec::new();
    for (feature, receipt) in in_flight_receipts(layout)? {
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
    Ok(findings)
}

pub(super) fn diagnose_orphan_worktrees(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
) -> Vec<Diagnosis> {
    let owned = match feature_worktrees(layout, manifest) {
        Ok(owned) => owned,
        // Incomplete ownership data would name live feature worktrees as orphans.
        Err(unreadable) => return unreadable,
    };
    let mut findings = Vec::new();
    for repo in manifest.repos() {
        let bare = layout.repo_bare(repo.name());
        if !bare.is_dir() {
            continue;
        }
        let entries = match git.list_worktrees(&bare) {
            Ok(entries) => entries,
            Err(error) => {
                findings.push(Diagnosis {
                    code: "repo.worktree_list_failed",
                    what: format!("could not list the worktrees of `{}`: {error}", repo.name()),
                    fix: format!("Run `git --git-dir {bare} worktree list` to see why."),
                });
                continue;
            }
        };
        findings.extend(diagnose_unregistered_checkouts(
            layout,
            repo.name(),
            &entries,
        ));
        for entry in entries {
            if let Some(owner) = integration_staging_owner(layout, repo.name(), &entry.path) {
                if integration_running(layout, &owner) {
                    continue;
                }
                findings.push(Diagnosis {
                    code: "integrate.staging_stale",
                    what: format!(
                        "`{}` is integration staging for `{}` left behind by an integrate that \
                         did not finish",
                        entry.path,
                        repo.name()
                    ),
                    fix: format!(
                        "Remove it with `git --git-dir {bare} worktree remove --force {}`.",
                        entry.path
                    ),
                });
                continue;
            }
            let branch = match (&entry.branch, entry.detached) {
                (Some(branch), _) => {
                    if branch == repo.default_branch().as_str()
                        || owned.contains(&(repo.name().clone(), branch.clone()))
                    {
                        continue;
                    }
                    branch.as_str()
                }
                (None, true) => "detached",
                (None, false) => continue,
            };
            findings.push(orphan_diagnosis(
                git,
                &bare,
                repo.name().as_str(),
                &entry,
                branch,
            ));
        }
    }
    findings
}

fn orphan_diagnosis(
    git: &impl Git,
    bare: &Utf8Path,
    repo: &str,
    entry: &WorktreeEntry,
    branch: &str,
) -> Diagnosis {
    let path = &entry.path;
    if entry.prunable {
        return Diagnosis {
            code: "repo.worktree_orphaned",
            what: format!(
                "`{path}` in `{repo}` is on branch `{branch}`, which no feature owns — its directory is gone"
            ),
            fix: "Run `ivar sync` to drop the stale registration.".to_owned(),
        };
    }
    let state = match git.worktree_dirty(path) {
        Ok(true) => " — it has uncommitted changes".to_owned(),
        Ok(false) => String::new(),
        Err(error) => format!(" (could not inspect: {error})"),
    };
    let remove = format!("git --git-dir {bare} worktree remove {path}");
    // A checkout that was the default or a base branch carries ivar's
    // read-only guard on its root, and git cannot delete the entries of a
    // directory it cannot write to: it drops the registration, then aborts,
    // leaving the files behind unregistered. Lift the guard first.
    let remove = if is_read_only(path) {
        format!("chmod u+w {path} && {remove}")
    } else {
        remove
    };
    Diagnosis {
        code: "repo.worktree_orphaned",
        what: format!("`{path}` in `{repo}` is on branch `{branch}`, which no feature owns{state}"),
        fix: format!("Inspect it; if nothing is needed, run `{remove}`."),
    }
}

/// Whether `path`'s root has no write bit — the read-only guard ivar puts on a
/// default-branch or base-branch checkout. A path whose mode cannot be read is
/// not reported as guarded.
fn is_read_only(path: &Utf8Path) -> bool {
    fs::unix_mode(path)
        .ok()
        .flatten()
        .is_some_and(|mode| mode & 0o222 == 0)
}

/// How deep below `repos/<repo>/` a checkout is looked for. A branch name nests
/// one directory per `/`, so this is far past any real name; it bounds the walk
/// through a directory that is not a checkout at all.
const MAX_CHECKOUT_DEPTH: usize = 8;

/// Directories under `repos/<repo>/` that are checkouts (hold a `.git`) but
/// that git does not list as a worktree of the repo's bare clone.
///
/// The usual cause is a `git worktree remove` that unregistered the worktree
/// and then failed to delete its files. The registration is gone, so
/// [`diagnose_orphan_worktrees`] has nothing to list and nothing else would
/// ever name the directory. `.git` marks a checkout's root, which is how a
/// branch prefix like `fix/` (a plain intermediate directory) is told apart
/// from a checkout; the walk does not descend into a checkout.
fn diagnose_unregistered_checkouts(
    layout: &Layout,
    repo: &RepoName,
    entries: &[WorktreeEntry],
) -> Vec<Diagnosis> {
    // Git reports real paths; the hall root may be reached through a symlink.
    let registered: HashSet<Utf8PathBuf> = entries
        .iter()
        .flat_map(|entry| {
            let real = fs::canonicalize(&entry.path).ok();
            [Some(entry.path.clone()), real]
        })
        .flatten()
        .collect();
    let mut checkouts = Vec::new();
    collect_checkouts(&layout.repo_dir(repo), 0, &mut checkouts);
    checkouts
        .into_iter()
        .filter(|path| {
            !registered.contains(path)
                && !fs::canonicalize(path).is_ok_and(|real| registered.contains(&real))
        })
        .map(|path| {
            let remove = if is_read_only(&path) {
                format!("chmod u+w {path} && rm -rf {path}")
            } else {
                format!("rm -rf {path}")
            };
            Diagnosis {
                code: "repo.worktree_unregistered",
                what: format!(
                    "`{path}` in `{repo}` is a checkout that git does not list as a worktree — \
                     a removal that failed part-way usually leaves this behind"
                ),
                // Not `git worktree remove`: there is no registration for it to act on.
                fix: format!(
                    "Inspect it for work you still need; if nothing is needed, run `{remove}`."
                ),
            }
        })
        .collect()
}

/// Collect every checkout root under `dir`, descending through the
/// directories that are not one. `.bare` and symlinks are skipped.
fn collect_checkouts(dir: &Utf8Path, depth: usize, found: &mut Vec<Utf8PathBuf>) {
    let Ok(mut children) = fs::read_dir(dir) else {
        return;
    };
    children.sort();
    for child in children {
        if child.file_name() == Some(".bare") && depth == 0 {
            continue;
        }
        if !fs::is_real_dir(&child).unwrap_or(false) {
            continue;
        }
        if fs::exists(&child.join(".git")).unwrap_or(false) {
            found.push(child);
        } else if depth < MAX_CHECKOUT_DEPTH {
            collect_checkouts(&child, depth + 1, found);
        }
    }
}

/// The feature whose integration staging `path` is. Local integration
/// stages a detached candidate and, for rebase, a temporary source worktree
/// under `<features>/<feature>/integration/<repo>/`.
fn integration_staging_owner(
    layout: &Layout,
    repo: &RepoName,
    path: &Utf8Path,
) -> Option<FeatureName> {
    let feature = path
        .strip_prefix(layout.features_dir())
        .ok()
        .and_then(|relative| relative.iter().next())
        .and_then(|name| FeatureName::new(name).ok())?;
    (path == layout.integration_candidate(&feature, repo)
        || path == layout.integration_source(&feature, repo))
    .then_some(feature)
}

/// Whether an integrate of `child` into its parent is running now, so its
/// staging is live rather than left behind.
fn integration_running(layout: &Layout, child: &FeatureName) -> bool {
    Feature::read(layout, child)
        .ok()
        .flatten()
        .and_then(|feature| feature.parent)
        .is_some_and(|parent| {
            crate::action::feature::integrate::parent_integration_running(layout, &parent)
        })
}

/// Every `(repo, branch)` a feature owns: its branch in each repo it
/// promoted, and — in each manifest repo it did not promote — the base
/// branch its sessions view ([`base_view::resolve`] → `BaseBranch`).
/// Fails with one diagnosis per feature record that cannot be read, and per
/// base view that cannot be resolved.
pub(crate) fn feature_worktrees(
    layout: &Layout,
    manifest: &Manifest,
) -> Result<HashSet<(RepoName, String)>, Vec<Diagnosis>> {
    let mut owned = HashSet::new();
    let mut unresolved = Vec::new();
    for feature in feature_records(layout)? {
        for repo in feature.promotions.keys() {
            owned.insert((repo.clone(), feature.branch.to_string()));
        }
        for repo in manifest.repos() {
            if feature.is_promoted(repo.name()) {
                continue;
            }
            match base_view::resolve(layout, repo, &feature) {
                Ok(BaseView::BaseBranch(branch)) => {
                    owned.insert((repo.name().clone(), branch.to_string()));
                }
                Ok(BaseView::Default | BaseView::Parent { .. }) => {}
                Err(error) => unresolved.push(Diagnosis {
                    code: "feature.record_unreadable",
                    what: format!(
                        "the base view of `{}` for feature `{}` cannot be resolved: {error}",
                        repo.name(),
                        feature.name
                    ),
                    fix: format!(
                        "Repair the feature records above `{}`; orphan worktrees are not checked until then.",
                        feature.name
                    ),
                }),
            }
        }
    }
    if unresolved.is_empty() {
        Ok(owned)
    } else {
        Err(unresolved)
    }
}

/// Promotions that are failed or whose worktree is gone, and branches more
/// than one feature records. Unreadable records are reported by
/// [`diagnose_orphan_worktrees`], so they are skipped here.
pub(super) fn diagnose_feature_state(layout: &Layout, git: &impl Git) -> Vec<Diagnosis> {
    let Ok(features) = feature_records(layout) else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    let mut owners: BTreeMap<&str, Vec<&FeatureName>> = BTreeMap::new();
    for feature in &features {
        owners
            .entry(feature.branch.as_str())
            .or_default()
            .push(&feature.name);
        for (repo, promotion) in &feature.promotions {
            let retry = format!(
                "Run `ivar feature promote {} {repo}` to retry it.",
                feature.name
            );
            let worktree = layout.repo_worktree(repo, &feature.branch);
            if promotion.worktree == WorktreeState::Failed {
                let reason = promotion.reason.as_deref().unwrap_or("setup script failed");
                findings.push(Diagnosis {
                    code: "feature.promotion_failed",
                    what: format!(
                        "`{repo}` in feature `{}` failed to promote: {reason}",
                        feature.name
                    ),
                    fix: retry,
                });
            } else if matches!(
                git.target_state(&worktree),
                Ok(TargetState::Absent | TargetState::Occupied)
            ) {
                findings.push(Diagnosis {
                    code: "feature.promotion_worktree_missing",
                    what: format!(
                        "`{repo}` is promoted into feature `{}` but `{worktree}` is not a worktree",
                        feature.name
                    ),
                    fix: retry,
                });
            }
        }
    }
    for (branch, names) in owners {
        if let [first, .., last] = names.as_slice() {
            findings.push(Diagnosis {
                code: "feature.branch_shared",
                what: format!(
                    "branch `{branch}` is recorded by features {}",
                    names
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                fix: format!(
                    "Keep the branch on one feature. If `{last}` has no promoted repos, give it \
                     its own with `ivar feature rename {last} --branch <new-branch>`; otherwise \
                     that rename moves the git branch `{first}` also uses, so decide which \
                     feature holds the work first."
                ),
            });
        }
    }
    findings
}

/// Every readable feature record in the hall. Fails with one diagnosis per
/// record that cannot be read.
fn feature_records(layout: &Layout) -> Result<Vec<Feature>, Vec<Diagnosis>> {
    let features_dir = layout.features_dir();
    if !fs::is_dir(&features_dir).unwrap_or(true) {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&features_dir).map_err(|error| {
        vec![Diagnosis {
            code: "feature.record_unreadable",
            what: format!("could not list the features in `{features_dir}`: {error}"),
            fix: "Repair the directory's permissions, then rerun `ivar doctor`.".to_owned(),
        }]
    })?;
    let mut records = Vec::new();
    let mut unreadable = Vec::new();
    for name in entries
        .iter()
        .filter_map(|entry| FeatureName::new(entry.file_name()?.to_owned()).ok())
    {
        match Feature::read(layout, &name) {
            Ok(Some(feature)) => records.push(feature),
            Ok(None) => {}
            Err(error) => unreadable.push(Diagnosis {
                code: "feature.record_unreadable",
                what: format!("the record of feature `{name}` cannot be read: {error}"),
                fix: format!(
                    "Repair or remove `{}`; orphan worktrees are not checked until then.",
                    layout.feature_dir(&name)
                ),
            }),
        }
    }
    if unreadable.is_empty() {
        Ok(records)
    } else {
        Err(unreadable)
    }
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

/// Detect working documents sitting in legacy locations (`plans/<feature>/` or
/// `docs/<feature>/discovery.md`) from earlier versions of ivar.
///
/// Since working documents live exclusively under `.ivar/features/<name>/`,
/// files left in committed `plans/` or `docs/` are no longer read or updated.
///
/// Precision note: `<hall>/docs/` holds topic docs (`memory/`,
/// `repo-relations/` or any custom doc folders). We ONLY flag directories under
/// `docs/` if they contain a `discovery.md` file AND are not reserved topic dirs.
pub(super) fn check_legacy_working_docs(
    layout: &Layout,
    findings: &mut Vec<Diagnosis>,
) -> Result<(), Failure> {
    let root = layout.root();

    // Check legacy <hall>/plans/<feature>/
    let plans_dir = root.join("plans");
    if fs::is_dir(&plans_dir)? {
        for entry in fs::read_dir(&plans_dir)? {
            if fs::is_dir(&entry)?
                && let Some(name) = entry.file_name()
                && FeatureName::new(name.to_owned()).is_ok()
            {
                findings.push(Diagnosis {
                    code: "hall.working_docs_legacy_location",
                    what: format!(
                        "working documents for feature `{name}` are in committed `plans/{name}/`; ivar no longer reads this path"
                    ),
                    fix: format!(
                        "Move working documents to `.ivar/features/{name}/` or delete `plans/{name}/` if no longer needed."
                    ),
                });
            }
        }
    }

    // Check legacy <hall>/docs/<feature>/discovery.md
    let docs_dir = root.join("docs");
    if fs::is_dir(&docs_dir)? {
        for entry in fs::read_dir(&docs_dir)? {
            if fs::is_dir(&entry)?
                && let Some(name) = entry.file_name()
                && !["memory", "repo-relations"].contains(&name)
                && FeatureName::new(name.to_owned()).is_ok()
            {
                let discovery = entry.join("discovery.md");
                if fs::is_file(&discovery)? {
                    findings.push(Diagnosis {
                        code: "hall.working_docs_legacy_location",
                        what: format!(
                            "discovery document for `{name}` is at `docs/{name}/discovery.md`; ivar no longer reads this path"
                        ),
                        fix: format!(
                            "Move `docs/{name}/discovery.md` to `.ivar/features/{name}/discovery.md` or delete it."
                        ),
                    });
                }
            }
        }
    }

    Ok(())
}
