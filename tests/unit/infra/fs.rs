#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::test_support::utf8_temp_dir;

#[test]
fn absent_file_is_ok_none_not_an_error() {
    let (_dir, root) = utf8_temp_dir();
    let missing = root.join("does-not-exist.txt");

    assert_eq!(read_text(&missing).unwrap(), None);
    assert_eq!(read_bytes(&missing).unwrap(), None);
    assert_eq!(stat(&missing).unwrap().map(|_| ()), None);
    assert_eq!(read_symlink(&missing).unwrap(), SymlinkTarget::Absent);
}

#[test]
fn read_symlink_reports_present_but_not_a_symlink_as_a_permanent_answer() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("real-file.txt");
    write_text(&path, "not a link").unwrap();

    assert_eq!(read_symlink(&path).unwrap(), SymlinkTarget::NotASymlink);

    let dir = root.join("real-dir");
    ensure_dir(&dir).unwrap();
    assert_eq!(read_symlink(&dir).unwrap(), SymlinkTarget::NotASymlink);
}

#[test]
fn read_symlink_resolves_a_real_symlink() {
    let (_dir, root) = utf8_temp_dir();
    let target = root.join("target.txt");
    write_text(&target, "x").unwrap();
    let link = root.join("link");
    create_symlink(&target, &link).unwrap();

    assert_eq!(read_symlink(&link).unwrap(), SymlinkTarget::Target(target));
}

#[cfg(unix)]
#[test]
fn unreadable_file_is_a_hard_error_not_absent() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("secret.txt");
    write_text(&path, "shh").unwrap();
    fs_err::set_permissions(path.as_std_path(), std::fs::Permissions::from_mode(0o000)).unwrap();

    let result = read_text(&path);

    // Restore permissions so TempDir can clean up.
    fs_err::set_permissions(path.as_std_path(), std::fs::Permissions::from_mode(0o644)).unwrap();

    assert!(
        matches!(result, Err(Error::Read { .. })),
        "expected a hard read error, got {result:?}"
    );
}

#[test]
fn write_atomic_replaces_content_and_leaves_no_temp_file() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");

    write_atomic(&path, b"first").unwrap();
    assert_eq!(read_bytes(&path).unwrap().unwrap(), b"first");

    write_atomic(&path, b"second").unwrap();
    assert_eq!(read_bytes(&path).unwrap().unwrap(), b"second");

    let leftovers: Vec<_> = read_dir(&root)
        .unwrap()
        .into_iter()
        .filter(|entry| entry != &path)
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
}

#[test]
fn write_atomic_never_leaves_a_half_written_file_on_the_happy_path() {
    // There is no window in which `path` exists with partial content: the
    // temp file is fully written before the rename, and rename is atomic.
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("state.json");
    let payload = vec![b'x'; 64 * 1024];

    write_atomic(&path, &payload).unwrap();

    assert_eq!(read_bytes(&path).unwrap().unwrap(), payload);
}

#[test]
fn replace_symlink_is_never_observed_missing_or_pointing_elsewhere() {
    // What `replace_symlink` actually guarantees (see its doc comment):
    // `rename()` never leaves `link` momentarily absent, so a reader must
    // never see `Ok(None)` (`ENOENT`) mid-replace, and every *successful*
    // read must name one of the two real targets, never a temp path or
    // garbage.
    //
    // What it does NOT guarantee, measured independently on macOS/APFS
    // (plain `std::fs`, no `fs-err`, no `ivar` code): a concurrent reader
    // resolving the link can still hit a transient error — three runs of
    // 300 concurrent replaces against one reader measured
    // `reads=1466 readlink_err=34 lstat_err=0 stat_err=22 open_err=14`.
    // `read_symlink` retries its own `readlink` call for exactly this
    // (and that retry alone cleared it completely over 300k+ iterations
    // in isolation), but this test still tolerates an `Err` surfacing
    // here rather than asserting zero errors — a wish is not a
    // guarantee, and the retry count is bounded, not infinite.
    let (_dir, root) = utf8_temp_dir();
    let target_a = root.join("a");
    let target_b = root.join("b");
    write_text(&target_a, "a").unwrap();
    write_text(&target_b, "b").unwrap();

    let link = root.join("link");
    create_symlink(&target_a, &link).unwrap();

    let observed_missing = Arc::new(AtomicBool::new(false));
    let observed_wrong = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    let reader = {
        let link = link.clone();
        let target_a = target_a.clone();
        let target_b = target_b.clone();
        let observed_missing = Arc::clone(&observed_missing);
        let observed_wrong = Arc::clone(&observed_wrong);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match read_symlink(&link) {
                    Ok(SymlinkTarget::Absent) => {
                        observed_missing.store(true, Ordering::Relaxed);
                    }
                    Ok(SymlinkTarget::NotASymlink) => {
                        observed_wrong.store(true, Ordering::Relaxed);
                    }
                    Ok(SymlinkTarget::Target(observed))
                        if observed != target_a && observed != target_b =>
                    {
                        observed_wrong.store(true, Ordering::Relaxed);
                    }
                    Ok(SymlinkTarget::Target(_)) => {}
                    // A transient resolution error (see the test comment
                    // above) — neither "missing" nor "wrong", so it does
                    // not fail the assertions this test makes.
                    Err(_) => {}
                }
            }
        })
    };

    for iteration in 0..200 {
        let target = if iteration % 2 == 0 {
            &target_b
        } else {
            &target_a
        };
        replace_symlink(target, &link).unwrap();
    }

    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    assert!(
        !observed_missing.load(Ordering::Relaxed),
        "a reader observed the symlink missing mid-replace"
    );
    assert!(
        !observed_wrong.load(Ordering::Relaxed),
        "a reader observed a target other than the two real ones"
    );
    assert!(matches!(
        read_symlink(&link).unwrap(),
        SymlinkTarget::Target(_)
    ));
}

