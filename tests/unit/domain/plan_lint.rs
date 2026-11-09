#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use super::*;

struct Tree(BTreeMap<&'static str, u64>);

impl RepoFiles for Tree {
    fn len(&self, path: &str) -> Result<Option<u64>, Failure> {
        Ok(self.0.get(path).copied())
    }
}

fn tree() -> Tree {
    Tree(BTreeMap::from([
        ("src/lib.rs", 1_200),
        ("src/big.rs", 25_000),
        ("tests/big.rs", 20_000),
        ("tests/small.rs", 900),
    ]))
}

/// A packet that passes every rule; each test breaks one thing in it.
fn packet_text(files: &str, extra: &str) -> String {
    format!(
        "### Task 1: Thing\n\
         **Files:**\n\
         {files}\n\
         **Design ref:** none\n\
         **Memory refs:** none\n\
         **Readers:**\n\
         - `git grep -n 'thing'`\n\
         \x20 ```\n\
         \x20 src/lib.rs:12:thing\n\
         \x20 ```\n\
         **Interfaces:**\n\
         - Consumes: none\n\
         - Produces: `fn thing() -> u32`\n\
         - [ ] **Step 1: Write the failing test** — the test's source:\n\
         \x20 ```rust\n\
         \x20 assert_eq!(thing(), 1);\n\
         \x20 ```\n\
         {extra}\n"
    )
}

const FILES: &str = "- Create: `src/new.rs`\n- Modify: `src/lib.rs:10-20`\n- Test: `tests/new.rs`";

fn source(file: &str, text: String) -> PacketSource {
    PacketSource {
        file: file.to_owned(),
        text,
    }
}

fn codes(plan: &str, packets: &[PacketSource]) -> Vec<&'static str> {
    let tree = tree();
    lint(plan, packets, Some(&tree))
        .unwrap()
        .into_iter()
        .map(|finding| finding.code)
        .collect()
}

fn codes_of(text: String) -> Vec<&'static str> {
    codes("", &[source("01-thing.md", text)])
}

#[test]
fn a_valid_packet_has_no_findings() {
    assert_eq!(codes_of(packet_text(FILES, "")), Vec::<&str>::new());
}

#[test]
fn a_todo_is_a_placeholder() {
    let text = packet_text(FILES, "- [ ] **Step 3: Write minimal implementation** TODO");
    assert_eq!(codes_of(text), [PLACEHOLDER]);
}

#[test]
fn deferred_prose_is_a_placeholder() {
    for phrase in [
        "Same signature as in Interfaces",
        "Run: `cargo test` (run at execution)",
        "Check the callers during execution",
    ] {
        let text = packet_text(FILES, &format!("- [ ] **Step 4: Run** {phrase}"));
        assert_eq!(codes_of(text), [PLACEHOLDER], "{phrase}");
    }
}

#[test]
fn a_stub_macro_is_a_placeholder() {
    let text = packet_text(FILES, "  ```rust\n  fn thing() -> u32 { todo!() }\n  ```");
    assert_eq!(codes_of(text), [PLACEHOLDER]);
}

#[test]
fn an_unfilled_number_marker_is_flagged() {
    let text = packet_text(FILES, "- [ ] **Step 2: Run** `cargo test --test t<NNN>`");
    assert_eq!(codes_of(text), [NUMBER_PLACEHOLDER]);
}

#[test]
fn readers_without_a_code_block_are_flagged() {
    let text = packet_text(FILES, "").replace("  ```\n  src/lib.rs:12:thing\n  ```\n", "");
    assert_eq!(codes_of(text), [READERS_WITHOUT_OUTPUT]);
}

#[test]
fn a_packet_without_readers_is_flagged() {
    let text = packet_text(FILES, "");
    let (head, rest) = text.split_once("**Readers:**").unwrap();
    let (_, tail) = rest.split_once("**Interfaces:**").unwrap();
    assert_eq!(
        codes_of(format!("{head}**Interfaces:**{tail}")),
        [READERS_MISSING]
    );
}

#[test]
fn a_sketch_needs_a_reason() {
    for marker in ["**Sketch:**", "**Sketch:** boilerplate", "**Sketch:** —"] {
        let text = packet_text(FILES, &format!("- [ ] **Step 3: Implement** {marker}"));
        assert_eq!(codes_of(text), [SKETCH_WITHOUT_REASON], "{marker}");
    }
    let reasoned = packet_text(
        FILES,
        "- [ ] **Step 3: Implement** **Sketch:** forty near-identical match arms follow the one shown",
    );
    assert_eq!(codes_of(reasoned), Vec::<&str>::new());
}

