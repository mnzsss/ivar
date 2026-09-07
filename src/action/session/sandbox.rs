//! Kernel-enforced write sandbox: computes the footprint of allowed write roots
//! from a `WritableSet`, backing git repositories, platform devices, and provider
//! runtime state.

use crate::action::session::guard::WritableSet;
use crate::domain::feature::Feature;
use crate::domain::provider::Provider;
use crate::error::Failure;
use crate::store::layout::Layout;
use camino::{Utf8Path, Utf8PathBuf};

/// The kernel-enforced projection of a `WritableSet`.
#[derive(Debug, Clone)]
pub(crate) struct Sandbox {
    roots: Vec<Utf8PathBuf>,
}

impl Sandbox {
    /// Derive all write-allowed roots for this session.
    pub(crate) fn from_writable_set(
        set: &WritableSet,
        layout: &Layout,
        feature: Option<&Feature>,
        provider: Provider,
    ) -> Result<Self, Failure> {
        let mut candidate_roots: Vec<Utf8PathBuf> = Vec::new();

        // 1. All roots defined by WritableSet (view dir, feature dir, worktrees).
        for r in set.roots() {
            candidate_roots.push(r.to_path_buf());
        }

        // 2. Backing bare git directories for all promoted repos in the feature.
        if let Some(feature) = feature {
            for repo in feature.promotions.keys() {
                let bare = layout.repo_bare(repo);
                candidate_roots.push(bare);
            }
        }

        // 3. /dev/null - required by git and child processes for writing / redirection.
        let dev_null = Utf8PathBuf::from("/dev/null");
        candidate_roots.push(dev_null);

        // 4. System temp directory.
        if let Ok(temp) = Utf8PathBuf::try_from(std::env::temp_dir()) {
            candidate_roots.push(temp);
        }

        // 5. Shared cargo target cache if it exists under the hall layout.
        let cache_dir = layout.root().join(".ivar").join("cache");
        candidate_roots.push(cache_dir);

        // 6. Provider runtime directories.
        for p_root in provider_runtime_roots(provider) {
            candidate_roots.push(p_root);
        }

        // Canonicalise and filter candidate roots to only those that exist on disk.
        // Landlock PathFd::new fails if a path does not exist.
        let mut final_roots: Vec<Utf8PathBuf> = Vec::new();
        for path in candidate_roots {
            if let Ok(canonical) = path.canonicalize_utf8() {
                if !final_roots.contains(&canonical) {
                    final_roots.push(canonical);
                }
            } else if path.exists() {
                if !final_roots.contains(&path) {
                    final_roots.push(path);
                }
            }
        }

        Ok(Self { roots: final_roots })
    }

    /// Return the list of canonical roots that will be added to the ruleset.
    pub(crate) fn roots(&self) -> &[Utf8PathBuf] {
        &self.roots
    }
}

/// Derive candidate runtime and state directories for the given provider.
fn provider_runtime_roots(provider: Provider) -> Vec<Utf8PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var("HOME").ok().map(Utf8PathBuf::from);

    match provider {
        Provider::ClaudeCode => {
            if let Some(home) = &home {
                dirs.push(home.join(".claude"));
                dirs.push(home.join(".claude.json"));
            }
        }
        Provider::Omp => {
            if let Some(home) = &home {
                dirs.push(home.join(".omp"));
            }
            if let Ok(pi_config) = std::env::var("PI_CONFIG_DIR") {
                dirs.push(Utf8PathBuf::from(pi_config));
            }
        }
        Provider::OpenCode => {
            if let Some(home) = &home {
                dirs.push(home.join(".local").join("share").join("opencode"));
                dirs.push(home.join(".local").join("state").join("opencode"));
                dirs.push(home.join(".config").join("opencode"));
            }
            if let Ok(data_dir) = crate::infra::fs::data_dir() {
                dirs.push(data_dir.join("opencode"));
            }
            if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
                dirs.push(Utf8PathBuf::from(xdg_state).join("opencode"));
            }
            if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
                dirs.push(Utf8PathBuf::from(xdg_config).join("opencode"));
            }
        }
    }
    dirs
}

#[cfg(test)]
#[path = "../../../tests/unit/action/session/sandbox.rs"]
mod tests;
