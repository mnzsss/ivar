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
fn the_header_carries_outcome_trigger_run_and_exit() {
    let html = receipt("").notify_summary().telegram_html();
    assert_eq!(
        html,
        "✅ <b>audit</b> · succeeded\n<i>manual · 20261105T210712Z-f9645724 · exit 0</i>"
    );
}

#[test]
fn output_markdown_becomes_telegram_html_inside_a_collapsed_quote() {
    let html = receipt("## Last 24h\n- **GAIO-1207:** see `a<b>` & more")
        .notify_summary()
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
    let html = receipt("**run `x` now**").notify_summary().telegram_html();
    assert!(html.contains("**run <code>x</code> now**"), "{html}");
}

#[test]
fn unpaired_markers_stay_literal() {
    let html = receipt("a ** b ` c").notify_summary().telegram_html();
    assert!(html.contains("a ** b ` c"), "{html}");
}

#[test]
fn a_cut_tail_starts_at_a_whole_line() {
    let output = format!("{}\nhalf a sentence\nwhole line", "x".repeat(3000));
    let html = receipt(&output).notify_summary().telegram_html();
    assert!(
        html.contains("<blockquote expandable>…\nhalf a sentence\nwhole line</blockquote>"),
        "{html}"
    );
}

#[test]
fn the_investigation_prompt_fits_the_copy_button_and_names_the_logs_command() {
    let prompt = receipt("").notify_summary().investigation_prompt();
    assert!(prompt.chars().count() <= 256, "{prompt}");
    assert!(
        prompt.contains("ivar agent logs audit --run 20261105T210712Z-f9645724"),
        "{prompt}"
    );
}