#[test]
fn create_of_an_existing_file_is_flagged() {
    let files = "- Create: `src/lib.rs`\n- Test: `tests/new.rs`";
    assert_eq!(codes_of(packet_text(files, "")), [CREATE_EXISTS]);
}

#[test]
fn modify_of_a_missing_file_is_flagged() {
    let files = "- Modify: `src/ghost.rs:1-9`\n- Test: `tests/new.rs`";
    assert_eq!(codes_of(packet_text(files, "")), [MODIFY_MISSING]);
}

#[test]
fn modify_of_a_file_an_earlier_packet_creates_is_allowed() {
    let first = packet_text("- Create: `src/new.rs`", "");
    let second = packet_text("- Modify: `src/new.rs`", "");
    let found = codes("", &[source("01-a.md", first), source("02-b.md", second)]);
    assert_eq!(found, Vec::<&str>::new());
}

#[test]
fn a_test_in_a_file_above_the_hook_limit_is_flagged() {
    let files = "- Modify: `src/big.rs`\n- Test: `src/big.rs`";
    assert_eq!(codes_of(packet_text(files, "")), [TEST_IN_OVERSIZED_FILE]);
}

#[test]
fn modifying_an_oversized_test_file_is_flagged_even_without_a_test_line() {
    let files = "- Modify: `tests/big.rs:5-9`";
    assert_eq!(codes_of(packet_text(files, "")), [TEST_IN_OVERSIZED_FILE]);
}

#[test]
fn modifying_an_oversized_source_file_without_new_tests_is_allowed() {
    let files = "- Modify: `src/big.rs:5-9`\n- Test: `tests/small.rs`";
    assert_eq!(codes_of(packet_text(files, "")), Vec::<&str>::new());
}

#[test]
fn file_rules_are_skipped_without_a_repo_tree() {
    let files = "- Create: `src/lib.rs`\n- Modify: `src/ghost.rs`";
    let packets = [source("01-thing.md", packet_text(files, ""))];
    assert!(lint("", &packets, None).unwrap().is_empty());
}

const PLAN: &str = "\
# Plan

## Interfaces

| Signature | Produced by | Consumed by |
| --- | --- | --- |
| `fn thing() -> u32` | 1 | 2, 3 |

### Wave 1 — base

| Task | Points | Blocked by | Outcome |
| --- | ---: | --- | --- |
| `tasks/01-thing.md` | 1 | — | thing |

### Wave 2 — users

| Task | Points | Blocked by | Outcome |
| --- | ---: | --- | --- |
| `tasks/02-user.md` | 1 | 1 | uses it |
| `tasks/03-later.md` | 1 | Task 2 | uses it too |
";

fn three_packets() -> Vec<PacketSource> {
    ["01-thing.md", "02-user.md", "03-later.md"]
        .into_iter()
        .map(|file| source(file, packet_text("- Test: `tests/new.rs`", "")))
        .collect()
}

#[test]
fn consumers_that_wait_for_the_producer_pass_even_transitively() {
    assert_eq!(codes(PLAN, &three_packets()), Vec::<&str>::new());
}

#[test]
fn a_consumer_without_the_producer_in_its_dependencies_is_flagged() {
    let plan = PLAN.replace("| 1 | uses it |", "| — | uses it |");
    let tree = tree();
    let findings = lint(&plan, &three_packets(), Some(&tree)).unwrap();
    // Task 2 no longer waits for 1; task 3 waits only for 2, so it is lost too.
    let flagged: Vec<(&str, &str)> = findings
        .iter()
        .map(|finding| (finding.file.as_str(), finding.code))
        .collect();
    assert_eq!(
        flagged,
        [
            ("02-user.md", MISSING_DEPENDENCY),
            ("03-later.md", MISSING_DEPENDENCY)
        ]
    );
    assert!(findings[0].what.contains("task 2"), "{}", findings[0].what);
    assert!(findings[0].what.contains("fn thing() -> u32"));
}

#[test]
fn a_signature_a_packet_consumes_counts_as_a_consumer() {
    let plan = PLAN.replace("| 2, 3 |", "| — |");
    let mut packets = three_packets();
    packets[1].text = packet_text("- Test: `tests/new.rs`", "").replace(
        "- Consumes: none",
        "- Consumes: `fn thing()\n  -> u32` from task 1",
    );
    let plan = plan.replace("| 1 | uses it |", "| — | uses it |");
    assert_eq!(codes(&plan, &packets), [MISSING_DEPENDENCY]);
}
