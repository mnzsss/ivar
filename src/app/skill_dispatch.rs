use std::io;
use std::process::ExitCode;

use camino::Utf8PathBuf;

use super::respond::{respond, respond_failure};
use crate::action::Ctx;
use crate::action::feedback as feedback_action;
use crate::action::script::list as script_list;
use crate::action::skill::{
    add as skill_add, create as skill_create, detach as skill_detach, doctor as skill_doctor,
    list as skill_list, remove as skill_remove, status as skill_status, sync as skill_sync,
    toggle as skill_toggle, update as skill_update,
};
use crate::cli::FeedbackCommand;
use crate::cli::root::{ScriptCommand, SkillCommand};
use crate::infra::term;

pub(super) fn dispatch_script(
    cmd: &ScriptCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        ScriptCommand::List => respond(script_list::list(ctx), json, stdout, stderr),
    }
}

pub(super) fn dispatch_skill(
    cmd: SkillCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        SkillCommand::List => respond(skill_list::list(ctx), json, stdout, stderr),
        SkillCommand::Create(args) => {
            respond(skill_create::create(ctx, args.into()), json, stdout, stderr)
        }
        SkillCommand::Add(args) => respond(skill_add::add(ctx, &args.into()), json, stdout, stderr),
        SkillCommand::Update(args) => respond(
            skill_update::update(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SkillCommand::Remove(args) => respond(
            skill_remove::remove(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SkillCommand::Detach(args) => respond(
            skill_detach::detach(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SkillCommand::Enable(args) => respond(
            skill_toggle::enable(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SkillCommand::Disable(args) => respond(
            skill_toggle::disable(ctx, &args.into()),
            json,
            stdout,
            stderr,
        ),
        SkillCommand::Sync => respond(skill_sync::sync(ctx), json, stdout, stderr),
        SkillCommand::Status => respond(skill_status::status(ctx), json, stdout, stderr),
        SkillCommand::Doctor => respond(skill_doctor::doctor(ctx), json, stdout, stderr),
    }
}

pub(super) fn dispatch_feedback(
    cmd: FeedbackCommand,
    ctx: &Ctx,
    json: bool,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> ExitCode {
    match cmd {
        FeedbackCommand::Add(add_args) => {
            let kind = match add_args.kind.as_str() {
                "proposal" => crate::domain::feedback::FeedbackKind::Proposal,
                _ => crate::domain::feedback::FeedbackKind::Bug,
            };
            let content_res = match add_args.file.as_deref() {
                Some("-") => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map(|_| Some(buf))
                        .map_err(|e| {
                            crate::error::Failure::failed(
                                "stdin.read_failed",
                                format!("failed to read stdin: {e}"),
                            )
                        })
                }
                Some(path) => {
                    let p = Utf8PathBuf::from(path);
                    match crate::infra::fs::read_text(&p) {
                        Ok(Some(text)) => Ok(Some(text)),
                        Ok(None) => Err(crate::error::Failure::blocked(
                            "feedback.file_not_found",
                            format!("file `{p}` not found"),
                        )),
                        Err(err) => Err(err.into()),
                    }
                }
                None => Ok(None),
            };
            let result = content_res.and_then(|body| {
                feedback_action::add::add(
                    ctx,
                    feedback_action::add::AddInput {
                        title: add_args.title,
                        kind,
                        body,
                    },
                )
            });
            respond(result, json, stdout, stderr)
        }
        FeedbackCommand::List(list_args) => {
            let status = list_args.status.as_deref().and_then(|s| match s {
                "open" => Some(crate::domain::feedback::FeedbackStatus::Open),
                "published" => Some(crate::domain::feedback::FeedbackStatus::Published),
                _ => None,
            });
            respond(
                feedback_action::list::list(ctx, feedback_action::list::ListInput { status }),
                json,
                stdout,
                stderr,
            )
        }
        FeedbackCommand::Show(show_args) => respond(
            feedback_action::show::show(
                ctx,
                feedback_action::show::ShowInput {
                    id: show_args.id,
                    redacted: show_args.redacted,
                },
            ),
            json,
            stdout,
            stderr,
        ),
        FeedbackCommand::Submit(submit_args) => {
            let input = feedback_action::submit::SubmitInput::from(submit_args);
            if input.mode == feedback_action::submit::SubmitMode::Interactive
                && !term::is_tty(term::Stream::Stdout)
            {
                respond_failure(
                    &crate::error::Failure::blocked(
                        "feedback.submit_needs_terminal",
                        "Feedback submission requires an interactive terminal for confirmation",
                    ),
                    json,
                    stdout,
                    stderr,
                )
            } else {
                respond(
                    feedback_action::submit::submit(ctx, input),
                    json,
                    stdout,
                    stderr,
                )
            }
        }
    }
}
