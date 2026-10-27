//! Resolves an explicit worktree path in a `graph_explore` query to that
//! worktree's own layer, or refuses.
//!
//! The graph answers from the layers its session mounts. An agent that names
//! `<hall>/.ivar/repos/api/add-auth/src/a.rs` from outside the `add-auth`
//! session would otherwise match the tail `src/a.rs` against the main rows and
//! read stale code with no sign of it. Here the path picks the tree: the
//! worktree's layer is indexed and mounted for the call, or the call answers
//! that the worktree is not indexed.

use std::collections::{BTreeMap, BTreeSet};

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::graph::layer::ensure_layer_indexed;
use crate::action::graph::query::find::{is_path_like, split_line_range};
use crate::action::graph::watch::worker::base_commit_for;
use crate::domain::feature::Feature;
use crate::domain::name::FeatureName;
use crate::infra::fs;
use crate::store::graph::db::GraphDb;
use crate::store::layout::Layout;
use crate::store::manifest::Manifest;

/// Characters an agent wraps around a path token; the resolver trims the same.
const EDGE: [char; 10] = [',', ';', ':', '"', '\'', '`', '(', ')', '{', '}'];

/// A worktree a path token points into.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Located {
    repo: String,
    /// Worktree directory relative to the repo's store (`add-auth`, `feat/x`).
    branch: String,
    /// Path inside the worktree; empty for the worktree root.
    rest: String,
    /// Named as `.ivar/repos/...` or an absolute path, rather than inferred
    /// from the working directory.
    explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Reference {
    Worktree(Located),
    /// An explicit path under the repo store with no worktree at it.
    Missing {
        path: String,
    },
}

/// The tree one repo is read from for this call.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    repo: String,
    /// `repo/<layer id>` of the layer to mount; `None` reads the main rows.
    layer_repo: Option<String>,
    label: String,
}

/// The outcome of scoping a query.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Scoped {
    /// Run this query under the mounts, labelling the answer with `labels`.
    Run(WorktreeScope),
    /// Answer with this text instead of running the query.
    Refused(String),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct WorktreeScope {
    /// The query with each worktree path rewritten to `repo/path`.
    pub query: String,
    mounts: Vec<Mount>,
    /// One line per explicit worktree path, naming the tree it was read from.
    pub labels: Vec<String>,
}

/// Restores the connection's session layers when the call is done.
pub(super) struct ScopeGuard<'a> {
    db: &'a GraphDb,
    original: Vec<(String, String)>,
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        let pairs: Vec<(&str, &str)> = self
            .original
            .iter()
            .map(|(repo, layer)| (repo.as_str(), layer.as_str()))
            .collect();
        let _ = if pairs.is_empty() {
            self.db.clear_session_layers()
        } else {
            self.db.configure_session_mode(&pairs)
        };
    }
}

impl WorktreeScope {
    /// Mounts the scope's layers over the session's for as long as the guard
    /// lives. A scope without a mount leaves the session untouched.
    pub(super) fn mount<'a>(&self, db: &'a GraphDb) -> Result<ScopeGuard<'a>, String> {
        let current = db
            .session_layers()
            .map_err(|e| format!("could not read the mounted layers: {e}"))?;
        let original: Vec<(String, String)> = current
            .iter()
            .map(|layer| (layer.repo.clone(), layer.layer_repo.clone()))
            .collect();
        let mut wanted: BTreeMap<String, String> = original.iter().cloned().collect();
        for mount in &self.mounts {
            match &mount.layer_repo {
                Some(layer) => wanted.insert(mount.repo.clone(), layer.clone()),
                None => wanted.remove(&mount.repo),
            };
        }
        let guard = ScopeGuard { db, original };
        let pairs: Vec<(&str, &str)> = wanted
            .iter()
            .map(|(repo, layer)| (repo.as_str(), layer.as_str()))
            .collect();
        let unchanged = pairs.len() == guard.original.len()
            && pairs
                .iter()
                .zip(&guard.original)
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1);
        if !unchanged {
            db.configure_session_mode(&pairs)
                .map_err(|e| format!("could not mount the worktree layer: {e}"))?;
        }
        Ok(guard)
    }
}

