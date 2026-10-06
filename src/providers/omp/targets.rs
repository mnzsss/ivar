use camino::Utf8PathBuf;

use crate::domain::guard::is_structured_write;

pub(crate) struct OmpWrite {
    pub(crate) targets: Vec<Utf8PathBuf>,
    pub(crate) writes: bool,
}

fn glob_part_has_dotdot(glob_part: &str) -> bool {
    glob_part.split(['/', '{', '}', ',']).any(|seg| seg == "..")
}

fn glob_literal_prefix(pattern: &str) -> Option<Utf8PathBuf> {
    let metachars = ['*', '?', '[', '{'];
    if let Some(idx) = pattern.find(|c| metachars.contains(&c)) {
        let (prefix, glob_part) = pattern.split_at(idx);
        if glob_part_has_dotdot(glob_part) {
            return None;
        }
        if let Some((dir, _)) = prefix.rsplit_once('/') {
            if dir.is_empty() {
                if pattern.starts_with('/') {
                    Some(Utf8PathBuf::from("/"))
                } else {
                    Some(Utf8PathBuf::from("."))
                }
            } else {
                Some(Utf8PathBuf::from(dir))
            }
        } else if pattern.starts_with('/') {
            Some(Utf8PathBuf::from("/"))
        } else {
            Some(Utf8PathBuf::from("."))
        }
    } else {
        Some(Utf8PathBuf::from(pattern))
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
                        match glob_literal_prefix(s) {
                            Some(target) => targets.push(target),
                            None => {
                                return OmpWrite {
                                    targets: Vec::new(),
                                    writes: true,
                                };
                            }
                        }
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
            let is = |name: &str| action.eq_ignore_ascii_case(name);
            // A non-boolean `apply` is not a value ivar can prove read-only.
            let apply = match obj.get("apply") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::Bool(b)) => Some(*b),
                Some(_) => Some(true),
            };

            let is_mutating = if is("rename") {
                apply.unwrap_or(true)
            } else if is("rename_file") || is("request") {
                true
            } else if is("code_actions") {
                apply.unwrap_or(false)
            } else {
                false
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

/// The path of a hashline file header `[PATH]` or `[PATH#TAG]`. omp accepts
/// a header without a tag, so the tag is optional here too.
fn header_path(line: &str) -> Option<&str> {
    let inside = line.strip_prefix('[')?.strip_suffix(']')?;
    let path = match inside.rsplit_once('#') {
        Some((path, tag)) if tag.len() == 4 && tag.bytes().all(|b| b.is_ascii_hexdigit()) => path,
        _ => inside,
    };
    (!path.is_empty()).then_some(path)
}

/// The destination of a hashline `MV DEST` row, lexed as omp does: leading
/// whitespace, any whitespace after the keyword, and `"` or `'` quotes.
fn move_dest(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("MV")?;
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with(':')) {
        return None;
    }
    let dest = rest.trim();
    let unquoted = ['"', '\'']
        .into_iter()
        .find_map(|q| dest.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(dest);
    (!unquoted.is_empty()).then_some(unquoted)
}

fn extract_hashline_and_patch_targets(args: &serde_json::Value, targets: &mut Vec<Utf8PathBuf>) {
    if let Some(input_str) = args.get("input").and_then(|v| v.as_str()) {
        for line in input_str.lines() {
            let line = line.trim_end();
            if let Some(path) = header_path(line) {
                targets.push(Utf8PathBuf::from(path));
                let trimmed = path.trim();
                if trimmed != path && !trimmed.is_empty() {
                    targets.push(Utf8PathBuf::from(trimmed));
                }
            } else if let Some(dest) = move_dest(line) {
                targets.push(Utf8PathBuf::from(dest));
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
    // omp trims the path and matches the `xd://` scheme case-insensitively.
    let device = raw_path.and_then(|p| {
        let p = p.trim();
        p.get(..5)
            .filter(|scheme| scheme.eq_ignore_ascii_case("xd://"))
            .and_then(|_| p.get(5..))
    });
    let is_xd_ast_edit = device.is_some_and(|d| d.eq_ignore_ascii_case("ast_edit"));
    let is_xd_lsp = device.is_some_and(|d| d.eq_ignore_ascii_case("lsp"));

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
