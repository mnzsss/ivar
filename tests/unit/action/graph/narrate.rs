//! Unit tests for decision-oriented exploration rendering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::domain::graph::{
    EdgeKind, ExploreImpact, OperationalRelation, Provenance, RelationDirection, RelationEndpoint,
    Span, Symbol, SymbolKind, SymbolSnippet,
};

fn symbol(name: &str, repo: &str, line: usize) -> Symbol {
    Symbol {
        id: Some(1),
        file_id: Some(1),
        repo: repo.into(),
        name: name.into(),
        kind: SymbolKind::Fn,
        scope: None,
        signature: None,
        docstring: None,
        span: Span {
            start_line: line,
            start_col: 0,
            end_line: line + 2,
            end_col: 1,
        },
        is_exported: true,
        complexity: None,
    }
}

fn snippet(name: &str, repo: &str, file: &str, line: usize) -> SymbolSnippet {
    SymbolSnippet {
        symbol: symbol(name, repo, line),
        file_path: file.into(),
        code: format!("{line}: fn {name}() {{}}"),
        start_line: line,
        end_line: line + 2,
    }
}

fn relation(
    caller: &str,
    caller_file: &str,
    caller_repo: &str,
    target: &str,
    target_file: &str,
    line: usize,
    provenance: Provenance,
) -> OperationalRelation {
    OperationalRelation {
        source: RelationEndpoint {
            repo: caller_repo.into(),
            file_path: caller_file.into(),
            symbol_name: caller.into(),
            symbol_kind: Some(SymbolKind::Fn),
        },
        target: RelationEndpoint {
            repo: "api".into(),
            file_path: target_file.into(),
            symbol_name: target.into(),
            symbol_kind: Some(SymbolKind::Fn),
        },
        direction: RelationDirection::Incoming,
        edge_kind: EdgeKind::Calls,
        provenance,
        confidence: if matches!(provenance, Provenance::Extracted) {
            1.0
        } else {
            0.55
        },
        line,
        hop_count: 1,
        cross_repo: caller_repo != "api",
    }
}

fn result(query: &str) -> ExploreResult {
    ExploreResult {
        query: query.into(),
        primary_symbols: Vec::new(),
        call_flows: Vec::new(),
        impact_summary: None,
        direct_relations: Vec::new(),
        entry_points: Vec::new(),
        transitive_consumers: Vec::new(),
    }
}

/// The callers a change would break must appear before the source, since a
/// model reading top-down decides whether to edit from the first screen.
#[test]
fn blast_radius_precedes_source() {
    let mut res = result("getSession");
    res.primary_symbols
        .push(snippet("getSession", "api", "src/auth/sessions.ts", 27));
    res.direct_relations.push(relation(
        "requireAuth",
        "src/auth/helpers.ts",
        "api",
        "getSession",
        "src/auth/sessions.ts",
        14,
        Provenance::Extracted,
    ));

    let out = narrate_explore(&res);
    let radius = out.find("Blast radius").expect("blast radius section");
    let source = out.find("**Source**").expect("source section");

    assert!(radius < source, "consequences must lead the answer");
    assert!(out.contains("`requireAuth` in src/auth/helpers.ts:14"));
}

/// Callers are resolved by name, so one definition per name is rendered and the
/// ambiguity is stated. Printing per-definition entries repeated identical
/// caller lists and implied a precision the index does not have.
#[test]
fn same_name_definitions_collapse_into_one_entry_naming_the_ambiguity() {
    let mut res = result("session");
    res.primary_symbols
        .push(snippet("session", "ivar", "src/store/mod.rs", 16));
    res.primary_symbols
        .push(snippet("session", "ivar", "src/action/mod.rs", 29));
    res.direct_relations.push(relation(
        "run_session",
        "src/action/session.rs",
        "api",
        "session",
        "src/store/mod.rs",
        40,
        Provenance::Extracted,
    ));

    let out = narrate_explore(&res);

    assert_eq!(
        out.matches("`run_session` in src/action/session.rs:40")
            .count(),
        1,
        "a caller must be listed once, not once per same-named definition"
    );
    assert!(out.contains("Defined in 2 places"));
    assert!(out.contains("src/store/mod.rs:16"));
    assert!(out.contains("src/action/mod.rs:29"));
}

/// One caller reached through several candidates must not inflate the count.
#[test]
fn duplicate_relations_are_counted_once() {
    let mut res = result("getSession");
    res.primary_symbols
        .push(snippet("getSession", "api", "src/auth/sessions.ts", 27));
    for _ in 0..3 {
        res.direct_relations.push(relation(
            "requireAuth",
            "src/auth/helpers.ts",
            "api",
            "getSession",
            "src/auth/sessions.ts",
            14,
            Provenance::Extracted,
        ));
    }

    let out = narrate_explore(&res);
    assert!(out.contains("1 caller\n"), "got: {out}");
}

