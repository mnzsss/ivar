#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::provider::Provider;
use crate::store::manifest::Providers;

#[test]
fn a_marker_is_a_line_that_git_would_have_written() {
    for marker in [
        "<<<<<<< HEAD",
        "<<<<<<<",
        ">>>>>>> abc123 (a commit)",
        "=======",
        "=======\r",
    ] {
        assert!(is_marker(marker), "{marker:?}");
    }
    for prose in [
        "",
        "a <<<<<<< b",
        "<<<<<< six",
        "<<<<<<<<eight",
        "======",
        "========",
        "==== heading ====",
        "    =======",
        "```",
    ] {
        assert!(!is_marker(prose), "{prose:?}");
    }
}

#[test]
fn the_first_marker_is_reported_by_its_line() {
    assert_eq!(marker_line("one\ntwo\n"), None);
    assert_eq!(marker_line("one\n<<<<<<< HEAD\ntwo\n"), Some(2));
    assert_eq!(marker_line("a\nb\nc\n>>>>>>> x\n"), Some(4));
}

#[test]
fn only_pages_under_docs_memory_are_checked_as_memory() {
    assert!(is_memory_page(Utf8Path::new("docs/memory/gotcha/x.md")));
    assert!(is_memory_page(Utf8Path::new("docs/memory/gotcha/X.MD")));
    assert!(!is_memory_page(Utf8Path::new("docs/memory/gotcha/x.json")));
    assert!(!is_memory_page(Utf8Path::new("docs/guide.md")));
    assert!(!is_memory_page(Utf8Path::new("docs/memory-notes/x.md")));
}

#[test]
fn a_page_is_broken_by_its_content_not_by_where_it_lives() {
    let good = "---\ntitle: T\ntags: [a]\n---\n# T\n";
    assert!(broken_page(Utf8Path::new("docs/memory/gotcha/x.md"), good).is_empty());

    let bad_frontmatter = "---\ntitle: [unclosed\n---\n# T\n";
    assert!(!broken_page(Utf8Path::new("docs/memory/gotcha/x.md"), bad_frontmatter).is_empty());

    let wrong_kind = "---\nkind: decision\n---\n# T\n";
    assert!(!broken_page(Utf8Path::new("docs/memory/gotcha/x.md"), wrong_kind).is_empty());

    // Misplaced and unknown-key findings are not the agent's doing.
    assert!(broken_page(Utf8Path::new("docs/memory/README.md"), good).is_empty());
    assert!(
        broken_page(
            Utf8Path::new("docs/memory/gotcha/x.md"),
            "---\nmystery: 1\n---\n# T\n"
        )
        .is_empty()
    );
}

#[test]
fn a_fence_is_longer_than_any_run_of_backticks_inside() {
    assert_eq!(fenced("plain"), "```\nplain\n```\n");
    assert_eq!(fenced("a ``` b"), "````\na ``` b\n````\n");
    assert_eq!(fenced("`````"), "``````\n`````\n``````\n");
}

#[test]
fn a_cut_never_splits_a_character() {
    assert_eq!(clip("short", 10), ("short", false));
    assert_eq!(clip("abcdef", 3), ("abc", true));
    // `é` is two bytes: cutting between them backs up to the boundary.
    assert_eq!(clip("aé", 2), ("a", true));
}

#[test]
fn the_agent_runs_with_the_default_provider_or_the_first_that_has_a_tool() {
    let manifest = |available: Vec<Provider>, default: Provider| {
        Manifest::new(
            crate::domain::name::HallName::new("acme").unwrap(),
            Providers::new(available, default),
            Vec::new(),
        )
        .unwrap()
    };
    assert_eq!(
        tool_for(&manifest(
            vec![Provider::ClaudeCode, Provider::OpenCode],
            Provider::OpenCode
        )),
        Some(Tool::Opencode)
    );
    assert_eq!(
        tool_for(&manifest(vec![Provider::ClaudeCode], Provider::ClaudeCode)),
        Some(Tool::Claude)
    );
    assert_eq!(
        tool_for(&manifest(
            vec![Provider::Omp, Provider::OpenCode],
            Provider::Omp
        )),
        Some(Tool::Opencode)
    );
    assert_eq!(
        tool_for(&manifest(vec![Provider::Omp], Provider::Omp)),
        None
    );
}

#[test]
fn the_spec_may_write_the_conflicted_files_only_and_a_path_is_never_read_as_a_home() {
    let files = [Utf8PathBuf::from("~/x.md"), Utf8PathBuf::from("docs/a.md")];

    let spec = spec(Tool::Claude, Duration::from_secs(7), &files, "p").unwrap();

    assert_eq!(spec.name, AGENT);
    assert_eq!(spec.runtime, Runtime::Host);
    assert_eq!(spec.timeout, Duration::from_secs(7));
    assert_eq!(
        spec.write,
        [
            WritePath::Hall("~/x.md".into()),
            WritePath::Hall("docs/a.md".into())
        ]
    );
    assert!(spec.notify.is_none(), "a failed run must not notify");
}