#[cfg(unix)]
#[test]
fn replace_symlink_if_changed_is_a_no_op_when_the_target_is_unchanged() {
    use std::os::unix::fs::MetadataExt;

    let (_dir, root) = utf8_temp_dir();
    let target = root.join("target");
    write_text(&target, "x").unwrap();
    let link = root.join("link");
    create_symlink(&target, &link).unwrap();

    let before_ino = fs_err::symlink_metadata(link.as_std_path()).unwrap().ino();

    replace_symlink_if_changed(&target, &link).unwrap();

    let after_ino = fs_err::symlink_metadata(link.as_std_path()).unwrap().ino();
    assert_eq!(
        before_ino, after_ino,
        "an unchanged target must not rename the link — same inode"
    );
}

#[cfg(unix)]
#[test]
fn replace_symlink_if_changed_replaces_when_the_target_differs() {
    use std::os::unix::fs::MetadataExt;

    let (_dir, root) = utf8_temp_dir();
    let target_a = root.join("a");
    let target_b = root.join("b");
    write_text(&target_a, "a").unwrap();
    write_text(&target_b, "b").unwrap();
    let link = root.join("link");
    create_symlink(&target_a, &link).unwrap();

    let before_ino = fs_err::symlink_metadata(link.as_std_path()).unwrap().ino();

    replace_symlink_if_changed(&target_b, &link).unwrap();

    let after_ino = fs_err::symlink_metadata(link.as_std_path()).unwrap().ino();
    assert_ne!(
        before_ino, after_ino,
        "a changed target must rename the link — different inode"
    );
    assert_eq!(
        read_symlink(&link).unwrap(),
        SymlinkTarget::Target(target_b)
    );
}

