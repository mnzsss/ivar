use super::*;

#[test]
fn structured_fanout_is_bounded_and_continues_in_the_omitted_consumers_repo() {
    let (db, temp) = fanout_fixture();
    for format in ["compact", "json"] {
        let (text, failed) = call_tool(
            &db,
            temp.path(),
            "graph_explore",
            json!({"query":"fanout_root","repo":"my-repo","format":format}),
        );
        assert!(!failed, "{text}");
        assert!(text.len() <= 18_000, "{format}: {} bytes", text.len());
        let (next, shown, omitted) = if format == "json" {
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["output"]["partial"], true);
            assert_eq!(value["output"]["budget_bytes"], 18_000);
            assert_eq!(value["primary_symbols"][0]["symbol"]["name"], "fanout_root");
            let shown = value["direct_relations"].as_array().unwrap().len();
            let omitted = value["output"]["omitted"]["direct_relations"]
                .as_u64()
                .unwrap();
            (value["output"]["next"]["arguments"].clone(), shown, omitted)
        } else {
            assert!(text.contains("|fanout_root|fn|"), "{text}");
            assert!(text.contains("#SCHEMA: partial|budget_bytes\ntrue|18000"));
            let next_line = text
                .lines()
                .find(|line| line.starts_with("graph_explore|{"))
                .unwrap();
            let next: Value = serde_json::from_str(next_line.split_once('|').unwrap().1).unwrap();
            let mut schema = "";
            let mut shown = 0;
            let mut omitted = None;
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("#SCHEMA: ") {
                    schema = value;
                    continue;
                }
                assert_eq!(line.split('|').count(), schema.split('|').count(), "{line}");
                if schema.starts_with("source_symbol|") {
                    shown += 1;
                }
                if schema == "omitted_category|omitted_records"
                    && let Some(count) = line.strip_prefix("direct_relations|")
                {
                    omitted = Some(count.parse::<u64>().unwrap());
                }
            }
            (next, shown, omitted.unwrap())
        };
        assert_eq!(shown as u64 + omitted, 400);
        assert!(omitted > 0);
        assert_eq!(next["repo"], "consumer-repo");
        assert_eq!(next["format"], "markdown");
        let requested = next["paths"][0].as_str().unwrap();
        let (path, range) = requested.rsplit_once(':').unwrap();
        let (start, end) = range.split_once('-').unwrap();
        let start: usize = start.parse().unwrap();
        let end: usize = end.parse().unwrap();
        assert!(start >= 1 && end >= start && end - start < 40);
        let filename = std::path::Path::new(path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let suffix = filename
            .strip_prefix("caller_")
            .unwrap()
            .strip_suffix(".rs")
            .unwrap();
        let marker = format!("// omitted_evidence_{suffix}");
        let (follow_up, follow_up_failed) = call_tool(&db, temp.path(), "graph_explore", next);
        assert!(!follow_up_failed, "{follow_up}");
        assert!(follow_up.contains(&marker), "missing {marker}: {follow_up}");
    }
}

#[test]
fn an_answer_names_the_indexed_commit_in_its_own_block() {
    let (db, temp) = setup_test_mcp_db();
    let (checkout, head) = hall_with_core_checkout(temp.path());
    db.insert_repo("core", checkout.as_str(), "main", Some(&head))
        .unwrap();
    let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();

    let blocks = explore_content(&db, temp.path(), &root);

    assert_eq!(blocks.len(), 2, "answer plus index notice: {blocks:?}");
    assert!(
        serde_json::from_str::<Value>(&blocks[0]).is_ok(),
        "the JSON answer stays parseable: {}",
        blocks[0]
    );
    assert_eq!(
        blocks[1],
        format!("Graph index commits: core@{}.", &head[..7])
    );
}

#[test]
fn an_answer_warns_when_the_index_lags_the_checkout_head() {
    let (db, temp) = setup_test_mcp_db();
    let (checkout, indexed) = hall_with_core_checkout(temp.path());
    db.insert_repo("core", checkout.as_str(), "main", Some(&indexed))
        .unwrap();
    std::fs::write(checkout.join("b.txt"), "b").unwrap();
    git_in(checkout.as_std_path(), &["add", "."]);
    git_in(checkout.as_std_path(), &["commit", "-m", "two"]);
    let head = git_in(checkout.as_std_path(), &["rev-parse", "HEAD"]);
    let root = camino::Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();

    let blocks = explore_content(&db, temp.path(), &root);

    let notice = blocks.last().unwrap();
    assert!(
        notice.contains(&format!("core@{}", &indexed[..7])),
        "{notice}"
    );
    assert!(notice.contains("Warning:"), "{notice}");
    assert!(
        notice.contains(&format!("checkout HEAD is {}", &head[..7])),
        "{notice}"
    );
}
