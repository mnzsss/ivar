#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::utf8_temp_dir;
use std::collections::BTreeSet;

#[test]
fn catalog_is_complete_unique_and_current() {
    let commands = catalog();
    assert_eq!(commands.len(), 15);

    let ids = commands
        .iter()
        .map(|command| command.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), commands.len());

    for command in commands {
        assert_eq!(command.file_name(), format!("ivar-{}.md", command.id));
        assert!(command.content.starts_with("---\n"));
        assert!(command.content.contains("description:"));
        assert!(command.content.contains("`ivar "));
        // Every source names its own user-facing command, so a provider
        // (and a reader) always knows what it is being invoked as.
        let slash_name = format!("/ivar-{}", command.id);
        assert!(
            command.content.contains(&slash_name),
            "{0} must name `{slash_name}` in its content",
            command.id
        );
        assert!(!command.content.contains("bifrost"));
        assert!(!command.content.contains("BIFROST_"));
    }
}

/// The relations command is the fifteenth: it has no Bifrost-era predecessor,
/// so it carries no legacy fingerprint — and every other command keeps its
/// exact digest, which is what legacy cleanup still recognises.
#[test]
fn relations_is_the_fifteenth_command_without_a_legacy_fingerprint() {
    let commands = catalog();

    let relations = commands
        .iter()
        .find(|command| command.id == "relations")
        .expect("relations is in the catalog");
    assert_eq!(relations.legacy_sha256, None);
    assert_eq!(relations.file_name(), "ivar-relations.md");
    assert_eq!(relations.legacy_file_name(), "relations.md");

    assert_eq!(
        commands
            .iter()
            .filter(|command| command.id != "relations")
            .count(),
        14,
        "the original fourteen commands must all remain"
    );
    for command in commands.iter().filter(|command| command.id != "relations") {
        assert!(
            command.legacy_sha256.is_some(),
            "{} must keep its legacy fingerprint",
            command.id
        );
    }
}

/// Every catalog legacy fingerprint is a real SHA-256 of the artifact it
/// claims to recognise — a typo would make the digest match nothing and
/// legacy cleanup would silently never fire. `relations`, which supersedes
/// nothing, carries `None` and is skipped.
#[test]
fn legacy_fingerprints_are_well_formed_hex_sha256() {
    for command in catalog() {
        let Some(fingerprint) = command.legacy_sha256 else {
            continue;
        };
        assert_eq!(fingerprint.len(), 64, "{}", command.id);
        assert!(
            fingerprint
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{}: `{}` is not lowercase hex",
            command.id,
            fingerprint
        );
    }
}

/// The one checked-in legacy fixture: the exact bytes of the Bifrost-era
/// `repo-list` command, whose digest must equal the catalog constant. This
/// is what the reconciliation tests use as a real legacy artifact.
const LEGACY_REPO_LIST: &str = "# Repo List\n\
        \n\
        List all repositories registered in the hall manifest, along with active sessions\n\
        and promoted repos.\n\
        \n\
        ## Usage\n\
        \n\
        ```bash\n\
        bifrost hall status\n\
        ```\n\
        \n\
        ## Output\n\
        \n\
        Shows all repos with their name, default branch, and URL. Also shows features,\n\
        sessions, lifecycle state, and promoted repos per feature.\n";

#[test]
fn the_legacy_fixture_digests_to_its_catalog_constant() {
    let command = catalog()
        .iter()
        .find(|c| c.id == "repo-list")
        .expect("repo-list is in the catalog");
    assert_eq!(hash::text(LEGACY_REPO_LIST), command.legacy_sha256.unwrap());
}

#[test]
fn the_legacy_fixture_writes_repo_list_md() {
    let command = catalog()
        .iter()
        .find(|c| c.id == "repo-list")
        .expect("repo-list is in the catalog");
    assert_eq!(command.legacy_file_name(), "repo-list.md");
    assert_eq!(command.file_name(), "ivar-repo-list.md");
}

// -- materialise ----------------------------------------------------------

fn commands_dir() -> (tempfile::TempDir, Utf8PathBuf) {
    let (guard, root) = utf8_temp_dir();
    (guard, root.join("commands"))
}

