use super::*;

#[test]
fn complete_json_at_the_byte_boundary_stays_complete() {
    for requested in [false, true] {
        let limit = if requested { 24_000 } else { 18_000 };
        let base = serde_json::to_string_pretty(&empty_result()).unwrap().len();
        for size in [limit - 1, limit, limit + 1] {
            let mut res = empty_result();
            res.query = "x".repeat(size - base);
            let text = render_explore(&res, ExploreFormat::Json, requested).unwrap();
            assert!(text.len() <= limit);
            assert_eq!(
                text,
                render_explore(&res, ExploreFormat::Json, requested).unwrap(),
                "same result and request must produce identical output"
            );
            let parsed: Value = serde_json::from_str(&text).unwrap();
            if size <= limit {
                assert_eq!(parsed["query"], res.query);
                assert!(parsed.get("output").is_none());
                assert_eq!(text.len(), size);
            } else {
                assert_eq!(parsed["query"], "");
                assert_eq!(parsed["output"]["partial"], true);
                assert!(
                    parsed["output"]["omitted_fields"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|field| field == "query")
                );
                assert!(parsed["output"].get("next").is_none());
            }
        }
    }
}

#[test]
fn complete_compact_at_the_byte_boundary_stays_complete() {
    for requested in [false, true] {
        let limit = if requested { 24_000 } else { 18_000 };
        let mut res = empty_result();
        res.file_matches.push(crate::domain::graph::FileMatch {
            weak: false,
            repo: "api".into(),
            file_path: "src/boundary.rs".into(),
            match_kind: crate::domain::graph::FileMatchKind::Content,
            start_line: 1,
            excerpt: String::new(),
            content_truncated: false,
        });
        let base = crate::action::graph::compact::encode_explore(&res).len();
        for size in [limit - 1, limit, limit + 1] {
            res.file_matches[0].excerpt = "x".repeat(size - base);
            let text = render_explore(&res, ExploreFormat::Compact, requested).unwrap();
            assert!(text.len() <= limit);
            assert_eq!(
                text,
                render_explore(&res, ExploreFormat::Compact, requested).unwrap()
            );
            if size <= limit {
                assert_eq!(text.len(), size);
                assert!(!text.contains("#SCHEMA: partial|budget_bytes"));
                let row = text.lines().nth(1).unwrap();
                assert!(row.starts_with("api|src/boundary.rs|content|1|"));
                assert!(row.ends_with("|false"));
            } else {
                assert!(text.contains("file_matches|1"));
                assert!(text.contains(&format!("true|{limit}")));
            }
        }
    }
}

#[test]
fn oversized_first_identity_does_not_starve_a_later_primary_record() {
    let mut res = empty_result();
    res.primary_symbols = vec![
        primary(
            &"💡".repeat(10_000),
            "src/oversized.rs",
            "fn oversized() {}".into(),
        ),
        primary("keep", "src/keep.rs", "fn keep() {}".into()),
    ];
    for requested in [false, true] {
        for format in [ExploreFormat::Compact, ExploreFormat::Json] {
            let text = render_explore(&res, format, requested).unwrap();
            assert!(text.len() <= if requested { 24_000 } else { 18_000 });
            if format == ExploreFormat::Json {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "keep");
                assert_eq!(value["output"]["omitted"]["primary_symbols"], 1);
                assert_eq!(value["output"]["next"]["arguments"]["repo"], "api");
            } else {
                assert!(text.contains("|keep|fn|src/keep.rs|1|1|"));
                assert!(text.contains("primary_symbols|1"));
                assert!(!text.contains('💡'));
            }
        }
    }
}

#[test]
fn giant_escaped_symbol_keeps_the_prefix_that_fits_and_continues_after_it() {
    let mut res = empty_result();
    let mut large = primary("large", "src/large.rs", numbered_code(1, 230, "\"💡\\ "));
    large.end_line = 230;
    res.primary_symbols.push(large);
    res.primary_symbols
        .push(primary("keep", "src/keep.rs", "fn keep() {}".into()));
    let text = render_explore(&res, ExploreFormat::Json, false).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert!(text.len() <= 18_000);
    let symbols = value["primary_symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0]["symbol"]["name"], "large");
    assert_eq!(symbols[1]["symbol"]["name"], "keep");
    assert!(value["output"]["omitted"].get("primary_symbols").is_none());

    let returned_end = symbols[0]["end_line"].as_u64().unwrap();
    assert!((1..230).contains(&returned_end), "end {returned_end}");
    let code = symbols[0]["code"].as_str().unwrap();
    assert_eq!(code.lines().count() as u64, returned_end);
    assert!(
        code.lines()
            .last()
            .unwrap()
            .starts_with(&format!("{returned_end}\t"))
    );
    assert_eq!(
        value["output"]["next"]["arguments"],
        json!({"repo":"api","paths":[format!("src/large.rs:{}-230", returned_end + 1)],"format":"markdown"})
    );
    assert_eq!(
        value["output"]["truncated"][0]["section"],
        "primary_symbols"
    );
    assert_eq!(
        value["output"]["truncated"][0]["next_start"],
        returned_end + 1
    );
}

#[test]
fn compact_metadata_escapes_pipes_without_cutting_the_next_argument() {
    let mut res = empty_result();
    res.file_matches.push(crate::domain::graph::FileMatch {
        weak: false,
        repo: "api".into(),
        file_path: "src/a|b.rs".into(),
        match_kind: crate::domain::graph::FileMatchKind::Content,
        start_line: 2,
        excerpt: "x".repeat(30_000),
        content_truncated: false,
    });
    let text = render_explore(&res, ExploreFormat::Compact, false).unwrap();
    assert!(text.len() <= 18_000);
    let row = text
        .lines()
        .find(|line| line.starts_with("graph_explore|{"))
        .unwrap();
    assert_eq!(row.split('|').count(), 2);
    let next: Value = serde_json::from_str(row.split_once('|').unwrap().1).unwrap();
    assert_eq!(next["paths"], json!(["src/a|b.rs:2-41"]));
    assert!(text.contains("file_matches|1"));
}
