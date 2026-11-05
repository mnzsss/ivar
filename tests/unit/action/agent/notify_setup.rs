#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::domain::agent::spec::NotifyTarget;
use crate::store::layout::Layout;
use crate::test_support::{FakeTelegram, Scripted, seeded_hall};

const ME: (u16, &str) = (200, r#"{"ok":true,"result":{"username":"ivar_bot"}}"#);
const SENT: (u16, &str) = (200, r#"{"ok":true,"result":{"message_id":1}}"#);

fn write_agent(layout: &Layout, name: &str, frontmatter: &str) {
    std::fs::create_dir_all(layout.agents_dir()).unwrap();
    std::fs::write(
        layout.agent_file(name),
        format!("---\nschedule: \"@daily\"\ntool: claude\n{frontmatter}---\nGo.\n"),
    )
    .unwrap();
}

#[test]
fn a_full_setup_saves_the_token_tests_the_chat_and_sets_the_chosen_agents() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    write_agent(&layout, "alpha", "notify: https://hooks.example.com/x\n");
    write_agent(&layout, "nightly", "");
    let fake = FakeTelegram::start(vec![
        ME,
        (
            200,
            r#"{"ok":true,"result":[{"update_id":1,"message":{"chat":{"id":-100123,"type":"group","title":"Ops"}}}]}"#,
        ),
        SENT,
    ]);
    let script = Scripted::new(
        vec![Some("123:secret")],
        vec![],
        vec![Some(0)],
        vec![vec![1]],
    );
    let ctx = Ctx::new(root).with_confirm(script.clone());

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    assert_eq!(setup.bot, "ivar_bot");
    assert_eq!(setup.chat_id, "-100123");
    assert_eq!(setup.token, TokenChange::Created);
    assert_eq!(setup.agents, vec!["nightly".to_owned()]);
    let requests = fake.requests();
    assert!(requests[0].starts_with("POST /bot123:secret/getMe "));
    assert!(requests[2].starts_with("POST /bot123:secret/sendMessage "));
    assert!(requests[2].contains(r#""chat_id":"-100123""#));
    assert_eq!(
        AgentSecrets::read(&layout).unwrap().get(TELEGRAM_TOKEN_KEY),
        Some("123:secret")
    );
    assert_eq!(
        store::agent::load(&layout, "nightly").unwrap().notify,
        Some(NotifyTarget::Telegram {
            chat_id: "-100123".into()
        })
    );
    assert_eq!(
        store::agent::load(&layout, "alpha").unwrap().notify,
        Some(NotifyTarget::Webhook("https://hooks.example.com/x".into()))
    );
    let shown = script.shown();
    assert!(
        shown.contains(&"-100123 — Ops (group)".to_owned()),
        "{shown:?}"
    );
    assert!(
        shown.contains(&"alpha — notify: https://hooks.example.com/x".to_owned()),
        "{shown:?}"
    );
}

#[test]
fn a_rejected_token_is_asked_again_and_a_typed_chat_is_accepted() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let fake = FakeTelegram::start(vec![
        (
            401,
            r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#,
        ),
        ME,
        (200, r#"{"ok":true,"result":[]}"#),
        SENT,
    ]);
    let script = Scripted::new(
        vec![Some("1:bad"), Some("2:good"), Some(" @ivar_runs ")],
        vec![true],
        vec![Some(1)],
        vec![],
    );
    let ctx = Ctx::new(root).with_confirm(script.clone());

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    fake.requests();
    assert_eq!(setup.chat_id, "@ivar_runs");
    assert!(setup.agents.is_empty());
    assert_eq!(
        AgentSecrets::read(&layout).unwrap().get(TELEGRAM_TOKEN_KEY),
        Some("2:good")
    );
    let shown = script.shown();
    assert!(
        shown.iter().all(|line| !line.contains("1:bad")),
        "{shown:?}"
    );
    assert!(
        shown.iter().any(|line| line.contains("@ivar_bot")),
        "{shown:?}"
    );
}

#[test]
fn an_existing_token_can_be_kept() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    AgentSecrets::set_and_write(&layout, TELEGRAM_TOKEN_KEY, "123:secret").unwrap();
    let fake = FakeTelegram::start(vec![ME, (200, r#"{"ok":true,"result":[]}"#), SENT]);
    let script = Scripted::new(vec![Some("-100123")], vec![true], vec![Some(1)], vec![]);
    let ctx = Ctx::new(root).with_confirm(script);

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    fake.requests();
    assert_eq!(setup.token, TokenChange::Kept);
    assert_eq!(setup.chat_id, "-100123");
}

#[test]
fn a_non_interactive_run_refuses_and_explains_the_manual_setup() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    let failure = setup_with(&Ctx::new(root), "http://127.0.0.1:9").unwrap_err();
    assert_eq!(failure.code, "agent.notify.interactive_only");
    assert!(
        failure
            .fix_actions
            .iter()
            .any(|fix| fix.what.contains(TELEGRAM_TOKEN_KEY))
    );
    assert_eq!(
        AgentSecrets::read(&layout).unwrap().get(TELEGRAM_TOKEN_KEY),
        None
    );
}

#[test]
fn a_chat_list_that_recovers_drops_the_earlier_error() {
    let (_guard, root) = seeded_hall();
    let fake = FakeTelegram::start(vec![
        ME,
        (
            409,
            r#"{"ok":false,"error_code":409,"description":"Conflict: can't use getUpdates method while webhook is active"}"#,
        ),
        (
            200,
            r#"{"ok":true,"result":[{"update_id":1,"message":{"chat":{"id":-100123,"type":"group","title":"Ops"}}}]}"#,
        ),
        SENT,
    ]);
    let script = Scripted::new(
        vec![Some("123:secret")],
        vec![],
        vec![Some(0), Some(0)],
        vec![],
    );
    let ctx = Ctx::new(root).with_confirm(script.clone());

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    fake.requests();
    assert_eq!(setup.chat_id, "-100123");
    let shown = script.shown();
    let last_chat_prompt = shown
        .iter()
        .rfind(|line| line.contains("Which chat gets the notifications?"))
        .unwrap();
    assert!(
        last_chat_prompt.contains("Send any message to @ivar_bot"),
        "{last_chat_prompt}"
    );
    assert!(!last_chat_prompt.contains("Conflict"), "{last_chat_prompt}");
}

#[test]
fn a_kept_token_that_telegram_rejects_can_be_replaced() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    AgentSecrets::set_and_write(&layout, TELEGRAM_TOKEN_KEY, "1:revoked").unwrap();
    let fake = FakeTelegram::start(vec![
        (
            401,
            r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#,
        ),
        ME,
        (200, r#"{"ok":true,"result":[]}"#),
        SENT,
    ]);
    let script = Scripted::new(
        vec![Some("2:fresh"), Some("-100123")],
        vec![true, true],
        vec![Some(1)],
        vec![],
    );
    let ctx = Ctx::new(root).with_confirm(script.clone());

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    fake.requests();
    assert_eq!(setup.token, TokenChange::Updated);
    assert_eq!(
        AgentSecrets::read(&layout).unwrap().get(TELEGRAM_TOKEN_KEY),
        Some("2:fresh")
    );
    assert!(
        script
            .shown()
            .iter()
            .any(|line| line.contains("Unauthorized")),
        "{:?}",
        script.shown()
    );
}

const ONE_CHAT: (u16, &str) = (
    200,
    r#"{"ok":true,"result":[{"update_id":1,"message":{"chat":{"id":-100123,"type":"group","title":"Ops"}}}]}"#,
);

#[test]
fn an_agent_that_cannot_take_notify_is_skipped_and_the_others_are_set() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    std::fs::create_dir_all(layout.agents_dir()).unwrap();
    std::fs::write(
        layout.agent_file("broken"),
        "---\nschedule: \"0 0 1 * 1\"\ntool: claude\n---\nGo.\n",
    )
    .unwrap();
    write_agent(&layout, "nightly", "");
    let fake = FakeTelegram::start(vec![ME, ONE_CHAT, SENT]);
    let script = Scripted::new(
        vec![Some("123:secret")],
        vec![],
        vec![Some(0)],
        vec![vec![0, 1]],
    );
    let ctx = Ctx::new(root).with_confirm(script);

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    fake.requests();
    assert_eq!(setup.agents, vec!["nightly".to_owned()]);
    assert_eq!(setup.skipped.len(), 1);
    assert_eq!(setup.skipped[0].agent, "broken");
    assert!(
        setup.skipped[0].reason.contains("schedule"),
        "{}",
        setup.skipped[0].reason
    );
    assert_eq!(
        store::agent::load(&layout, "nightly").unwrap().notify,
        Some(NotifyTarget::Telegram {
            chat_id: "-100123".into()
        })
    );
    let mut human = Vec::new();
    setup.write_human(&mut human).unwrap();
    let human = String::from_utf8(human).unwrap();
    assert!(human.contains("broken"), "{human}");
}

#[derive(Debug)]
struct DismissesAgentPicker(std::sync::Arc<Scripted>);

impl crate::action::confirm::Confirm for DismissesAgentPicker {
    fn confirm(&self, question: &str, caveat: Option<&str>) -> Result<bool, Failure> {
        self.0.confirm(question, caveat)
    }

    fn select_many(&self, _prompt: &str, _options: &[SelectOption]) -> Result<Vec<usize>, Failure> {
        Err(Failure::blocked("confirm.no_answer", "dismissed"))
    }

    fn select_one(&self, prompt: &str, options: &[SelectOption]) -> Result<Option<usize>, Failure> {
        self.0.select_one(prompt, options)
    }

    fn ask_text(&self, prompt: &str, secret: bool) -> Result<Option<String>, Failure> {
        self.0.ask_text(prompt, secret)
    }

    fn is_interactive(&self) -> bool {
        true
    }
}

#[test]
fn dismissing_the_agent_picker_sets_no_agent_and_keeps_the_earlier_steps() {
    let (_guard, root) = seeded_hall();
    let layout = Layout::at(root.clone());
    write_agent(&layout, "nightly", "");
    let fake = FakeTelegram::start(vec![ME, ONE_CHAT, SENT]);
    let script = Scripted::new(vec![Some("123:secret")], vec![], vec![Some(0)], vec![]);
    let ctx = Ctx::new(root).with_confirm(std::sync::Arc::new(DismissesAgentPicker(script)));

    let setup = setup_with(&ctx, &fake.api).unwrap().value;

    fake.requests();
    assert!(setup.agents.is_empty());
    assert!(setup.skipped.is_empty());
    assert_eq!(setup.chat_id, "-100123");
    assert_eq!(store::agent::load(&layout, "nightly").unwrap().notify, None);
    assert_eq!(
        AgentSecrets::read(&layout).unwrap().get(TELEGRAM_TOKEN_KEY),
        Some("123:secret")
    );
}
