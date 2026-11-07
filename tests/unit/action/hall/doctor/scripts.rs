#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::infra::fs;
use crate::test_support::seeded_hall;

fn put(dir: &camino::Utf8Path, name: &str, content: &str) {
    fs::ensure_dir(dir).unwrap();
    fs::write_text(&dir.join(name), content).unwrap();
}

#[test]
fn valid_scripts_and_dotfiles_are_not_findings() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    put(
        &layout.hall_scripts(),
        "ok.sh",
        "#!/bin/bash\n# description: d\n# usage: u\n",
    );
    put(&layout.hall_scripts(), ".gitkeep", "");
    assert!(script_diagnoses(&layout).is_empty());
}

#[test]
fn malformed_headers_and_bad_extensions_are_named_with_their_path() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    put(&layout.hall_scripts(), "a.sh", "echo hi\n");
    put(&layout.hall_scripts_local(), "b.py", "# description: d\n");
    put(&layout.hall_scripts(), "c.rb", "puts 1\n");

    let findings = script_diagnoses(&layout);
    let summary: Vec<_> = findings
        .iter()
        .map(|f| (f.code, f.what.contains(".ivar/scripts")))
        .collect();
    assert_eq!(
        summary,
        [
            ("script.missing_usage", true),
            ("script.missing_description", true),
            ("script.unsupported_extension", true),
        ]
    );
    assert!(findings.iter().all(|f| !f.fix.is_empty()));
}

const HEADER: &str = "#!/bin/bash\n# description: d\n# usage: u\n";

#[test]
fn a_secret_in_a_script_is_a_warning_naming_line_and_pattern_but_not_the_secret() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let secret = "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB";
    put(
        &layout.hall_scripts(),
        "leak.sh",
        &format!("{HEADER}curl -H \"x: {secret}\"\n"),
    );
    put(
        &layout.hall_scripts_local(),
        "ok.sh",
        &format!("{HEADER}echo hi\n"),
    );

    let findings = script_diagnoses(&layout);

    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = findings.first().unwrap();
    assert_eq!(finding.code, "script.secret_found");
    assert!(
        finding.what.contains(".ivar/scripts/leak.sh"),
        "{}",
        finding.what
    );
    assert!(
        finding.what.contains("line 4 (github_token)"),
        "{}",
        finding.what
    );
    assert!(!finding.what.contains(secret));
    assert!(!finding.fix.is_empty());
}

#[test]
fn a_reference_to_an_env_file_is_a_warning_with_the_line() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    put(
        &layout.hall_scripts(),
        "load.sh",
        &format!("{HEADER}set -a\nsource .env\nsource ~/.ivar/secrets/agents.env\n"),
    );
    put(
        &layout.hall_scripts(),
        "fine.sh",
        &format!("{HEADER}cp .env.example x\n"),
    );

    let findings = script_diagnoses(&layout);

    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = findings.first().unwrap();
    assert_eq!(finding.code, "script.secret_file");
    assert!(finding.what.contains("load.sh"), "{}", finding.what);
    assert!(finding.what.contains("line 5 (`.env`)"), "{}", finding.what);
    assert!(
        finding.what.contains("line 6 (`agents.env`)"),
        "{}",
        finding.what
    );
}

#[test]
fn a_script_with_a_bad_header_is_still_searched_for_secrets() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    put(&layout.hall_scripts(), "bad.sh", "source .env\n");
    put(&layout.hall_scripts(), "notes.txt", "source .env\n");

    let codes: Vec<_> = script_diagnoses(&layout).iter().map(|f| f.code).collect();

    assert_eq!(
        codes,
        [
            "script.missing_description",
            "script.secret_file",
            "script.unsupported_extension"
        ]
    );
}
