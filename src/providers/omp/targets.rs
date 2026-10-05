use camino::Utf8PathBuf;

use crate::domain::guard::is_structured_write;

pub(crate) struct OmpWrite {
    pub(crate) targets: Vec<Utf8PathBuf>,
    pub(crate) writes: bool,
}

fn glob_literal_prefix(pattern: &str) -> Utf8PathBuf {
    let metachars = ['*', '?', '[', '{'];
    if let Some(idx) = pattern.find(|c| metachars.contains(&c)) {
        let prefix = &pattern[..idx];
        if let Some((dir, _)) = prefix.rsplit_once('/') {
            if dir.is_empty() {
                Utf8PathBuf::from(".")
            } else {
                Utf8PathBuf::from(dir)
            }
        } else {
            Utf8PathBuf::from(".")
        }
    } else {
        Utf8PathBuf::from(pattern)
    }
}

fn parse_device_content(is_direct: bool, args: &serde_json::Value) -> Option<serde_json::Value> {
    if is_direct {
        Some(args.clone())
    } else {
        args.get("content")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str(s).ok())
    }
}

fn extract_ast_edit(is_direct: bool, args: &serde_json::Value) -> OmpWrite {
    match parse_device_content(is_direct, args) {
        Some(obj) => {
            let mut targets = Vec::new();
            if let Some(paths) = obj.get("paths").and_then(|v| v.as_array()) {
                for p in paths {
                    if let Some(s) = p.as_str() {
                        targets.push(glob_literal_prefix(s));
                    }
                }
            }
            OmpWrite {
                targets,
                writes: true,
            }
        }
        None => OmpWrite {
            targets: Vec::new(),
            writes: true,
        },
    }
}

fn extract_lsp(is_direct: bool, args: &serde_json::Value) -> OmpWrite {
    match parse_device_content(is_direct, args) {
        Some(obj) => {
            let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("");
            let apply = obj.get("apply").and_then(|v| v.as_bool());

            let is_mutating = match action {
                "rename" => apply.unwrap_or(true),
                "rename_file" | "request" => true,
                "code_actions" => apply.unwrap_or(false),
                _ => false,
            };

            if is_mutating {
                let mut targets = Vec::new();
                if let Some(file) = obj.get("file").and_then(|v| v.as_str()) {
                    if file == "*" {
                        targets.push(Utf8PathBuf::from("."));
                    } else {
                        targets.push(Utf8PathBuf::from(file));
                    }
                }
                if action == "rename_file"
                    && let Some(new_name) = obj.get("new_name").and_then(|v| v.as_str())
                {
                    targets.push(Utf8PathBuf::from(new_name));
                }
                OmpWrite {
                    targets,
                    writes: true,
                }
            } else {
                OmpWrite {
                    targets: Vec::new(),
                    writes: false,
                }
            }
        }
        None => OmpWrite {
            targets: Vec::new(),
            writes: true,
        },
    }
}

fn extract_hashline_and_patch_targets(args: &serde_json::Value, targets: &mut Vec<Utf8PathBuf>) {
    if let Some(input_str) = args.get("input").and_then(|v| v.as_str()) {
        for line in input_str.lines() {
            let line = line.trim_end();
            if line.starts_with('[') && line.ends_with(']') {
                let inside = &line[1..line.len() - 1];
                if let Some((path_part, tag)) = inside.rsplit_once('#')
                    && tag.len() == 4
                    && tag.chars().all(|c| c.is_ascii_hexdigit())
                {
                    targets.push(Utf8PathBuf::from(path_part.trim()));
                }
            } else if let Some(dest) = line.strip_prefix("MV ") {
                let dest = dest.trim();
                let unquoted = if dest.starts_with('"') && dest.ends_with('"') && dest.len() >= 2 {
                    &dest[1..dest.len() - 1]
                } else {
                    dest
                };
                if !unquoted.is_empty() {
                    targets.push(Utf8PathBuf::from(unquoted));
                }
            } else if let Some(path) = line
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
}

pub(crate) fn extract(tool: &str, args: &serde_json::Value) -> OmpWrite {
    let raw_path = args
        .get("filePath")
        .or_else(|| args.get("file_path"))
        .or_else(|| args.get("path"))
        .and_then(|v| v.as_str());

    let is_direct_ast_edit = tool == "ast_edit";
    let is_direct_lsp = tool == "lsp";
    let is_xd_ast_edit = raw_path == Some("xd://ast_edit");
    let is_xd_lsp = raw_path == Some("xd://lsp");

    if is_direct_ast_edit || is_xd_ast_edit {
        return extract_ast_edit(is_direct_ast_edit, args);
    }

    if is_direct_lsp || is_xd_lsp {
        return extract_lsp(is_direct_lsp, args);
    }

    // Default target extraction (hashline / MV / apply-patch / single path)
    let mut targets = Vec::new();
    if let Some(p) = raw_path {
        targets.push(Utf8PathBuf::from(p));
    }

    extract_hashline_and_patch_targets(args, &mut targets);

    let writes = is_structured_write(tool);
    OmpWrite { targets, writes }
}

#[cfg(test)]
#[path = "../../../tests/unit/providers/omp/targets.rs"]
mod tests;
