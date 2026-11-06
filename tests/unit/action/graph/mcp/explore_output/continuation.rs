use super::fanout::file_with_many_callers;
use super::*;

struct BigFile {
    db: crate::store::graph::db::GraphDb,
    temp: tempfile::TempDir,
}

fn big_file_fixture(lines: usize) -> BigFile {
    let temp = tempfile::tempdir().unwrap();
    let repo_dir = temp.path().join("api");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    let content: String = (1..=lines)
        .map(|n| format!("    let value_{n} = compute_{n}(); // filler filler filler\n"))
        .collect();
    std::fs::write(repo_dir.join("src/big.rs"), &content).unwrap();
    let db = crate::store::graph::db::GraphDb::open_in_memory().unwrap();
    db.insert_repo("api", repo_dir.to_str().unwrap(), "main", None)
        .unwrap();
    let file_id = db
        .upsert_file(
            "api",
            "src/big.rs",
            &crate::infra::hash::text(&content),
            1,
            content.len() as i64,
        )
        .unwrap();
    let mut run = primary("run", "src/big.rs", String::new()).symbol;
    run.id = None;
    run.file_id = Some(file_id);
    run.span = Span::new(10, 1, lines - 10, 1);
    db.insert_symbols(&[run]).unwrap();
    BigFile { db, temp }
}

/// Numbered lines of every excerpt in a JSON answer, as `(line, text)`.
fn source_lines(value: &Value) -> Vec<(usize, String)> {
    value["sources"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|source| source["excerpts"].as_array().unwrap())
        .flat_map(|excerpt| excerpt["code"].as_str().unwrap().lines())
        .map(|line| {
            let (number, text) = line.split_once('\t').unwrap();
            (number.parse().unwrap(), text.to_owned())
        })
        .collect()
}

#[test]
fn following_next_rebuilds_a_large_file_without_gaps_or_overlap() {
    const LINES: usize = 1_200;
    let fixture = big_file_fixture(LINES);
    let mut query = "src/big.rs".to_owned();
    let mut rebuilt: Vec<usize> = Vec::new();
    let mut hops = 0;
    loop {
        hops += 1;
        assert!(hops < 50, "continuation does not converge");
        let res = crate::action::graph::explore::explore_files(
            &fixture.db,
            fixture.temp.path(),
            &query,
            Some("api"),
        )
        .unwrap();
        let text = render_explore(&res, ExploreFormat::Json, true).unwrap();
        assert!(text.len() <= 24_000);
        let value: Value = serde_json::from_str(&text).unwrap();
        let lines = source_lines(&value);
        assert!(!lines.is_empty(), "hop {hops} returned no source: {text}");
        rebuilt.extend(lines.iter().map(|(n, _)| *n));

        let truncated = value["output"]["truncated"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["section"] == "sources");
        let Some(truncated) = truncated else {
            break;
        };
        let last = lines.last().unwrap().0;
        assert_eq!(truncated["next_start"], last + 1);
        assert_eq!(truncated["next_end"], LINES);
        let next = &value["output"]["next"]["arguments"]["paths"][0];
        assert_eq!(next, &json!(format!("src/big.rs:{}-{LINES}", last + 1)));
        query = next.as_str().unwrap().to_owned();
    }
    assert!(hops > 1, "the file was expected to exceed one response");
    assert_eq!(rebuilt, (1..=LINES).collect::<Vec<_>>());
}

#[test]
fn a_ranged_markdown_continuation_stays_inside_its_range() {
    let fixture = big_file_fixture(1_200);
    let res = crate::action::graph::explore::explore_files(
        &fixture.db,
        fixture.temp.path(),
        "src/big.rs:100-700",
        Some("api"),
    )
    .unwrap();
    let text = crate::action::graph::narrate::narrate_requested_files(&res);
    assert!(text.contains("…truncated after line"), "got: {text}");
    assert!(
        text.contains("-700\"] for the rest"),
        "continuation must end at the requested range: {text}"
    );
}

