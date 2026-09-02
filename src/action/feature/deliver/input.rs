//! Input options for `ivar feature deliver`.

/// Supplied metadata fields for a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullRequestMetadata {
    /// The supplied title, if any.
    pub title: Option<String>,
    /// The supplied body value (either inline text or `./*.md` / `./*.txt` path), if any.
    pub body: Option<String>,
    /// Whether the pull request should be created as a draft.
    pub draft: Option<bool>,
}

/// A repository-scoped metadata override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoMetadataOverride {
    /// The target repository name.
    pub repo: String,
    /// The metadata specified for this repository.
    pub metadata: PullRequestMetadata,
}

/// Input options for `ivar feature deliver`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeliverInput {
    /// The feature to deliver.
    pub feature: String,
    /// Preview only: compute and print the summary, push nothing.
    pub preview: bool,
    /// Land feature branches into default branches locally (fast-forward only).
    pub land: bool,
    /// The fingerprint from the preview the human approved. Required for
    /// apply; the push is refused when the current state does not fingerprint
    /// to it.
    pub fingerprint: Option<String>,
    /// Global pull request metadata.
    pub global_metadata: PullRequestMetadata,
    /// Ordered repository-scoped metadata overrides.
    pub repo_overrides: Vec<RepoMetadataOverride>,
}
