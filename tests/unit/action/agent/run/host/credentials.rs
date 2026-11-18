use super::*;
use crate::domain::agent::tool::Credential;

#[test]
fn seeding_copies_only_the_login_and_drops_a_stale_copy() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let state = dir.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("stale.json"), "{}").unwrap();
    std::fs::write(dir.join("auth.json"), "{\"t\":1}").unwrap();
    let creds = vec![
        Credential {
            real: dir.join("auth.json"),
            private: "auth.json",
            write_back: true,
        },
        Credential {
            real: dir.join("missing.json"),
            private: "sub/missing.json",
            write_back: false,
        },
    ];
    std::fs::create_dir_all(state.join("sub")).unwrap();
    std::fs::write(state.join("sub/missing.json"), "{\"old\":1}").unwrap();
    let seeded = seed_credentials(&state, creds).unwrap();
    assert_eq!(
        std::fs::read_to_string(state.join("auth.json")).unwrap(),
        "{\"t\":1}"
    );
    assert!(!state.join("sub/missing.json").exists());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(seeded.len(), 2);
}

#[test]
fn only_a_changed_valid_login_is_written_back() {
    let (_guard, dir) = crate::test_support::utf8_temp_dir();
    let state = dir.join("state");
    let real = dir.join("auth.json");
    let seed = || {
        std::fs::write(&real, "{\"t\":1}").unwrap();
        seed_credentials(
            &state,
            vec![Credential {
                real: real.clone(),
                private: "auth.json",
                write_back: true,
            }],
        )
        .unwrap()
    };
    for invalid in ["", "{}", "not json", "[1]"] {
        let seeded = seed();
        std::fs::write(state.join("auth.json"), invalid).unwrap();
        write_back_credentials(&state, &seeded).unwrap();
        assert_eq!(
            std::fs::read_to_string(&real).unwrap(),
            "{\"t\":1}",
            "{invalid:?}"
        );
    }
    let seeded = seed();
    std::fs::write(state.join("auth.json"), "{\"t\":2}").unwrap();
    std::fs::write(&real, "{\"t\":9}").unwrap();
    write_back_credentials(&state, &seeded).unwrap();
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "{\"t\":9}",
        "a newer login was clobbered"
    );

    let seeded = seed();
    std::fs::write(state.join("auth.json"), "{\"t\":2}").unwrap();
    write_back_credentials(&state, &seeded).unwrap();
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"t\":2}");
}
