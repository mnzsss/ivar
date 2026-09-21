const SEARCH_PREFIXES: [&str; 5] = ["rtk proxy rg", "rtk rg", "rtk grep", "rg", "grep"];

pub(super) fn bash_search_command(command: &str) -> Option<String> {
    first_commands(command)
        .into_iter()
        .map(str::trim)
        .find(|segment| is_search(segment))
        .map(str::to_owned)
}

fn is_search(segment: &str) -> bool {
    SEARCH_PREFIXES.iter().any(|prefix| {
        segment.strip_prefix(prefix).is_some_and(|rest| {
            (rest.is_empty() || rest.starts_with(' ')) && !targets_only_non_code(rest)
        })
    })
}

const NON_CODE_DIRS: [&str; 5] = ["dist", "build", "node_modules", "target", ".next"];
const NON_CODE_EXTENSIONS: [&str; 3] = ["log", "txt", "out"];

// ponytail: whitespace tokenising; quoted paths with spaces are misread, upgrade to a word splitter if that shows up in misses
fn targets_only_non_code(args: &str) -> bool {
    let mut positional = args.split_whitespace().filter(|t| !t.starts_with('-'));
    let _pattern = positional.next();
    let targets: Vec<&str> = positional.collect();
    !targets.is_empty() && targets.iter().all(|t| is_non_code(t))
}

fn is_non_code(target: &str) -> bool {
    let path = camino::Utf8Path::new(target.trim_matches(['\'', '"']));
    path.starts_with("/tmp")
        || path
            .extension()
            .is_some_and(|ext| NON_CODE_EXTENSIONS.contains(&ext))
        || path
            .components()
            .any(|c| NON_CODE_DIRS.contains(&c.as_str()))
}

/// The first command of every pipeline in `command`, split on operators
/// outside quotes. Stops at the end of a line that opens a heredoc.
fn first_commands(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let (mut out, mut start, mut pipe_cut) = (Vec::new(), 0, None::<usize>);
    let (mut quote, mut escaped, mut heredoc) = (None::<u8>, false, false);
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(b'"') | None, b'\\') => escaped = true,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(b),
            (None, b'<') if bytes.get(i + 1) == Some(&b'<') => {
                heredoc = true;
                i += 1;
            }
            (None, b'|') if bytes.get(i + 1) == Some(&b'|') => {
                out.push(&command[start..pipe_cut.unwrap_or(i)]);
                i += 1;
                start = i + 1;
                pipe_cut = None;
            }
            (None, b'|') => {
                pipe_cut.get_or_insert(i);
            }
            (None, b'&' | b';' | b'\n') => {
                out.push(&command[start..pipe_cut.unwrap_or(i)]);
                if b == b'\n' && heredoc {
                    return out;
                }
                if b == b'&' && bytes.get(i + 1) == Some(&b'&') {
                    i += 1;
                }
                start = i + 1;
                pipe_cut = None;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&command[start..pipe_cut.unwrap_or(bytes.len())]);
    out
}
