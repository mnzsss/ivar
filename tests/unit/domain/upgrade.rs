#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use camino::{Utf8Path, Utf8PathBuf};

use super::*;

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

// -- Version ---------------------------------------------------------------

#[test]
fn parses_with_or_without_a_leading_v() {
    assert_eq!(
        v("0.13.0"),
        Version {
            major: 0,
            minor: 13,
            patch: 0
        }
    );
    assert_eq!(
        v("v1.2.3"),
        Version {
            major: 1,
            minor: 2,
            patch: 3
        }
    );
}

#[test]
fn rejects_prereleases_and_malformed_versions() {
    for bad in [
        "",
        "v",
        "1.2",
        "1.2.3.4",
        "1.2.x",
        "1.2.3-rc.1",
        " 1.2.3",
        "vv1.2.3",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?} must not parse");
    }
}

#[test]
fn orders_numerically_not_lexically() {
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert!(v("0.12.1") > v("0.12.0"));
}

#[test]
fn displays_without_the_v() {
    assert_eq!(v("v0.13.0").to_string(), "0.13.0");
    assert_eq!(serde_json::to_string(&v("0.13.0")).unwrap(), r#""0.13.0""#);
}

#[test]
fn reads_the_version_from_a_release_redirect() {
    assert_eq!(
        tag_from_location("https://github.com/mnzsss/ivar/releases/tag/v0.13.0"),
        Some(v("0.13.0"))
    );
    assert_eq!(
        tag_from_location("https://github.com/mnzsss/ivar/releases"),
        None
    );
    assert_eq!(
        tag_from_location("https://github.com/mnzsss/ivar/releases/tag/nightly"),
        None
    );
}

// -- staleness ---------------------------------------------------------------

#[test]
fn a_missing_check_is_stale() {
    assert!(is_stale(None, 1_000));
}

#[test]
fn stale_only_after_the_interval() {
    let now = 10 * CHECK_INTERVAL_SECS;
    assert!(!is_stale(Some(now - CHECK_INTERVAL_SECS + 1), now));
    assert!(is_stale(Some(now - CHECK_INTERVAL_SECS), now));
}

#[test]
fn a_check_from_the_future_is_stale() {
    // A clock that went backwards must not silence the check until it catches up.
    assert!(is_stale(Some(5_000), 1_000));
}

// -- channel -----------------------------------------------------------------

fn env() -> ChannelEnv {
    ChannelEnv {
        home: Some(Utf8PathBuf::from("/home/u")),
        cargo_home: None,
        ivar_install_dir: None,
    }
}

#[test]
fn classifies_each_install_channel() {
    let cases: [(&str, ChannelEnv, Channel); 6] = [
        ("/home/u/.cargo/bin/ivar", env(), Channel::Cargo),
        (
            "/opt/cargo/bin/ivar",
            ChannelEnv {
                cargo_home: Some(Utf8PathBuf::from("/opt/cargo")),
                ..env()
            },
            Channel::Cargo,
        ),
        ("/usr/bin/ivar", env(), Channel::System),
        (
            "/home/u/.local/bin/ivar",
            env(),
            Channel::Installer {
                dir: Utf8PathBuf::from("/home/u/.local/bin"),
            },
        ),
        (
            "/srv/tools/ivar",
            ChannelEnv {
                ivar_install_dir: Some(Utf8PathBuf::from("/srv/tools")),
                ..env()
            },
            Channel::Installer {
                dir: Utf8PathBuf::from("/srv/tools"),
            },
        ),
        ("/usr/local/bin/ivar", env(), Channel::Unknown),
    ];
    for (exe, env, expected) in cases {
        assert_eq!(classify(Utf8Path::new(exe), &env), expected, "{exe}");
    }
}

#[test]
fn an_explicit_cargo_home_replaces_the_default() {
    let env = ChannelEnv {
        cargo_home: Some(Utf8PathBuf::from("/opt/cargo")),
        ..env()
    };
    assert_eq!(
        classify(Utf8Path::new("/home/u/.cargo/bin/ivar"), &env),
        Channel::Unknown
    );
}

#[test]
fn no_home_still_classifies_system_and_unknown() {
    let env = ChannelEnv::default();
    assert_eq!(
        classify(Utf8Path::new("/usr/bin/ivar"), &env),
        Channel::System
    );
    assert_eq!(classify(Utf8Path::new("/x/ivar"), &env), Channel::Unknown);
}

// -- notice gate -------------------------------------------------------------

fn interactive() -> NoticeContext {
    NoticeContext {
        stderr_tty: true,
        release_build: true,
        ..NoticeContext::default()
    }
}

#[test]
fn an_interactive_release_run_shows_the_notice() {
    assert!(notice_enabled(&interactive()));
}

#[test]
fn every_suppressing_condition_turns_it_off() {
    let off = [
        NoticeContext {
            opted_out: true,
            ..interactive()
        },
        NoticeContext {
            ci: true,
            ..interactive()
        },
        NoticeContext {
            stderr_tty: false,
            ..interactive()
        },
        NoticeContext {
            machine_output: true,
            ..interactive()
        },
        NoticeContext {
            machine_verb: true,
            ..interactive()
        },
        NoticeContext {
            release_build: false,
            ..interactive()
        },
    ];
    for ctx in off {
        assert!(!notice_enabled(&ctx), "{ctx:?}");
    }
}

#[test]
fn the_notice_names_both_versions_and_the_command() {
    let line = notice_line(v("0.12.0"), v("0.13.0")).unwrap();
    assert!(line.contains("0.12.0"), "{line}");
    assert!(line.contains("0.13.0"), "{line}");
    assert!(line.contains("ivar upgrade"), "{line}");
    assert!(!line.contains('\n'), "one line: {line}");
}

#[test]
fn no_notice_when_current_or_ahead() {
    assert_eq!(notice_line(v("0.13.0"), v("0.13.0")), None);
    assert_eq!(notice_line(v("0.14.0"), v("0.13.0")), None);
}

// -- upgrade plan ------------------------------------------------------------

#[test]
fn the_installer_channel_reruns_the_installer_into_the_same_dir() {
    let plan = upgrade_plan(&Channel::Installer {
        dir: Utf8PathBuf::from("/home/u/.local/bin"),
    });
    assert_eq!(
        plan,
        UpgradePlan::Run {
            program: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "curl -fsSL https://ivar.run/install | sh".to_owned()
            ],
            env: vec![(
                "IVAR_INSTALL_DIR".to_owned(),
                "/home/u/.local/bin".to_owned()
            )],
        }
    );
}

