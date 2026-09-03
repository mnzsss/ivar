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
    assert_eq!(commands.len(), 14);

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

/// `relations`, `feature-cleanup` and `connect` have no Bifrost-era
/// predecessor, so they carry no legacy fingerprint — and every other command
/// keeps its exact digest, which is what legacy cleanup still recognises.
#[test]
fn commands_without_a_bifrost_predecessor_carry_no_legacy_fingerprint() {
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
            .filter(|command| command.legacy_sha256.is_some())
            .count(),
        11,
        "every command with a Bifrost-era predecessor must keep its digest"
    );
    for command in commands
        .iter()
        .filter(|command| command.legacy_sha256.is_some())
    {
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
    assert_eq!(first.len(), 14);
    assert!(first.iter().all(|change| change.change == Change::Created));

    fs::write_text(&dir.join("ivar-plan.md"), "changed").unwrap();
    let repaired = materialise(&dir).unwrap();
    assert_eq!(change(&repaired, "ivar-plan.md").change, Change::Updated);
    assert_eq!(
        fs::read_text(&dir.join("ivar-plan.md")).unwrap().unwrap(),
        catalog().iter().find(|c| c.id == "plan").unwrap().content
    );

    let third = materialise(&dir).unwrap();
    assert_eq!(third.len(), 14);
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

    assert_eq!(changes.len(), 14);
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

    assert_eq!(inspections.len(), 14);
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

    assert_eq!(inspections.len(), 14);
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

/// `/ivar-execute` marks each wave complete in `plan.md` at the wave
/// checkpoint while the active provider coordinates native subagents itself.
#[test]
fn execute_marks_waves_complete_and_uses_native_coordination() {
    let content = embedded("execute");

    for required in [
        "ivar plan status",
        "native subagent",
        "wave checkpoint",
        "mark the wave complete",
        "Done",
        "exit criteria",
        "✅",
        "coordinator",
        "child Feature",
    ] {
        assert!(
            content.contains(required),
            "missing `{required}`: {content}"
        );
    }

    for removed in [
        "ivar feature execute status",
        "ivar feature execute start",
        "ivar feature execute finish",
        "accept-revision",
        "--resume",
        "--restart",
        "--report-json",
        "workstream",
        "write_contract",
        "Execution Board",
        "execution graph",
        "execute tick",
        "execute prepare",
    ] {
        assert!(!content.contains(removed), "stale `{removed}`: {content}");
    }
}

#[test]
fn plan_has_three_approval_gates_and_hands_off_to_execute() {
    let content = embedded("plan");

    assert!(content.contains("approve requirements"), "was: {content}");
    assert!(content.contains("approve analysis"), "was: {content}");
    assert!(content.contains("approve plan"), "was: {content}");
    assert!(content.contains("/ivar-execute"), "was: {content}");
    assert!(content.contains("Done"), "was: {content}");
    assert!(content.contains("✅"), "was: {content}");
    assert!(!content.contains("approve graph"), "was: {content}");
}

/// A task packet must declare who reads what it writes, with the grep that
/// found them. Three waves were reverted because a packet edited its declared
/// files and broke a reader outside the list.
#[test]
fn plan_packet_template_requires_declared_readers() {
    let content = embedded("plan");

    assert!(content.contains("**Readers:**"), "was: {content}");
    assert!(content.contains("grep -rn"), "was: {content}");
    assert!(content.contains("no readers outside"), "was: {content}");
}

/// The plan reviewer checks that packets declare their readers. Without this
/// the Readers field is advisory, which is how the parent feature's
/// R-DELEGATE defect was born.
#[test]
fn plan_reviewer_checks_declared_readers() {
    let content = embedded("plan");

    assert!(content.contains("Blast Radius"), "was: {content}");

    let table = content
        .find("| Category | What to Look For |")
        .expect("plan has a reviewer checklist");
    let after = &content[table..];
    let end = after.find("Reviewer output format").expect("checklist ends");
    let checklist = &after[..end];

    assert!(checklist.contains("Blast Radius"), "was: {checklist}");
    assert!(checklist.contains("Readers"), "was: {checklist}");
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

/// The execute command identifies the invoking agent as the coordinator and
/// repeats the same decision tree; it never asks permission before creating a
/// child, and it marks waves complete at each checkpoint.
#[test]
fn execute_defines_provider_native_coordination_and_wave_marking() {
    let content = embedded("execute");

    assert!(content.contains("provider-native"), "was: {content}");
    assert!(content.contains("coordinator"), "was: {content}");
    assert!(content.contains("child Feature"), "was: {content}");
    assert!(content.contains("wave checkpoint"), "was: {content}");
    assert!(content.contains("Done"), "was: {content}");
    assert!(!content.contains("accept-revision"), "was: {content}");
    assert!(!content.contains("--report-json"), "was: {content}");
}

/// OpenCode substitutes `$ARGUMENTS` into the command template and drops
/// anything the template never references — unlike Claude Code, which appends
/// unmatched arguments to the prompt. So a command that declares an
/// `argument-hint` must also *consume* the argument, or `/ivar-connect
/// <feature>` loses `<feature>` under OpenCode.
#[test]
fn every_command_declaring_an_argument_hint_consumes_arguments() {
    let dropped = catalog()
        .iter()
        .filter(|command| {
            let frontmatter = command
                .content
                .split("\n---")
                .next()
                .expect("every command opens with frontmatter");
            frontmatter.contains("argument-hint:")
                && !command.content[frontmatter.len()..].contains("$ARGUMENTS")
        })
        .map(|command| command.id)
        .collect::<Vec<_>>();

    assert!(
        dropped.is_empty(),
        "these commands declare an argument-hint but never reference \
         `$ARGUMENTS`, so OpenCode drops the argument: {dropped:?}"
    );
}
