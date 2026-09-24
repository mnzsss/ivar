//! Refuses delivery of commits or PR text that carry AI attribution.

use crate::domain::feature::DeliveryPreview;
use crate::error::{Failure, FixAction};
use crate::git::exec;
use crate::store::layout::Layout;

pub(crate) fn is_attribution(line: &str) -> bool {
    let line = line.trim().to_lowercase();
    line.contains("generated with [claude code]")
        || line.contains("generated with claude code")
        || line.starts_with("🤖 generated with")
        || (line.starts_with("co-authored-by:")
            && (line.contains("noreply@anthropic.com") || line.contains("claude")))
}

pub(super) fn check(layout: &Layout, preview: &DeliveryPreview) -> Result<(), Failure> {
    let mut findings = Vec::new();
    for repo in &preview.repos {
        let metadata = [repo.pr_title.as_deref(), repo.pr_body.as_deref()];
        for line in metadata.into_iter().flatten().flat_map(str::lines) {
            if is_attribution(line) {
                findings.push(format!("`{}` PR metadata: {}", repo.repo, line.trim()));
            }
        }
        let commits = exec::commit_messages(
            &layout.repo_bare(&repo.repo),
            repo.base_branch.as_str(),
            repo.local_branch.as_str(),
        )?;
        for (sha, message) in commits {
            for line in message.lines().filter(|line| is_attribution(line)) {
                findings.push(format!("`{}` commit {sha}: {}", repo.repo, line.trim()));
            }
        }
    }
    if findings.is_empty() {
        return Ok(());
    }
    Err(Failure::blocked(
        "deliver.ai_attribution",
        format!("feature `{}` carries AI attribution", preview.feature),
    )
    .expected("no AI attribution in commit messages, PR titles or PR bodies")
    .actual(findings.join("; "))
    .fix(FixAction::safe(
        "deliver.remove_ai_attribution",
        "Rewrite the listed commit messages without those lines (for example `git rebase <base> --exec \"git log -1 --format=%B | grep -viE 'co-authored-by:.*(claude|anthropic)|generated with' | git commit --amend -F -\"`), drop them from --name/--body, then run the preview again.",
    )))
}
