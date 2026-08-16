//! The architectural rules from `ARCHITECTURE.md`, enforced.
//!
//! A convention nobody remembers is not a boundary. This walks `src/` and its
//! mirrored unit-test tree, groups files by their top-level module, and fails
//! if any of them imports a module the rule forbids.
//!
//! It is deliberately a lexical check over `use crate::…` and `crate::…` paths
//! rather than anything clever. It cannot be fooled by a rename, and it costs
//! nothing to keep working.
//!
//! # Comments are not imports
//!
//! Lines that are comments are skipped, so an intra-doc link like
//! `[`crate::domain::name`]` from inside `git` does not read as a dependency.
//! That is the rule saying what it means: `git` may not *depend on* `domain`,
//! and it may absolutely explain that names reach it already validated by one.
//! A doc link creates no compile-time edge, and forbidding it would only make
//! the module that most needs the explanation the one that cannot give it.
//!
//! The known gap: a `use crate::…` inside a ```` ```rust ```` doc example is a
//! real dependency of the doctest, and this skips it. No module here has one,
//! and a doctest that reached across a layer would fail to compile against a
//! private item long before anyone noticed the rule was silent.
//!
//! # Physical versus compilation ownership
//!
//! A unit test's *compilation* home is the production module it tests — the
//! `#[path]`-linked `mod tests;` declaration keeps it a child of that module
//! inside the library test crate. Its *physical* home is the mirrored file
//! under `tests/unit/`. The two are deliberately different, and both rules
//! here track the physical tree:
//!
//! - the layering rule scans `tests/unit/<module>/` with the same allowed
//!   imports as `src/<module>/`, because a relocated test that imports across
//!   a layer is still a dependency the architecture forbids;
//! - a second rule scans `src/` and refuses any `#[test]`, `#[rstest]`, or
//!   inline `mod tests { … }` body there — only a path-linked semicolon
//!   declaration may keep its test module in the production tree.
//!
//! Those two rules assume the mirror itself is intact — that every file
//! under `tests/unit/` is actually reachable from a `#[path]` declaration in
//! `src/`. Nothing enforced that assumption, and it broke: a 279-line file
//! sat under `tests/unit/domain/feature/integration.rs` with no link
//! pointing at it, so `cargo test` never compiled it and every case inside
//! went unrun with no error from anyone. A third rule closes that gap by
//! comparing the physical `tests/unit/` tree against every `#[path]` string
//! in `src/`, in both directions — a file with no link, and a link to a file
//! that does not exist, are both mirror drift.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
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
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src");
    let unit = manifest.join("tests/unit");
    let mut violations = Vec::new();

    for (module, allowed) in ALLOWED {
        for dir in [src.join(module), unit.join(module)] {
            if !dir.is_dir() {
                // Slice not landed yet. Absent is fine; wrong is not.
                continue;
            }
            violations.extend(check_module_imports(manifest, &dir, module, allowed));
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

/// Check every file under `dir` against `module`'s allowed-import set.
///
/// Separate from the test body so the rule itself can be exercised against
/// synthetic text — a forbidden import in a mirrored `tests/unit/` path must
/// be caught the same way as one in `src/`.
fn check_module_imports(
    manifest: &Path,
    dir: &Path,
    module: &str,
    allowed: &[&str],
) -> Vec<String> {
    let mut violations = Vec::new();
    for file in rust_files(dir) {
        let text = fs::read_to_string(&file).unwrap();
        for imported in crate_imports(&text) {
            let permitted = imported == module
                || allowed.contains(&imported.as_str())
                || UNIVERSAL.contains(&imported.as_str());
            if !permitted {
                violations.push(format!(
                    "{}: `{module}` must not import `{imported}`",
                    display_path(manifest, &file)
                ));
            }
        }
    }
    violations
}

/// The centralization invariant: no test body may live under `src/`.
///
/// A production file may keep only a path-linked semicolon declaration:
///
/// ```rust
/// #[cfg(test)]
/// #[path = "../../tests/unit/error.rs"]
/// mod tests;
/// ```
///
/// Anything else — a `#[test]` or `#[rstest]` attribute, or an inline
/// `mod tests { … }` body — belongs in the mirrored file under `tests/unit/`.
#[test]
fn no_test_bodies_live_under_src() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src");
    let mut violations = Vec::new();

    for file in rust_files(&src) {
        let text = fs::read_to_string(&file).unwrap();
        for found in misplaced_test_bodies(&text) {
            violations.push(format!("{}: {found}", display_path(manifest, &file)));
        }
    }

    assert!(
        violations.is_empty(),
        "test-layout violations ({}):\n  {}\n\nTest bodies belong under \
         `tests/unit/`, mirrored to their owning production module; `src/` may \
         keep only a `#[cfg(test)]` `#[path]`-linked `mod tests;` declaration.",
        violations.len(),
        violations.join("\n  ")
    );
}

/// The body of the centralization rule: what a scan of `src/` must refuse.
///
/// Returns one message per offending line: a `#[test]` or `#[rstest]`
/// attribute, or an inline `mod tests { … }` body. A path-linked semicolon
/// declaration (`#[cfg(test)] #[path = "…"] mod tests;`) reports nothing.
/// Comment lines are skipped, so a doc comment is not a test body.
fn misplaced_test_bodies(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let at = index + 1;
        if trimmed.starts_with("#[test]") || trimmed.starts_with("#[rstest]") {
            found.push(format!("{at}: `{trimmed}` must live under `tests/unit/`"));
        }
        if trimmed.starts_with("mod tests {") {
            found.push(format!(
                "{at}: inline `mod tests {{ … }}` body must live under `tests/unit/`"
            ));
        }
    }
    found
}