fn change<'a>(changes: &'a [CommandChange], file_name: &str) -> &'a CommandChange {
    changes
        .iter()
        .find(|change| change.file_name == file_name)
        .unwrap_or_else(|| panic!("no `{file_name}` change in {changes:?}"))
}

fn inspection<'a>(inspections: &'a [Inspection], file_name: &str) -> &'a Inspection {
    inspections
        .iter()
        .find(|inspection| {
            inspection
                .path
                .file_name()
                .is_some_and(|name| name == file_name)
        })
        .unwrap_or_else(|| panic!("no inspection for `{file_name}` in {inspections:?}"))
}

#[test]
fn materialise_creates_repairs_and_then_becomes_idempotent() {
    let (_guard, dir) = commands_dir();

    let first = materialise(&dir).unwrap();
    assert_eq!(first.len(), 15);
    assert!(first.iter().all(|change| change.change == Change::Created));

    fs::write_text(&dir.join("ivar-plan.md"), "changed").unwrap();
    let repaired = materialise(&dir).unwrap();
    assert_eq!(change(&repaired, "ivar-plan.md").change, Change::Updated);
    assert_eq!(
        fs::read_text(&dir.join("ivar-plan.md")).unwrap().unwrap(),
        catalog().iter().find(|c| c.id == "plan").unwrap().content
    );

    let third = materialise(&dir).unwrap();
    assert_eq!(third.len(), 15);
    assert!(
        third
            .iter()
            .all(|change| change.change == Change::Unchanged),
        "expected everything unchanged, got {third:?}"
    );
    assert_eq!(
        fs::read_text(&dir.join("ivar-plan.md")).unwrap().unwrap(),
        catalog().iter().find(|c| c.id == "plan").unwrap().content
    );
}

#[test]
fn materialise_preserves_unrelated_user_commands() {
    let (_guard, dir) = commands_dir();
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(&dir.join("custom.md"), "my own command\n").unwrap();

    materialise(&dir).unwrap();

    assert_eq!(
        fs::read_text(&dir.join("custom.md")).unwrap().unwrap(),
        "my own command\n"
    );
}

#[test]
fn materialise_removes_unknown_files_in_reserved_ivar_namespace() {
    let (_guard, dir) = commands_dir();
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(&dir.join("ivar-retired.md"), "old\n").unwrap();

    let changes = materialise(&dir).unwrap();

    assert!(!fs::exists(&dir.join("ivar-retired.md")).unwrap());
    let removed = change(&changes, "ivar-retired.md");
    assert_eq!(removed.change, Change::Removed);
}

#[test]
fn remove_deletes_only_reserved_ivar_commands() {
    let (_guard, dir) = commands_dir();
    materialise(&dir).unwrap();
    fs::write_text(&dir.join("custom.md"), "my own command\n").unwrap();

    let changes = remove(&dir).unwrap();

    assert_eq!(changes.len(), 15);
    assert!(
        changes
            .iter()
            .all(|change| change.change == Change::Removed)
    );
    for command in catalog() {
        assert!(
            !fs::exists(&dir.join(command.file_name())).unwrap(),
            "{} should be gone",
            command.file_name()
        );
    }
    assert_eq!(
        fs::read_text(&dir.join("custom.md")).unwrap().unwrap(),
        "my own command\n"
    );
}

#[test]
fn matching_legacy_command_is_removed() {
    let (_guard, dir) = commands_dir();
    fs::ensure_dir(&dir).unwrap();
    fs::write_text(&dir.join("repo-list.md"), LEGACY_REPO_LIST).unwrap();

    let changes = materialise(&dir).unwrap();

    assert!(!fs::exists(&dir.join("repo-list.md")).unwrap());
    let removed = change(&changes, "repo-list.md");
    assert_eq!(removed.change, Change::Removed);
    // The shipped command now sits in its place.
    assert!(fs::exists(&dir.join("ivar-repo-list.md")).unwrap());
}

