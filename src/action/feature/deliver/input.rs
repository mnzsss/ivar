//! Input options for `ivar feature deliver`.

/// Input options for `ivar feature deliver`.
#[derive(Debug, Clone)]
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
}