#[test]
fn read_dir_is_sorted() {
    let (_dir, root) = utf8_temp_dir();
    for name in ["c", "a", "b"] {
        write_text(&root.join(name), name).unwrap();
    }

    let entries: Vec<_> = read_dir(&root)
        .unwrap()
        .into_iter()
        .map(|path| path.file_name().unwrap().to_owned())
        .collect();

    assert_eq!(
        entries,
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

#[test]
fn ensure_dir_is_idempotent() {
    let (_dir, root) = utf8_temp_dir();
    let nested = root.join("a").join("b").join("c");

    ensure_dir(&nested).unwrap();
    ensure_dir(&nested).unwrap();

    assert!(is_dir(&nested).unwrap());
}

#[test]
fn remove_path_is_idempotent_and_recursive() {
    let (_dir, root) = utf8_temp_dir();
    let nested = root.join("a").join("b");
    ensure_dir(&nested).unwrap();
    write_text(&nested.join("f.txt"), "x").unwrap();

    remove_path(&root.join("a")).unwrap();
    assert!(!exists(&root.join("a")).unwrap());

    // Removing it again is success, not an error.
    remove_path(&root.join("a")).unwrap();
}

#[test]
fn prune_empty_parents_reclaims_a_nested_prefix_up_to_the_boundary() {
    let (_dir, root) = utf8_temp_dir();
    let leaf = root.join("repo").join("feat").join("login");
    ensure_dir(&leaf).unwrap();

    remove_path(&leaf).unwrap();
    prune_empty_parents(&leaf, &root.join("repo"));

    assert!(!exists(&root.join("repo/feat")).unwrap());
    // The boundary itself is never removed, empty or not.
    assert!(is_dir(&root.join("repo")).unwrap());
}

#[test]
fn prune_empty_parents_stops_at_a_prefix_a_sibling_still_occupies() {
    let (_dir, root) = utf8_temp_dir();
    let boundary = root.join("repo");
    let leaf = boundary.join("feat").join("login");
    let sibling = boundary.join("feat").join("signup");
    ensure_dir(&leaf).unwrap();
    ensure_dir(&sibling).unwrap();

    remove_path(&leaf).unwrap();
    prune_empty_parents(&leaf, &boundary);

    assert!(is_dir(&sibling).unwrap());
    assert!(is_dir(&boundary.join("feat")).unwrap());
}

#[test]
fn prune_empty_parents_never_walks_outside_the_boundary() {
    let (_dir, root) = utf8_temp_dir();
    let outside = root.join("elsewhere").join("leaf");
    ensure_dir(&outside).unwrap();
    let boundary = root.join("repo");
    ensure_dir(&boundary).unwrap();

    remove_path(&outside).unwrap();
    prune_empty_parents(&outside, &boundary);

    // `outside` is not under `boundary`, so nothing was pruned.
    assert!(is_dir(&root.join("elsewhere")).unwrap());
}

#[test]
fn remove_path_unlinks_a_symlink_without_following_it() {
    let (_dir, root) = utf8_temp_dir();
    let real_dir = root.join("real");
    ensure_dir(&real_dir).unwrap();
    write_text(&real_dir.join("keep.txt"), "keep").unwrap();

    let link = root.join("link-to-real");
    create_symlink(&real_dir, &link).unwrap();

    remove_path(&link).unwrap();

    assert!(!exists(&link).unwrap());
    assert!(exists(&real_dir.join("keep.txt")).unwrap());
}

#[cfg(unix)]
#[test]
fn chmod_clears_write_bits_for_read_only_worktrees() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("f.txt");
    write_text(&path, "x").unwrap();

    let current_mode = stat(&path).unwrap().unwrap().permissions().mode();
    chmod(&path, current_mode & !0o222).unwrap();

    let mode = stat(&path).unwrap().unwrap().permissions().mode();
    assert_eq!(mode & 0o222, 0);

    // Restore so TempDir can clean up.
    chmod(&path, current_mode).unwrap();
}

// -- the read-only guard: unix_mode / clear_write_bits / restore_write_bits

#[cfg(unix)]
#[test]
fn unix_mode_reports_mode_bits_or_absence() {
    let (_dir, root) = utf8_temp_dir();
    let path = root.join("f.txt");
    write_text(&path, "x").unwrap();

    let mode = unix_mode(&path).unwrap().unwrap();
    assert_ne!(mode & 0o222, 0, "a freshly written file is writable");
    assert_eq!(unix_mode(&root.join("missing")).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn clear_write_bits_removes_them_and_is_idempotent() {
    use std::os::unix::fs::MetadataExt;

    let (_dir, root) = utf8_temp_dir();
    let path = root.join("f.txt");
    write_text(&path, "x").unwrap();
    let original = unix_mode(&path).unwrap().unwrap();

    clear_write_bits(&path).unwrap();
    assert_eq!(unix_mode(&path).unwrap().unwrap() & 0o222, 0);

    // Idempotent: the second call must not even touch the inode.
    let ino = fs_err::symlink_metadata(path.as_std_path()).unwrap().ino();
    clear_write_bits(&path).unwrap();
    assert_eq!(
        fs_err::symlink_metadata(path.as_std_path()).unwrap().ino(),
        ino,
        "an already-guarded path must not be re-chmodded"
    );

    // Restore so TempDir can clean up.
    chmod(&path, original).unwrap();
}

/// A lift that restored `mode | 0o222` handed a 755 worktree back as 777 —
/// world-writable, and left that way if the process died mid-lift. Only the
/// owner's bit comes back, which is the one ivar runs as.
#[cfg(unix)]
#[test]
fn restore_write_bits_never_widens_past_the_owner() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o755).unwrap();

    clear_write_bits(&dir).unwrap();
    assert_eq!(unix_mode(&dir).unwrap().unwrap() & 0o777, 0o555);
    restore_write_bits(&dir).unwrap();

    assert_eq!(
        unix_mode(&dir).unwrap().unwrap() & 0o777,
        0o755,
        "the lift must not hand back group or other write"
    );

    // A group-writable path comes back owner-writable: the guard does not
    // record what it cleared, and widening is the worse of the two errors.
    chmod(&dir, 0o664).unwrap();
    clear_write_bits(&dir).unwrap();
    restore_write_bits(&dir).unwrap();
    assert_eq!(unix_mode(&dir).unwrap().unwrap() & 0o777, 0o644);
}

#[cfg(unix)]
#[test]
fn restore_write_bits_undoes_the_guard_and_is_idempotent() {
    use std::os::unix::fs::MetadataExt;

    let (_dir, root) = utf8_temp_dir();
    let path = root.join("f.txt");
    write_text(&path, "x").unwrap();
    chmod(&path, 0o644).unwrap();
    let original = unix_mode(&path).unwrap().unwrap();

    clear_write_bits(&path).unwrap();
    restore_write_bits(&path).unwrap();
    assert_eq!(
        unix_mode(&path).unwrap().unwrap(),
        original,
        "a lift must hand back the mode the guard took, bit for bit"
    );

    let ino = fs_err::symlink_metadata(path.as_std_path()).unwrap().ino();
    restore_write_bits(&path).unwrap();
    assert_eq!(
        fs_err::symlink_metadata(path.as_std_path()).unwrap().ino(),
        ino,
        "an already-writable path must not be re-chmodded"
    );
}

#[cfg(unix)]
#[test]
fn write_bits_are_restored_when_the_operation_fails() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o755).unwrap();
    clear_write_bits(&dir).unwrap();
    let before = unix_mode(&dir).unwrap();

    let unreadable_dir = root.join("unreadable");
    fs_err::create_dir_all(unreadable_dir.as_std_path()).unwrap();
    let file_inside = unreadable_dir.join("file.txt");
    write_text(&file_inside, "test").unwrap();
    chmod(&unreadable_dir, 0o000).unwrap();

    let result = LiftedGuard::lift(&[&dir, &file_inside]);
    assert!(result.is_err());

    chmod(&unreadable_dir, 0o755).unwrap();

    assert_eq!(
        unix_mode(&dir).unwrap(),
        before,
        "a failed operation must not leave a read-only repo writable"
    );
}

