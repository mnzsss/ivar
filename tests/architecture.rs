//! The layering rule from `ARCHITECTURE.md`, enforced.
//!
//! A convention nobody remembers is not a boundary. This walks `src/`, groups
//! files by their top-level module, and fails if any of them imports a module the
//! rule forbids.
//!
//! It is deliberately a lexical check over `use crate::…` and `crate::…` paths
//! rather than anything clever. It cannot be fooled by a rename, and it costs
//! nothing to keep working.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

/// `(module, modules it may import)`. Anything not listed is forbidden.
/// `error` is importable from everywhere and is therefore omitted from the lists.
const ALLOWED: &[(&str, &[&str])] = &[
    ("cli", &["action"]),
    (
        "action",
        &["domain", "store", "git", "harness", "tui", "infra"],
    ),
    ("domain", &[]),
    ("store", &["domain", "infra"]),
    ("git", &["infra"]),
    ("harness", &["domain", "infra"]),
    ("tui", &["domain", "infra"]),
    ("infra", &[]),
];

/// Importable from anywhere. `error` is the bottom of the layering;
/// `test_support` is `cfg(test)` scaffolding and carries no production rules.
const UNIVERSAL: &[&str] = &["error", "test_support"];

#[test]
fn module_imports_respect_the_layering_rule() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();

    for (module, allowed) in ALLOWED {
        let dir = src.join(module);
        if !dir.is_dir() {
            // Slice not landed yet. Absent is fine; wrong is not.
            continue;
        }

        for file in rust_files(&dir) {
            let text = fs::read_to_string(&file).unwrap();
            for imported in crate_imports(&text) {
                let permitted = imported == *module
                    || allowed.contains(&imported.as_str())
                    || UNIVERSAL.contains(&imported.as_str());
                if !permitted {
                    violations.push(format!(
                        "{}: `{module}` must not import `{imported}`",
                        file.strip_prefix(&src).unwrap().display()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "layering violations ({}):\n  {}\n\nSee the table in ARCHITECTURE.md. \
         If the rule is wrong, change the rule in a commit that says so.",
        violations.len(),
        violations.join("\n  ")
    );
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// Pulls the first path segment out of every `crate::<segment>` occurrence.
fn crate_imports(text: &str) -> Vec<String> {
    let mut modules = Vec::new();
    for (index, _) in text.match_indices("crate::") {
        let rest = &text[index + "crate::".len()..];
        let segment: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !segment.is_empty() && !modules.contains(&segment) {
            modules.push(segment);
        }
    }
    modules
}