/// Scopes `query` to the worktrees its explicit paths name.
///
/// `cwd` lets a relative path that exists inside a worktree count as naming
/// it; such an inferred path never refuses, it only redirects when the layer
/// is available.
pub(super) fn scope_query(
    db: &GraphDb,
    layout: &Layout,
    cwd: Option<&Utf8Path>,
    query: &str,
) -> Scoped {
    let tokens: Vec<Token> = query.split_whitespace().map(Token::parse).collect();
    let references: Vec<Option<Reference>> = tokens
        .iter()
        .map(|token| {
            token
                .path()
                .filter(|path| is_path_like(path))
                .and_then(|path| locate(layout, cwd, path))
        })
        .collect();
    if references.iter().all(Option::is_none) {
        return Scoped::Run(WorktreeScope {
            query: query.to_owned(),
            mounts: Vec::new(),
            labels: Vec::new(),
        });
    }

    let Ok(current) = db.session_layers() else {
        return Scoped::Run(WorktreeScope {
            query: query.to_owned(),
            mounts: Vec::new(),
            labels: Vec::new(),
        });
    };
    match plan(db, layout, &current, &references) {
        Ok(decisions) => Scoped::Run(rewrite(&tokens, &references, &decisions, &current)),
        Err(answer) => Scoped::Refused(answer),
    }
}

type Key = (String, String);

impl Located {
    fn key(&self) -> Key {
        (self.repo.clone(), self.branch.clone())
    }
}

/// One decision per distinct worktree, or the refusal the call answers with.
/// A refusal only stands when an explicit path asked for the worktree.
fn plan(
    db: &GraphDb,
    layout: &Layout,
    current: &[SessionLayerView],
    references: &[Option<Reference>],
) -> Result<BTreeMap<Key, Decision>, String> {
    let mut decisions: BTreeMap<Key, Decision> = BTreeMap::new();
    let mut explicit_keys: BTreeSet<Key> = BTreeSet::new();
    for reference in references.iter().flatten() {
        match reference {
            Reference::Missing { path } => {
                return Err(format!(
                    "No worktree exists at `{path}`, so nothing is indexed there. \
                     Use a path under an existing `.ivar/repos/<repo>/<worktree>/`, or read \
                     the file directly."
                ));
            }
            Reference::Worktree(located) => {
                if located.explicit {
                    explicit_keys.insert(located.key());
                }
                decisions
                    .entry(located.key())
                    .or_insert_with(|| decide(db, layout, current, located));
            }
        }
    }

    for key in &explicit_keys {
        if let Some(Decision::Refuse(message)) = decisions.get(key) {
            return Err(message.clone());
        }
    }

    let mut trees: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for key in &explicit_keys {
        trees.entry(&key.0).or_default().push(&key.1);
    }
    if let Some((repo, branches)) = trees.iter().find(|(_, branches)| branches.len() > 1) {
        let names: Vec<String> = branches.iter().map(|b| format!("`{b}`")).collect();
        return Err(format!(
            "The paths name more than one tree of `{repo}` ({}). One call reads one tree of a \
             repo: ask for each worktree in its own call.",
            names.join(", ")
        ));
    }
    Ok(decisions)
}

/// The scope that mounts each decided layer and rewrites its tokens.
fn rewrite(
    tokens: &[Token],
    references: &[Option<Reference>],
    decisions: &BTreeMap<Key, Decision>,
    current: &[SessionLayerView],
) -> WorktreeScope {
    let mut query = Vec::with_capacity(tokens.len());
    let mut mounts: BTreeMap<String, Mount> = BTreeMap::new();
    let mut labels: Vec<String> = Vec::new();
    for (token, reference) in tokens.iter().zip(references) {
        let redirected = match reference {
            Some(Reference::Worktree(located)) => match decisions.get(&located.key()) {
                Some(Decision::Use { mount, redirect }) if *redirect || located.explicit => {
                    Some((located, mount))
                }
                _ => None,
            },
            _ => None,
        };
        let Some((located, mount)) = redirected else {
            query.push(token.raw.to_owned());
            continue;
        };
        if let Some(mount) = mount {
            mounts.insert(mount.repo.clone(), mount.clone());
        }
        let label = match mount {
            Some(mount) => mount.label.clone(),
            None => unmounted_label(located, current),
        };
        if !labels.contains(&label) {
            labels.push(label);
        }
        query.push(token.rewrite(&located.repo, &located.rest));
    }
    WorktreeScope {
        query: query.join(" "),
        mounts: mounts.into_values().collect(),
        labels,
    }
}

/// What reading one worktree takes.
#[derive(Debug, Clone)]
enum Decision {
    Use {
        /// The layer to mount; `None` when the session already reads this tree.
        mount: Option<Mount>,
        /// An inferred path is rewritten only when its mount changes the tree.
        redirect: bool,
    },
    Refuse(String),
}

