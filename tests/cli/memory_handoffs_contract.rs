#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::{Utf8Path, Utf8PathBuf};
use ivar::domain::capability::{Support, provider_support};
use ivar::domain::provider::Provider;

use crate::common::{hall_root, ivar};

fn sync_hall(provider: Provider) -> (tempfile::TempDir, Utf8PathBuf) {
    let mut available = vec!["claude-code", provider.id()];
    available.dedup();
    let (guard, root) = hall_with(&available, provider);
    ivar().current_dir(&root).arg("sync").assert().success();
    (guard, root)
}

fn hall_with(available: &[&str], provider: Provider) -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    ivar().current_dir(&root).arg("init").assert().success();
    let path = root.join("ivar.json");
    let mut manifest: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest.insert("name".to_owned(), "acme".into());
    manifest.insert(
        "providers".to_owned(),
        serde_json::json!({"available": available, "default": provider.id()}),
    );
    std::fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    (guard, root)
}

fn brief_materialised(root: &Utf8Path, provider: Provider) -> bool {
    match provider {
        Provider::ClaudeCode | Provider::T3 => {
            std::fs::read_to_string(root.join(".claude/settings.json"))
                .unwrap()
                .contains("ivar memory brief --provider claude-code")
        }
        Provider::Omp => root.join(".omp/hooks/pre/ivar-brief.js").is_file(),
        Provider::OpenCode => walkdir::WalkDir::new(root.join(provider.config_dir()))
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .any(|entry| {
                std::fs::read_to_string(entry.path())
                    .is_ok_and(|text| text.contains("memory brief"))
            }),
    }
}

#[test]
fn the_brief_is_materialised_exactly_where_the_matrix_says_supported() {
    for provider in Provider::SELECTABLE {
        let (_guard, root) = sync_hall(provider);
        let expected = matches!(provider_support(provider).memory_brief, Support::Supported);
        assert_eq!(brief_materialised(&root, provider), expected, "{provider}");
    }
}

#[test]
fn t3_is_never_synced_without_the_claude_code_settings_that_carry_its_brief() {
    let (_guard, root) = hall_with(&["t3"], Provider::T3);

    ivar()
        .current_dir(&root)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicates::str::contains("needs `claude-code`"));
    assert!(!root.join(".claude/settings.json").exists());
}

#[test]
fn an_open_hall_handoff_reaches_the_brief_of_every_provider_that_has_one() {
    for provider in Provider::SELECTABLE {
        if !matches!(provider_support(provider).memory_brief, Support::Supported) {
            continue;
        }
        let (_guard, root) = sync_hall(provider);
        ivar()
            .current_dir(&root)
            .env_remove("IVAR_SESSION_ID")
            .args([
                "memory",
                "handoff",
                "begin",
                "--summary",
                "Resume the checkout refactor",
                "--hall",
            ])
            .assert()
            .success();

        let out = ivar()
            .current_dir(&root)
            .env_remove("IVAR_SESSION_ID")
            .args(["memory", "brief", "--provider", provider.id()])
            .write_stdin(serde_json::json!({"cwd": root.as_str()}).to_string())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("Resume the checkout refactor"),
            "{provider}: {text}"
        );
    }
}
