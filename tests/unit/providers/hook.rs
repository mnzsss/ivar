// tests/unit/providers/hook.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use camino::Utf8PathBuf;
use crate::domain::provider::Provider;
use crate::providers;

#[test]
fn opencode_plugin_bytes_characterization_unchanged() {
    let artifacts = providers::managed_artifacts(Provider::OpenCode);
    assert_eq!(artifacts.len(), 1, "OpenCode must declare exactly 1 managed artifact");
    let artifact = &artifacts[0];
    assert_eq!(artifact.relative_path, Utf8PathBuf::from(".opencode/plugins/ivar.js"));
    assert!(artifact.contents.contains("ivar session plugin for OpenCode"));
    assert!(artifact.contents.contains("shell.env"));
    assert!(artifact.contents.contains("tool.execute.before"));
    assert!(artifact.contents.contains("ivar guard --provider opencode"));
    assert!(artifact.contents.contains("ivar session env --json --cwd"));
}

#[test]
fn omp_hook_artifact_declared_at_exact_path_and_dependency_free() {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    assert_eq!(artifacts.len(), 1, "OMP must declare exactly 1 managed hook artifact");
    let artifact = &artifacts[0];
    assert_eq!(artifact.relative_path, Utf8PathBuf::from(".omp/hooks/pre/ivar.js"));
    assert!(artifact.contents.contains("// ivar pre-tool guard hook for OMP"));
    // OMP loads hook modules in-process: a default-exported factory that
    // registers handlers and *returns* the block verdict. It is not a
    // stdin/stdout filter.
    assert!(artifact.contents.contains("export default"));
    assert!(artifact.contents.contains("pi.on(\"tool_call\""));
    assert!(artifact.contents.contains("block: true"));
    // It shells out to the same guard binary every provider uses.
    assert!(artifact.contents.contains("\"guard\""));
    assert!(artifact.contents.contains("\"--provider\""));
    assert!(artifact.contents.contains("\"omp\""));
    // Dependency-free plain ESM: node builtins only, no CommonJS require.
    assert!(!artifact.contents.contains("require("));
    assert!(artifact.contents.contains("node:child_process"));
}

#[test]
fn claude_code_declares_no_raw_file_artifacts() {
    let artifacts = providers::managed_artifacts(Provider::ClaudeCode);
    assert!(artifacts.is_empty(), "Claude Code hooks live in settings.json, not standalone files");
}
