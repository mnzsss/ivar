#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn path(repo: Option<&str>, p: &str) -> Candidate {
    Candidate::Path {
        repo: repo.map(str::to_owned),
        path: p.to_owned(),
    }
}

fn symbol(scope: Option<&str>, name: &str) -> Candidate {
    Candidate::Symbol {
        scope: scope.map(str::to_owned),
        name: name.to_owned(),
    }
}

#[test]
fn code_spans_become_path_and_symbol_candidates_in_every_accepted_form() {
    let body = "\
See `src/action/memory/reindex.rs`, `ivar:src/store/memory/schema.rs`,
`.ivar/repos/ivar/main/src/action/memory/query.rs` and `README.md`.
It calls `refresh_locked`, `query_in()` and `MemoryDb::open`.
The [guard](../../src/action/session/guard/usage.rs#L12) records misses.
";
    assert_eq!(
        extract_candidates(body),
        vec![
            path(None, "src/action/memory/reindex.rs"),
            path(Some("ivar"), "src/store/memory/schema.rs"),
            path(Some("ivar"), "src/action/memory/query.rs"),
            path(None, "README.md"),
            symbol(None, "refresh_locked"),
            symbol(None, "query_in"),
            symbol(Some("MemoryDb"), "open"),
            path(None, "src/action/session/guard/usage.rs"),
        ]
    );
}

#[test]
fn spans_with_spaces_short_spans_directories_urls_and_fences_are_rejected() {
    let body = "\
Run `ivar memory reindex`, set `ok`, open `src/action/` or [site](https://ivar.run/x.rs).
```
`fenced_symbol`
```
Line refs `src/a.rs:12-30` keep only the path; duplicates `src/a.rs` collapse.
";
    assert_eq!(extract_candidates(body), vec![path(None, "src/a.rs")]);
}

#[test]
fn links_to_memory_pages_are_not_candidates() {
    assert!(extract_candidates("[x](../decision/a.md) and `docs/memory/gotcha/b.md`").is_empty());
}