fn unmounted_label(located: &Located, current: &[SessionLayerView]) -> String {
    match current.iter().find(|layer| layer.repo == located.repo) {
        Some(_) => format!(
            "`{}`: read from the session's worktree `{}`.",
            located.repo, located.branch
        ),
        None => format!(
            "`{}`: read from the main checkout `{}`.",
            located.repo, located.branch
        ),
    }
}

type SessionLayerView = crate::store::graph::db::layer::SessionLayer;

fn decide(
    db: &GraphDb,
    layout: &Layout,
    current: &[SessionLayerView],
    located: &Located,
) -> Decision {
    let worktree = worktree_dir(layout, located);
    let mounted = current.iter().find(|layer| layer.repo == located.repo);

    if is_default_worktree(layout, located) {
        return match mounted {
            None => Decision::Use {
                mount: None,
                redirect: false,
            },
            Some(_) => Decision::Use {
                mount: Some(Mount {
                    repo: located.repo.clone(),
                    layer_repo: None,
                    label: format!(
                        "`{}`: read from the main checkout `{}`, not the session's worktree.",
                        located.repo, located.branch
                    ),
                }),
                redirect: true,
            },
        };
    }

    if mounted.is_some_and(|layer| same_dir(Utf8Path::new(&layer.worktree), &worktree)) {
        return Decision::Use {
            mount: None,
            redirect: false,
        };
    }

    let Some((feature, base)) = promoted_feature(layout, located) else {
        return Decision::Refuse(format!(
            "Worktree `{}` of `{}` is not indexed: it is neither the repo's default checkout \
             nor a promoted feature worktree. Read the file directly.",
            located.branch, located.repo
        ));
    };
    let Some(base_commit) = base_commit_for(db, &located.repo, base.as_deref()) else {
        return Decision::Refuse(format!(
            "Worktree `{}` of `{}` is not indexed: the base graph index for `{}` is missing. \
             Call `refresh_index`, or read the file directly.",
            located.branch, located.repo, located.repo
        ));
    };
    match ensure_layer_indexed(db, layout, &feature, &located.repo, &worktree, &base_commit) {
        Ok(result) => Decision::Use {
            mount: Some(Mount {
                repo: located.repo.clone(),
                layer_repo: Some(format!("{}/{}", located.repo, result.layer_id)),
                label: format!(
                    "`{}`: read from worktree `{}` (feature `{feature}`), not main.",
                    located.repo, located.branch
                ),
            }),
            redirect: true,
        },
        Err(err) => Decision::Refuse(format!(
            "Worktree `{}` of `{}` is not indexed: {err}. Read the file directly.",
            located.branch, located.repo
        )),
    }
}

fn worktree_dir(layout: &Layout, located: &Located) -> Utf8PathBuf {
    layout.repos_dir().join(&located.repo).join(&located.branch)
}

fn same_dir(a: &Utf8Path, b: &Utf8Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize_utf8(), b.canonicalize_utf8()),
            (Ok(a), Ok(b)) if a == b
        )
}

fn is_default_worktree(layout: &Layout, located: &Located) -> bool {
    let Ok(Some(manifest)) = Manifest::read(layout) else {
        return false;
    };
    manifest.repos().iter().any(|repo| {
        repo.name().as_str() == located.repo && repo.default_branch().as_str() == located.branch
    })
}

/// The feature whose promotion owns this worktree, with the promotion's base.
fn promoted_feature(layout: &Layout, located: &Located) -> Option<(String, Option<String>)> {
    let features_dir = layout.features_dir();
    if !fs::is_dir(&features_dir).ok()? {
        return None;
    }
    let mut names: Vec<FeatureName> = fs::read_dir(&features_dir)
        .ok()?
        .iter()
        .filter_map(|entry| entry.file_name().and_then(|n| FeatureName::new(n).ok()))
        .collect();
    names.sort();
    names.into_iter().find_map(|name| {
        let feature = Feature::read(layout, &name).ok().flatten()?;
        if feature.branch.as_str() != located.branch {
            return None;
        }
        let promotion = feature
            .promotions
            .iter()
            .find(|(repo, _)| repo.as_str() == located.repo)
            .map(|(_, promotion)| promotion)?;
        Some((
            feature.name.to_string(),
            promotion.base.as_ref().map(|base| base.as_str().to_owned()),
        ))
    })
}

/// A whitespace-delimited query token: the path between its edge characters.
struct Token<'a> {
    raw: &'a str,
    start: usize,
    end: usize,
}

