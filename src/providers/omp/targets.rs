use camino::Utf8PathBuf;

use crate::domain::guard::is_structured_write;

pub(crate) struct OmpWrite {
    pub(crate) targets: Vec<Utf8PathBuf>,
    pub(crate) writes: bool,
}

pub(crate) fn extract(tool: &str, args: &serde_json::Value) -> OmpWrite {
    let mut targets = Vec::new();

    // 1. Direct path fields
    if let Some(p) = args
        .get("filePath")
        .or_else(|| args.get("file_path"))
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str())
    {
        targets.push(Utf8PathBuf::from(p));
    }

    // 2. Hashline headers, MV destinations, and apply-patch headers in args.input string
    if let Some(input_str) = args.get("input").and_then(|v| v.as_str()) {
        for line in input_str.lines() {
            let line = line.trim_end();
            // Hashline header: starts with '[' and ends with ']'
            if line.starts_with('[') && line.ends_with(']') {
                let inside = &line[1..line.len() - 1];
                if let Some((path_part, tag)) = inside.rsplit_once('#')
                    && tag.len() == 4
                    && tag.chars().all(|c| c.is_ascii_hexdigit())
                {
                    targets.push(Utf8PathBuf::from(path_part.trim()));
                }
            }
            // MV destination
            else if let Some(dest) = line.strip_prefix("MV ") {
                let dest = dest.trim();
                let unquoted = if dest.starts_with('"') && dest.ends_with('"') && dest.len() >= 2 {
                    &dest[1..dest.len() - 1]
                } else {
                    dest
                };
                if !unquoted.is_empty() {
                    targets.push(Utf8PathBuf::from(unquoted));
                }
            }
            // Apply patch headers
            else if let Some(path) = line
                .strip_prefix("*** Add File:")
                .or_else(|| line.strip_prefix("*** Update File:"))
                .or_else(|| line.strip_prefix("*** Delete File:"))
                .or_else(|| line.strip_prefix("*** Move to:"))
            {
                let path = path.trim();
                if !path.is_empty() {
                    targets.push(Utf8PathBuf::from(path));
                }
            }
        }
    }

    let writes = is_structured_write(tool);
    OmpWrite { targets, writes }
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/targets.rs"]
mod tests;
