#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use serde::Serialize;

use super::*;
use crate::test_support::utf8_temp_dir;

#[test]
fn struct_fields_declared_out_of_order_still_serialize_sorted() {
    // Field declaration order is deliberately NOT alphabetical. If
    // `to_canonical_string` ever regressed to `serde_json::to_string`, this
    // would emit `{"zebra":...,"apple":...,"mango":...}` instead.
    #[derive(Serialize)]
    struct OutOfOrder {
        zebra: u8,
        apple: u8,
        mango: u8,
    }

    let rendered = to_canonical_string(&OutOfOrder {
        zebra: 1,
        apple: 2,
        mango: 3,
    })
    .unwrap();

    assert_eq!(
        rendered,
        "{\n  \"apple\": 2,\n  \"mango\": 3,\n  \"zebra\": 1\n}\n"
    );
}

#[test]
fn nested_objects_are_sorted_at_every_depth() {
    #[derive(Serialize)]
    struct Inner {
        zeta: u8,
        beta: u8,
    }

    #[derive(Serialize)]
    struct Outer {
        wombat: Inner,
        alpha: u8,
    }

    let rendered = to_canonical_string(&Outer {
        wombat: Inner { zeta: 1, beta: 2 },
        alpha: 3,
    })
    .unwrap();

    assert_eq!(
        rendered,
        "{\n  \"alpha\": 3,\n  \"wombat\": {\n    \"beta\": 2,\n    \"zeta\": 1\n  }\n}\n"
    );
}

#[test]
fn canonical_string_uses_two_space_indent_lf_and_one_trailing_newline() {
    #[derive(Serialize)]
    struct Point {
        x: u8,
        y: u8,
    }

    let rendered = to_canonical_string(&Point { x: 1, y: 2 }).unwrap();

    assert!(!rendered.contains('\r'), "must never emit CRLF");
    assert_eq!(rendered.matches('\n').count(), rendered.lines().count());
    assert!(rendered.ends_with('\n') && !rendered.ends_with("\n\n"));
    assert!(rendered.contains("\n  \"x\": 1,\n  \"y\": 2\n"));
}

#[test]
fn write_canonical_is_atomic_and_readable_back() {
    #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
    struct State {
        count: u32,
        name: String,
    }

    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let value = State {
        count: 3,
        name: "hall".to_owned(),
    };

    write_canonical(&path, &value).unwrap();

    let roundtripped: Option<State> = read(&path).unwrap();
    assert_eq!(roundtripped, Some(value));

    // No leftover temp file from the write-then-rename.
    let entries = fs::read_dir(&root).unwrap();
    assert_eq!(entries, vec![path]);
}

#[test]
fn write_canonical_overwrite_leaves_no_partial_state() {
    #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
    struct State {
        value: String,
    }

    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");

    write_canonical(
        &path,
        &State {
            value: "first".to_owned(),
        },
    )
    .unwrap();
    write_canonical(
        &path,
        &State {
            value: "second".to_owned(),
        },
    )
    .unwrap();

    let roundtripped: Option<State> = read(&path).unwrap();
    assert_eq!(
        roundtripped,
        Some(State {
            value: "second".to_owned()
        })
    );
}

#[test]
fn absent_file_reads_as_ok_none() {
    let (_dir, root) = utf8_temp_dir();
    let missing = root.join("missing.json");

    let value: Option<serde_json::Value> = read(&missing).unwrap();
    assert_eq!(value, None);
}

#[test]
fn unparseable_file_is_a_hard_error_naming_the_path() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("broken.json");
    fs::write_text(&path, "{ not json").unwrap();

    let result: Result<Option<serde_json::Value>, Error> = read(&path);

    match result {
        Err(Error::Parse { path: err_path, .. }) => assert_eq!(err_path, path),
        other => panic!("expected Error::Parse, got {other:?}"),
    }
}

#[test]
fn reading_accepts_non_canonical_but_valid_json() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("loose.json");
    // Unsorted keys, no trailing newline, four-space indent — none of that
    // matters for reading.
    fs::write_text(&path, "{\"b\":1,\"a\":2}").unwrap();

    let value: Option<serde_json::Value> = read(&path).unwrap();
    assert_eq!(value, Some(serde_json::json!({"a": 2, "b": 1})));
}
