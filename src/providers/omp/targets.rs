use crate::domain::guard::is_structured_write;
use camino::Utf8PathBuf;

pub(crate) struct OmpWrite {
    pub(crate) targets: Vec<Utf8PathBuf>,
    pub(crate) writes: bool,
}

pub(crate) fn extract(tool: &str, args: &serde_json::Value) -> OmpWrite {
    let mut targets = Vec::new();
    if let Some(p) = args
        .get("filePath")
        .or_else(|| args.get("file_path"))
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
    {
        targets.push(Utf8PathBuf::from(p));
    }
    let writes = is_structured_write(tool);
    OmpWrite { targets, writes }
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/targets.rs"]
mod tests;
