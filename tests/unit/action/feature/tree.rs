#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;

#[test]
fn empty_input_produces_empty_output() {
    assert_eq!(tree_prefixes(&[]), Vec::<String>::new());
}

#[test]
fn roots_only_produce_empty_prefixes() {
    let depths = [0, 0, 0];
    assert_eq!(tree_prefixes(&depths), vec!["", "", ""]);
}

#[test]
fn nested_tree_renders_standard_and_last_child_branches() {
    // Structure:
    // root (depth 0)
    // ├── child_a (depth 1)
    // │   └── grand_a (depth 2)
    // └── child_b (depth 1)
    let depths = [0, 1, 2, 1];
    assert_eq!(
        tree_prefixes(&depths),
        vec![
            "".to_owned(),
            "├── ".to_owned(),
            "│   └── ".to_owned(),
            "└── ".to_owned(),
        ]
    );
}

#[test]
fn last_child_continuation_spaces_do_not_draw_vertical_bars() {
    // Structure:
    // root1 (depth 0)
    // └── child_a (depth 1)
    //     └── grand_a (depth 2)
    // root2 (depth 0)
    let depths = [0, 1, 2, 0];
    assert_eq!(
        tree_prefixes(&depths),
        vec![
            "".to_owned(),
            "└── ".to_owned(),
            "    └── ".to_owned(),
            "".to_owned(),
        ]
    );
}

#[test]
fn multiple_siblings_at_various_depths() {
    // root (0)
    // ├── a (1)
    // │   ├── a1 (2)
    // │   └── a2 (2)
    // └── b (1)
    let depths = [0, 1, 2, 2, 1];
    assert_eq!(
        tree_prefixes(&depths),
        vec![
            "".to_owned(),
            "├── ".to_owned(),
            "│   ├── ".to_owned(),
            "│   └── ".to_owned(),
            "└── ".to_owned(),
        ]
    );
}