#[cfg(unix)]
#[test]
fn write_bits_are_restored_after_successful_scope() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o755).unwrap();
    clear_write_bits(&dir).unwrap();
    let before = unix_mode(&dir).unwrap();

    {
        let _lifted = LiftedGuard::lift(&[&dir]).expect("lift");
        assert!(unix_mode(&dir).unwrap().unwrap() & 0o222 != 0);
    }

    assert_eq!(
        unix_mode(&dir).unwrap(),
        before,
        "exiting guard scope must restore original read-only permissions"
    );
}

#[cfg(unix)]
#[test]
fn partial_write_guard_lift_failure_restores_already_lifted_worktree() {
    let (_dir, root) = utf8_temp_dir();
    let dir1 = root.join("worktree1");
    ensure_dir(&dir1).unwrap();
    chmod(&dir1, 0o755).unwrap();
    clear_write_bits(&dir1).unwrap();
    let before = unix_mode(&dir1).unwrap();

    let unreadable_dir = root.join("unreadable");
    fs_err::create_dir_all(unreadable_dir.as_std_path()).unwrap();
    let file_inside = unreadable_dir.join("file.txt");
    write_text(&file_inside, "test").unwrap();
    chmod(&unreadable_dir, 0o000).unwrap();

    let result = LiftedGuard::lift(&[&dir1, &file_inside]);
    assert!(result.is_err(), "lift must fail on inaccessible path");

    chmod(&unreadable_dir, 0o755).unwrap();

    let after = unix_mode(&dir1).unwrap();
    assert_eq!(
        before, after,
        "already lifted worktree must be restored if lift fails on a later worktree"
    );
}

#[cfg(unix)]
#[test]
fn exact_mode_restoration_preserves_original_permission_bits() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o500).unwrap();
    let before = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(before & 0o777, 0o500);

    {
        let _lifted = LiftedGuard::lift(&[&dir]).expect("lift");
    }

    let after = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(
        after & 0o777,
        0o500,
        "exact mode bits 0o500 must be restored, not altered to 0o555"
    );
}

