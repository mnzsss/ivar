#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::domain::agent::receipt::{AgentRunOutcome, AgentRunReceipt, Trigger};

fn receipt(output: &str) -> AgentRunReceipt {
    let mut receipt = AgentRunReceipt::skipped(
        "20261105T210712Z-f9645724".into(),
        "audit".into(),
        Trigger::Manual,
        "t".into(),
    );
    receipt.outcome = AgentRunOutcome::Succeeded;
    receipt.exit_code = Some(0);
    receipt.output = output.into();
    receipt
}

#[test]
fn the_header_carries_hall_trigger_and_run_and_drops_exit_zero() {
    let html = receipt("").notify_summary("valhalla-hall").telegram_html();
    assert_eq!(
        html,
        "✅ <b>audit</b> · succeeded\n<i>valhalla-hall · manual · 20261105T210712Z-f9645724</i>"
    );
}

#[test]
fn output_markdown_becomes_telegram_html_inside_a_collapsed_quote() {
    let html = receipt("## Last 24h\n- **GAIO-1207:** see `a<b>` & more")
        .notify_summary("valhalla-hall")
        .telegram_html();
    assert!(
        html.ends_with(
            "<blockquote expandable><b>Last 24h</b>\n• <b>GAIO-1207:</b> see <code>a&lt;b&gt;</code> &amp; more</blockquote>"
        ),
        "{html}"
    );
}

#[test]
fn bold_never_wraps_code_because_telegram_refuses_it() {
    let html = receipt("**run `x` now**")
        .notify_summary("valhalla-hall")
        .telegram_html();
    assert!(html.contains("**run <code>x</code> now**"), "{html}");
}

#[test]
fn unpaired_markers_stay_literal() {
    let html = receipt("a ** b ` c")
        .notify_summary("valhalla-hall")
        .telegram_html();
    assert!(html.contains("a ** b ` c"), "{html}");
}

#[test]
fn a_cut_tail_starts_at_a_whole_line() {
    let output = format!("{}\nhalf a sentence\nwhole line", "x".repeat(3000));
    let html = receipt(&output)
        .notify_summary("valhalla-hall")
        .telegram_html();
    assert!(
        html.contains("<blockquote expandable>…\nhalf a sentence\nwhole line</blockquote>"),
        "{html}"
    );
}

#[test]
fn the_investigation_prompt_fits_the_copy_button_and_names_the_logs_command() {
    let prompt = receipt("")
        .notify_summary("valhalla-hall")
        .investigation_prompt();
    assert!(prompt.chars().count() <= 256, "{prompt}");
    assert!(
        prompt.contains("ivar agent logs audit --run 20261105T210712Z-f9645724"),
        "{prompt}"
    );
    assert!(prompt.starts_with("In hall valhalla-hall,"), "{prompt}");
}

#[test]
fn a_real_hall_and_agent_name_keep_the_whole_prompt() {
    let mut receipt = receipt("");
    receipt.agent = "backend-agents-audit".into();
    let prompt = receipt
        .notify_summary("gaio-workdir")
        .investigation_prompt();
    assert!(
        prompt.ends_with("report only what needs action."),
        "{prompt}"
    );
}

#[test]
fn an_agent_task_card_carries_the_scheduled_and_the_real_time() {
    let mut task = receipt("found nothing");
    task.trigger = Trigger::Schedule;
    task.started_at = "2026-11-08T12:00:03.000000000Z".into();
    task.scheduled_at = Some("2026-11-08T09:00-03:00".into());

    let html = task.notify_summary("valhalla-hall").telegram_html();
    assert!(
        html.contains(
            "\n<i>scheduled 2026-11-08T09:00-03:00 · ran 2026-11-08T12:00:03.000000000Z</i>"
        ),
        "{html}"
    );
    let text = task.notify_summary("valhalla-hall").telegram_text();
    assert!(
        text.contains("\nscheduled 2026-11-08T09:00-03:00 · ran 2026-11-08T12:00:03.000000000Z"),
        "{text}"
    );
    assert!(
        !receipt("x")
            .notify_summary("h")
            .telegram_text()
            .contains("scheduled")
    );
}