#[test]
fn modified_legacy_command_is_preserved_and_reported() {
    let (_guard, dir) = commands_dir();
    fs::ensure_dir(&dir).unwrap();
    let customized = format!("{LEGACY_REPO_LIST}x");
    fs::write_text(&dir.join("repo-list.md"), &customized).unwrap();

    let changes = materialise(&dir).unwrap();
    assert!(
        !changes
            .iter()
            .any(|change| change.change == Change::Removed),
        "a modified legacy file must never be deleted: {changes:?}"
    );
    assert_eq!(
        fs::read_text(&dir.join("repo-list.md")).unwrap().unwrap(),
        customized,
        "the user's customized file must survive byte for byte"
    );

    let inspections = inspect(&dir, true).unwrap();
    assert_eq!(
        inspection(&inspections, "repo-list.md").integrity,
        Integrity::LegacyModified
    );
}

// -- inspect --------------------------------------------------------------

#[test]
fn inspect_sees_a_healthy_directory_as_current() {
    let (_guard, dir) = commands_dir();
    materialise(&dir).unwrap();

    let inspections = inspect(&dir, true).unwrap();

    assert_eq!(inspections.len(), 15);
    assert!(
        inspections
            .iter()
            .all(|inspection| inspection.integrity == Integrity::Current)
    );
}

#[test]
fn inspect_reports_missing_and_modified_shipped_commands() {
    let (_guard, dir) = commands_dir();
    materialise(&dir).unwrap();
    fs::remove_file(&dir.join("ivar-plan.md")).unwrap();
    fs::write_text(&dir.join("ivar-sync.md"), "tampered\n").unwrap();

    let inspections = inspect(&dir, true).unwrap();

    let plan = inspections
        .iter()
        .find(|inspection| inspection.id == "plan")
        .unwrap();
    assert_eq!(plan.integrity, Integrity::Missing);
    let sync = inspections
        .iter()
        .find(|inspection| inspection.id == "sync")
        .unwrap();
    assert_eq!(sync.integrity, Integrity::Modified);
}

#[test]
fn inspect_marks_leftover_files_stale_for_a_disabled_provider() {
    let (_guard, dir) = commands_dir();
    materialise(&dir).unwrap();

    let inspections = inspect(&dir, false).unwrap();

    assert_eq!(inspections.len(), 15);
    assert!(
        inspections
            .iter()
            .all(|inspection| inspection.integrity == Integrity::Stale),
        "a disabled provider's leftovers are all stale: {inspections:?}"
    );
}

// -- living-context checkpoints -------------------------------------------