#[cfg(unix)]
#[test]
fn lifted_guard_restores_the_exact_mode_on_drop() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o755).unwrap();
    clear_write_bits(&dir).unwrap();

    let cleared_mode = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(cleared_mode & 0o777, 0o555);
    assert_eq!(cleared_mode & 0o222, 0);

    {
        let _guard = LiftedGuard::lift(&[&dir]).expect("lift");
        let during_mode = unix_mode(&dir).unwrap().unwrap();
        assert_eq!(during_mode & 0o200, 0o200);
    }

    let restored_mode = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(restored_mode & 0o777, 0o555);
}

#[cfg(unix)]
#[test]
fn lifted_guard_restores_on_early_return() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o755).unwrap();
    clear_write_bits(&dir).unwrap();
    let before = unix_mode(&dir).unwrap().unwrap();

    let helper = |path: &camino::Utf8Path| -> Result<(), &'static str> {
        let _guard = LiftedGuard::lift(&[path]).map_err(|_| "lift failed")?;
        Err("early error")
    };

    let res = helper(&dir);
    assert!(res.is_err());

    let after = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(
        after, before,
        "guard must be restored on early return from function"
    );
}

#[cfg(unix)]
#[test]
fn lifted_guard_lifting_an_unguarded_path_records_nothing() {
    let (_dir, root) = utf8_temp_dir();
    let dir = root.join("worktree");
    ensure_dir(&dir).unwrap();
    chmod(&dir, 0o755).unwrap();
    let before = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(before & 0o777, 0o755);

    {
        let _guard = LiftedGuard::lift(&[&dir]).expect("lift");
    }

    let after = unix_mode(&dir).unwrap().unwrap();
    assert_eq!(after & 0o777, 0o755);
}

#[cfg(unix)]
#[test]
fn lifted_guard_lifting_an_absent_path_is_not_an_error() {
    let (_dir, root) = utf8_temp_dir();
    let absent = root.join("nonexistent");

    let result = LiftedGuard::lift(&[&absent]);
    assert!(result.is_ok());
}

#[test]
fn rename_moves_a_directory() {
    let (_dir, root) = utf8_temp_dir();
    let from = root.join("from");
    fs_err::create_dir_all(from.as_std_path()).unwrap();
    write_text(&from.join("keep.txt"), "x").unwrap();
    let to = root.join("to");

    rename(&from, &to).unwrap();

    assert!(!exists(&from).unwrap());
    assert!(is_file(&to.join("keep.txt")).unwrap());
}

#[test]
fn not_utf8_error_converts_to_a_blocked_failure() {
    let failure: Failure = Error::NotUtf8 {
        display: "bad".to_owned(),
    }
    .into();
    assert_eq!(failure.code, "fs.not_utf8");
}

// -- data_dir: tested through the pure `data_dir_from` cascade so nothing
// here touches the process environment (which races across concurrently-run
// tests — see `github::token_from` for the same pattern).

