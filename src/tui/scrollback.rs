//! The plain-text scrollback decoder: PTY bytes in, a bounded `Vec<String>`
//! of lines out, with every escape sequence stripped along the way.
//!
//! This is not the emulator swap point — `screen.rs` is (ARCHITECTURE.md,
//! `tui` module map), and owns the live viewport's colour and cursor state
//! via `vt100`. Scrollback is a second, cheaper consumer of the same PTY
//! bytes: a "last N lines" plain-text approximation with no styling to
//! carry, kept separate so a scrollback change can never touch the emulator
//! seam and a `vt100` swap can never have to think about scrollback.

/// How many lines of plain scrollback each shell keeps for scroll mode,
/// beyond the emulator's live viewport. Bounds the memory a long-running
/// build or test run can accumulate.
pub(crate) const MAX_BUFFER_LINES: usize = 5000;

/// Where a plain-text decode left off. Escape sequences arrive split across
/// PTY chunks as often as not, so the parser has to be resumable — a state
/// machine, not a regex over one chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decode {
    /// Ordinary text.
    Text,
    /// Just saw `ESC`; the next byte says which kind of sequence this is.
    Escape,
    /// Inside a CSI (`ESC [ … final`) — the colours, the cursor moves, all
    /// of it. Ends at the first byte in `0x40..=0x7e`.
    Csi,
    /// Inside an OSC (`ESC ] … BEL` or `… ESC \`) — window titles, and the
    /// shell integration marks a modern prompt emits.
    Osc,
    /// Saw `ESC` inside an OSC: a `\` ends the sequence, anything else is
    /// still OSC payload.
    OscEscape,
    /// A two-byte escape whose second byte carries no meaning here (charset
    /// selection, `ESC ( B` and friends).
    Skip,
    /// Just saw `\r`, and what it means depends on the next byte: `\r\n` is
    /// the line ending a PTY sends, while a lone `\r` rewrites the line.
    CarriageReturn,
}

/// Append `bytes` to a shell's scrollback as plain text: escape sequences
/// stripped, `\r` treated the way a terminal treats it (the line starts
/// over), and a trailing partial line joined onto the previous one.
///
/// The emulator renders the live viewport, so this is only what scroll mode
/// reads — and there it has to be *text*. Keeping the raw bytes put the
/// shell's own escape sequences on screen as `[32m` and `[A`, which is
/// exactly the noise scrolling back is meant to look past.
pub(crate) fn append_to_buffer(buffer: &mut Vec<String>, state: &mut Decode, bytes: &[u8]) {
    for ch in String::from_utf8_lossy(bytes).chars() {
        match *state {
            Decode::Text => feed_text(buffer, state, ch),
            // A lone `\r` rewrites the line it is on — progress bars and
            // spinners are one line, redrawn — but `\r\n` is just the line
            // ending, and clearing on it would empty every line there is.
            Decode::CarriageReturn => {
                *state = Decode::Text;
                if ch == '\n' {
                    buffer.push(String::new());
                } else {
                    if let Some(last) = buffer.last_mut() {
                        last.clear();
                    }
                    feed_text(buffer, state, ch);
                }
            }
            Decode::Escape => {
                *state = match ch {
                    '[' => Decode::Csi,
                    ']' => Decode::Osc,
                    '(' | ')' | '#' | '%' => Decode::Skip,
                    _ => Decode::Text,
                };
            }
            // A CSI ends at its final byte; everything before it is
            // parameters and intermediates.
            Decode::Csi => {
                if matches!(ch, '\x40'..='\x7e') {
                    *state = Decode::Text;
                }
            }
            Decode::Osc => match ch {
                '\x07' => *state = Decode::Text,
                '\x1b' => *state = Decode::OscEscape,
                _ => {}
            },
            Decode::OscEscape => {
                *state = if ch == '\\' {
                    Decode::Text
                } else {
                    Decode::Osc
                };
            }
            Decode::Skip => *state = Decode::Text,
        }
    }
    if buffer.len() > MAX_BUFFER_LINES {
        let excess = buffer.len() - MAX_BUFFER_LINES;
        buffer.drain(..excess);
    }
}

/// One character of ordinary text, with the control characters that mean
/// something to a line of text handled and the rest dropped.
fn feed_text(buffer: &mut Vec<String>, state: &mut Decode, ch: char) {
    match ch {
        '\x1b' => *state = Decode::Escape,
        '\n' => buffer.push(String::new()),
        '\r' => *state = Decode::CarriageReturn,
        '\t' => push_char(buffer, '\t'),
        // Every other control byte is an instruction to a terminal, not
        // text: BEL, backspace, the lot.
        _ if ch.is_control() => {}
        _ => push_char(buffer, ch),
    }
}

/// Append one character to the line the buffer is currently on, starting a
/// first line if there is none yet.
fn push_char(buffer: &mut Vec<String>, ch: char) {
    match buffer.last_mut() {
        Some(last) => last.push(ch),
        None => buffer.push(ch.to_string()),
    }
}
