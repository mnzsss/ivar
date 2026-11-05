#![allow(clippy::unwrap_used)]

use super::*;
use crate::test_support::utf8_temp_dir;

#[test]
fn try_lock_exclusive_takes_a_lock_held_only_for_an_instant() {
    let (_guard, dir) = utf8_temp_dir();
    let path = dir.join("locks/memory.lock");
    let held = try_lock_exclusive(&path).unwrap().unwrap();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(5));
        drop(held);
    });
    assert!(try_lock_exclusive(&path).unwrap().is_some());
    releaser.join().unwrap();
}
