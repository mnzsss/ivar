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
    deliver(&spec(Some(url)), &mut receipt);
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
    deliver(&spec(Some(url)), &mut receipt);
    server.join().unwrap();
    assert!(receipt.notify_error.as_deref().unwrap().contains("500"));
    assert_eq!(receipt.outcome, AgentRunOutcome::Skipped);
}

#[test]
fn no_notify_url_sends_nothing() {
    let mut receipt =
        AgentRunReceipt::skipped("r1".into(), "nightly".into(), Trigger::Schedule, "t".into());
    deliver(&spec(None), &mut receipt);
    assert_eq!(receipt.notify_error, None);
}
