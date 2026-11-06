use super::*;

#[test]
fn a_file_changed_since_the_index_warns_that_symbol_lines_may_have_moved() {
    let mut res = result("getSession");
    res.primary_symbols
        .push(snippet("getSession", "api", "src/auth/sessions.ts", 27));
    let mut changed = source("api", "src/auth/sessions.ts", 40);
    changed.changed_since_index = true;
    res.sources.push(changed);

    let out = narrate_explore(&res);

    assert!(out.contains("changed since the last index"), "got: {out}");
}

#[test]
fn the_path_between_named_symbols_leads_the_answer() {
    let mut res = result("handleLogin saveSession");
    res.primary_symbols
        .push(snippet("handleLogin", "api", "src/routes/auth.ts", 3));
    res.flows.push(PathResult {
        from: "handleLogin".into(),
        to: "saveSession".into(),
        steps: vec![
            PathStep {
                source: "handleLogin".into(),
                target: "createSession".into(),
                edge_kind: EdgeKind::Calls,
                line: 5,
            },
            PathStep {
                source: "createSession".into(),
                target: "saveSession".into(),
                edge_kind: EdgeKind::Calls,
                line: 12,
            },
        ],
    });

    let out = narrate_explore(&res);

    assert!(
        out.contains(
            "- `handleLogin` → `createSession` (calls at line 5) → `saveSession` (calls at line 12)"
        ),
        "got: {out}"
    );
    assert!(
        out.find("Flow between").expect("flow section")
            < out.find("**Source**").expect("source section")
    );
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
    assert!(
        out.contains(
            "- `getSession` (src/auth/sessions.ts:27) — 1 caller in `src/auth/helpers.ts`"
        ),
        "got: {out}"
    );
}

#[test]
fn same_name_definitions_list_their_own_consumers() {
    let mut res = result("Session");
    res.primary_symbols
        .push(snippet("Session", "api", "src/session.ts", 3));
    res.primary_symbols
        .push(snippet("Session", "web", "src/session.ts", 8));
    res.direct_relations.push(relation(
        "login",
        "src/login.ts",
        "api",
        "Session",
        "src/session.ts",
        12,
        Provenance::Extracted,
    ));
    let mut web_caller = relation(
        "render",
        "src/page.ts",
        "web",
        "Session",
        "src/session.ts",
        20,
        Provenance::Extracted,
    );
    web_caller.target.repo = "web".into();
    web_caller.cross_repo = false;
    res.direct_relations.push(web_caller);
    let mut api_type_use = relation(
        "",
        "src/types.ts",
        "api",
        "Session",
        "src/session.ts",
        2,
        Provenance::Extracted,
    );
    api_type_use.edge_kind = EdgeKind::References;
    res.direct_relations.push(api_type_use);

    let out = narrate_explore(&res);

    assert!(
        out.contains(
            "- `Session` (src/session.ts:3) — 1 caller in `src/login.ts`; 1 type use at `src/types.ts:2`\n"
        ),
        "got: {out}"
    );
    assert!(
        out.contains("- `Session` (src/session.ts:8) — 1 caller in `src/page.ts`\n"),
        "got: {out}"
    );
    assert!(!out.contains("definitions share this name"), "got: {out}");
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
    assert!(
        out.contains("1 caller in `src/auth/helpers.ts`\n"),
        "got: {out}"
    );
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
    assert!(
        out.contains("1 inferred, not certain, verify before relying on them"),
        "got: {out}"
    );

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
        !narrate_explore(&certain).contains("verify before relying"),
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
    assert!(out.contains("1 from other repos"), "got: {out}");
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
        .push(snippet("session", "api", "src/store/mod.rs", 16));
    for i in 0..20 {
        res.direct_relations.push(relation(
            &format!("caller{i}"),
            &format!("src/callers/c{i:02}.rs"),
            "api",
            "session",
            "src/store/mod.rs",
            100 + i,
            Provenance::Extracted,
        ));
    }

    let out = narrate_explore(&res);
    assert!(
        out.contains("20 callers in"),
        "the total must stay visible: {out}"
    );
    assert!(out.contains("+17 more files"), "got: {out}");
}

