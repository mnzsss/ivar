#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rstest::rstest;
use std::time::Duration;

use super::*;

fn base() -> AgentFrontmatter {
    AgentFrontmatter {
        schedule: Some("0 9 * * 1-5".to_owned()),
        tool: Some("claude".to_owned()),
        ..AgentFrontmatter::default()
    }
}

#[test]
fn a_minimal_agent_defaults_to_read_access_and_thirty_minutes() {
    let spec = AgentSpec::from_parts("nightly", base(), "Summarise open PRs.\n").unwrap();
    assert_eq!(spec.access, Access::Read);
    assert_eq!(spec.timeout, DEFAULT_TIMEOUT);
    assert_eq!(
        spec.schedule.on_calendar(),
        "Mon,Tue,Wed,Thu,Fri *-*-* 09:00:00"
    );
    assert_eq!(
        spec.launch,
        Launch::Tool {
            tool: Tool::Claude,
            model: None
        }
    );
    assert_eq!(spec.prompt, "Summarise open PRs.\n");
    assert_eq!(spec.feature_name(), "agent-nightly");
}

#[test]
fn image_and_command_override_the_tool() {
    let fm = AgentFrontmatter {
        tool: None,
        image: Some("ghcr.io/me/bot@sha256:abc".to_owned()),
        command: Some(vec!["bot".to_owned(), "--once".to_owned()]),
        ..base()
    };
    let spec = AgentSpec::from_parts("bot", fm, "").unwrap();
    assert!(
        matches!(spec.launch, Launch::Image { ref command, .. } if command == &["bot", "--once"])
    );
}

#[rstest]
#[case::bad_name("Nightly!", base(), "name")]
#[case::no_schedule("a", AgentFrontmatter { schedule: None, ..base() }, "schedule")]
#[case::untranslatable_schedule("a", AgentFrontmatter { schedule: Some("0 0 1 * 1".into()), ..base() }, "schedule")]
#[case::unknown_tool("a", AgentFrontmatter { tool: Some("gemini".into()), ..base() }, "tool")]
#[case::no_tool_no_image("a", AgentFrontmatter { tool: None, ..base() }, "tool")]
#[case::tool_and_image("a", AgentFrontmatter { image: Some("x".into()), command: Some(vec!["y".into()]), ..base() }, "image")]
#[case::image_without_command("a", AgentFrontmatter { tool: None, image: Some("x".into()), ..base() }, "command")]
#[case::domain_with_scheme("a", AgentFrontmatter { network: vec!["https://api.openai.com".into()], ..base() }, "network")]
#[case::bad_secret_name("a", AgentFrontmatter { secrets: vec!["1TOKEN".into()], ..base() }, "secrets")]
#[case::lowercase_secret("a", AgentFrontmatter { secrets: vec!["token".into()], ..base() }, "secrets")]
#[case::domain_with_port("a", AgentFrontmatter { network: vec!["api.openai.com:443".into()], ..base() }, "network")]
#[case::domain_with_path("a", AgentFrontmatter { network: vec!["github.com/org".into()], ..base() }, "network")]
#[case::absolute_workdir("a", AgentFrontmatter { workdir: Some("/etc".into()), ..base() }, "workdir")]
#[case::notify_not_http("a", AgentFrontmatter { notify: Some("ftp://x".into()), ..base() }, "notify")]
#[case::bad_timeout("a", AgentFrontmatter { timeout: Some("forever".into()), ..base() }, "timeout")]
#[case::escaping_workdir("a", AgentFrontmatter { workdir: Some("../etc".into()), ..base() }, "workdir")]
#[case::feature_without_workdir("a", AgentFrontmatter { access: Access::Feature, ..base() }, "workdir")]
fn names_the_failing_field(#[case] name: &str, #[case] fm: AgentFrontmatter, #[case] field: &str) {
    let error = AgentSpec::from_parts(name, fm, "").expect_err("invalid");
    assert_eq!(error.field, field, "{}", error.reason);
    assert!(!error.reason.is_empty());
}

#[rstest]
#[case("90s", 90)]
#[case("45m", 2700)]
#[case("2h", 7200)]
fn parses_timeouts(#[case] text: &str, #[case] secs: u64) {
    assert_eq!(parse_timeout(text).unwrap(), Duration::from_secs(secs));
}

#[test]
fn refuses_a_timeout_above_a_day() {
    assert!(parse_timeout("25h").unwrap_err().contains("24h"));
}

#[rstest]
#[case::multibyte_unit("2é")]
#[case::empty("")]
#[case::overflow("99999999999999999999h")]
#[case::mul_overflow("18446744073709551615h")]
fn refuses_malformed_timeouts_without_panicking(#[case] text: &str) {
    assert!(parse_timeout(text).is_err());
}

#[test]
fn an_overflowing_timeout_is_an_invalid_field() {
    let fm = AgentFrontmatter {
        timeout: Some("18446744073709551615h".into()),
        ..base()
    };
    assert_eq!(
        AgentSpec::from_parts("a", fm, "").unwrap_err().field,
        "timeout"
    );
}

#[test]
fn runtime_defaults_to_docker_and_accepts_host_with_a_tool() {
    assert_eq!(
        AgentSpec::from_parts("a", base(), "").unwrap().runtime,
        Runtime::Docker
    );
    let host = AgentFrontmatter {
        runtime: Runtime::Host,
        ..base()
    };
    assert_eq!(
        AgentSpec::from_parts("a", host, "").unwrap().runtime,
        Runtime::Host
    );
}

#[rstest]
#[case::host_with_image(AgentFrontmatter { runtime: Runtime::Host, tool: None, image: Some("x".into()), command: Some(vec!["y".into()]), ..base() }, "runtime")]
#[case::host_with_command(AgentFrontmatter { runtime: Runtime::Host, command: Some(vec!["y".into()]), ..base() }, "runtime")]
#[case::host_with_network(AgentFrontmatter { runtime: Runtime::Host, network: vec!["api.anthropic.com".into()], ..base() }, "network")]
fn a_host_runtime_refuses_what_it_cannot_honour(#[case] fm: AgentFrontmatter, #[case] field: &str) {
    let error = AgentSpec::from_parts("a", fm, "").unwrap_err();
    assert_eq!(error.field, field);
    assert!(error.reason.contains("runtime: host"), "{}", error.reason);
}

#[test]
fn a_host_runtime_without_a_tool_names_the_tool_field() {
    let fm = AgentFrontmatter {
        runtime: Runtime::Host,
        tool: None,
        ..base()
    };
    assert_eq!(
        AgentSpec::from_parts("a", fm, "").unwrap_err().field,
        "tool"
    );
}
