// tests/unit/providers/extension.rs
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Executes the embedded `.omp/extensions/ivar.js` in `node` against a fake
//! omp extension API, so the tests observe what omp would register and send.

use camino::Utf8Path;
use serde_json::{Value, json};

use crate::domain::provider::Provider;
use crate::providers;
use crate::test_support::utf8_temp_dir;

/// Fake omp `ExtensionAPI`. argv: extension path, commands dir, JSON steps.
/// A step is `{"invoke":[name,args]}` or `{"write":[file,content]}`.
const HARNESS: &str = r#"
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [, , extensionPath, commandsDir, stepsJson] = process.argv;
const commands = new Map();
const sent = [];
const events = [];
const pi = {
  registerCommand(name, options) { commands.set(name, options); },
  sendUserMessage(content) { sent.push(content); },
  on(event, handler) { events.push([event, handler]); },
};
const { default: factory } = await import(pathToFileURL(extensionPath).href);
factory(pi);

const descriptions = {};
for (const [name, options] of commands) descriptions[name] = options.description ?? null;

let autocomplete = false;
for (const [event, handler] of events) {
  if (event !== "session_start") continue;
  let wrap = null;
  await handler({}, { ui: { addAutocompleteProvider(f) { wrap = f; } } });
  if (wrap) {
    const current = { getSuggestions: async () => ({ items: [{ value: "from-current" }], prefix: "" }) };
    const result = await wrap(current).getSuggestions(["hello"], 0, 5, undefined);
    autocomplete = result?.items?.[0]?.value === "from-current";
  }
}

const ctx = { ui: { notify() {} } };
for (const step of JSON.parse(stepsJson)) {
  if (step.invoke) await commands.get(step.invoke[0]).handler(step.invoke[1], ctx);
  if (step.write) writeFileSync(join(commandsDir, step.write[0]), step.write[1]);
}

console.log(JSON.stringify({ names: [...commands.keys()].sort(), descriptions, sent, autocomplete }));
"#;

/// Lays out `<root>/.omp/extensions/ivar.js` exactly where `ivar sync` puts
/// it, plus `<root>/.omp/commands/<file>` for each `(file, content)`, and runs
/// the harness. `commands: None` leaves the commands dir absent.
fn run_extension(commands: Option<&[(&str, &str)]>, steps: &Value) -> Value {
    let artifacts = providers::managed_artifacts(Provider::Omp);
    let ext_rel = Utf8Path::new(".omp/extensions/ivar.js");
    let extension = artifacts
        .iter()
        .find(|a| a.relative_path == ext_rel)
        .unwrap_or_else(|| panic!("expected artifact at {ext_rel}, found: {artifacts:?}"));

    let (_guard, root) = utf8_temp_dir();
    let ext_path = root.join(ext_rel);
    std::fs::create_dir_all(ext_path.parent().unwrap()).unwrap();
    std::fs::write(&ext_path, extension.contents).unwrap();
    let commands_dir = root.join(".omp/commands");
    if let Some(files) = commands {
        std::fs::create_dir_all(&commands_dir).unwrap();
        for (name, content) in files {
            std::fs::write(commands_dir.join(name), content).unwrap();
        }
    }
    let harness = root.join("harness.mjs");
    std::fs::write(&harness, HARNESS).unwrap();

    let output = std::process::Command::new("node")
        .arg(&harness)
        .arg(&ext_path)
        .arg(&commands_dir)
        .arg(steps.to_string())
        .env_remove("IVAR_FEATURE")
        .output()
        .unwrap_or_else(|e| panic!("node is required to test the embedded omp extension: {e}"));
    assert!(
        output.status.success(),
        "extension harness failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

const CONNECT: &str = "---\ndescription: Attach to a session\nargument-hint: <feature-name>\n---\nRun `ivar session connect --feature $ARGUMENTS --create`.\n";
const SYNC: &str = "---\ndescription: Sync the hall\n---\nSync body.\n";

#[test]
fn registers_one_command_per_ivar_file_with_its_description() {
    let out = run_extension(
        Some(&[
            ("ivar-connect.md", CONNECT),
            ("ivar-sync.md", SYNC),
            ("ivar-broken.md", "---\ndescription: never closed\nbody\n"),
            ("notes.md", "---\ndescription: not ours\n---\nx\n"),
        ]),
        &json!([]),
    );

    assert_eq!(out["names"], json!(["ivar-connect", "ivar-sync"]));
    assert_eq!(out["descriptions"]["ivar-connect"], "Attach to a session");
    assert_eq!(out["descriptions"]["ivar-sync"], "Sync the hall");
    assert_eq!(out["autocomplete"], true);
}

#[test]
fn invocation_expands_arguments_and_rereads_the_file() {
    let out = run_extension(
        Some(&[("ivar-connect.md", CONNECT), ("ivar-sync.md", SYNC)]),
        &json!([
            {"invoke": ["ivar-connect", "  demo-feature  "]},
            {"invoke": ["ivar-connect", ""]},
            {"invoke": ["ivar-sync", ""]},
            {"invoke": ["ivar-sync", "extra words"]},
            {"write": ["ivar-connect.md", "---\ndescription: changed\n---\nRewritten $ARGUMENTS\n"]},
            {"invoke": ["ivar-connect", "x"]},
        ]),
    );

    assert_eq!(
        out["sent"],
        json!([
            "Run `ivar session connect --feature demo-feature --create`.\n",
            "Run `ivar session connect --feature  --create`.\n",
            "Sync body.\n",
            "Sync body.\n\nextra words",
            "Rewritten x\n",
        ])
    );
}

#[test]
fn missing_commands_dir_registers_nothing_and_keeps_autocomplete() {
    let out = run_extension(None, &json!([]));

    assert_eq!(out["names"], json!([]));
    assert_eq!(out["autocomplete"], true);
}
