//! `ivar agent notify telegram setup`: the bot token, the chat, and the agents that use them.

use std::io;

use serde::Serialize;

use super::notify::TELEGRAM_TOKEN_KEY;
use crate::action::confirm::SelectOption;
use crate::action::{Ctx, discover_hall};
use crate::domain::agent::spec::{NotifyTarget, is_telegram_chat_id};
use crate::error::{Failure, FixAction, Outcome, Report, WriteHuman};
use crate::infra::telegram;
use crate::store;
use crate::store::agent::AgentFile;
use crate::store::agent_secrets::AgentSecrets;
use crate::store::layout::Layout;
use crate::store::mcp_secrets::Change;

const LOOK_AGAIN: &str = "Look again";
const TYPE_IT: &str = "Type the chat id or @channel";
const TEST_TEXT: &str = "ivar: notifications for this chat are set up.";

#[derive(Debug, Serialize)]
pub struct TelegramSetup {
    pub bot: String,
    pub chat_id: String,
    pub token: TokenChange,
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenChange {
    Created,
    Updated,
    Kept,
}

/// # Errors
///
/// `agent.notify.interactive_only` without a terminal, `agent.notify.cancelled`
/// when a prompt is dismissed, `agent.notify.token_rejected`, or a store failure.
pub fn telegram_setup(ctx: &Ctx) -> Outcome<TelegramSetup> {
    setup_with(ctx, telegram::API)
}

pub(crate) fn setup_with(ctx: &Ctx, api: &str) -> Outcome<TelegramSetup> {
    if !ctx.confirm.is_interactive() {
        return Err(not_interactive());
    }
    let layout = discover_hall(ctx)?;
    let (token, bot, change) = token_step(ctx, &layout, api)?;
    let chat_id = chat_step(ctx, api, &token, &bot)?;
    let agents = agents_step(ctx, &layout, &chat_id)?;
    Ok(Report::new(TelegramSetup {
        bot,
        chat_id,
        token: change,
        agents,
    }))
}

fn token_step(
    ctx: &Ctx,
    layout: &Layout,
    api: &str,
) -> Result<(String, String, TokenChange), Failure> {
    let current = AgentSecrets::read(layout)?
        .get(TELEGRAM_TOKEN_KEY)
        .map(str::to_owned);
    if let Some(token) = current
        && ctx.confirm.confirm(
            &format!("Keep the {TELEGRAM_TOKEN_KEY} already in .ivar/secrets/agents.env?"),
            None,
        )?
    {
        let bot = telegram::get_me(api, &token).map_err(|reason| token_rejected(&reason))?;
        return Ok((token, bot, TokenChange::Kept));
    }
    loop {
        let token = ctx
            .confirm
            .ask_text("Bot token from @BotFather:", true)?
            .ok_or_else(cancelled)?
            .trim()
            .to_owned();
        match telegram::get_me(api, &token) {
            Ok(bot) => {
                let change = match AgentSecrets::set_and_write(layout, TELEGRAM_TOKEN_KEY, &token)?
                {
                    Change::Created => TokenChange::Created,
                    Change::Updated => TokenChange::Updated,
                    Change::Unchanged => TokenChange::Kept,
                };
                return Ok((token, bot, change));
            }
            Err(reason) => {
                let caveat = format!("Telegram did not accept that token: {reason}");
                if !ctx.confirm.confirm("Try another token?", Some(&caveat))? {
                    return Err(token_rejected(&reason));
                }
            }
        }
    }
}

fn chat_step(ctx: &Ctx, api: &str, token: &str, bot: &str) -> Result<String, Failure> {
    let mut note = format!(
        "Send any message to @{bot}, or add it to the group or channel (as admin for a channel), then pick \"{LOOK_AGAIN}\"."
    );
    loop {
        let chats = match telegram::get_updates(api, token) {
            Ok(chats) => chats,
            Err(reason) => {
                note = format!("Could not list the chats @{bot} can see: {reason}");
                Vec::new()
            }
        };
        let mut options: Vec<SelectOption> = chats
            .iter()
            .map(|chat| {
                SelectOption::new(
                    chat.id.clone(),
                    Some(format!("{} ({})", chat.name, chat.kind)),
                )
            })
            .collect();
        options.push(SelectOption::new(LOOK_AGAIN, None::<String>));
        options.push(SelectOption::new(TYPE_IT, None::<String>));
        let prompt = format!("{note}\nWhich chat gets the notifications?");
        let index = ctx
            .confirm
            .select_one(&prompt, &options)?
            .ok_or_else(cancelled)?;
        let chat_id = match chats.get(index) {
            Some(chat) => chat.id.clone(),
            None if index == chats.len() => continue,
            None => {
                let typed = ctx
                    .confirm
                    .ask_text("Chat id or @channel:", false)?
                    .ok_or_else(cancelled)?;
                let typed = typed.trim();
                if !is_telegram_chat_id(typed) {
                    note = format!(
                        "`{typed}` is not a Telegram chat id; use a number like -100123 or @channel_name."
                    );
                    continue;
                }
                typed.to_owned()
            }
        };
        match telegram::send_message(api, token, &chat_id, TEST_TEXT) {
            Ok(()) => return Ok(chat_id),
            Err(reason) => note = format!("The test message to {chat_id} failed: {reason}"),
        }
    }
}

fn agents_step(ctx: &Ctx, layout: &Layout, chat_id: &str) -> Result<Vec<String>, Failure> {
    let files = store::agent::list_files(layout)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let value = format!("telegram:{chat_id}");
    let options: Vec<SelectOption> = files
        .iter()
        .map(|file| {
            SelectOption::new(
                file.name.clone(),
                current_notify(file).map(|n| format!("notify: {n}")),
            )
        })
        .collect();
    let chosen = match ctx
        .confirm
        .select_many(&format!("Set `notify: {value}` on which agents?"), &options)
    {
        Ok(chosen) => chosen,
        Err(failure) if failure.code == "confirm.no_answer" => Vec::new(),
        Err(failure) => return Err(failure),
    };
    chosen
        .into_iter()
        .filter_map(|index| files.get(index))
        .map(|file| {
            store::agent::set_notify(file, &value)?;
            Ok(file.name.clone())
        })
        .collect()
}

fn current_notify(file: &AgentFile) -> Option<String> {
    store::agent::parse(file)
        .ok()?
        .notify
        .map(|target| match target {
            NotifyTarget::Webhook(url) => url,
            NotifyTarget::Telegram { chat_id } => format!("telegram:{chat_id}"),
        })
}

fn not_interactive() -> Failure {
    Failure::blocked(
        "agent.notify.interactive_only",
        "`ivar agent notify telegram setup` asks for the bot token and the chat, so it needs a terminal",
    )
    .fix(FixAction::safe(
        "agent.notify.manual_setup",
        format!(
            "Put {TELEGRAM_TOKEN_KEY}=<token> in .ivar/secrets/agents.env (mode 600) and set `notify: telegram:<chat_id>` in the agent's frontmatter."
        ),
    ))
}

fn cancelled() -> Failure {
    Failure::blocked(
        "agent.notify.cancelled",
        "setup cancelled; the steps already finished stay saved",
    )
}

fn token_rejected(reason: &str) -> Failure {
    Failure::blocked(
        "agent.notify.token_rejected",
        format!("Telegram did not accept the bot token: {reason}"),
    )
}

impl WriteHuman for TelegramSetup {
    fn write_human(&self, w: &mut impl io::Write) -> io::Result<()> {
        let token = match self.token {
            TokenChange::Created => "saved",
            TokenChange::Updated => "replaced",
            TokenChange::Kept => "kept",
        };
        writeln!(
            w,
            "Telegram bot @{}: {TELEGRAM_TOKEN_KEY} {token}",
            self.bot
        )?;
        writeln!(w, "Chat {} received a test message.", self.chat_id)?;
        if self.agents.is_empty() {
            writeln!(
                w,
                "No agent changed; add `notify: telegram:{}` to one to use it.",
                self.chat_id
            )?;
        } else {
            writeln!(w, "notify set on: {}", self.agents.join(", "))?;
        }
        writeln!(
            w,
            "No `ivar agent sync` needed: each run reads `notify` from the agent file."
        )
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/action/agent/notify_setup.rs"]
mod tests;
