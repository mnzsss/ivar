//! The effective base of one promoted repo, for every verb that reads it
//! back: `status`, `rebase`, `prune`, `deliver`. One place so they cannot
//! disagree with what `promote` recorded.

use crate::domain::feature::{Feature, Promotion, effective_base};
use crate::domain::name::BranchName;

/// `promotion`'s recorded base, or — for a promotion recorded before this
/// field existed — [`effective_base`] of the feature's declared base against
/// `default_branch`, the same fallback `promote` itself resolves.
pub(crate) fn resolve(
    feature: &Feature,
    promotion: &Promotion,
    default_branch: &BranchName,
) -> BranchName {
    promotion
        .base
        .clone()
        .unwrap_or_else(|| effective_base(feature.base.as_ref(), default_branch))
}