/// A heuristic edge must carry its caution inline; a bare enum name in JSON was
/// silently treated as fact.
#[test]
fn uncertain_edges_are_flagged_for_verification() {
    let mut res = result("handler");
    res.primary_symbols
        .push(snippet("handler", "api", "src/routes.ts", 10));
    res.direct_relations.push(relation(
        "dispatch",
        "src/router.ts",
        "api",
        "handler",
        "src/routes.ts",
        88,
        Provenance::Ambiguous,
    ));

    let out = narrate_explore(&res);
    assert!(out.contains("ambiguous"), "got: {out}");
    assert!(out.contains("verify this line"));
    assert!(out.contains("0.55"));

    let mut certain = result("handler");
    certain
        .primary_symbols
        .push(snippet("handler", "api", "src/routes.ts", 10));
    certain.direct_relations.push(relation(
        "dispatch",
        "src/router.ts",
        "api",
        "handler",
        "src/routes.ts",
        88,
        Provenance::Extracted,
    ));
    assert!(
        !narrate_explore(&certain).contains("verify this line"),
        "an AST-extracted edge needs no caveat"
    );
}

/// Cross-repo callers are the edges grep cannot find, so they are marked.
#[test]
fn cross_repo_callers_are_marked_with_their_repo() {
    let mut res = result("createSession");
    res.primary_symbols
        .push(snippet("createSession", "api", "src/auth/sessions.ts", 8));
    res.direct_relations.push(relation(
        "login",
        "src/pages/Login.tsx",
        "web",
        "createSession",
        "src/auth/sessions.ts",
        22,
        Provenance::Extracted,
    ));

    let out = narrate_explore(&res);
    assert!(out.contains("1 across repos"));
    assert!(out.contains("[repo `web`]"));
}

/// An empty result must route the reader forward rather than dead-end.
#[test]
fn empty_result_explains_the_next_move() {
    let out = narrate_explore(&result("nonexistent"));
    assert!(out.contains("No symbols matched"));
    assert!(
        out.contains("refresh_index"),
        "a stale index is the likely cause and the fix must be named"
    );
}

/// Long caller lists are truncated with the remainder counted, so the answer
/// stays bounded without hiding scale.
#[test]
fn long_caller_lists_report_the_untruncated_total() {
    let mut res = result("session");
    res.primary_symbols
        .push(snippet("session", "ivar", "src/store/mod.rs", 16));
    for i in 0..20 {
        res.direct_relations.push(relation(
            &format!("caller{i}"),
            "tests/unit/run.rs",
            "api",
            "session",
            "src/store/mod.rs",
            100 + i,
            Provenance::Extracted,
        ));
    }

    let out = narrate_explore(&res);
    assert!(out.contains("20 callers"), "the total must stay visible");
    assert!(out.contains("…and 14 more"));
}

/// Transitive reach answers "what else could this break" beyond direct callers.
#[test]
fn transitive_consumers_are_reported_with_their_path() {
    let mut res = result("getSession");
    res.primary_symbols
        .push(snippet("getSession", "api", "src/auth/sessions.ts", 27));
    res.transitive_consumers.push(ExploreImpact {
        symbol_name: "adminRoute".into(),
        repo: "api".into(),
        file_path: "src/routes/admin.ts".into(),
        depth: 2,
        path_via: vec!["getSession".into(), "requireAuth".into()],
        cross_repo: false,
    });

    let out = narrate_explore(&res);
    assert!(out.contains("Reaches 1 symbol transitively"));
    assert!(out.contains("`adminRoute`"));
    assert!(out.contains("getSession -> requireAuth"));
}

/// The Source block must tell the consumer the snippet is current and already
/// equivalent to a Read — without this, models re-read the same file, wasting
/// tokens that the indexed graph was designed to avoid.
#[test]
fn source_block_declares_source_is_current_and_no_re_read_needed() {
    let mut res = result("getSession");
    res.primary_symbols
        .push(snippet("getSession", "api", "src/auth/sessions.ts", 27));

    let out = narrate_explore(&res);
    let source = out.find("**Source**").expect("source section");
    let source_text = &out[source..];

    assert!(
        source_text.contains("already read") || source_text.contains("already been read"),
        "Source block must declare the snippet is equivalent to a Read already done, \
         but got: {}",
        &source_text[..source_text.len().min(300)],
    );
    assert!(
        source_text.contains("do not re-read"),
        "Source block must explicitly prohibit re-reading the displayed file, \
         but got: {}",
        &source_text[..source_text.len().min(300)],
    );
}
