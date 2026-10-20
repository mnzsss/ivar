#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::test_support::seeded_hall;

#[cfg(unix)]
fn owner_only(path: &camino::Utf8Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(not(unix))]
fn owner_only(_path: &camino::Utf8Path) {}

#[test]
fn selects_only_the_named_secrets_in_the_requested_order() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.secrets_dir()).unwrap();
    std::fs::write(layout.agent_secrets_env(), "A=1\nB=2\nC=3\n").unwrap();
    owner_only(&layout.agent_secrets_env());

    let secrets = AgentSecrets::read(&layout).unwrap();
    let picked = secrets.select(&["C".to_owned(), "A".to_owned()]).unwrap();
    assert_eq!(
        picked,
        [
            ("C".to_owned(), "3".to_owned()),
            ("A".to_owned(), "1".to_owned())
        ]
    );
}

#[test]
fn a_missing_secret_is_named_without_any_value() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    let failure = AgentSecrets::read(&layout)
        .unwrap()
        .select(&["TOKEN".to_owned()])
        .unwrap_err();
    assert_eq!(failure.code, "agent.secret_missing");
    assert!(failure.what.contains("TOKEN"));
    assert!(failure.what.contains("agents.env"));
}

#[test]
fn debug_shows_keys_but_never_values() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.secrets_dir()).unwrap();
    std::fs::write(layout.agent_secrets_env(), "TOKEN=hunter2\n").unwrap();
    owner_only(&layout.agent_secrets_env());
    let rendered = format!("{:?}", AgentSecrets::read(&layout).unwrap());
    assert!(rendered.contains("TOKEN"));
    assert!(!rendered.contains("hunter2"));
}

#[cfg(unix)]
#[test]
fn a_file_readable_by_others_is_refused_without_reading_it() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    std::fs::create_dir_all(layout.secrets_dir()).unwrap();
    std::fs::write(layout.agent_secrets_env(), "TOKEN=hunter2\n").unwrap();
    std::fs::set_permissions(
        layout.agent_secrets_env(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let failure = AgentSecrets::read(&layout).unwrap_err();
    assert_eq!(failure.code, "agent.secrets_mode");
    assert!(
        failure.what.contains("0644") && failure.what.contains("chmod 600"),
        "{}",
        failure.what
    );
    assert!(!failure.what.contains("hunter2"));
}

#[cfg(unix)]
#[test]
fn ensure_creates_an_owner_only_file() {
    use std::os::unix::fs::PermissionsExt;
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root);
    AgentSecrets::ensure_file(&layout).unwrap();
    let mode = std::fs::metadata(layout.agent_secrets_env())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn env_file_refuses_a_multiline_value() {
    let pairs = [("K".to_owned(), "a\nb".to_owned())];
    assert_eq!(
        render_env_file(&pairs).unwrap_err().code,
        "agent.secret_multiline"
    );
    assert_eq!(
        render_env_file(&[("K".into(), "v".into())]).unwrap(),
        "K=v\n"
    );
}