impl<'a> Token<'a> {
    fn parse(raw: &'a str) -> Self {
        let start = raw.len() - raw.trim_start_matches(EDGE).len();
        let end = raw.trim_end_matches(EDGE).len().max(start);
        Self { raw, start, end }
    }

    fn core(&self) -> &'a str {
        &self.raw[self.start..self.end]
    }

    /// The path without its trailing `:start-end` line range.
    fn path(&self) -> Option<&'a str> {
        let core = self.core();
        (!core.is_empty()).then(|| split_line_range(core).0)
    }

    /// The token with its path replaced by `repo/rest`, keeping the line range
    /// and surrounding punctuation.
    fn rewrite(&self, repo: &str, rest: &str) -> String {
        let core = self.core();
        let path_len = split_line_range(core).0.len();
        let target = if rest.is_empty() {
            repo.to_owned()
        } else {
            format!("{repo}/{rest}")
        };
        format!(
            "{}{target}{}{}",
            &self.raw[..self.start],
            &core[path_len..],
            &self.raw[self.end..]
        )
    }
}

/// The worktree a path token points into, if any.
fn locate(layout: &Layout, cwd: Option<&Utf8Path>, token: &str) -> Option<Reference> {
    let token = token.trim_start_matches("./");
    let (path, explicit) = candidate_path(layout, cwd, token)?;
    let (store, relative) = under_repos_dir(layout, &path)?;
    let mut components = relative.components().map(|c| c.as_str());
    let repo = components.next()?;
    let inside: Vec<&str> = components.collect();
    if inside.is_empty() || inside.first() == Some(&".bare") {
        return None;
    }
    let repo_dir = store.join(repo);
    for split in 1..=inside.len() {
        let Some((branch, rest)) = inside.split_at_checked(split) else {
            break;
        };
        let candidate = branch
            .iter()
            .fold(repo_dir.clone(), |dir, part| dir.join(part));
        if candidate.join(".git").exists() {
            return Some(Reference::Worktree(Located {
                repo: repo.to_owned(),
                branch: branch.join("/"),
                rest: rest.join("/"),
                explicit,
            }));
        }
    }
    explicit.then(|| Reference::Missing {
        path: path.to_string(),
    })
}

/// The absolute form of a path token, and whether the token named it
/// explicitly. A relative path that is not `.ivar/repos/...` counts only when
/// it exists under the working directory.
fn candidate_path(
    layout: &Layout,
    cwd: Option<&Utf8Path>,
    token: &str,
) -> Option<(Utf8PathBuf, bool)> {
    let path = Utf8Path::new(token);
    if path.is_absolute() {
        return Some((normalize(path), true));
    }
    if path.starts_with(".ivar/repos") {
        return Some((normalize(&layout.root().join(path)), true));
    }
    let joined = normalize(&cwd?.join(path));
    joined.exists().then_some((joined, false))
}

/// `path` relative to the repo store, with the store directory it was matched
/// against (a canonical form when the path reaches the store through a link).
fn under_repos_dir(layout: &Layout, path: &Utf8Path) -> Option<(Utf8PathBuf, Utf8PathBuf)> {
    let store = layout.repos_dir();
    if let Ok(relative) = path.strip_prefix(&store) {
        return Some((store, relative.to_owned()));
    }
    let canonical_store = store.canonicalize_utf8().ok()?;
    let canonical_path = path.canonicalize_utf8().ok().unwrap_or_else(|| {
        // A path that does not exist still names a worktree through a linked
        // ancestor: canonicalise the longest existing prefix.
        let mut existing = path.to_owned();
        let mut tail = Vec::new();
        while !existing.exists() {
            let Some(name) = existing.file_name().map(str::to_owned) else {
                return path.to_owned();
            };
            tail.push(name);
            if !existing.pop() {
                return path.to_owned();
            }
        }
        let mut resolved = existing.canonicalize_utf8().unwrap_or(existing);
        for name in tail.iter().rev() {
            resolved.push(name);
        }
        resolved
    });
    let relative = canonical_path.strip_prefix(&canonical_store).ok()?;
    Some((canonical_store, relative.to_owned()))
}

/// Drops `.` and resolves `..` lexically.
fn normalize(path: &Utf8Path) -> Utf8PathBuf {
    let mut out = Utf8PathBuf::new();
    for component in path.components() {
        match component {
            camino::Utf8Component::CurDir => {}
            camino::Utf8Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_str()),
        }
    }
    out
}

#[cfg(test)]
#[path = "../../../../tests/unit/action/graph/mcp/worktree_scope.rs"]
mod tests;
