//! Validation, file body resolution, and field inheritance for deliver PR metadata.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::action::Ctx;
use crate::action::feature::deliver::input::{DeliverInput, PullRequestMetadata};
use crate::domain::feature::Feature;
use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction};

/// The `deliver.unpromoted_repo_override` refusal, shared by an unparseable
/// `--repo` name and a parseable one the feature never promoted.
fn unpromoted_repo_override(feature: &Feature, repo: &str) -> Failure {
    Failure::blocked(
        "deliver.unpromoted_repo_override",
        format!(
            "repository `{repo}` is not promoted in feature `{}`",
            feature.name
        ),
    )
    .expected(format!(
        "only repositories promoted in feature `{}`",
        feature.name
    ))
    .actual(format!("unpromoted repository `{repo}`"))
    .fix(FixAction::safe(
        "deliver.remove_unpromoted_repo_override",
        format!(
            "Remove `--repo {repo}` or promote it with `ivar feature promote {} {repo}`.",
            feature.name
        ),
    ))
}

/// The `deliver.unpromoted_only_repo` refusal: an `--only` name that is unparseable or not promoted.
fn unpromoted_only_repo(feature: &Feature, repo: &str) -> Failure {
    let promoted = feature
        .promotions
        .keys()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Failure::blocked(
        "deliver.unpromoted_only_repo",
        format!(
            "repository `{repo}` is not promoted in feature `{}`",
            feature.name
        ),
    )
    .expected(format!(
        "`--only` to name a promoted repository: {promoted}"
    ))
    .actual(format!(
        "`--only {repo}`; promoted repositories are {promoted}"
    ))
    .fix(FixAction::safe(
        "deliver.fix_only_selection",
        format!("Pass `--only` with one of: {promoted}."),
    ))
}

/// The `deliver.repo_override_outside_only` refusal: a `--repo` metadata group for a repository `--only` leaves out.
fn repo_override_outside_only(repo: &str) -> Failure {
    Failure::blocked(
        "deliver.repo_override_outside_only",
        format!("`--repo {repo}` sets metadata for a repository `--only` does not deliver"),
    )
    .expected("every `--repo` group to name a repository selected by `--only`")
    .actual(format!("`{repo}` is not among the `--only` repositories"))
    .fix(FixAction::safe(
        "deliver.align_repo_with_only",
        format!("Add `--only {repo}` or drop the `--repo {repo}` group."),
    ))
}

/// The promoted repositories this delivery covers: every one, unless `--only` narrows it.
pub(crate) fn selected_repos(
    feature: &Feature,
    input: &DeliverInput,
) -> Result<BTreeSet<RepoName>, Failure> {
    if input.only.is_empty() {
        return Ok(feature.promotions.keys().cloned().collect());
    }
    input
        .only
        .iter()
        .map(|raw| {
            RepoName::new(raw)
                .ok()
                .filter(|name| feature.promotions.contains_key(name))
                .ok_or_else(|| unpromoted_only_repo(feature, raw))
        })
        .collect()
}

fn validate_repo_overrides(feature: &Feature, input: &DeliverInput) -> Result<(), Failure> {
    let has_global = input.global_metadata.title.is_some()
        || input.global_metadata.body.is_some()
        || input.global_metadata.draft.is_some();
    let has_overrides = !input.repo_overrides.is_empty()
        || input
            .repo_overrides
            .iter()
            .any(|o| o.metadata.draft.is_some());

    if input.land && (has_global || has_overrides) {
        return Err(Failure::blocked(
            "deliver.metadata_in_land_mode",
            "pull request options (--name, --body, --draft, --repo) cannot be used in land mode",
        )
        .expected("land mode (--land) without pull request options")
        .actual("pull request options were supplied with --land")
        .fix(FixAction::safe(
            "deliver.drop_metadata_or_land",
            "Remove pull request options when landing, or remove --land to create/update pull requests.",
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

        let repo_name = RepoName::new(&r_override.repo)
            .map_err(|_| unpromoted_repo_override(feature, &r_override.repo))?;

        if !feature.promotions.contains_key(&repo_name) {
            return Err(unpromoted_repo_override(feature, &r_override.repo));
        }

        if !input.only.is_empty() && !input.only.contains(&r_override.repo) {
            return Err(repo_override_outside_only(&r_override.repo));
        }
    }

    Ok(())
}

/// Resolve delivery metadata across all promoted repositories of the target feature.
pub(crate) fn resolve(
    ctx: &Ctx,
    feature: &Feature,
    input: &DeliverInput,
) -> Result<BTreeMap<RepoName, PullRequestMetadata>, Failure> {
    validate_repo_overrides(feature, input)?;

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

    // A single-line, space-free value ending in `.md` or `.txt` is a path. It
    // resolves against the cwd (`join` on an absolute path yields that path).
    // A bare name with no path separator (e.g. `README.md`) is read as a file
    // only when it exists, and is otherwise kept as inline text. A value with a
    // separator that names no file is refused rather than published as text.
    let looks_like_path = !raw.contains(char::is_whitespace)
        && !raw.is_empty()
        && (raw.ends_with(".md") || raw.ends_with(".txt"));
    if looks_like_path {
        let full_path = ctx.cwd.join(raw);
        let has_separator = raw.contains('/');
        let read = std::fs::read(&full_path);
        if let Err(e) = &read
            && e.kind() == std::io::ErrorKind::NotFound
        {
            if !has_separator {
                return Ok(Some(raw.to_owned()));
            }
            return Err(Failure::blocked(
                "deliver.body_looks_like_path",
                format!("`--body {raw}` looks like a file path, but `{full_path}` does not exist"),
            )
            .expected(format!("an existing file at `{full_path}`, or inline body text"))
            .actual("no such file")
            .fix(FixAction::safe(
                "deliver.fix_body_path",
                format!(
                    "Correct the path so `{full_path}` exists, or pass inline text containing a space or several lines."
                ),
            )));
        }
        let bytes = read.map_err(|e| {
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
