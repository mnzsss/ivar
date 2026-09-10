//! Unit tests for decision-oriented exploration rendering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::action::graph::query::{CalleeInfo, CallerInfo, FileOutline, ImpactItem, ImpactResult};
use crate::domain::graph::{
    Edge, EdgeKind, ExploreImpact, OperationalRelation, Provenance, RelationDirection,
    RelationEndpoint, SourceExcerpt, SourceFile, Span, Symbol, SymbolKind, SymbolSnippet,
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

fn source(repo: &str, file: &str, lines: usize) -> SourceFile {
    let code = (1..=lines)
        .map(|n| format!("{n}: line {n} of {file}"))
        .collect::<Vec<_>>()
        .join("\n");
    SourceFile {
        repo: repo.into(),
        file_path: file.into(),
        line_count: lines,
        excerpts: vec![SourceExcerpt {
            start_line: 1,
            end_line: lines,
            code,
        }],
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
        sources: Vec::new(),
    }
}

fn caller(name: &str, file: &str, line: usize, provenance: Provenance) -> CallerInfo {
    CallerInfo {
        caller: symbol(name, "web", 1),
        caller_file_path: file.into(),
        edge_kind: EdgeKind::Calls,
        provenance,
        confidence: if matches!(provenance, Provenance::Extracted) {
            0.95
        } else {
            0.7
        },
        line,
        col: 5,
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
    res.sources.push(source("api", "src/auth/sessions.ts", 40));

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

#[test]
fn source_is_grouped_by_file_and_shown_once() {
    let mut res = result("session");
    res.primary_symbols
        .push(snippet("createSession", "api", "src/auth/sessions.ts", 8));
    res.primary_symbols
        .push(snippet("getSession", "api", "src/auth/sessions.ts", 27));
    res.sources.push(source("api", "src/auth/sessions.ts", 40));

    let out = narrate_explore(&res);
    let source_text = &out[out.find("**Source**").expect("source section")..];

    assert_eq!(
        source_text.matches("`src/auth/sessions.ts`").count(),
        1,
        "got: {source_text}"
    );
    assert_eq!(
        source_text
            .matches("1: line 1 of src/auth/sessions.ts")
            .count(),
        1
    );
    assert!(source_text.contains("40: line 40 of src/auth/sessions.ts"));
    assert!(source_text.contains("`createSession` 8"));
    assert!(source_text.contains("`getSession` 27"));
}

/// Hosts save a tool result above ~25K characters to a file the agent then has
/// to Read, so source stops at a file boundary and names what it left out.
#[test]
fn source_stays_under_the_output_limit_and_names_the_files_left_out() {
    let mut res = result("routes");
    for i in 0..12 {
        let file = format!("src/routes/r{i}.ts");
        res.primary_symbols
            .push(snippet(&format!("route{i}"), "api", &file, 1));
        res.sources.push(source("api", &file, 120));
    }

    let out = narrate_explore(&res);

    assert!(out.len() <= MAX_OUTPUT_CHARS + 600, "len {}", out.len());
    assert!(out.contains("1: line 1 of src/routes/r0.ts"));
    assert!(
        out.contains("`src/routes/r11.ts`"),
        "left-out files must still be named"
    );
    assert!(out.contains("graph_explore"));
    assert!(!out.contains("use Read"));
}

#[test]
fn a_file_larger_than_the_limit_is_cut_at_a_line() {
    let mut res = result("huge");
    res.primary_symbols
        .push(snippet("huge", "api", "src/huge.ts", 1));
    res.sources.push(source("api", "src/huge.ts", 2_000));

    let out = narrate_explore(&res);

    assert!(out.len() <= MAX_OUTPUT_CHARS + 200, "len {}", out.len());
    assert_eq!(
        out.matches("```").count() % 2,
        0,
        "every code fence is closed"
    );
    assert!(
        out.contains("truncated after line"),
        "got tail: {}",
        &out[out.len() - 200..]
    );
}

/// The JSON form spent its tokens on ids, spans and nulls the model never used.
#[test]
fn callers_render_one_line_each_and_flag_uncertain_edges() {
    let callers = vec![
        caller(
            "useAccessControl",
            "src/hooks/useAccessControl.ts",
            12,
            Provenance::Extracted,
        ),
        caller(
            "ProtectedRoute",
            "src/components/ProtectedRoute.tsx",
            9,
            Provenance::Inferred,
        ),
    ];

    let out = narrate_callers("evaluateAccess", &callers);

    assert!(
        out.contains("**Callers of `evaluateAccess`: 2**"),
        "got: {out}"
    );
    assert!(out.contains("`useAccessControl` in src/hooks/useAccessControl.ts:12"));
    assert!(out.contains("verify this line"));
    assert!(out.len() * 3 < serde_json::to_string_pretty(&callers).unwrap().len());
}

#[test]
fn no_callers_names_the_likely_causes() {
    let out = narrate_callers("orphan", &[]);
    assert!(out.contains("No callers of `orphan`"), "got: {out}");
    assert!(out.contains("refresh_index"));
}

#[test]
fn callees_show_where_each_call_lands() {
    let callees = vec![
        CalleeInfo {
            callee_name: "getSession".into(),
            callee_symbol: Some(symbol("getSession", "api", 27)),
            callee_file_path: Some("src/auth/sessions.ts".into()),
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            confidence: 1.0,
            line: 14,
            col: 3,
        },
        CalleeInfo {
            callee_name: "reply.send".into(),
            callee_symbol: None,
            callee_file_path: None,
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Inferred,
            confidence: 0.85,
            line: 20,
            col: 3,
        },
    ];

    let out = narrate_callees("requireAuth", &callees);

    assert!(
        out.contains("**Calls made by `requireAuth`: 2**"),
        "got: {out}"
    );
    assert!(out.contains("line 14 → `getSession` in src/auth/sessions.ts:27"));
    assert!(out.contains("line 20 → `reply.send` (not in the index)"));
}

#[test]
fn impact_groups_consumers_by_depth_in_a_fraction_of_the_json() {
    let affected_symbols: Vec<ImpactItem> = (0..30)
        .map(|i| ImpactItem {
            symbol: symbol(&format!("consumer{i}"), "api", 10 + i),
            file_path: format!("src/c{}.ts", i % 5),
            depth: 1 + i % 2,
            path_via: vec!["Session".into(), format!("consumer{i}")],
        })
        .collect();
    let res = ImpactResult {
        root_symbol: symbol("Session", "api", 8),
        affected_files: (0..5).map(|i| format!("src/c{i}.ts")).collect(),
        total_affected: affected_symbols.len(),
        affected_symbols,
    };

    let out = narrate_impact(&res);

    assert!(
        out.contains("**Impact of `Session`: 30 symbols in 5 files**"),
        "got: {out}"
    );
    assert!(out.contains("Depth 1"));
    assert!(out.contains("Depth 2"));
    assert!(out.contains("`consumer0` src/c0.ts:10"));
    assert!(out.len() * 4 < serde_json::to_string_pretty(&res).unwrap().len());
}

#[test]
fn outline_lists_symbols_with_their_lines_and_imports() {
    let outline = FileOutline {
        file_path: "src/routes/registry.ts".into(),
        repo: "web".into(),
        symbols: vec![symbol("routes", "web", 3), symbol("findRoute", "web", 42)],
        imports: vec![Edge {
            id: None,
            repo: "web".into(),
            file_id: Some(1),
            from_symbol_id: None,
            to_symbol_id: None,
            to_name: Some("./pages/Admin".into()),
            kind: EdgeKind::Imports,
            provenance: Provenance::Extracted,
            line: 1,
            col: 1,
            confidence: 0.95,
        }],
    };

    let out = narrate_outline(&outline);

    assert!(
        out.contains("**Outline of `src/routes/registry.ts` (web)**: 2 symbols, 1 import"),
        "got: {out}"
    );
    assert!(out.contains("`routes` fn 3-5 [exported]"));
    assert!(out.contains("`./pages/Admin`"));
}

fn outgoing(caller: &str, callee: &str, callee_file: &str) -> OperationalRelation {
    OperationalRelation {
        direction: RelationDirection::Outgoing,
        ..relation(
            caller,
            "src/auth/helpers.ts",
            "api",
            callee,
            callee_file,
            4,
            Provenance::Extracted,
        )
    }
}

#[test]
fn blast_radius_is_omitted_when_nothing_calls_the_matches() {
    let mut res = result("adminRoutes");
    res.primary_symbols
        .push(snippet("adminRoutes", "api", "src/routes/admin.ts", 5));
    res.direct_relations
        .push(outgoing("adminRoutes", "reply.code", ""));

    let out = narrate_explore(&res);

    assert!(!out.contains("Blast radius"), "got: {out}");
}

#[test]
fn call_flows_leave_out_calls_that_resolve_to_nothing_in_the_index() {
    let mut res = result("requireAuth");
    res.primary_symbols
        .push(snippet("requireAuth", "api", "src/auth/helpers.ts", 3));
    for (callee, file) in [("reply.code", ""), ("getSession", "src/auth/sessions.ts")] {
        res.direct_relations
            .push(outgoing("requireAuth", callee, file));
        res.call_flows.push(crate::domain::graph::CallFlowItem {
            caller: "requireAuth".into(),
            callee: callee.into(),
            edge_kind: EdgeKind::Calls,
            provenance: Provenance::Extracted,
            line: 4,
        });
    }

    let out = narrate_explore(&res);

    assert!(out.contains("`requireAuth` -> `getSession`"), "got: {out}");
    assert!(!out.contains("reply.code"), "got: {out}");
}
