use super::*;

/// Graph freshness: every indexed base repo against its worktree HEAD, and the
/// schema against the one this binary migrates to. A hall that never built a
/// graph has nothing to report.
pub(super) fn graph_diagnoses(
    layout: &Layout,
    manifest: &Manifest,
    git: &impl Git,
) -> Vec<Diagnosis> {
    let db_path = layout.ivar_dir().join("memory.db");
    if !db_path.as_std_path().exists() {
        return Vec::new();
    }
    let db = match GraphDb::open_read_only(db_path.as_std_path()) {
        Ok(db) => db,
        Err(error) => {
            return vec![Diagnosis {
                code: "graph.open_failed",
                what: format!("could not open the graph database at `{db_path}`: {error}"),
                fix: "Remove it and run `ivar graph index` to rebuild it.".to_owned(),
            }];
        }
    };

    let mut findings = Vec::new();
    let version: i64 = db
        .conn()
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap_or(0);
    if version != SCHEMA_VERSION {
        findings.push(Diagnosis {
            code: "graph.schema_mismatch",
            what: format!("graph schema is version {version}, this ivar expects {SCHEMA_VERSION}"),
            fix: "Run `ivar graph index` to migrate it, or run `ivar upgrade` if it is newer."
                .to_owned(),
        });
    }

    for repo in manifest.repos() {
        let Ok(Some(row)) = db.get_repo(repo.name().as_str()) else {
            continue;
        };
        let worktree = layout.repo_worktree(repo.name(), repo.default_branch());
        let Ok(head) = git.head_commit(&worktree) else {
            continue;
        };
        if row.last_indexed_commit.as_deref() != Some(head.as_str()) {
            let indexed = row.last_indexed_commit.as_deref().unwrap_or("nothing");
            findings.push(Diagnosis {
                code: "graph.repo_stale",
                what: format!(
                    "graph for `{}` is indexed at {indexed} but its worktree HEAD is {head}",
                    repo.name()
                ),
                fix: format!("Run `ivar graph index --repo {}`.", repo.name()),
            });
        }
    }
    findings.extend(unindexed_hall_dirs_diagnosis(layout, manifest));
    let facts = HallFacts::gather(layout, manifest);
    findings.extend(orphan_graph_diagnoses(&db, &facts));
    let leader = crate::action::graph::watch::lease::leader_pid(layout);
    for row in db.watch_scopes().unwrap_or_default() {
        if !facts.scope_is_live(&row.scope) {
            findings.push(Diagnosis {
                code: "graph.watch_scope_orphaned",
                what: format!(
                    "graph scope `{}` has no repo, feature or path behind it",
                    row.scope
                ),
                fix: "Run `ivar graph index`.".to_owned(),
            });
        } else if let Some(error) = &row.error {
            findings.push(Diagnosis {
                code: "graph.watch_scope_failed",
                what: format!(
                    "the graph watcher could not reindex `{}`: {error}",
                    row.scope
                ),
                fix: "Run `ivar graph index`; a leader retries once the scope is watched again."
                    .to_owned(),
            });
        } else if leader.is_none() && (row.needs_catchup || row.indexed < row.observed) {
            findings.push(Diagnosis {
                code: "graph.watch_catchup_pending",
                what: format!("graph scope `{}` has changes no watcher indexed", row.scope),
                fix: "Run `ivar graph index`, or start `ivar graph mcp` to catch up.".to_owned(),
            });
        }
    }
    findings
}

/// Hall directories that hold documents, brand material or skills, which the
/// graph never reads until `graph.paths` lists them.
const HALL_DOC_DIRS: &[&str] = &["docs", "brand", ".ivar/skills"];

/// A hall with documentation directories and an empty `graph.paths`: the graph
/// answers for the repos only, and every path in those directories misses.
fn unindexed_hall_dirs_diagnosis(layout: &Layout, manifest: &Manifest) -> Option<Diagnosis> {
    if !manifest.graph_paths().is_empty() {
        return None;
    }
    let present: Vec<&str> = HALL_DOC_DIRS
        .iter()
        .copied()
        .filter(|dir| {
            layout.root().join(dir).is_dir()
                && crate::action::graph::index::walk_hall_dir(layout.root().as_std_path(), dir)
                    .is_ok_and(|files| !files.is_empty())
        })
        .collect();
    if present.is_empty() {
        return None;
    }
    let names = present
        .iter()
        .map(|dir| format!("`{dir}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let flags = present
        .iter()
        .map(|dir| format!("--path {dir}"))
        .collect::<Vec<_>>()
        .join(" ");
    Some(Diagnosis {
        code: "graph.hall_paths_unset",
        what: format!(
            "the hall has {names} but `graph.paths` in `ivar.json` is empty, so the graph \
             cannot answer for files in them"
        ),
        fix: format!(
            "Run `ivar graph index {flags}` to list them in `graph.paths` and index them."
        ),
    })
}

/// Base repos the manifest no longer declares and layers whose feature is
/// gone: the rows `ivar graph index` reconciles away, named so a user learns
/// why the graph is bigger than the hall.
fn orphan_graph_diagnoses(db: &GraphDb, facts: &HallFacts) -> Vec<Diagnosis> {
    let mut findings = Vec::new();
    for row in db.list_base_repos().unwrap_or_default() {
        if row.id == crate::action::graph::index::HALL_REPO || facts.repo_is_declared(&row.id) {
            continue;
        }
        let held = match db.repo_counts(&row.id) {
            Ok(counts) => format!(
                "{} files, {} symbols, {} edges",
                counts.files, counts.symbols, counts.edges
            ),
            Err(error) => format!("counts unavailable: {error}"),
        };
        findings.push(Diagnosis {
            code: "graph.repo_not_in_manifest",
            what: format!(
                "graph still holds `{}` ({held}) but `ivar.json` no longer declares it",
                row.id
            ),
            fix: format!(
                "Run `ivar graph clean --repo {}`, or `ivar graph index` to reconcile everything.",
                row.id
            ),
        });
    }
    for layer in db.list_layer_records().unwrap_or_default() {
        if !facts.layer_is_live(&layer.feature) {
            findings.push(Diagnosis {
                code: "graph.layer_orphaned",
                what: format!(
                    "graph layer `{}` for `{}` belongs to a feature that no longer exists",
                    layer.feature, layer.repo
                ),
                fix: "Run `ivar graph index`.".to_owned(),
            });
        }
    }
    findings
}
