#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn healthy() -> RepoHealth {
    RepoHealth {
        bare_cloned: true,
        default_worktree_present: Some(true),
        ahead_of_bare: false,
    }
}

#[test]
fn no_repos_is_operational() {
    assert_eq!(Health::derive(&[]), Health::Operational);
}

#[test]
fn everything_present_is_operational() {
    let repos = [healthy(), healthy()];
    assert_eq!(Health::derive(&repos), Health::Operational);
}

#[test]
fn a_missing_bare_clone_is_degraded() {
    let repos = [RepoHealth {
        bare_cloned: false,
        default_worktree_present: None,
        ahead_of_bare: false,
    }];
    assert_eq!(Health::derive(&repos), Health::Degraded);
}

#[test]
fn a_missing_default_worktree_is_degraded() {
    let repos = [RepoHealth {
        bare_cloned: true,
        default_worktree_present: Some(false),
        ahead_of_bare: false,
    }];
    assert_eq!(Health::derive(&repos), Health::Degraded);
}

#[test]
fn a_repo_ahead_of_its_bare_clone_is_stale() {
    let repos = [RepoHealth {
        bare_cloned: true,
        default_worktree_present: Some(true),
        ahead_of_bare: true,
    }];
    assert_eq!(Health::derive(&repos), Health::Stale);
}

#[test]
fn degraded_beats_stale() {
    let repos = [
        healthy(),
        RepoHealth {
            bare_cloned: false,
            default_worktree_present: None,
            ahead_of_bare: true,
        },
    ];
    assert_eq!(Health::derive(&repos), Health::Degraded);
}