fn weak_result(confidence: crate::domain::graph::ExploreConfidence) -> ExploreResult {
    let mut res = empty_result();
    res.query = "intent query".into();
    res.confidence = confidence;
    for i in 0..1500 {
        let mut weak = primary(
            &format!("noise{i}"),
            &format!("src/noise{i}.rs"),
            String::new(),
        );
        weak.weak = true;
        res.primary_symbols.push(weak);
    }
    res
}

#[test]
fn json_carries_the_confidence_whole_and_partial() {
    use crate::domain::graph::ExploreConfidence::{NoMatch, Partial};
    for confidence in [NoMatch, Partial] {
        let mut res = weak_result(confidence);
        res.primary_symbols.truncate(2);
        let whole: Value =
            serde_json::from_str(&render_explore(&res, ExploreFormat::Json, false).unwrap())
                .unwrap();
        assert_eq!(whole["confidence"], confidence.as_str());
        assert_eq!(whole["primary_symbols"][0]["weak"], true);
        assert!(whole.get("output").is_none());

        let big = weak_result(confidence);
        let text = render_explore(&big, ExploreFormat::Json, false).unwrap();
        assert!(text.len() <= 18_000);
        let cut: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(cut["output"]["partial"], true);
        assert_eq!(cut["confidence"], confidence.as_str());
    }
}

#[test]
fn json_leaves_a_high_confidence_out_even_when_cut() {
    let mut res = weak_result(crate::domain::graph::ExploreConfidence::High);
    res.primary_symbols.iter_mut().for_each(|s| s.weak = false);
    let cut: Value =
        serde_json::from_str(&render_explore(&res, ExploreFormat::Json, false).unwrap()).unwrap();
    assert_eq!(cut["output"]["partial"], true);
    assert!(cut.get("confidence").is_none());
}

#[test]
fn compact_leads_with_the_confidence_and_a_weak_column_whole_and_partial() {
    use crate::domain::graph::ExploreConfidence::NoMatch;
    let mut small = weak_result(NoMatch);
    small.primary_symbols.truncate(2);
    let text = render_explore(&small, ExploreFormat::Compact, false).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[..3],
        [
            "#SCHEMA: confidence",
            "none",
            "#SCHEMA: id|name|kind|file|line|col|complexity|weak"
        ]
    );
    assert!(lines[3].ends_with("|true"));

    let big = weak_result(NoMatch);
    let text = render_explore(&big, ExploreFormat::Compact, false).unwrap();
    assert!(text.len() <= 18_000);
    assert!(text.starts_with("#SCHEMA: confidence\nnone\n#SCHEMA: id|name"));
    assert!(text.contains("#SCHEMA: partial|budget_bytes"));
    assert!(text.contains("primary_symbols|"));
}

const RELATED_MEMORY_SCHEMA_FOR_TEST: &str = crate::action::graph::compact::RELATED_MEMORY_SCHEMA;

#[test]
fn related_memory_is_dropped_first_and_replaced_by_a_memory_query_continuation() {
    let mut res = file_with_many_callers();
    res.related_memory = vec![crate::domain::memory_anchor::RelatedPage {
        path: "docs/memory/gotcha/a.md".into(),
        kind: "gotcha".into(),
        title: "x".repeat(400),
        anchors: 1,
    }];
    let full = render_explore(&res, ExploreFormat::Json, false).unwrap();
    let value: Value = serde_json::from_str(&full).unwrap();
    assert!(value.get("related_memory").is_none(), "{full}");
    assert_eq!(value["related_memory_omitted"]["tool"], "memory_query");
    assert_eq!(
        value["related_memory_omitted"]["arguments"]["query"],
        res.query
    );

    let compact = render_explore(&res, ExploreFormat::Compact, false).unwrap();
    assert!(!compact.contains(RELATED_MEMORY_SCHEMA_FOR_TEST));
    assert!(compact.contains("memory_query|"), "{compact}");
}
