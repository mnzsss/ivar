// tests/unit/providers/hook.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use crate::domain::provider::Provider;
use crate::providers;
use camino::Utf8PathBuf;

#[test]
fn opencode_plugin_bytes_characterization_unchanged() {
    let artifacts = providers::managed_artifacts(Provider::OpenCode);
    assert_eq!(
        artifacts.len(),
        1,
        "OpenCode must declare exactly 1 managed artifact"
    );
    let artifact = &artifacts[0];
    assert_eq!(
        artifact.relative_path,
        Utf8PathBuf::from(".opencode/plugins/ivar.js")
    );
    assert!(
        artifact
            .contents
            .contains("ivar session plugin for OpenCode")
    );
    assert!(artifact.contents.contains("shell.env"));
    assert!(artifact.contents.contains("tool.execute.before"));
    assert!(artifact.contents.contains("ivar guard --provider opencode"));
    assert!(artifact.contents.contains("ivar session env --json --cwd"));
}

#[test]
fn omp_hook_artifact_declared_at_exact_path_and_dependency_free() {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let hook_path = Utf8PathBuf::from(".omp/hooks/pre/ivar.js");
    let artifact = artifacts
        .iter()
        .find(|a| a.relative_path == hook_path)
        .unwrap_or_else(|| panic!("expected artifact at {hook_path}, found: {artifacts:?}"));
    assert!(
        artifact
            .contents
            .contains("// ivar pre-tool guard hook for OMP")
    );
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
    assert!(
        artifacts.is_empty(),
        "Claude Code hooks live in settings.json, not standalone files"
    );
}

#[test]
fn omp_managed_artifacts_findable_by_path() {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let hook_path = camino::Utf8Path::new(".omp/hooks/pre/ivar.js");
    let hook = artifacts
        .iter()
        .find(|a| a.relative_path == hook_path)
        .unwrap_or_else(|| panic!("missing artifact at {hook_path}"));
    assert!(
        hook.contents
            .contains("// ivar pre-tool guard hook for OMP")
    );
    // Path that is not registered fails lookup
    let missing_path = camino::Utf8Path::new(".omp/hooks/pre/nonexistent.js");
    let missing = artifacts.iter().find(|a| a.relative_path == missing_path);
    assert!(
        missing.is_none(),
        "unregistered path should not be found in managed artifacts"
    );
}

