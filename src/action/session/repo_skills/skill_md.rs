//! Reading and rewriting a skill's `SKILL.md` frontmatter. Only `name`
//! matters here: a field ivar cannot parse is ignored, never the file.

use camino::Utf8Path;

use crate::infra::{frontmatter, fs};

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct NameOnly {
    name: Option<String>,
}

/// Extract the first top-level `name:` value from frontmatter lines, stripping
/// surrounding matching quotes (`'` or `"`).
fn extract_name_line(lines: impl Iterator<Item = impl AsRef<str>>) -> Option<String> {
    for line in lines {
        let line = line.as_ref();
        if let Some(rest) = line.strip_prefix("name:") {
            let val = rest.trim();
            let stripped = if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
                || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
            {
                val[1..val.len() - 1].trim()
            } else {
                val
            };
            if !stripped.is_empty() {
                return Some(stripped.to_owned());
            }
        }
    }
    None
}

/// The identity a harness gives `dir/SKILL.md`: its frontmatter `name`, or
/// the directory name when the frontmatter has none or cannot be parsed.
/// `Err` is returned only when the file cannot be read from disk (I/O error).
pub(super) fn skill_name(dir: &Utf8Path) -> Result<Option<String>, String> {
    let path = dir.join("SKILL.md");
    let Some(text) = fs::read_text(&path).map_err(|e| e.to_string())? else {
        return Ok(None);
    };

    let fallback_dir = || dir.file_name().unwrap_or_default().to_owned();

    match frontmatter::split(&text) {
        Ok(split) => {
            if let Some(block) = split.frontmatter {
                if let Ok(meta) = serde_saphyr::from_str::<NameOnly>(block)
                    && let Some(n) = meta.name.filter(|n| !n.trim().is_empty())
                {
                    return Ok(Some(n));
                }
                let name = extract_name_line(block.lines()).unwrap_or_else(fallback_dir);
                Ok(Some(name))
            } else {
                Ok(Some(fallback_dir()))
            }
        }
        Err(_) => {
            // Unterminated fence: scan lines after the opening fence up to the
            // first blank or `#` line / end of file.
            let mut lines = text.lines();
            if let Some(first) = lines.next()
                && first.trim() == "---"
            {
                let block_lines = lines.take_while(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with('#')
                });
                let name = extract_name_line(block_lines).unwrap_or_else(fallback_dir);
                return Ok(Some(name));
            }
            Ok(Some(fallback_dir()))
        }
    }
}

/// `source` with its frontmatter `name:` line set to `new_name` — a textual
/// edit, so every other key survives byte-for-byte.
pub(crate) fn rename_frontmatter(source: &str, new_name: &str) -> String {
    let name_line = format!("name: {new_name}");
    let Ok(frontmatter::Split {
        frontmatter: Some(block),
        body,
    }) = frontmatter::split(source)
    else {
        return format!("---\n{name_line}\n---\n{source}");
    };
    let mut replaced = false;
    let lines: Vec<String> = block
        .lines()
        .map(|line| {
            if !replaced && line.starts_with("name:") {
                replaced = true;
                name_line.clone()
            } else {
                line.to_owned()
            }
        })
        .collect();
    let mut fm = lines.join("\n");
    if !replaced {
        fm = if fm.is_empty() {
            name_line
        } else {
            format!("{name_line}\n{fm}")
        };
    }
    format!("---\n{fm}\n---\n{body}")
}