/// The embedded source of the shipped command `id`, with line-wrap
/// whitespace collapsed so a phrase that happens to straddle a wrap still
/// matches.
fn embedded(id: &str) -> String {
    let content = catalog()
        .iter()
        .find(|command| command.id == id)
        .unwrap_or_else(|| panic!("no `{id}` in the catalog"))
        .content;
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The plan checkpoint sits at the beginning of Analysis: read `HALL.md` and
/// the linked topics of potentially affected Repos, record the context, and
/// never let a deferred review block approval.
#[test]
fn plan_checks_relation_context_at_the_start_of_analysis() {
    let content = embedded("plan");
    let analysis = content
        .find("## Phase 2: Analysis")
        .expect("plan has an Analysis phase");
    let after = &content[analysis..];
    let lower = after.to_lowercase();

    assert!(lower.contains("read `hall.md`"), "was: {after}");
    assert!(after.contains("linked topics"), "was: {after}");
    assert!(after.contains("`analysis.md`"), "was: {after}");
    assert!(after.contains("evidence"), "was: {after}");
    assert!(after.contains("/ivar-relations"), "was: {after}");
    assert!(after.contains("never blocks"), "was: {after}");
}

/// The execute checkpoint runs only after every workstream is terminal, and
/// the offer is neither replan nor reconcile.
#[test]
fn execute_checks_relation_context_after_every_workstream_is_terminal() {
    let content = embedded("execute");

    assert!(
        content.contains("every workstream is terminal"),
        "was: {content}"
    );
    assert!(content.contains("journal"), "was: {content}");
    assert!(content.contains("evidence"), "was: {content}");
    assert!(content.contains("/ivar-relations"), "was: {content}");
    assert!(
        content.contains("does not alter execution completion"),
        "was: {content}"
    );
    assert!(
        content.contains("not a replan or reconcile"),
        "was: {content}"
    );
}

/// The deliver checkpoint sits between preview and apply, and deferring it
/// neither blocks apply nor invalidates the fingerprint.
#[test]
fn deliver_checks_relation_context_between_preview_and_apply() {
    let content = embedded("deliver");

    assert!(content.contains("preview"), "was: {content}");
    assert!(content.contains("apply"), "was: {content}");
    assert!(content.contains("HALL.md"), "was: {content}");
    assert!(content.contains("evidence"), "was: {content}");
    assert!(content.contains("/ivar-relations"), "was: {content}");
    assert!(content.contains("fingerprint"), "was: {content}");
    assert!(content.contains("does not block apply"), "was: {content}");
}

/// Every checkpoint is evidence-driven, offers `/ivar-relations`, and never
/// blocks the flow or writes `HALL.md` directly.
#[test]
fn every_checkpoint_is_evidence_driven_non_blocking_and_never_writes_directly() {
    for id in ["plan", "execute", "deliver"] {
        let content = embedded(id);
        assert!(content.contains("evidence"), "{id}");
        assert!(content.contains("/ivar-relations"), "{id}");
        assert!(
            content.contains("never"),
            "{id} must state what it never does"
        );
        assert!(
            content.contains("only"),
            "{id} must bound the offer to evidence"
        );
    }
}

// -- nested subfeature coordination -----------------------------------------

/// The feature-create command defines automatic nested creation: a
/// coordinator creates an isolatable child itself and announces it, without
/// asking permission.
#[test]
fn feature_create_defines_automatic_nested_creation() {
    let content = embedded("feature-create");

    assert!(
        content.contains("`ivar feature create <child> --parent <current>`"),
        "was: {content}"
    );
    assert!(content.contains("announce"), "was: {content}");
    assert!(content.contains("do not ask"), "was: {content}");
    assert!(
        content.contains("outside the approved plan"),
        "was: {content}"
    );
}

/// The plan command carries the decision split: isolatable work outside the
/// approved plan becomes a child automatically; a structural correction to
/// the approved plan is a replan; an implementation-only local divergence is
/// a reconcile.
#[test]
fn plan_defines_the_child_replan_reconcile_decision_split() {
    let content = embedded("plan");

    assert!(
        content.contains("isolatable"),
        "was: {content}"
    );
    assert!(
        content.contains("`ivar feature create <child> --parent <current>`"),
        "was: {content}"
    );
    assert!(content.contains("replan"), "was: {content}");
    assert!(content.contains("reconcile"), "was: {content}");
    assert!(
        content.contains("Structural correction"),
        "was: {content}"
    );
    assert!(
        content.contains("Implementation-only"),
        "was: {content}"
    );
}

/// The execute command identifies the invoking agent as the coordinator and
/// repeats the same decision tree; it never asks permission before creating a
/// child.
#[test]
fn execute_defines_the_coordinator_and_the_same_decision_tree() {
    let content = embedded("execute");

    assert!(content.contains("coordinator"), "was: {content}");
    assert!(
        content.contains("`ivar feature create <child> --parent <current>`"),
        "was: {content}"
    );
    assert!(content.contains("replan"), "was: {content}");
    assert!(content.contains("reconcile"), "was: {content}");
    assert!(
        content.contains("no permission question"),
        "was: {content}"
    );
}

/// The executor boundary is in the shipped execute bytes: an executor never
/// creates, reparents, promotes, integrates, closes, deletes, or otherwise
/// mutates shared feature state, and stops to report instead.
#[test]
fn execute_bounds_the_executor_against_mutating_feature_state() {
    let content = embedded("execute");

    assert!(content.contains("The executor is not the coordinator"), "was: {content}");
    assert!(content.contains("stops and reports"), "was: {content}");
    assert!(
        content.contains("never create, reparent, promote, integrate, close, delete, or otherwise mutate shared feature state"),
        "was: {content}"
    );
}
