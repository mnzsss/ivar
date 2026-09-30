//! Promoting a repo into a child's parent before the child can use it —
//! asked on an interactive run, refused with the exact command otherwise.
//! Shared by `integrate` (the child's work lands on the parent's branch) and
//! `promote` (the child's branch starts from the parent's).

use crate::action::Ctx;
use crate::domain::feature::Feature;
use crate::domain::name::RepoName;
use crate::error::{Failure, FixAction, Warning};

use super::promote::{self, PromoteInput};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Caller {
    Integrate,
    Promote,
}

impl Caller {
    const fn required_code(self) -> &'static str {
        match self {
            Self::Integrate => "integration.parent_promotion_required",
            Self::Promote => "feature.parent_promotion_required",
        }
    }

    const fn failed_code(self) -> &'static str {
        match self {
            Self::Integrate => "integration.parent_promotion_failed",
            Self::Promote => "feature.parent_promotion_failed",
        }
    }

    const fn promote_parent_fix(self) -> &'static str {
        match self {
            Self::Integrate => "integration.promote_parent",
            Self::Promote => "feature.promote_parent",
        }
    }

    const fn promote_manually_fix(self) -> &'static str {
        match self {
            Self::Integrate => "integration.promote_manually",
            Self::Promote => "feature.promote_manually",
        }
    }

    fn stake(self, child: &Feature, parent: &Feature, repo: &RepoName) -> String {
        match self {
            Self::Integrate => format!("`{repo}`'s work will land on its branch"),
            Self::Promote => format!(
                "`{}` branches from `{}` in `{repo}`",
                child.name, parent.name
            ),
        }
    }

    fn retry(self) -> &'static str {
        match self {
            Self::Integrate => "integrate again",
            Self::Promote => "promote the child again",
        }
    }

    fn not_recorded(self, child: &Feature) -> String {
        match self {
            Self::Integrate => "no receipt was recorded".to_owned(),
            Self::Promote => format!("`{}` was not promoted", child.name),
        }
    }
}

pub(crate) fn ensure(
    ctx: &Ctx,
    caller: Caller,
    child: &Feature,
    parent: &Feature,
    repo: &RepoName,
) -> Result<Vec<Warning>, Failure> {
    let question = format!(
        "Feature `{}` does not promote `{repo}`, but {}. Promote `{repo}` into `{}`?",
        parent.name,
        caller.stake(child, parent, repo),
        parent.name
    );
    if !ctx.confirm(
        &question,
        Some("This promotes the repo into the parent feature."),
    )? {
        return Err(required(caller, child, parent, repo));
    }
    promote::promote(
        ctx,
        PromoteInput {
            feature: parent.name.to_string(),
            repo: repo.to_string(),
            base: None,
        },
    )
    .map(|report| report.warnings)
    .map_err(|failure| {
        Failure::failed(
            caller.failed_code(),
            format!(
                "promoting `{repo}` into `{}` failed; {}",
                parent.name,
                caller.not_recorded(child)
            ),
        )
        .actual(failure.what)
        .fix(
            FixAction::safe(
                caller.promote_manually_fix(),
                format!(
                    "Run `ivar feature promote {} {repo}`, then {}.",
                    parent.name,
                    caller.retry()
                ),
            )
            .command(format!("ivar feature promote {} {repo}", parent.name)),
        )
    })
}

fn required(caller: Caller, child: &Feature, parent: &Feature, repo: &RepoName) -> Failure {
    Failure::blocked(
        caller.required_code(),
        format!(
            "`{repo}` is not promoted into `{}`, which {}",
            parent.name,
            match caller {
                Caller::Integrate => format!("must receive `{}`'s work", child.name),
                Caller::Promote => format!("`{}` branches from", child.name),
            }
        ),
    )
    .expected("the parent to promote every repo the child promotes")
    .actual(format!("`{repo}` is missing from `{}`", parent.name))
    .fix(
        FixAction::safe(
            caller.promote_parent_fix(),
            format!(
                "Promote `{repo}` into `{}`, then {}.",
                parent.name,
                caller.retry()
            ),
        )
        .command(format!("ivar feature promote {} {repo}", parent.name)),
    )
}
