use rstest::rstest;

use super::*;
use crate::domain::agent::spec::WritePath;
use crate::domain::agent::tool::Tool;

#[test]
fn write_roots_names_a_missing_path_with_its_code() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let failure = write_roots(&[WritePath::Hall("nope".into())], &fake.home, &root, &[])
        .expect_err("nope does not exist");
    assert_eq!(failure.code, "agent.write_missing");
}

#[rstest]
#[case::filesystem_root("/")]
#[case::home("~")]
#[case::hall_root(".")]
#[case::hall_state(".ivar/secrets")]
#[case::hall_scripts(".ivar/scripts")]
#[case::hall_scripts_local(".ivar/scripts-local")]
#[case::hall_git(".git")]
#[case::agent_files("agents")]
#[case::tool_login_dir("~/.claude")]
#[case::inside_tool_login_dir("~/.claude/hooks")]
#[case::hall_env(".env")]
#[case::hall_mcp_config(".mcp.json")]
#[case::provider_hook_config_dir(".claude")]
fn write_roots_refuse_what_would_expose_the_hall_home_or_login(#[case] raw: &str) {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    for dir in [
        ".ivar/secrets",
        ".ivar/scripts",
        ".ivar/scripts-local",
        ".git",
        "agents",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    std::fs::write(root.join(".env"), "").unwrap();
    std::fs::write(root.join(".mcp.json"), "{}").unwrap();
    std::fs::create_dir_all(fake.home.join(".claude/hooks")).unwrap();
    let credentials = Tool::Claude.credentials(&fake.home, &|_| None);

    let failure = write_roots(
        &[WritePath::parse(raw).unwrap()],
        &fake.home,
        &root,
        &credentials,
    )
    .expect_err(raw);
    assert_eq!(failure.code, "agent.write_refused", "{}", failure.what);
}

#[test]
fn write_roots_allow_a_sibling_sharing_the_hall_name_prefix() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let sibling = Utf8PathBuf::from(format!("{root}-out"));
    std::fs::create_dir_all(&sibling).unwrap();

    let roots = write_roots(
        &[WritePath::Absolute(sibling.clone())],
        &fake.home,
        &root,
        &[],
    )
    .unwrap();
    assert_eq!(roots, vec![sibling]);
}

#[test]
fn write_roots_grant_hall_and_home_paths_when_the_hall_lives_in_the_home() {
    let (_guard, tmp) = crate::test_support::canonical_temp_dir();
    let home = tmp.join("home");
    let hall = home.join("personal/hall");
    std::fs::create_dir_all(hall.join("brand/metricas")).unwrap();
    std::fs::create_dir_all(home.join(".cache/ivar-metrics")).unwrap();
    let credentials = Tool::Claude.credentials(&home, &|_| None);

    let roots = write_roots(
        &[
            WritePath::Hall("brand/metricas".into()),
            WritePath::Home(".cache/ivar-metrics".into()),
        ],
        &home,
        &hall,
        &credentials,
    )
    .unwrap();
    assert_eq!(
        roots,
        vec![
            hall.join("brand/metricas"),
            home.join(".cache/ivar-metrics")
        ]
    );
}

#[cfg(unix)]
#[test]
fn write_roots_refuse_a_symlink_into_the_hall_state() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(root.join(".ivar/secrets")).unwrap();
    std::os::unix::fs::symlink(root.join(".ivar/secrets"), fake.home.join("out")).unwrap();

    let failure = write_roots(&[WritePath::Home("out".into())], &fake.home, &root, &[])
        .expect_err("the link resolves into .ivar");
    assert_eq!(failure.code, "agent.write_refused");
}

#[test]
fn a_login_dir_that_holds_the_home_refuses_write_with_its_variable() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(fake.home.join(".cache/m")).unwrap();
    let home = fake.home.to_string();
    let credentials = Tool::Claude.credentials(&fake.home, &|key| {
        (key == "CLAUDE_CONFIG_DIR").then(|| home.clone())
    });

    let failure = write_roots(
        &[WritePath::Home(".cache/m".into())],
        &fake.home,
        &root,
        &credentials,
    )
    .expect_err("tool config would sit directly in the home");
    assert_eq!(failure.code, "agent.write_refused");
    assert!(
        failure.what.contains("CLAUDE_CONFIG_DIR"),
        "{}",
        failure.what
    );
}

#[test]
fn an_agent_without_write_never_checks_protected_paths() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    let home = fake.home.to_string();
    let credentials = Tool::Claude.credentials(&fake.home, &|key| {
        (key == "CLAUDE_CONFIG_DIR").then(|| home.clone())
    });
    assert!(
        write_roots(&[], &fake.home, &root, &credentials)
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn an_unresolvable_protected_path_is_named_in_the_refusal() {
    let (_guard, root) = hall_with_agent("runtime: host\ntool: claude\n");
    let fake = HostFake::new(&root);
    std::fs::create_dir_all(root.join("brand")).unwrap();
    let _ = std::fs::remove_file(root.join(".mcp.json"));
    std::os::unix::fs::symlink("missing/../elsewhere", root.join(".mcp.json")).unwrap();

    let failure = write_roots(&[WritePath::Hall("brand".into())], &fake.home, &root, &[])
        .expect_err("an unresolvable protected path fails closed");
    assert_eq!(failure.code, "agent.write_refused");
    assert!(failure.what.contains(".mcp.json"), "{}", failure.what);
    assert!(
        failure.what.contains("does not resolve"),
        "{}",
        failure.what
    );
}
