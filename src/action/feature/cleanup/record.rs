use super::*;
use crate::action::memory::legacy;

const FINGERPRINT_PREFIX: &str = "sha256:";

pub(super) fn write_record_outcome(
    layout: &crate::store::layout::Layout,
    mut record: CleanupRecord,
    record_path: &Utf8Path,
    apply_outcome: &CleanupApplyOutcome,
) -> Result<(), Failure> {
    let abs_record_path = if record_path.is_absolute() {
        record_path.to_path_buf()
    } else {
        layout.root().join(record_path)
    };
    record.outcome = Some(apply_outcome.clone());
    let record_json = json::to_canonical_string(&record)?;
    fs::write_atomic(&abs_record_path, record_json.as_bytes()).map_err(|source| {
        Failure::failed(
            "feature.cleanup_record_write_failed",
            format!("could not write cleanup outcome to record `{record_path}`: {source}"),
        )
    })
}

/// The migration command, when `docs/updates` still holds pages `feature cleanup` cannot read.
fn legacy_migration_fix(layout: &crate::store::layout::Layout) -> Option<String> {
    if !legacy::updates_need_migration(layout) {
        return None;
    }
    legacy::migration_command(layout, crate::git::inside_work_tree(layout.root()))
}

pub(super) fn load_and_validate_record(
    record_path: &Utf8Path,
    layout: &crate::store::layout::Layout,
) -> Result<CleanupRecord, Failure> {
    let update_pages = layout.update_pages_dir();

    let abs_record_path = if record_path.is_absolute() {
        record_path.to_path_buf()
    } else {
        layout.root().join(record_path)
    };

    let outside = |reason: String| {
        let mut failure = Failure::blocked("feature.cleanup_record_outside_docs_updates", reason);
        if let Some(command) = legacy_migration_fix(layout) {
            failure = failure.fix(
                FixAction::safe(
                    "feature.cleanup_migrate_legacy_updates",
                    "This hall still keeps its pages in `docs/updates`; move them under `docs/memory`, commit the move, then pass the record from `docs/memory/update`.",
                )
                .command(command),
            );
        }
        failure
    };

    let canonical_update_pages = fs_err::canonicalize(&update_pages).map_err(|_| {
        outside(format!(
            "the update pages directory `{update_pages}` does not exist, so cleanup record `{record_path}` cannot be read from it"
        ))
    })?;

    let canonical_record = fs_err::canonicalize(&abs_record_path).map_err(|_| {
        Failure::blocked(
            "feature.cleanup_record_not_found",
            format!("cleanup record `{record_path}` does not exist"),
        )
    })?;

    if !canonical_record.starts_with(&canonical_update_pages) {
        return Err(outside(format!(
            "cleanup record `{record_path}` must resolve inside `{update_pages}`"
        )));
    }

    let content = crate::infra::fs::read_text(&abs_record_path)
        .map_err(|err| {
            Failure::blocked(
                "feature.cleanup_record_not_found",
                format!("failed to read cleanup record `{record_path}`: {err}"),
            )
        })?
        .ok_or_else(|| {
            Failure::blocked(
                "feature.cleanup_record_not_found",
                format!("cleanup record `{record_path}` does not exist"),
            )
        })?;

    let record: CleanupRecord = serde_json::from_str(&content).map_err(|err| {
        Failure::blocked(
            "feature.cleanup_record_malformed",
            format!("failed to parse cleanup record `{record_path}`: {err}"),
        )
    })?;

    record.validate().map_err(|err| {
        Failure::blocked(
            "feature.cleanup_record_invalid",
            format!("cleanup record at `{record_path}` is invalid: {err}"),
        )
    })?;

    Ok(record)
}

pub(super) fn validate_record_against_preview(
    record: &CleanupRecord,
    preview: &CleanupPreview,
    forge_consulted: bool,
) -> Result<(), Failure> {
    if record.feature != preview.feature || record.branch != preview.branch {
        return Err(Failure::blocked(
            "feature.cleanup_record_feature_mismatch",
            format!(
                "cleanup record feature `{}` (branch `{}`) does not match feature `{}` (branch `{}`)",
                record.feature, record.branch, preview.feature, preview.branch
            ),
        ));
    }

    let recorded = record
        .fingerprint
        .strip_prefix(FINGERPRINT_PREFIX)
        .unwrap_or(&record.fingerprint);
    if recorded != preview.fingerprint {
        let mut failure = Failure::blocked(
            "feature.cleanup_fingerprint_mismatch",
            format!(
                "the state of feature `{}` has drifted since the cleanup record was written",
                preview.feature
            ),
        )
        .expected(format!("record fingerprint `{}`", record.fingerprint))
        .actual(format!(
            "current preview fingerprint `{}`",
            preview.fingerprint
        ))
        .fix(FixAction::safe(
            "feature.cleanup_re_preview",
            format!(
                "Rerun `/ivar-feature-cleanup {}` to update the docs and record with the new fingerprint.",
                preview.feature
            ),
        ));
        if forge_consulted {
            failure = failure.fix(FixAction::safe(
                "feature.cleanup_forge_moved",
                "A repo's delivery rests on a pull-request lookup, and the forge now answers differently than it did for the record. Re-run the preview to see the forge's current answer.",
            ));
        }
        return Err(failure);
    }

    if !record.approvals.delivery.approved {
        return Err(Failure::blocked(
            "feature.cleanup_delivery_not_approved",
            format!(
                "delivery approval is false in cleanup record for feature `{}`",
                preview.feature
            ),
        ));
    }

    if !record.approvals.teardown.approved {
        return Err(Failure::blocked(
            "feature.cleanup_teardown_not_approved",
            format!(
                "teardown approval is false in cleanup record for feature `{}`",
                preview.feature
            ),
        ));
    }

    if preview.has_blockers() {
        let command = format!("ivar feature cleanup {} --preview", preview.feature);
        return Err(Failure::blocked(
            "feature.cleanup_blocked",
            format!(
                "feature `{}` cannot be cleaned up due to blockers",
                preview.feature
            ),
        )
        .fix(
            FixAction::safe(
                "feature.cleanup_resolve_blockers",
                format!("List the blockers with `{command}`, resolve them, then clean up again."),
            )
            .command(command),
        ));
    }

    Ok(())
}
