#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::action::provider::add::{AddInput, add};
use crate::domain::name::SessionId;
use crate::domain::session::SessionState;
use crate::error::Status;
use crate::infra::fs;
use crate::store::layout::Layout;
use crate::test_support::{hall_root, seeded_hall};

fn register(ctx: &Ctx, name: &str) {
    add(
        ctx,
        &AddInput {
            name: name.to_owned(),
        },
    )
    .unwrap();
}

fn input(name: &str, default: Option<&str>) -> RemoveInput {
    RemoveInput {
        name: name.to_owned(),
        default: default.map(str::to_owned),
    }
}

fn persisted(root: &Utf8PathBuf) -> (Vec<Provider>, Provider) {
    let manifest = Manifest::read(&Layout::at(root.clone())).unwrap().unwrap();
    (
        manifest.providers().available().to_vec(),
        manifest.providers().default_provider(),
    )
}

fn manifest_bytes(root: &Utf8PathBuf) -> String {
    fs::read_text(&root.join("ivar.json")).unwrap().unwrap()
}

#[test]
fn remove_unregisters_a_provider_and_keeps_the_default() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    register(&ctx, "opencode");

    let report = remove(&ctx, &input("opencode", None)).unwrap();

    assert_eq!(report.value.provider, Provider::OpenCode);
    assert_eq!(report.value.available, vec![Provider::ClaudeCode]);
    assert_eq!(report.value.default, Provider::ClaudeCode);
    assert_eq!(
        persisted(&root),
        (vec![Provider::ClaudeCode], Provider::ClaudeCode)
    );
    for command in crate::harness::commands::catalog() {
        assert!(
            !root
                .join(".opencode/commands")
                .join(command.file_name())
                .exists(),
            "{} must be torn down with the provider",
            command.file_name()
        );
    }
}

#[test]
fn removing_the_default_with_a_new_default_tears_down_its_settings() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    crate::action::sync::sync(&ctx, &crate::action::sync::SyncInput::default()).unwrap();
    register(&ctx, "opencode");
    assert!(root.join(".claude/settings.json").is_file());

    let report = remove(&ctx, &input("claude-code", Some("opencode"))).unwrap();

    assert_eq!(report.value.default, Provider::OpenCode);
    assert_eq!(
        persisted(&root),
        (vec![Provider::OpenCode], Provider::OpenCode)
    );
    assert!(!root.join(".claude/settings.json").exists());
}

#[test]
fn default_flag_switches_the_default_when_removing_another_provider() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    register(&ctx, "opencode");
    register(&ctx, "omp");

    remove(&ctx, &input("omp", Some("opencode"))).unwrap();

    assert_eq!(
        persisted(&root),
        (
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::OpenCode
        )
    );
}

#[test]
fn removing_omp_keeps_the_agents_alias_opencode_still_owns() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    register(&ctx, "opencode");
    register(&ctx, "omp");

    remove(&ctx, &input("omp", None)).unwrap();

    assert_eq!(
        fs::read_symlink(&root.join("AGENTS.md")).unwrap(),
        fs::SymlinkTarget::Target(Utf8PathBuf::from("HALL.md"))
    );
}

#[test]
fn a_live_session_of_the_removed_provider_is_warned_not_blocking() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());
    register(&ctx, "opencode");
    let id = SessionId::new("2c6e6f1e-2d8a-4b3a-9c2a-6a7f6f9a1b2c").unwrap();
    let view_dir = Layout::at(root.clone()).discovery_session(&id);
    fs::ensure_dir(&view_dir).unwrap();
    SessionState::new(Provider::ClaudeCode, "2026-09-29T00:00:00Z")
        .write(&view_dir)
        .unwrap();

    let report = remove(&ctx, &input("claude-code", Some("opencode"))).unwrap();

    let warning = report
        .warnings
        .iter()
        .find(|w| w.code == "provider.session_unguarded")
        .expect("the live claude-code session must be reported");
    assert_eq!(warning.subject, id.as_str());
    assert!(warning.what.contains("ivar guard"));
    assert_eq!(persisted(&root).1, Provider::OpenCode);
}

#[test]
fn refusals_leave_ivar_json_untouched() {
    let (_guard, root) = seeded_hall();
    let ctx = Ctx::new(root.clone());

    let cases: [(RemoveInput, &str); 2] = [
        (input("bogus", None), "provider.unknown_id"),
        (input("opencode", None), "provider.not_available"),
    ];
    for (request, code) in cases {
        let before = manifest_bytes(&root);
        let failure = remove(&ctx, &request).unwrap_err();
        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, code);
        assert_eq!(manifest_bytes(&root), before, "{code} must not write");
    }

    let before = manifest_bytes(&root);
    let failure = remove(&ctx, &input("claude-code", None)).unwrap_err();
    assert_eq!(failure.code, "provider.last_provider");
    assert_eq!(manifest_bytes(&root), before);

    register(&ctx, "opencode");
    let cases: [(RemoveInput, &str); 3] = [
        (input("claude-code", None), "provider.default_required"),
        (
            input("claude-code", Some("claude-code")),
            "provider.invalid_default",
        ),
        (
            input("claude-code", Some("omp")),
            "provider.invalid_default",
        ),
    ];
    for (request, code) in cases {
        let before = manifest_bytes(&root);
        let failure = remove(&ctx, &request).unwrap_err();
        assert_eq!(failure.status, Status::Blocked);
        assert_eq!(failure.code, code);
        assert_eq!(manifest_bytes(&root), before, "{code} must not write");
    }
}

#[test]
fn remove_outside_a_hall_is_blocked() {
    let (_guard, root) = hall_root();

    let failure = remove(&Ctx::new(root), &input("opencode", None)).unwrap_err();

    assert_eq!(failure.code, "hall.not_found");
}

#[test]
fn the_human_surface_names_the_removed_provider() {
    let outcome = RemoveOutcome {
        root: Utf8PathBuf::from("/hall"),
        provider: Provider::Omp,
        available: vec![Provider::ClaudeCode, Provider::OpenCode],
        default: Provider::ClaudeCode,
    };

    let mut out = Vec::new();
    outcome.write_human(&mut out).unwrap();

    assert_eq!(
        String::from_utf8(out).unwrap(),
        "Removed provider `omp` from /hall — available: claude-code, opencode \
             (default: claude-code).\n"
    );
}
