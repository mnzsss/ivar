use camino::Utf8PathBuf;

use crate::providers::SessionProjection;

/// OMP discovers hooks under `.omp/hooks/pre/`, so a session must see the
/// hall's hook directory the way it sees the command catalog. Claude Code
/// and OpenCode have no equivalent second surface.
pub(crate) fn extra_projections() -> Vec<SessionProjection> {
    vec![SessionProjection {
        hall_source: Utf8PathBuf::from(".omp/hooks/pre"),
        config_relative_dest: Utf8PathBuf::from("hooks/pre"),
    }]
}
