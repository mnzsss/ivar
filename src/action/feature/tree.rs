//! Tree-glyph prefix generation for pre-ordered hierarchical listings.

/// Tree-glyph prefixes for a pre-ordered list of depths (0 = a root).
///
/// Depth 0 → `""`.
/// Depth d ≥ 1 → for each ancestor level 1..d: `"│   "` when that
/// ancestor still has a later sibling, else `"    "`; then `"├── "` when the entry
/// has a later sibling at depth d (before any shallower entry), else `"└── "`.
pub(crate) fn tree_prefixes(depths: &[usize]) -> Vec<String> {
    let mut prefixes = Vec::with_capacity(depths.len());

    for (i, &d) in depths.iter().enumerate() {
        if d == 0 {
            prefixes.push(String::new());
            continue;
        }

        let mut prefix = String::new();
        let following = depths.get((i + 1)..).unwrap_or_default();

        // For each ancestor level 1..d (1-indexed depth ancestor):
        // An ancestor at level `k` has a later sibling if there exists an entry after i
        // with depth == k before any entry with depth < k.
        for k in 1..d {
            let mut has_later_sibling = false;
            for &next_d in following {
                if next_d < k {
                    break;
                }
                if next_d == k {
                    has_later_sibling = true;
                    break;
                }
            }
            if has_later_sibling {
                prefix.push_str("│   ");
            } else {
                prefix.push_str("    ");
            }
        }

        // Connector at depth d:
        // Entry at depth d has a later sibling if there is an entry after i with depth == d
        // before any entry with depth < d.
        let mut has_sibling_at_d = false;
        for &next_d in following {
            if next_d < d {
                break;
            }
            if next_d == d {
                has_sibling_at_d = true;
                break;
            }
        }

        if has_sibling_at_d {
            prefix.push_str("├── ");
        } else {
            prefix.push_str("└── ");
        }

        prefixes.push(prefix);
    }

    prefixes
}

#[cfg(test)]
#[path = "../../../tests/unit/action/feature/tree.rs"]
mod tests;