#[test]
fn data_dir_from_uses_xdg_data_home_when_absolute() {
    let resolved = data_dir_from(
        Some("/custom/data".to_owned()),
        Some("/home/someone".to_owned()),
        None,
        None,
        "linux",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/custom/data"));
}

#[test]
fn data_dir_from_falls_through_when_xdg_data_home_is_relative() {
    let resolved = data_dir_from(
        Some("relative/data".to_owned()),
        Some("/home/someone".to_owned()),
        None,
        None,
        "linux",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/home/someone/.local/share"));
}

#[test]
fn data_dir_from_falls_through_when_xdg_data_home_is_empty() {
    let resolved = data_dir_from(
        Some(String::new()),
        Some("/home/someone".to_owned()),
        None,
        None,
        "linux",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/home/someone/.local/share"));
}

#[test]
fn data_dir_from_uses_home_when_xdg_data_home_is_unset() {
    let resolved =
        data_dir_from(None, Some("/home/someone".to_owned()), None, None, "linux").unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/home/someone/.local/share"));
}

#[test]
fn data_dir_from_fails_naming_what_it_looked_for_when_neither_resolves() {
    let failure = data_dir_from(None, None, None, None, "linux").unwrap_err();
    assert_eq!(failure.code, "fs.data_dir");
    assert!(
        failure
            .expected
            .as_deref()
            .unwrap_or("")
            .contains("XDG_DATA_HOME")
    );
    assert!(failure.expected.as_deref().unwrap_or("").contains("HOME"));
}

#[test]
fn data_dir_from_fails_when_xdg_data_home_is_relative_and_home_is_unset() {
    let failure =
        data_dir_from(Some("relative/data".to_owned()), None, None, None, "linux").unwrap_err();
    assert_eq!(failure.code, "fs.data_dir");
}

// -- data_dir_from: macOS cases ------------------------------------------------

#[test]
fn data_dir_from_macos_uses_xdg_when_absolute() {
    let resolved = data_dir_from(
        Some("/custom/data".to_owned()),
        Some("/Users/someone".to_owned()),
        Some("/Users/someone/AppData/Roaming".to_owned()),
        None,
        "macos",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/custom/data"));
}

#[test]
fn data_dir_from_macos_falls_through_to_home_library_application_support() {
    let resolved =
        data_dir_from(None, Some("/Users/someone".to_owned()), None, None, "macos").unwrap();
    assert_eq!(
        resolved,
        Utf8PathBuf::from("/Users/someone/Library/Application Support")
    );
}

#[test]
fn data_dir_from_macos_falls_through_when_xdg_is_relative() {
    let resolved = data_dir_from(
        Some("relative/data".to_owned()),
        Some("/Users/someone".to_owned()),
        None,
        None,
        "macos",
    )
    .unwrap();
    assert_eq!(
        resolved,
        Utf8PathBuf::from("/Users/someone/Library/Application Support")
    );
}

#[test]
fn data_dir_from_macos_fails_without_xdg_or_home() {
    let failure = data_dir_from(None, None, None, None, "macos").unwrap_err();
    assert_eq!(failure.code, "fs.data_dir");
}

// -- data_dir_from: Windows cases ----------------------------------------------

#[test]
fn data_dir_from_windows_uses_xdg_when_absolute() {
    let resolved = data_dir_from(
        Some("/custom/data".to_owned()),
        None,
        Some("/home/someone/AppData/Roaming".to_owned()),
        Some("/home/someone/AppData/Local".to_owned()),
        "windows",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/custom/data"));
}

#[test]
fn data_dir_from_windows_uses_appdata_when_xdg_unset() {
    let resolved = data_dir_from(
        None,
        None,
        Some("/home/someone/AppData/Roaming".to_owned()),
        Some("/home/someone/AppData/Local".to_owned()),
        "windows",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/home/someone/AppData/Roaming"));
}

#[test]
fn data_dir_from_windows_falls_back_to_localappdata() {
    let resolved = data_dir_from(
        None,
        None,
        None,
        Some("/home/someone/AppData/Local".to_owned()),
        "windows",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/home/someone/AppData/Local"));
}

#[test]
fn data_dir_from_windows_rejects_relative_appdata() {
    let resolved = data_dir_from(
        None,
        None,
        Some("relative/appdata".to_owned()),
        Some("/home/someone/AppData/Local".to_owned()),
        "windows",
    )
    .unwrap();
    assert_eq!(resolved, Utf8PathBuf::from("/home/someone/AppData/Local"));
}

#[test]
fn data_dir_from_windows_rejects_empty_localappdata() {
    let failure = data_dir_from(None, None, Some("".to_owned()), None, "windows").unwrap_err();
    assert_eq!(failure.code, "fs.data_dir");
}

#[test]
fn data_dir_from_windows_fails_without_any_path() {
    let failure = data_dir_from(None, None, None, None, "windows").unwrap_err();
    assert_eq!(failure.code, "fs.data_dir");
}

// -- write_sensitive_atomic -------------------------------------------------

#[test]
fn write_sensitive_atomic_creates_parent_and_writes_content() {
    let (_dir, root) = utf8_temp_dir();
    let secret_path = root.join("nested").join("secrets").join("mcp.env");

    write_sensitive_atomic(&secret_path, b"TEST_KEY=\"secret_val\"\n").unwrap();
    assert_eq!(
        read_text(&secret_path).unwrap().as_deref(),
        Some("TEST_KEY=\"secret_val\"\n")
    );

    #[cfg(unix)]
    {
        let mode = stat(&secret_path).unwrap().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn write_sensitive_atomic_overwrites_existing_atomically() {
    let (_dir, root) = utf8_temp_dir();
    let secret_path = root.join("mcp.env");

    write_sensitive_atomic(&secret_path, b"KEY1=\"val1\"\n").unwrap();
    write_sensitive_atomic(&secret_path, b"KEY2=\"val2\"\n").unwrap();

    assert_eq!(
        read_text(&secret_path).unwrap().as_deref(),
        Some("KEY2=\"val2\"\n")
    );

    #[cfg(unix)]
    {
        let mode = stat(&secret_path).unwrap().unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