#[test]
fn omp_hook_sends_the_agent_and_returns_the_guard_stdout_as_context() {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let hook = artifacts
        .iter()
        .find(|a| a.relative_path == ".omp/hooks/pre/ivar.js")
        .unwrap();
    let js = hook.contents;
    // Each subagent keys its own delivery state.
    assert!(js.contains(r#"agent: ctx?.agent?.id ?? "main""#), "{js}");
    // A non-blank stdout on exit 0 becomes the tool call's extra context...
    assert!(js.contains("return { additionalContext: out };"), "{js}");
    // ...and a non-zero exit still blocks.
    assert!(js.contains("return { block: true, reason };"), "{js}");
}

#[test]
fn opencode_plugin_appends_the_guard_stdout_to_the_tool_output() {
    let artifacts = providers::managed_artifacts(Provider::OpenCode);
    let js = artifacts[0].contents;
    // The session resolves even though opencode sends no cwd.
    assert!(
        js.contains("cwd: input.cwd || output.cwd || process.cwd(),"),
        "{js}"
    );
    assert!(js.contains("agent: input.sessionID,"), "{js}");
    // `before` keeps the stdout per call; `after` appends it.
    assert!(js.contains("contexts.set(input.callID, context);"), "{js}");
    assert!(
        js.contains(r#""tool.execute.after": async (input, output) => {"#),
        "{js}"
    );
    assert!(
        js.contains(
            "output.output += `\\n\\n<system-reminder>\\n${context}\\n</system-reminder>`;"
        ),
        "{js}"
    );
    assert!(js.contains("contexts.delete(input.callID);"), "{js}");
    // opencode 1.18 loads a named async plugin function, not a default object.
    assert!(
        js.contains("export const IvarPlugin = async () => ({"),
        "{js}"
    );
    assert!(!js.contains("export default"), "{js}");
}

#[test]
fn capture_artifacts_are_separate_from_the_guard_artifacts() {
    assert!(providers::capture_artifacts(Provider::ClaudeCode).is_empty());
    assert!(providers::capture_artifacts(Provider::T3).is_empty());

    let omp = providers::capture_artifacts(Provider::Omp);
    assert_eq!(omp.len(), 1);
    assert_eq!(
        omp[0].relative_path,
        Utf8PathBuf::from(".omp/hooks/pre/ivar-memory.js")
    );

    let opencode = providers::capture_artifacts(Provider::OpenCode);
    assert_eq!(opencode.len(), 1);
    assert_eq!(
        opencode[0].relative_path,
        Utf8PathBuf::from(".opencode/plugins/ivar-memory.js")
    );
    assert_eq!(providers::managed_artifacts(Provider::OpenCode).len(), 1);
}

#[test]
fn the_omp_capture_artifact_sits_in_the_directory_session_views_mirror() {
    let mirrored = camino::Utf8Path::new(Provider::OMP_HOOKS_DIR).join("pre");
    let artifact = providers::capture_artifacts(Provider::Omp).remove(0);
    assert_eq!(artifact.relative_path.parent(), Some(mirrored.as_path()));
}

#[test]
fn the_omp_capture_hook_is_pure_esm_registers_the_native_events_and_swallows_errors() {
    let hook = providers::capture_artifacts(Provider::Omp)
        .remove(0)
        .contents;
    assert!(hook.contains("export default"));
    assert!(!hook.contains("require("));
    assert!(hook.contains("node:child_process"));
    for event in [
        "session_start",
        "before_agent_start",
        "tool_result",
        "session_shutdown",
    ] {
        assert!(hook.contains(&format!("pi.on(\"{event}\"")), "{event}");
    }
    assert!(
        !hook.contains("tool_call"),
        "capture must not touch the pre-tool event"
    );
    assert!(hook.contains("\"memory\", \"capture\", \"--provider\", \"omp\""));
    assert!(
        hook.matches("catch").count() >= 4,
        "every handler swallows its errors"
    );
}

#[test]
fn the_opencode_capture_plugin_has_no_session_end_and_never_blocks() {
    let plugin = providers::capture_artifacts(Provider::OpenCode)
        .remove(0)
        .contents;
    assert!(plugin.contains("export const IvarMemoryPlugin"));
    assert!(plugin.contains("\"chat.message\""));
    assert!(plugin.contains("\"tool.execute.after\""));
    assert!(plugin.contains("session.created"));
    assert!(
        !plugin.contains("session.idle"),
        "idle is a turn boundary, not a session end"
    );
    assert!(plugin.contains("\"memory\", \"capture\", \"--provider\", \"opencode\""));
    assert!(!plugin.contains("throw"));
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

const OVERSIZED_EVENT_LIMIT: usize = 100_000;

/// Loads the artifact as an ES module in `node`, with a stand-in `ivar` that
/// writes its stdin to a file, runs `driver` against it and returns what the
/// spawned capture process received.
fn captured_stdin(contents: &str, driver: &str) -> serde_json::Value {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let fake = bin.join("ivar");
    std::fs::write(&fake, "#!/bin/sh\ncat > \"$CAPTURED\"\n").unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(tmp.path().join("hook.mjs"), contents).unwrap();
    std::fs::write(tmp.path().join("driver.mjs"), driver).unwrap();
    let captured = tmp.path().join("captured.json");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());

    let status = std::process::Command::new("node")
        .arg(tmp.path().join("driver.mjs"))
        .env("PATH", path)
        .env("CAPTURED", &captured)
        .status()
        .unwrap();
    assert!(status.success());

    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&captured)
            && let Ok(value) = serde_json::from_str(&text)
        {
            return value;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("the capture process never received a complete payload");
}

#[test]
fn the_omp_capture_hook_bounds_an_oversized_tool_result_before_serialising_it() {
    if !node_available() {
        return;
    }
    let driver = r#"
import ivarMemory from "./hook.mjs";
const handlers = {};
ivarMemory({ on: (name, handler) => { handlers[name] = handler; } });
await handlers.tool_result({
  toolName: "bash",
  input: { command: "x".repeat(5_000_000) },
  content: [{ type: "text", text: "y".repeat(5_000_000) }],
}, { cwd: "/tmp" });
"#;
    let contents = providers::capture_artifacts(Provider::Omp)
        .remove(0)
        .contents;

    let payload = captured_stdin(contents, driver);

    assert_eq!(payload["event"], "tool");
    assert!(payload.to_string().len() < OVERSIZED_EVENT_LIMIT);
    assert!(payload["result"].as_str().unwrap().contains("yyyy"));
}

#[test]
fn the_opencode_capture_plugin_bounds_an_oversized_tool_result_before_serialising_it() {
    if !node_available() {
        return;
    }
    let driver = r#"
import { IvarMemoryPlugin } from "./hook.mjs";
const hooks = await IvarMemoryPlugin();
await hooks["tool.execute.after"](
  { sessionID: "s", tool: "bash", args: { command: "x".repeat(5_000_000) } },
  { output: "y".repeat(5_000_000) },
);
"#;
    let contents = providers::capture_artifacts(Provider::OpenCode)
        .remove(0)
        .contents;

    let payload = captured_stdin(contents, driver);

    assert_eq!(payload["event"], "tool");
    assert!(payload.to_string().len() < OVERSIZED_EVENT_LIMIT);
    assert!(payload["result"].as_str().unwrap().contains("yyyy"));
}

#[test]
fn both_capture_artifacts_are_valid_javascript_modules() {
    if !node_available() {
        return;
    }
    for (name, artifact) in [
        ("omp.mjs", providers::capture_artifacts(Provider::Omp)),
        (
            "opencode.mjs",
            providers::capture_artifacts(Provider::OpenCode),
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(name);
        std::fs::write(&file, artifact[0].contents).unwrap();

        let output = std::process::Command::new("node")
            .arg("--check")
            .arg(&file)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn omp_brief_hook_injects_once_through_before_agent_start() {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let path = Utf8PathBuf::from(".omp/hooks/pre/ivar-brief.js");
    let artifact = artifacts
        .iter()
        .find(|a| a.relative_path == path)
        .unwrap_or_else(|| panic!("expected artifact at {path}, found: {artifacts:?}"));
    let js = artifact.contents;

    assert!(js.contains("export default"), "{js}");
    assert!(js.contains("pi.on(\"before_agent_start\""), "{js}");
    assert!(js.contains("let shown = false;"), "{js}");
    assert!(js.contains("if (shown) return;"), "{js}");
    assert!(
        js.contains("[\"memory\", \"brief\", \"--provider\", \"omp\"]"),
        "{js}"
    );
    assert!(js.contains("customType: \"ivar-brief\""), "{js}");
    assert!(!js.contains("require("), "{js}");
    assert!(
        providers::capture_artifacts(Provider::Omp)
            .iter()
            .all(|a| a.relative_path != path)
    );
}
