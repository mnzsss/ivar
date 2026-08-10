#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::test_support::utf8_temp_dir;

#[test]
fn bytes_and_text_agree() {
    assert_eq!(bytes(b"hello"), text("hello"));
}

#[test]
fn bytes_is_lowercase_hex_sha256_with_no_prefix() {
    // Known vector: sha256("hello")
    assert_eq!(
        bytes(b"hello"),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn file_hashes_content_and_errors_if_absent() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("f.txt");
    fs::write_text(&path, "hello").unwrap();

    assert_eq!(file(&path).unwrap(), text("hello"));

    let missing = root.join("missing.txt");
    assert!(matches!(file(&missing), Err(Error::NotFound { .. })));
}

#[test]
fn tree_of_one_file_matches_the_documented_framing() {
    let (_dir, root) = utf8_temp_dir();
    fs::write_text(&root.join("a.txt"), "hello").unwrap();

    let expected_line = format!("a.txt:{}", text("hello"));
    let expected = format!("sha256:{}", text(&expected_line));

    assert_eq!(tree(&root).unwrap(), expected);
}

#[test]
fn tree_is_stable_regardless_of_creation_order() {
    let (_dir_a, root_a) = utf8_temp_dir();
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write_text(&root_a.join(name), name).unwrap();
    }

    let (_dir_b, root_b) = utf8_temp_dir();
    for name in ["c.txt", "a.txt", "b.txt"] {
        fs::write_text(&root_b.join(name), name).unwrap();
    }

    assert_eq!(tree(&root_a).unwrap(), tree(&root_b).unwrap());
}

#[test]
fn tree_changes_when_a_file_is_renamed_not_just_reordered() {
    let (_dir, root) = utf8_temp_dir();
    fs::write_text(&root.join("a.txt"), "same-content").unwrap();
    let before = tree(&root).unwrap();

    fs::remove_file(&root.join("a.txt")).unwrap();
    fs::write_text(&root.join("b.txt"), "same-content").unwrap();
    let after = tree(&root).unwrap();

    assert_ne!(before, after, "renaming a file must change the digest");
}

#[test]
fn tree_is_stable_across_nested_directories() {
    let (_dir, root) = utf8_temp_dir();
    fs::ensure_dir(&root.join("nested").join("deeper")).unwrap();
    fs::write_text(&root.join("top.txt"), "top").unwrap();
    fs::write_text(&root.join("nested").join("mid.txt"), "mid").unwrap();
    fs::write_text(&root.join("nested").join("deeper").join("low.txt"), "low").unwrap();

    let expected_lines = {
        let mut lines = vec![
            format!("nested/deeper/low.txt:{}", text("low")),
            format!("nested/mid.txt:{}", text("mid")),
            format!("top.txt:{}", text("top")),
        ];
        lines.sort();
        lines
    };
    let expected = format!("sha256:{}", text(&expected_lines.join("\n")));

    assert_eq!(tree(&root).unwrap(), expected);
}

#[test]
fn tree_excludes_dot_named_files_and_directories() {
    let (_dir, root) = utf8_temp_dir();
    fs::write_text(&root.join("visible.txt"), "visible").unwrap();
    fs::write_text(&root.join(".hidden-file"), "hidden").unwrap();
    fs::ensure_dir(&root.join(".hidden-dir")).unwrap();
    fs::write_text(&root.join(".hidden-dir").join("inside.txt"), "inside").unwrap();

    let expected_line = format!("visible.txt:{}", text("visible"));
    let expected = format!("sha256:{}", text(&expected_line));

    assert_eq!(tree(&root).unwrap(), expected);
}

#[test]
fn tree_excludes_symlinks_entirely() {
    let (_dir, root) = utf8_temp_dir();
    fs::write_text(&root.join("real.txt"), "real").unwrap();
    fs::create_symlink(&root.join("real.txt"), &root.join("link.txt")).unwrap();

    let expected_line = format!("real.txt:{}", text("real"));
    let expected = format!("sha256:{}", text(&expected_line));

    assert_eq!(tree(&root).unwrap(), expected);
}

#[test]
fn tree_of_empty_directory_is_the_hash_of_the_empty_string() {
    let (_dir, root) = utf8_temp_dir();

    assert_eq!(tree(&root).unwrap(), format!("sha256:{}", text("")));
}
