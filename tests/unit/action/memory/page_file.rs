#![allow(clippy::unwrap_used)]

use super::*;
use crate::test_support::seeded_hall;

#[test]
fn a_safe_native_id_names_the_file_and_an_unsafe_one_is_hashed() {
    assert_eq!(page_rel("abc-1.2_x"), ".ivar/memory/session/abc-1.2_x.md");
    let hashed = page_rel("../etc/passwd");
    assert!(hashed.starts_with(".ivar/memory/session/"));
    assert_eq!(
        hashed.len(),
        ".ivar/memory/session/".len() + 16 + ".md".len()
    );
    assert_eq!(page_rel(&"a".repeat(129)).len(), hashed.len());
    assert_ne!(page_rel(".hidden"), ".ivar/memory/session/.hidden.md");
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_temp_write_leaves_no_temp_file_behind() {
    let (_guard, root) = seeded_hall();
    let dir = root.join("docs/memory/decision");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink("/dev/full", temp_path(&dir)).unwrap();

    let result = publish(
        &dir.join("full.md"),
        "docs/memory/decision/full.md",
        "x\n",
        false,
    );

    assert!(result.is_err());
    let stray: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    assert!(stray.is_empty(), "{stray:?}");
}
