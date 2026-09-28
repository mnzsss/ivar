#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::graph::watch::scopes::{Debouncer, Scope, WatchSet};

fn base() -> Scope {
    Scope::Base { repo: "api".into() }
}

const ROOT: &str = "/hall/.ivar/repos/api/main";

#[test]
fn scope_keys_are_stable_and_distinct() {
    assert_eq!(base().key(), "base:api");
    assert_eq!(
        Scope::Layer {
            feature: "feat".into(),
            repo: "api".into()
        }
        .key(),
        "layer:feat:api"
    );
}

#[test]
fn only_tracked_directories_are_watched_even_though_the_hall_path_holds_dot_ivar() {
    let mut set = WatchSet::default();
    let tracked: Vec<Utf8PathBuf> = [
        "README.md",
        "src/lib.rs",
        "src/a/b.rs",
        "node_modules/x/i.js",
    ]
    .iter()
    .map(Utf8PathBuf::from)
    .collect();
    let dirs = set.add_worktree(&base(), Utf8Path::new(ROOT), &tracked);
    assert_eq!(
        dirs,
        vec![
            Utf8PathBuf::from(ROOT),
            Utf8PathBuf::from(format!("{ROOT}/src")),
            Utf8PathBuf::from(format!("{ROOT}/src/a")),
        ]
    );
    assert_eq!(
        set.classify(Utf8Path::new(&format!("{ROOT}/src/lib.rs"))),
        Some(base())
    );
    assert_eq!(
        set.classify(Utf8Path::new(&format!("{ROOT}/src/new.rs"))),
        Some(base())
    );
    assert_eq!(
        set.classify(Utf8Path::new(&format!("{ROOT}/app.min.js"))),
        None
    );
    assert_eq!(set.classify(Utf8Path::new("/elsewhere/lib.rs")), None);
}

#[test]
fn git_metadata_only_reacts_to_the_named_files_never_the_index() {
    let mut set = WatchSet::default();
    set.add_git_meta(
        &base(),
        Utf8Path::new("/hall/.ivar/repos/api/.bare/worktrees/main"),
        &["HEAD"],
    );
    assert_eq!(
        set.classify(Utf8Path::new(
            "/hall/.ivar/repos/api/.bare/worktrees/main/HEAD"
        )),
        Some(base())
    );
    assert_eq!(
        set.classify(Utf8Path::new(
            "/hall/.ivar/repos/api/.bare/worktrees/main/index"
        )),
        None
    );
    assert_eq!(
        set.classify(Utf8Path::new(
            "/hall/.ivar/repos/api/.bare/worktrees/main/index.lock"
        )),
        None
    );
}

#[test]
fn new_directories_are_watched_unless_ignored_and_removing_a_scope_unwatches_it() {
    let mut set = WatchSet::default();
    set.add_worktree(
        &base(),
        Utf8Path::new(ROOT),
        &[Utf8PathBuf::from("src/lib.rs")],
    );
    assert_eq!(
        set.new_dir(Utf8Path::new(&format!("{ROOT}/src/fresh"))),
        Some(Utf8PathBuf::from(format!("{ROOT}/src/fresh")))
    );
    assert_eq!(set.new_dir(Utf8Path::new(&format!("{ROOT}/target"))), None);
    assert_eq!(set.new_dir(Utf8Path::new("/elsewhere/dir")), None);
    let mut gone = set.remove_scope(&base());
    gone.sort();
    assert_eq!(gone.len(), 3);
    assert_eq!(
        set.classify(Utf8Path::new(&format!("{ROOT}/src/lib.rs"))),
        None
    );
}

#[test]
fn a_burst_fires_after_the_quiet_window_and_is_capped_under_continuous_events() {
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let mut deb = Debouncer::new(ms(150), ms(1000));

    assert!(deb.record(base(), t0), "first event opens a burst");
    assert!(!deb.record(base(), t0 + ms(100)), "same burst");
    assert!(deb.due(t0 + ms(200)).is_empty(), "only 100 ms quiet");
    assert_eq!(deb.due(t0 + ms(250)), vec![base()]);
    assert!(deb.due(t0 + ms(900)).is_empty(), "burst consumed");

    let t1 = t0 + ms(2000);
    assert!(deb.record(base(), t1));
    for step in 1..=10 {
        deb.record(base(), t1 + ms(100 * step));
    }
    assert_eq!(
        deb.due(t1 + ms(1000)),
        vec![base()],
        "cap fires despite constant events"
    );
    assert_eq!(deb.next_deadline(), None);
}

#[test]
fn session_directories_are_control_paths_not_scopes() {
    let mut set = WatchSet::default();
    set.add_control(Utf8Path::new("/hall/.ivar/features/feat/sessions"));
    assert!(set.is_control(Utf8Path::new("/hall/.ivar/features/feat/sessions/0a2d7418")));
    assert_eq!(
        set.classify(Utf8Path::new("/hall/.ivar/features/feat/sessions/0a2d7418")),
        None
    );
    assert!(!set.is_control(Utf8Path::new("/hall/.ivar/features/feat/plan.md")));
}
