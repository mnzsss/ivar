use super::*;

/// A webhook that records the one request it receives: the fake notifier.
fn fake_webhook() -> (String, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
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
            let text = String::from_utf8_lossy(&request).into_owned();
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
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request).into_owned()
    });
    (url, handle)
}

const SCRIPT_PATH: &str = ".ivar/scripts/report.sh";
const SCRIPT: &str = "#!/bin/bash\n# description: d\n# usage: u\necho hi\n";

fn put_script(root: &Utf8PathBuf, content: &str) {
    let dir = Layout::at(root.clone()).hall_scripts();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("report.sh"), content).unwrap();
}

fn approve_script(root: &Utf8PathBuf) {
    crate::action::script::approve::approve(
        &Ctx::new(root.clone()),
        &Utf8PathBuf::from(SCRIPT_PATH),
    )
    .unwrap();
}

#[test]
fn an_agent_whose_listed_scripts_are_approved_launches_normally() {
    let (_guard, root) = hall_with_agent(&format!("tool: claude\nscripts: [{SCRIPT_PATH}]\n"));
    put_script(&root, SCRIPT);
    approve_script(&root);
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let receipt = run_manual(&root);

    assert_eq!(receipt.outcome, AgentRunOutcome::Succeeded);
    assert!(fake.log().contains("claude -p"), "{}", fake.log());
}

#[rstest::rstest]
#[case::never_approved(false, "no approval")]
#[case::changed_since_approval(true, "changed since approval")]
fn an_unreviewed_script_blocks_the_launch_writes_a_failed_receipt_and_notifies(
    #[case] approved_first: bool,
    #[case] why: &str,
) {
    let (url, webhook) = fake_webhook();
    let (_guard, root) = hall_with_agent(&format!(
        "tool: claude\nscripts: [{SCRIPT_PATH}]\nnotify: {url}\n"
    ));
    put_script(&root, SCRIPT);
    if approved_first {
        approve_script(&root);
        put_script(
            &root,
            "#!/bin/bash\n# description: d\n# usage: u\necho changed\n",
        );
    }
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    let receipt = run_manual(&root);

    assert_eq!(receipt.outcome, AgentRunOutcome::Failed);
    assert_eq!(receipt.exit_code, None);
    assert!(
        receipt.output.starts_with("agent.script_unreviewed:"),
        "{}",
        receipt.output
    );
    assert!(receipt.output.contains(SCRIPT_PATH), "{}", receipt.output);
    assert!(receipt.output.contains(why), "{}", receipt.output);
    assert_eq!(stored(&root, &receipt), receipt);
    assert_eq!(receipt.notify_error, None, "the notifier answered");
    let request = webhook.join().unwrap();
    assert!(request.starts_with("POST /hook "), "{request}");
    assert!(request.contains("agent.script_unreviewed"), "{request}");
    assert!(request.contains(SCRIPT_PATH), "{request}");
    assert!(request.contains("\"failed\""), "{request}");
    assert!(
        !fake.log().contains("run "),
        "nothing was launched: {}",
        fake.log()
    );
    assert!(!root.join(".ivar/agents/tmp").join(&receipt.run_id).exists());
}

#[test]
fn approving_the_script_lets_the_next_run_proceed() {
    let (_guard, root) = hall_with_agent(&format!("tool: claude\nscripts: [{SCRIPT_PATH}]\n"));
    put_script(&root, SCRIPT);
    let fake = FakeDocker::install(&root);
    let _r = redirect_on_this_thread("docker", fake_docker_command(&fake));

    assert_eq!(run_manual(&root).outcome, AgentRunOutcome::Failed);
    approve_script(&root);
    assert_eq!(run_manual(&root).outcome, AgentRunOutcome::Succeeded);
}
