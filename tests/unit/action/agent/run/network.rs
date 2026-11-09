#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::{Cell, RefCell};

use super::*;

const INTERVAL: Duration = Duration::from_secs(2);

#[test]
fn it_waits_until_the_host_resolves() {
    let attempts = Cell::new(0);
    let slept = RefCell::new(Vec::new());
    let resolves = |host: &str| {
        assert_eq!(host, "api.anthropic.com");
        attempts.set(attempts.get() + 1);
        attempts.get() == 3
    };

    let ready = wait_for_dns(
        "api.anthropic.com",
        &resolves,
        &|d| slept.borrow_mut().push(d),
        Duration::from_secs(120),
        INTERVAL,
    );

    assert!(ready);
    assert_eq!(*slept.borrow(), vec![INTERVAL, INTERVAL]);
}

#[test]
fn it_gives_up_after_its_budget_and_lets_the_run_go_on() {
    let slept = RefCell::new(Vec::new());

    let ready = wait_for_dns(
        "api.openai.com",
        &|_| false,
        &|d| slept.borrow_mut().push(d),
        Duration::from_secs(10),
        INTERVAL,
    );

    assert!(!ready);
    assert_eq!(slept.borrow().len(), 5);
}

#[test]
fn a_resolving_host_costs_no_sleep() {
    let ready = wait_for_dns(
        "api.anthropic.com",
        &|_| true,
        &|_| panic!("slept"),
        Duration::from_secs(120),
        INTERVAL,
    );
    assert!(ready);
}
