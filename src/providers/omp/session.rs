use camino::Utf8PathBuf;

use crate::providers::SessionProjection;

/// OMP discovers hooks under `.omp/hooks/pre/` and extensions under
/// `.omp/extensions/`, so a session must see the hall's hook and extension
/// directories the way it sees the command catalog. Claude Code and OpenCode
/// have no equivalent surfaces.
pub(crate) fn extra_projections() -> Vec<SessionProjection> {
    vec![
        SessionProjection {
            hall_source: Utf8PathBuf::from(".omp/hooks/pre"),
            config_relative_dest: Utf8PathBuf::from("hooks/pre"),
        },
        SessionProjection {
            hall_source: Utf8PathBuf::from(".omp/extensions"),
            config_relative_dest: Utf8PathBuf::from("extensions"),
        },
    ]
}
