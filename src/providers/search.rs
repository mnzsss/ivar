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

const VALUE_FLAGS: [&str; 14] = [
    "-g",
    "-t",
    "-T",
    "-e",
    "-f",
    "-A",
    "-B",
    "-C",
    "-m",
    "--glob",
    "--type",
    "--type-not",
    "--max-count",
    "--regexp",
];

// ponytail: whitespace tokenising; quoted paths with spaces are misread, upgrade to a word splitter if that shows up in misses
fn targets_only_non_code(args: &str) -> bool {
    let mut tokens = args.split_whitespace();
    let mut positional = Vec::new();
    let mut pattern_from_flag = false;
    while let Some(token) = tokens.next() {
        if VALUE_FLAGS.contains(&token) {
            pattern_from_flag |= matches!(token, "-e" | "-f" | "--regexp");
            tokens.next();
        } else if !token.starts_with('-') {
            positional.push(token);
        }
    }
    let targets = &positional[usize::from(!pattern_from_flag).min(positional.len())..];
    !targets.is_empty() && targets.iter().all(|t| is_non_code(t))
}

fn is_non_code(target: &str) -> bool {
    let path = camino::Utf8Path::new(target.trim_matches(['\'', '"']));
    let relative = path.strip_prefix("./").unwrap_or(path);
    path.starts_with("/tmp")
        || path
            .extension()
            .is_some_and(|ext| NON_CODE_EXTENSIONS.contains(&ext))
        || relative
            .components()
            .next()
            .is_some_and(|c| NON_CODE_DIRS.contains(&c.as_str()))
}

fn opens_heredoc(after_operator: &str) -> bool {
    after_operator
        .trim_start_matches('-')
        .trim_start()
        .starts_with(|c: char| c.is_ascii_alphabetic() || matches!(c, '_' | '\'' | '"' | '\\'))
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
            (None, b'<') if command[i..].starts_with("<<<") => i += 2,
            (None, b'<') if command[i..].starts_with("<<") => {
                heredoc |= opens_heredoc(&command[i + 2..]);
                i += 1;
            }
            (None, b'#') if i == 0 || bytes[i - 1].is_ascii_whitespace() => {
                out.push(&command[start..pipe_cut.unwrap_or(i)]);
                let Some(newline) = command[i..].find('\n') else {
                    return out;
                };
                i += newline;
                start = i;
                pipe_cut = None;
                continue;
            }
            (None, b'&') if i > 0 && bytes[i - 1] == b'>' || bytes.get(i + 1) == Some(&b'>') => {}
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
