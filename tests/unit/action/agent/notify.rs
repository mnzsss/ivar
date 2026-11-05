#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::{Read, Write};
use std::net::TcpListener;

use super::*;
use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, Trigger};
use crate::domain::agent::spec::{AgentFrontmatter, AgentSpec};
use crate::store::layout::Layout;
use crate::test_support::{FakeTelegram, seeded_hall};

fn no_token() -> Result<String, String> {
    Err("no token in this test".into())
}

fn skipped() -> AgentRunReceipt {
    AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Schedule, "t".into())
}

fn hall_with_agent_secrets(text: &str) -> (tempfile::TempDir, Layout) {
    let (guard, root) = seeded_hall();
    let layout = Layout::at(root);
    AgentSecrets::ensure_file(&layout).unwrap();
    std::fs::write(layout.agent_secrets_env(), text).unwrap();
    (guard, layout)
}

fn spec(notify: Option<String>) -> AgentSpec {
    AgentSpec::from_parts(
        "nightly",
        AgentFrontmatter {
            schedule: Some("@daily".into()),
            tool: Some("claude".into()),
            notify,
            ..AgentFrontmatter::default()
        },
        "",
    )
    .unwrap()
}

fn one_shot_server(status: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&request);
            let Some(head_end) = text.find("\r\n\r\n") else {
                continue;
            };
            let length = text[..head_end]
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= head_end + 4 + length {
                break;
            }
        }
        stream
            .write_all(
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    (url, handle)
}

#[test]
fn posts_the_summary_as_json() {
    let (url, server) = one_shot_server("200 OK");
    let mut receipt =
        AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Schedule, "t".into());
    deliver_to(
        &spec(Some(url)),
        &mut receipt,
        "valhalla-hall",
        no_token,
        "",
    );
    let request = server.join().unwrap();
    assert!(request.starts_with("POST /hook "));
    assert!(
        request
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert!(request.contains("\"skipped\""));
    assert_eq!(receipt.notify_error, None);
}

#[test]
fn a_failed_post_is_recorded_and_leaves_the_outcome() {
    let (url, server) = one_shot_server("500 Internal Server Error");
    let mut receipt =
        AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Schedule, "t".into());
    deliver_to(
        &spec(Some(url)),
        &mut receipt,
        "valhalla-hall",
        no_token,
        "",
    );
    server.join().unwrap();
    assert!(receipt.notify_error.as_deref().unwrap().contains("500"));
    assert_eq!(receipt.outcome, AgentRunOutcome::Skipped);
}

#[test]
fn no_notify_url_sends_nothing() {
    let mut receipt =
        AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Schedule, "t".into());
    deliver_to(&spec(None), &mut receipt, "valhalla-hall", no_token, "");
    assert_eq!(receipt.notify_error, None);
}

#[test]
fn telegram_sends_the_run_text_to_the_chat() {
    let fake = FakeTelegram::start(vec![(200, r#"{"ok":true,"result":{}}"#)]);
    let mut receipt = skipped();
    deliver_to(
        &spec(Some("telegram:-100123".into())),
        &mut receipt,
        "valhalla-hall",
        || Ok("123:secret".into()),
        &fake.api,
    );
    let request = &fake.requests()[0];
    assert!(
        request.starts_with("POST /bot123:secret/sendMessage "),
        "{request}"
    );
    assert!(request.contains(r#""chat_id":"-100123""#), "{request}");
    assert!(request.contains("<b>nightly</b> · skipped"), "{request}");
    assert!(request.contains("<i>valhalla-hall · schedule"), "{request}");
    assert!(
        request.contains("ivar agent logs nightly --run r1"),
        "{request}"
    );
    assert_eq!(receipt.notify_error, None);
}

#[test]
fn telegram_sends_a_timed_out_run_with_its_exit_code_and_tail() {
    let fake = FakeTelegram::start(vec![(200, r#"{"ok":true,"result":{}}"#)]);
    let mut receipt = skipped();
    receipt.outcome = AgentRunOutcome::TimedOut;
    receipt.exit_code = Some(124);
    receipt.output = "step 3 of 9\nstill waiting".into();
    deliver_to(
        &spec(Some("telegram:-100123".into())),
        &mut receipt,
        "valhalla-hall",
        || Ok("123:secret".into()),
        &fake.api,
    );
    let request = &fake.requests()[0];
    assert!(request.contains("<b>nightly</b> · timed_out"), "{request}");
    assert!(request.contains("exit 124"), "{request}");
    assert!(request.contains(r"step 3 of 9\nstill waiting"), "{request}");
    assert_eq!(receipt.notify_error, None);
}

#[test]
fn a_failed_telegram_send_never_records_the_token() {
    let fake = FakeTelegram::start(vec![(
        401,
        r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#,
    )]);
    let mut receipt = skipped();
    deliver_to(
        &spec(Some("telegram:@ivar_runs".into())),
        &mut receipt,
        "valhalla-hall",
        || Ok("123:secret".into()),
        &fake.api,
    );
    fake.requests();
    let error = receipt.notify_error.unwrap();
    assert!(error.contains("Unauthorized"), "{error}");
    assert!(!error.contains("123:secret"), "{error}");
    assert_eq!(receipt.outcome, AgentRunOutcome::Skipped);
}

#[test]
fn a_missing_telegram_token_is_recorded_without_a_request() {
    let mut receipt = skipped();
    deliver_to(
        &spec(Some("telegram:123".into())),
        &mut receipt,
        "valhalla-hall",
        || Err(format!("secret `{TELEGRAM_TOKEN_KEY}` is not set")),
        "http://127.0.0.1:9",
    );
    assert!(receipt.notify_error.unwrap().contains("TELEGRAM_BOT_TOKEN"));
}

#[test]
fn the_telegram_token_comes_from_agents_env() {
    let (_guard, layout) = hall_with_agent_secrets("TELEGRAM_BOT_TOKEN=123:secret\n");
    assert_eq!(telegram_token(&layout).unwrap(), "123:secret");
}

#[test]
fn a_missing_telegram_token_names_the_key() {
    let (_guard, layout) = hall_with_agent_secrets("OTHER=x\n");
    let error = telegram_token(&layout).unwrap_err();
    assert!(error.contains("TELEGRAM_BOT_TOKEN"), "{error}");
}
