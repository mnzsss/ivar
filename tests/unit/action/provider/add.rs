#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::hall::{self, InitInput};
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::hall_root;

fn seeded_hall() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = hall_root();
    let ctx = Ctx::new(root.clone());
    hall::init(
        &ctx,
        InitInput {
            path: Utf8PathBuf::from("."),
            name: Some("acme".to_owned()),
            provider: None,
        },
    )
    .unwrap();
    (guard, root)
}

fn add_input(name: &str) -> AddInput {
    AddInput {
        name: name.to_owned(),
    }
}

/// The providers recorded in ivar.json, read back off disk.
fn persisted_available(root: &Utf8PathBuf) -> Vec<Provider> {
    let layout = Layout::at(root.clone());
    Manifest::read(&layout)
        .unwrap()
        .unwrap()
        .providers()
        .available()
        .to_vec()
}

#[test]
fn add_registers_a_provider_and_keeps_the_default() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, add_input("opencode")).unwrap();

    assert!(report.is_clean());
    assert_eq!(report.value.provider, Provider::OpenCode);
    assert_eq!(
        report.value.available,
        vec![Provider::ClaudeCode, Provider::OpenCode]
    );
    assert_eq!(report.value.default, Provider::ClaudeCode);
    assert_eq!(
        persisted_available(&root),
        vec![Provider::ClaudeCode, Provider::OpenCode],
        "ivar.json must record the new provider"
    );
}

#[test]
fn provider_add_materialises_the_new_providers_commands() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, add_input("opencode")).unwrap();

    assert!(report.is_clean());
    for command in crate::harness::commands::catalog() {
        assert!(
            std::path::Path::new(&root)
                .join(".opencode/commands")
                .join(command.file_name())
                .is_file(),
            "{} should be materialised immediately, without a follow-up sync",
            command.file_name()
        );
    }
}

#[test]
fn provider_add_creates_the_new_aliases_through_the_shared_reconciler() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, add_input("opencode")).unwrap();

    assert!(report.is_clean());
    assert_eq!(
        fs::read_symlink(&root.join("AGENTS.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md")),
        "adding OpenCode must immediately create its relative alias"
    );
    // The existing Claude alias is untouched.
    assert_eq!(
        fs::read_symlink(&root.join("CLAUDE.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
}

#[test]
fn provider_add_conflict_warns_and_keeps_the_provider_persisted() {
    let (_guard, root) = seeded_hall();
    // An occupied AGENTS.md the reconciler must preserve byte for byte.
    fs::write_text(&root.join("AGENTS.md"), "legacy, precious\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, add_input("opencode")).unwrap();

    assert!(!report.is_clean(), "a conflict must not be a clean run");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "instructions.adoption_required"),
        "warnings: {:?}",
        report.warnings
    );
    assert_eq!(
        fs::read_text(&root.join("AGENTS.md")).unwrap().unwrap(),
        "legacy, precious\n"
    );
    assert_eq!(
        persisted_available(&root),
        vec![Provider::ClaudeCode, Provider::OpenCode],
        "the provider must stay registered even when its alias conflicted"
    );
}

#[test]
fn provider_add_returns_warning_when_commands_cannot_be_written() {
    let (_guard, root) = seeded_hall();
    // Occupy OpenCode's command-directory parent with a regular file, so
    // `ensure_dir` cannot create `.opencode/commands` under it.
    fs::write_text(&root.join(".opencode"), "not a directory\n").unwrap();
    let ctx = Ctx::new(root.clone());

    let report = add(&ctx, add_input("opencode")).unwrap();

    assert!(
        !report.is_clean(),
        "a failed command write must not be clean"
    );
    assert_eq!(
        report.warnings[0].code,
        "provider.commands_not_materialised"
    );
    assert_eq!(report.warnings[0].subject, "opencode");
    assert!(report.warnings[0].what.contains("ivar sync"));
    assert_eq!(
        persisted_available(&root),
        vec![Provider::ClaudeCode, Provider::OpenCode],
        "the provider must stay registered even when its commands could not be written"
    );
}

#[test]
fn add_refuses_an_unknown_provider() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let failure = add(&ctx, add_input("bogus")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "provider.unknown_id");
    assert!(
        failure.what.contains("claude-code") && failure.what.contains("opencode"),
        "the refusal must name the valid ids: {}",
        failure.what
    );
    assert_eq!(
        persisted_available(&root),
        vec![Provider::ClaudeCode],
        "an unknown provider must not touch ivar.json"
    );
}

#[test]
fn add_does_not_duplicate_an_existing_provider() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    add(&ctx, add_input("opencode")).unwrap();

    let failure = add(&ctx, add_input("opencode")).unwrap_err();

    assert_eq!(failure.status, Status::Blocked);
    assert_eq!(failure.code, "provider.already_available");
    assert_eq!(
        persisted_available(&root),
        vec![Provider::ClaudeCode, Provider::OpenCode],
        "a duplicate add must not change ivar.json"
    );
}

#[test]
fn add_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();
    let ctx = Ctx::new(root);

    let failure = add(&ctx, add_input("opencode")).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn the_human_surface_names_the_registered_provider() {
    let outcome = AddOutcome {
        root: Utf8PathBuf::from("/hall"),
        provider: Provider::OpenCode,
        available: vec![Provider::ClaudeCode, Provider::OpenCode],
        default: Provider::ClaudeCode,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Registered provider `opencode` in /hall — available: claude-code, opencode \
             (default: claude-code).\n"
    );
}