/// The mirror invariant: every file under `tests/unit/` must be reachable
/// from a `#[path]` declaration in `src/`, or it compiles nowhere and its
/// tests never run — silently, since `cargo test` has no way to know the
/// file was supposed to exist.
///
/// This walks `tests/unit/**/*.rs` for the files that physically exist,
/// scans `src/**/*.rs` for every `#[path = "...tests/unit/..."]` string, and
/// normalises each to the same repo-relative form the physical walk
/// produces. The two sets must then be equal. `tests/support/` is not part
/// of this walk: it holds the shared fixtures linked from `src/lib.rs`'s
/// `mod test_support`, one file for the whole crate rather than a per-module
/// mirror, so it is a different mechanism and out of scope here by
/// construction — `rust_files` is only ever pointed at `tests/unit`.
#[test]
fn every_unit_test_file_is_linked_from_src() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unit = manifest.join("tests/unit");
    let src = manifest.join("src");

    let mirrored: BTreeSet<String> = rust_files(&unit)
        .iter()
        .map(|file| display_path(manifest, file))
        .collect();

    let mut linked = BTreeSet::new();
    for file in rust_files(&src) {
        let text = fs::read_to_string(&file).unwrap();
        linked.extend(path_links_to_unit_tests(&text));
    }

    let orphans: Vec<&String> = mirrored.difference(&linked).collect();
    let dangling: Vec<&String> = linked.difference(&mirrored).collect();

    assert!(
        orphans.is_empty() && dangling.is_empty(),
        "the tests/unit/ mirror has drifted from src/'s #[path] links:\n  \
         orphans, present under tests/unit/ but linked from nowhere so never \
         compiled: {orphans:?}\n  \
         dangling, linked by a #[path] whose target does not exist: {dangling:?}\n\n\
         Every file under tests/unit/ needs a `#[cfg(test)] #[path = \"…\"] \
         mod tests;` declaration in the src/ file it tests.",
    );
}

