//! Validation, file body resolution, and field inheritance for deliver PR metadata.

use std::collections::{BTreeMap, HashSet};

use crate::action::Ctx;
use crate::action::feature::deliver::input::{DeliverInput, PullRequestMetadata};
use crate::domain::feature::Feature;
use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction};

/// Resolve delivery metadata across all promoted repositories of the target feature.
pub(crate) fn resolve(
    ctx: &Ctx,
    feature: &Feature,
    input: &DeliverInput,
) -> Result<BTreeMap<RepoName, PullRequestMetadata>, Failure> {
    let has_global = input.global_metadata.title.is_some() || input.global_metadata.body.is_some();
    let has_overrides = !input.repo_overrides.is_empty();

    if input.land && (has_global || has_overrides) {
        return Err(Failure::blocked(
            "deliver.metadata_in_land_mode",
            "pull request metadata (--name, --body, --repo) cannot be used in land mode",
        )
        .expected("land mode (--land) without pull request metadata")
        .actual("pull request metadata was supplied with --land")
        .fix(FixAction::safe(
            "deliver.drop_metadata_or_land",
            "Remove pull request metadata options when landing, or remove --land to create/update pull requests.",
        )));
    }

    let mut seen_repos: HashSet<&str> = HashSet::new();
    for r_override in &input.repo_overrides {
        if !seen_repos.insert(r_override.repo.as_str()) {
            return Err(Failure::blocked(
                "deliver.duplicate_repo_group",
                format!(
                    "repository `{}` is specified more than once in --repo groups",
                    r_override.repo
                ),
            )
            .expected("each --repo group to name a unique repository")
            .actual(format!("repository `{}` was repeated", r_override.repo))
            .fix(FixAction::safe(
                "deliver.remove_duplicate_repo_group",
                format!(
                    "Combine metadata for `{}` into a single --repo group.",
                    r_override.repo
                ),
            )));
        }

        let repo_name = RepoName::new(&r_override.repo).map_err(|_| {
            Failure::blocked(
                "deliver.unpromoted_repo_override",
                format!(
                    "repository `{}` is not promoted in feature `{}`",
                    r_override.repo, feature.name
                ),
            )
            .expected(format!(
                "only repositories promoted in feature `{}`",
                feature.name
            ))
            .actual(format!("unpromoted repository `{}`", r_override.repo))
            .fix(FixAction::safe(
                "deliver.remove_unpromoted_repo_override",
                format!(
                    "Remove `--repo {}` or promote it with `ivar feature promote {} {}`.",
                    r_override.repo, feature.name, r_override.repo
                ),
            ))
        })?;

        if !feature.promotions.contains_key(&repo_name) {
            return Err(Failure::blocked(
                "deliver.unpromoted_repo_override",
                format!(
                    "repository `{}` is not promoted in feature `{}`",
                    r_override.repo, feature.name
                ),
            )
            .expected(format!(
                "only repositories promoted in feature `{}`",
                feature.name
            ))
            .actual(format!("unpromoted repository `{}`", r_override.repo))
            .fix(FixAction::safe(
                "deliver.remove_unpromoted_repo_override",
                format!(
                    "Remove `--repo {}` or promote it with `ivar feature promote {} {}`.",
                    r_override.repo, feature.name, r_override.repo
                ),
            )));
        }
    }

    let global_resolved = PullRequestMetadata {
        title: input.global_metadata.title.clone(),
        body: resolve_body(ctx, input.global_metadata.body.as_deref())?,
        draft: input.global_metadata.draft,
    };

    let mut overrides_by_repo: BTreeMap<RepoName, PullRequestMetadata> = BTreeMap::new();
    for r_override in &input.repo_overrides {
        let repo_name = RepoName::new(&r_override.repo)?;
        let resolved_body = resolve_body(ctx, r_override.metadata.body.as_deref())?;
        overrides_by_repo.insert(
            repo_name,
            PullRequestMetadata {
                title: r_override.metadata.title.clone(),
                body: resolved_body,
                draft: r_override.metadata.draft,
            },
        );
    }

    let mut result = BTreeMap::new();
    for repo_name in feature.promotions.keys() {
        let repo_override = overrides_by_repo.get(repo_name);
        let title = repo_override
            .and_then(|m| m.title.clone())
            .or_else(|| global_resolved.title.clone());
        let body = repo_override
            .and_then(|m| m.body.clone())
            .or_else(|| global_resolved.body.clone());
        let draft = repo_override
            .and_then(|m| m.draft)
            .or(global_resolved.draft);

        result.insert(
            repo_name.clone(),
            PullRequestMetadata { title, body, draft },
        );
    }

    Ok(result)
}

fn resolve_body(ctx: &Ctx, body: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(raw) = body else {
        return Ok(None);
    };

    if raw.starts_with("./") && (raw.ends_with(".md") || raw.ends_with(".txt")) {
        let relative_path = raw.trim_start_matches("./");
        let full_path = ctx.cwd.join(relative_path);
        let bytes = std::fs::read(&full_path).map_err(|e| {
            Failure::blocked(
                "deliver.body_file_read_failed",
                format!("failed to read pull request body file `{raw}`: {e}"),
            )
            .expected(format!("readable file at `{full_path}`"))
            .actual(e.to_string())
            .fix(FixAction::safe(
                "deliver.check_body_file_path",
                format!("Ensure `{full_path}` exists and is readable, or pass inline text."),
            ))
        })?;

        let content = String::from_utf8(bytes).map_err(|e| {
            Failure::blocked(
                "deliver.body_file_not_utf8",
                format!("pull request body file `{raw}` is not valid UTF-8: {e}"),
            )
            .expected("valid UTF-8 encoded text file")
            .actual("invalid UTF-8 byte sequence")
            .fix(FixAction::safe(
                "deliver.convert_body_file_utf8",
                format!("Ensure `{full_path}` is saved with UTF-8 encoding."),
            ))
        })?;

        Ok(Some(content))
    } else {
        Ok(Some(raw.to_owned()))
    }
}