/// CodeGraph leaves relation sections out on small repositories. In tokens3,
/// entry points, call flows and transitive consumers took 1K to 4.5K characters
/// of every ivar answer, and every later turn paid for them again.
#[test]
fn relation_sections_other_than_the_blast_radius_stay_out_of_the_answer() {
    let mut res = result("requireAuth");
    res.primary_symbols
        .push(snippet("requireAuth", "api", "src/auth/helpers.ts", 3));
    res.call_flows.push(crate::domain::graph::CallFlowItem {
        caller: "requireAuth".into(),
        callee: "getSession".into(),
        edge_kind: EdgeKind::Calls,
        provenance: Provenance::Extracted,
        line: 4,
    });
    res.transitive_consumers.push(ExploreImpact {
        symbol_name: "adminRoute".into(),
        repo: "api".into(),
        file_path: "src/routes/admin.ts".into(),
        depth: 2,
        path_via: vec!["requireAuth".into(), "adminRoute".into()],
        cross_repo: false,
    });

    let out = narrate_explore(&res);

    for section in [
        "**Call flows**",
        "**Entry points**",
        "transitively",
        "**Impact**",
    ] {
        assert!(!out.contains(section), "{section} leaked into: {out}");
    }
}

/// With a per-call file budget, the files left out are named with their symbols
/// so the next call is another explore instead of a Read.
#[test]
fn files_left_out_are_named_with_their_symbols_for_the_next_explore() {
    let mut res = result("routes");
    res.primary_symbols
        .push(snippet("authRoutes", "api", "src/routes/auth.ts", 6));
    res.sources.push(source("api", "src/routes/auth.ts", 20));
    res.not_shown.push(FileMention {
        repo: "api".into(),
        file_path: "src/routes/admin.ts".into(),
        symbols: vec![MentionedSymbol {
            name: "adminRoutes".into(),
            line: 7,
        }],
    });

    let out = narrate_explore(&res);

    let not_shown = out.find("**Not shown above").expect("not shown section");
    assert!(out.find("**Source**").expect("source section") < not_shown);
    assert!(
        out.contains("- src/routes/admin.ts: adminRoutes:7"),
        "got: {out}"
    );
    assert!(!out.contains("use Read"));
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
        source_text.contains("a Read you have already performed"),
        "Source block must declare the snippet is equivalent to a Read already done, \
         but got: {}",
        &source_text[..source_text.len().min(300)],
    );
    assert!(
        source_text.contains("do not Read a file shown here"),
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
            .matches("1\tline 1 of src/auth/sessions.ts")
            .count(),
        1
    );
    assert!(source_text.contains("40\tline 40 of src/auth/sessions.ts"));
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
    assert!(out.contains("1\tline 1 of src/routes/r0.ts"));
    assert!(
        out.contains("- src/routes/r11.ts: route11:1"),
        "left-out files must still be named, got: {out}"
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
    assert!(
        out.contains("Call `graph_explore` with paths: [\"src/huge.ts:"),
        "got tail: {}",
        &out[out.len() - 200..]
    );
}

#[test]
fn test_narrate_truncation_note_names_next_range() {
    let res = ExploreResult {
        related_memory: Vec::new(),
        confidence: crate::domain::graph::ExploreConfidence::High,
        query: "huge".to_owned(),
        file_matches: Vec::new(),
        primary_symbols: vec![SymbolSnippet {
            weak: false,
            symbol: Symbol {
                id: Some(1),
                file_id: Some(1),
                repo: "api".to_owned(),
                name: "huge".to_owned(),
                kind: SymbolKind::Fn,
                scope: None,
                signature: Some("pub fn huge()".to_owned()),
                docstring: None,
                span: Span::new(1, 1, 1000, 1),
                is_exported: true,
                complexity: None,
            },
            file_path: "src/huge.ts".to_owned(),
            code: "huge code".to_owned(),
            start_line: 1,
            end_line: 1000,
        }],
        call_flows: Vec::new(),
        impact_summary: None,
        direct_relations: Vec::new(),
        entry_points: Vec::new(),
        transitive_consumers: Vec::new(),
        sources: vec![SourceFile {
            repo: "api".to_owned(),
            file_path: "src/huge.ts".to_owned(),
            line_count: 2_000,
            excerpts: vec![SourceExcerpt {
                start_line: 1,
                end_line: 2_000,
                code: (1..=2_000).map(|i| format!("{i}\tline {i}\n")).collect(),
            }],
            changed_since_index: false,
        }],
        flows: Vec::new(),
        not_shown: Vec::new(),
    };

    let out = narrate_explore(&res);
    assert!(
        out.contains("…truncated after line"),
        "must contain truncation notice, got: {out}"
    );
    assert!(
        out.contains("Call `graph_explore` with paths: [\"src/huge.ts:")
            && out.contains("-2000\"] for the rest; do not Read it."),
        "must name next line range follow up call, got: {out}"
    );
}

#[test]
fn related_memory_closes_the_markdown_answer() {
    let text = narrate_explore(&with_related(result("fetch")));
    let section = text.find("**Related memory**").expect(&text);
    assert!(
        text[section..].contains("- `docs/memory/gotcha/a.md` (gotcha) A | pipe"),
        "{text}"
    );
    assert!(!narrate_explore(&result("fetch")).contains("Related memory"));
}
