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