#[test]
fn the_cargo_channel_runs_cargo_install() {
    assert_eq!(
        upgrade_plan(&Channel::Cargo),
        UpgradePlan::Run {
            program: "cargo".to_owned(),
            args: vec![
                "install".to_owned(),
                "ivar".to_owned(),
                "--locked".to_owned()
            ],
            env: vec![],
        }
    );
}

#[test]
fn system_and_unknown_only_print() {
    let UpgradePlan::Print { commands } = upgrade_plan(&Channel::System) else {
        panic!("system installs must never run a package manager");
    };
    assert!(commands.iter().all(|c| !c.contains("sudo")));
    assert!(commands.iter().any(|c| c.contains("ivar-bin")));

    let UpgradePlan::Print { commands } = upgrade_plan(&Channel::Unknown) else {
        panic!("an unknown install must never run anything");
    };
    assert!(commands.iter().any(|c| c.contains("ivar.run/install")));
    assert!(
        commands
            .iter()
            .any(|c| c.contains("cargo install ivar --locked"))
    );
    assert!(commands.iter().any(|c| c.contains("ivar-bin")));
}

#[test]
fn channel_names_are_stable() {
    assert_eq!(Channel::Cargo.name(), "cargo");
    assert_eq!(Channel::System.name(), "system");
    assert_eq!(Channel::Unknown.name(), "unknown");
    assert_eq!(
        Channel::Installer {
            dir: Utf8PathBuf::from("/x")
        }
        .name(),
        "installer"
    );
}