/// Pulls every `#[path = "…tests/unit/…"]` string out of a `src/` file's
/// text and normalises it to a repo-relative `tests/unit/…` path.
///
/// `#[path]` is resolved relative to the file that declares it, so the
/// literal's `../` prefix grows with directory depth — `../../tests/unit/x.rs`
/// two levels down, `../../../../tests/unit/x.rs` four levels down. There is
/// no need to count the prefix: finding the fixed substring `tests/unit` and
/// keeping the literal from there on lands on the same repo-relative form
/// regardless of depth, which is the same trick [`display_path`] uses to
/// turn an absolute filesystem path into one comparable across machines.
fn path_links_to_unit_tests(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let Some(after_attr) = line.find("#[path") else {
            continue;
        };
        let rest = &line[after_attr..];
        let Some(open) = rest.find('"') else {
            continue;
        };
        let Some(close) = rest[open + 1..].find('"') else {
            continue;
        };
        let literal = &rest[open + 1..open + 1 + close];
        if let Some(index) = literal.find("tests/unit") {
            links.push(literal[index..].to_owned());
        }
    }
    links
}

/// The layering rule extends to relocated unit tests, so a test that imports
/// across a layer is caught in `tests/unit/` exactly as it was in `src/`.
#[test]
fn relocated_unit_tests_are_scanned_with_their_production_module() {
    // A `store` test importing `crate::action` would be a layering violation;
    // the scanner must see it through the mirrored path.
    let text = "use crate::action::sync;\nuse crate::store::layout::Layout;\n";
    let imports = crate_imports(text);
    assert!(imports.contains(&"action".to_owned()));
    assert!(imports.contains(&"store".to_owned()));
}

/// Synthetic regression: a forbidden `crate::store` import inside a mirrored
/// `tests/unit/domain/` file is reported by the same check that scans `src/`.
#[test]
fn a_forbidden_import_in_the_mirrored_unit_tree_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let mirrored = dir.path().join("tests/unit/domain");
    std::fs::create_dir_all(&mirrored).unwrap();
    std::fs::write(
        mirrored.join("fixture.rs"),
        "use crate::store::manifest::Manifest;\n",
    )
    .unwrap();

    // `domain` may import nothing (besides the universal `error`/`test_support`),
    // so a `store` import is a violation wherever the file lives.
    let violations = check_module_imports(dir.path(), &mirrored, "domain", &[]);
    let [violation] = violations.as_slice() else {
        panic!("one import, one violation: {violations:?}");
    };

    assert!(
        violation.contains("tests/unit/domain/fixture.rs"),
        "the message must name the mirrored path: {violation}"
    );
    assert!(
        violation.contains("`domain` must not import `store`"),
        "the message must name module and import: {violation}"
    );
}

/// Synthetic regression: an inline `#[test]` or `#[rstest]` in the scanner's
/// input is reported, and a path-linked semicolon declaration is not.
#[test]
fn the_scanner_reports_test_attributes_and_inline_bodies() {
    let reported = misplaced_test_bodies(
        "#[cfg(test)]\n#[path = \"../../tests/unit/error.rs\"]\nmod tests;\n\
         #[test]\nfn x() {}\n\
         #[rstest]\nfn y() {}\n\
         mod tests { let _ = 1; }\n\
         // #[test] in a comment is not a body\n",
    );
    assert!(
        reported.iter().any(|m| m.contains("#[test]")),
        "a #[test] attribute must be reported: {reported:?}"
    );
    assert!(
        reported.iter().any(|m| m.contains("#[rstest]")),
        "a #[rstest] attribute must be reported: {reported:?}"
    );
    assert!(
        reported.iter().any(|m| m.contains("mod tests {")),
        "an inline mod tests body must be reported: {reported:?}"
    );
    assert_eq!(
        reported.len(),
        3,
        "exactly the three offending lines, not the path-linked declaration or \
         the comment: {reported:?}"
    );
}

/// The path under the manifest, for readable failure messages.
fn display_path(manifest: &Path, file: &Path) -> String {
    file.strip_prefix(manifest)
        .unwrap_or(file)
        .display()
        .to_string()
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

/// Pulls the first path segment out of every `crate::<segment>` occurrence in
/// code. Comment lines are skipped — see the module doc comment.
fn crate_imports(text: &str) -> Vec<String> {
    let mut modules = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for (index, _) in line.match_indices("crate::") {
            let rest = &line[index + "crate::".len()..];
            let segment: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !segment.is_empty() && !modules.contains(&segment) {
                modules.push(segment);
            }
        }
    }
    modules
}
