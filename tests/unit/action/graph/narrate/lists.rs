use super::*;

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

    let out = narrate_callers("evaluateAccess", &[], &callers, &[]);

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
    let out = narrate_callers("orphan", &[], &[], &[]);
    assert!(out.contains("No callers of `orphan`"), "got: {out}");
    assert!(out.contains("refresh_index"));
}

/// An agent given only call sites grepped the name to find the definition and
/// the imports, two turns and 19K characters per discovery run in `tokens2`.
#[test]
fn callers_name_the_definition_type_uses_and_module_level_references() {
    let definitions = vec![SymbolLocation {
        symbol: symbol("Session", "web", 8),
        file_path: "src/auth/access.ts".into(),
    }];
    let mut type_use = caller(
        "AuthContextType",
        "src/components/AuthProvider.tsx",
        8,
        Provenance::Extracted,
    );
    type_use.edge_kind = EdgeKind::References;
    let references = vec![ReferenceSite {
        repo: "web".into(),
        file_path: "src/api/client.ts".into(),
        line: 1,
    }];

    let out = narrate_callers("Session", &definitions, &[type_use], &references);

    assert!(
        out.contains("Defined at src/auth/access.ts:8 (web)"),
        "got: {out}"
    );
    assert!(
        out.contains("`AuthContextType` in src/components/AuthProvider.tsx:8 · type use"),
        "got: {out}"
    );
    assert!(out.contains("src/api/client.ts:1"), "got: {out}");
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
            inferred: false,
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
