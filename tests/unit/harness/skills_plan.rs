#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn skill_test_files_fit_the_commit_hook_limit() {
    for rel in [
        "tests/unit/harness/skills.rs",
        "tests/unit/harness/skills_plan.rs",
    ] {
        let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::metadata(&path).unwrap().len();
        assert!(bytes < 20_000, "{rel} is {bytes} bytes");
    }
}

fn plan_text() -> String {
    catalog()
        .iter()
        .find(|skill| skill.id == "plan")
        .expect("plan skill shipped")
        .files
        .iter()
        .map(|file| file.content)
        .collect::<Vec<_>>()
        .join("\n")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn ivar_plan_skill_ships_plan_and_task_templates() {
    let skill = catalog()
        .iter()
        .find(|s| s.id == "plan")
        .expect("plan skill");
    let file = |path: &str| {
        skill
            .files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .content
    };

    assert!(skill.skill_md().starts_with("---\nname: ivar-plan\n"));
    assert!(skill.skill_md().contains("references/plan-template.md"));
    assert!(skill.skill_md().contains("references/task-template.md"));
    assert!(file("references/plan-template.md").contains("### Wave N"));
    assert!(file("references/plan-template.md").contains("Lightweight validation"));
    assert!(file("references/task-template.md").contains("**Readers:**"));
    assert!(file("references/task-template.md").contains("**Sketch:**"));
}

/// The plan checkpoint sits at the beginning of Analysis: read `HALL.md` and
/// the linked topics of potentially affected Repos, record the context, and
/// never let a deferred review block approval.
#[test]
fn plan_checks_relation_context_at_the_start_of_analysis() {
    let content = plan_text();
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

#[test]
fn plan_has_three_approval_gates_and_hands_off_to_execute() {
    let content = plan_text();

    assert!(content.contains("approve requirements"), "was: {content}");
    assert!(content.contains("approve analysis"), "was: {content}");
    assert!(content.contains("approve plan"), "was: {content}");
    assert!(content.contains("ivar-execute"), "was: {content}");
    assert!(!content.contains("| Done |"), "was: {content}");
    assert!(!content.contains("✅"), "was: {content}");
    assert!(!content.contains("approve graph"), "was: {content}");
}

/// A task packet must declare who reads what it writes, with the grep that
/// found them. Three waves were reverted because a packet edited its declared
/// files and broke a reader outside the list.
#[test]
fn plan_packet_template_requires_declared_readers() {
    let content = plan_text();

    assert!(content.contains("**Readers:**"), "was: {content}");
    assert!(content.contains("git grep -n"), "was: {content}");
    assert!(content.contains("no readers outside"), "was: {content}");
}

/// The Readers grep must cover the whole tree, not a scope someone derives.
/// Two controlled runs found the same failure twice: a reader in `examples/`
/// escaped a hardcoded `src/ tests/`, and a reader in `docs/` escaped a scope
/// derived from the workspace members. Both times every agent reported
/// success. `git grep` with no pathspec ends the derivation: tracked files in,
/// build artifacts out, nothing to get wrong.
#[test]
fn plan_readers_grep_is_scoped_to_the_whole_tree() {
    let content = plan_text();

    assert!(content.contains("git grep -n '<symbol>'"), "was: {content}");
    assert!(content.contains("no pathspec"), "was: {content}");
    assert!(
        content.contains("not only code that compiles"),
        "was: {content}"
    );
}

/// A build check narrower than the readers it must defend passes while a
/// reader outside it breaks. In the controlled run every agent ran the
/// packet's `cargo build`, saw green, and shipped an `examples/` target that
/// failed to compile.
#[test]
fn plan_verification_covers_every_reader_it_declares() {
    let content = plan_text();

    assert!(content.contains("as wide as the readers"), "was: {content}");
}

/// The plan reviewer checks that packets declare their readers. Without this
/// the Readers field is advisory, which is how the parent feature's
/// R-DELEGATE defect was born.
#[test]
fn plan_reviewer_checks_declared_readers() {
    let content = plan_text();

    assert!(content.contains("Blast Radius"), "was: {content}");

    let table = content
        .find("| Category | What to Look For |")
        .expect("plan has a reviewer checklist");
    let after = &content[table..];
    let end = after
        .find("Reviewer output format")
        .expect("checklist ends");
    let checklist = &after[..end];

    assert!(checklist.contains("Blast Radius"), "was: {checklist}");
    assert!(checklist.contains("Readers"), "was: {checklist}");
}

/// The packet template's own Step 1 must show assertions, not describe them.
/// Two shipped plans (`ivar-manifest-schema`, `omp-support`: 20 packets)
/// carried zero code while `:176-180` already forbade it in prose. A rule
/// stated beside a form that contradicts it loses to the form.
#[test]
fn plan_packet_template_requires_literal_test() {
    let content = plan_text();

    assert!(content.contains("literal source"), "was: {content}");
    assert!(
        content.contains("Step 1 carries the test's literal source"),
        "was: {content}"
    );
    // The example inside the template is real code, not a placeholder.
    assert!(content.contains("assert_eq!"), "was: {content}");
}

/// The escape is bounded on both sides: Step 1 may never use it, and a
/// sketch still names its signatures. `omp-support`'s packet 01 produced
/// `Provider::Omp` "with stable id `omp`, config dir `.omp`" — an enum
/// variant whose actual shape no packet ever wrote down, leaving the next
/// packet's `Consumes` citing nothing.
#[test]
fn plan_sketch_escape_is_bounded() {
    let content = plan_text();

    assert!(content.contains("**Sketch:**"), "was: {content}");
    assert!(content.contains("never Step 1"), "was: {content}");
    assert!(
        content.contains("relaxes a body, never an interface"),
        "was: {content}"
    );
    assert!(content.contains("exact signature"), "was: {content}");
}

/// The reviewer checks the code obligation, or it is advisory — which is
/// how the prose at `:176-180` failed. The row must judge the *reason* on a
/// sketch, not merely the marker's presence: an unjudged escape becomes the
/// default.
#[test]
fn plan_reviewer_checks_literal_code() {
    let content = plan_text();

    let table = content
        .find("| Category | What to Look For |")
        .expect("plan has a reviewer checklist");
    let after = &content[table..];
    let end = after
        .find("Reviewer output format")
        .expect("checklist ends");
    let checklist = &after[..end];

    assert!(checklist.contains("Literal Code"), "was: {checklist}");
    assert!(checklist.contains("Step 1"), "was: {checklist}");
    assert!(checklist.contains("**Sketch:**"), "was: {checklist}");
    assert!(checklist.contains("reason"), "was: {checklist}");
}

#[test]
fn plan_uses_graph_evidence_with_fallback_and_no_approval_bypass() {
    let content = plan_text();
    assert!(content.contains("ivar graph explore"), "was: {content}");
    assert!(content.contains("advisory"), "was: {content}");
    assert!(
        content.contains("fallback") || content.contains("fall back"),
        "was: {content}"
    );
    assert!(
        content.contains("never creates, approves, or bypasses"),
        "was: {content}"
    );
}

#[test]
fn plan_consults_memory_at_the_start_of_analysis_and_never_blocks_on_it() {
    let content = plan_text();
    let analysis = &content[content.find("## Phase 2: Analysis").unwrap()..];
    assert!(analysis.contains("ivar memory query"), "was: {analysis}");
    assert!(analysis.contains("`memory_query`"), "was: {analysis}");
    assert!(analysis.contains("Conhecimento prévio"), "was: {analysis}");
    assert!(analysis.contains("memória indisponível"), "was: {analysis}");
    assert!(!analysis.contains("docs/product") && !analysis.contains("docs/updates"));
}
