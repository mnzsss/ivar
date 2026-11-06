use super::*;

#[test]
fn an_intent_answer_never_exceeds_the_character_cap() {
    let query = (0..600)
        .map(|n| format!("services/api/src/file{n}.ts"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut res = result(&query);
    for n in 0..20 {
        let file = format!("src/module{n}.ts");
        res.primary_symbols
            .push(snippet(&format!("handler{n}"), "api", &file, 1));
        res.sources.push(source("api", &file, 400));
    }
    res.not_shown = (0..15)
        .map(|n| FileMention {
            repo: "api".into(),
            file_path: format!("services/api/src/deeply/nested/directory/not_shown_{n}.ts"),
            symbols: (0..8)
                .map(|s| MentionedSymbol {
                    name: format!("aLongDescriptiveSymbolName{s}"),
                    line: s,
                })
                .collect(),
        })
        .collect();

    let out = narrate_explore(&res);

    assert!(out.len() <= 24_000, "answer has {} chars", out.len());
}
fn file_match(path: String, excerpt: &str) -> FileMatch {
    FileMatch {
        weak: false,
        repo: "api".into(),
        file_path: path,
        match_kind: FileMatchKind::Content,
        start_line: 1,
        excerpt: excerpt.into(),
        content_truncated: false,
    }
}

fn source_for_match(fm: &FileMatch) -> SourceFile {
    SourceFile {
        repo: fm.repo.clone(),
        file_path: fm.file_path.clone(),
        line_count: 50,
        excerpts: vec![SourceExcerpt {
            start_line: 1,
            end_line: 50,
            code: format!("1\t{}\n", fm.excerpt),
        }],
        changed_since_index: false,
    }
}

fn oversized_ranked_result() -> ExploreResult {
    let query = "explore oversized ranked symbols";
    let mut res = result(query);
    for n in 0..25 {
        let file = format!("src/module_{n}.ts");
        res.primary_symbols
            .push(snippet(&format!("handlerFunction{n}"), "api", &file, 10));
        res.sources.push(source("api", &file, 300));
    }
    res.not_shown = (0..20)
        .map(|n| FileMention {
            repo: "api".into(),
            file_path: format!("services/api/src/deep/nested/extra_file_{n}.ts"),
            symbols: (0..5)
                .map(|s| MentionedSymbol {
                    name: format!("symbol{s}"),
                    line: s * 10 + 1,
                })
                .collect(),
        })
        .collect();
    res
}

#[test]
fn importer_list_uses_paths_without_source_bodies() {
    let mut res = result("session.service importers test files");
    res.file_matches = (0..30)
        .map(|n| {
            file_match(
                format!("tests/session/importer-{n}.test.ts"),
                "large source body",
            )
        })
        .collect();
    res.sources = res.file_matches.iter().map(source_for_match).collect();

    let out = narrate_explore(&res);

    assert!(
        out.contains("tests/session/importer-0.test.ts"),
        "got: {out}"
    );
    assert!(!out.contains("large source body"), "got: {out}");
    assert!(out.contains("Next: call `graph_explore`"), "got: {out}");
    assert!(out.len() <= MAX_OUTPUT_CHARS, "len: {}", out.len());
}

#[test]
fn truncated_output_names_omissions_and_exact_follow_up() {
    let res = oversized_ranked_result();

    let out = narrate_explore(&res);

    assert!(out.len() <= MAX_OUTPUT_CHARS, "len: {}", out.len());
    assert!(out.contains("Not shown"), "got: {out}");
    assert!(out.contains("{\"paths\":["), "got: {out}");
    assert!(out.contains("graph_explore"), "got: {out}");
}

#[test]
fn a_partial_answer_shows_source_for_strong_files_and_lists_weak_ones_last() {
    let mut res = result("changeset policy");
    res.confidence = crate::domain::graph::ExploreConfidence::Partial;
    res.primary_symbols
        .push(snippet("checkChangeset", "api", "src/changeset.ts", 10));
    let mut weak = snippet("loadRoute", "api", "src/route.ts", 3);
    weak.weak = true;
    weak.code.clear();
    res.primary_symbols.push(weak);
    res.sources.push(source("api", "src/changeset.ts", 20));

    let out = narrate_explore(&res);

    assert!(out.contains("Found 1 symbol across 1 file."), "{out}");
    assert!(out.contains("Confidence: partial"), "{out}");
    assert!(out.contains("line 1 of src/changeset.ts"), "{out}");
    let weak_at = out.find("**Weak matches").expect("weak list");
    assert!(weak_at > out.find("**Source**").expect("source section"));
    assert!(out.contains("- src/route.ts:3 `loadRoute`"), "{out}");
    assert!(!out.contains("line 1 of src/route.ts"));
}

#[test]
fn split_graph_output_files_stay_under_the_commit_size_limit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in [
        "src/action/graph/narrate/mod.rs",
        "src/action/graph/narrate/explore.rs",
        "src/action/graph/narrate/source.rs",
        "tests/unit/action/graph/narrate/explore.rs",
        "tests/unit/action/graph/mcp/explore_output/fanout.rs",
    ] {
        let size = std::fs::metadata(root.join(rel)).expect(rel).len();
        assert!(size < 20_000, "{rel} is {size} bytes");
    }
}
