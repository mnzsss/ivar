//! Pure reducer: `(mode, key) -> (mode, action)`.
//!
//! The one place a keystroke becomes intent, and the one place that must
//! never touch the outside world — no reads, no writes, no clock. Two
//! renders of the same `(mode, key)` produce the same `(mode, action)`, so
//! this module is exhaustively testable without any harness at all.
//!
//! # Modes
//!
//! - [`Mode::Focus`] — the default. The selected shell's PTY owns every
//!   keystroke; raw bytes flow straight through. Only the prefix key is
//!   intercepted and switches to [`Mode::Nav`]; which chord that is comes
//!   from the host loop, so it can be configured.
//! - [`Mode::Nav`] — navigate the sidebar: `j`/`k` (or the arrows) move the
//!   selection, `Enter` focuses the selected repo's shell, `[` opens
//!   [`Mode::Scroll`], `q` (or `Ctrl+C`) quits.
//! - [`Mode::Scroll`] — read the focused shell's scrollback: `PgUp`/`PgDn`
//!   scroll, `q` or `Esc` returns to [`Mode::Focus`].
//!
//! # The wheel, and a shell that has exited
//!
//! Two inputs are not `(mode, key)`, and each gets its own entry point so
//! the mapping still lives in exactly one place:
//!
//! - [`reduce_wheel`] — a mouse wheel notch. It changes no mode: scrolling
//!   is something the user does *to* the panel, whatever the keyboard is
//!   doing. Without it the terminal turns the wheel into arrow keys (that is
//!   what it sends while the alternate screen is up) and scrolling would
//!   type `\x1b[A` into the shell.
//! - [`reduce_exited`] — the focused shell's process is gone. Its keys have
//!   nowhere to go, so Focus rebinds them to the two things left to do:
//!   restart the shell, or leave.
//!
//! # Actions
//!
//! [`Action`] is intent, not I/O. The driver decides *how* to perform it
//! (which is exactly why the reducer stays pure).

/// What the user is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A shell has focus; keys flow to it.
    Focus,
    /// Sidebar navigation over the promoted repos.
    Nav,
    /// Scrolling the focused shell's scrollback.
    Scroll,
}

/// A keystroke the reducer understands. Everything else — all raw bytes,
/// notably when in [`Mode::Focus`] — flows straight to the shell uninterpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Home,
    End,
    Insert,
    /// A character, as typed — including non-ASCII ones, which is why this
    /// carries a `char` and the encoding happens at the last moment.
    Char(char),
    /// `Ctrl` + this character.
    Ctrl(char),
    /// `Alt` + this character.
    Alt(char),
    /// The prefix key: the one key Focus forwards nowhere. Which physical
    /// chord produces it is the host loop's business, not the reducer's.
    Prefix,
    PgUp,
    PgDn,
}

/// What a keystroke means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Move the selection up one row (or first row).
    Up,
    /// Move the selection down one row (or last row).
    Down,
    /// Give the selected shell's PTY focus (spawning it if needed).
    FocusShell,
    /// Enter sidebar navigation from Focus.
    EnterNav,
    /// Enter scrollback mode from Nav.
    EnterScroll,
    /// Leave scrollback mode back to Focus.
    ExitScroll,
    /// Scroll the focused shell's scrollback up one page.
    ScrollUp,
    /// Scroll the focused shell's scrollback down one page.
    ScrollDown,
    /// Scroll the focused shell's scrollback by a number of lines — what a
    /// wheel notch means, as opposed to a page.
    ScrollLines(Direction, usize),
    /// Spawn a fresh process for a shell whose own has exited.
    Restart,
    /// Quit the TUI.
    Quit,
    /// Bytes to write into the focused shell's PTY, verbatim.
    WriteBytes(Vec<u8>),
    /// Nothing happened; ignore this key.
    None,
}

/// Reduce one keystroke: `(mode, key) -> (mode, action)`.
///
/// Navigation *movement* is not the reducer's job — the driver owns the
/// selection, applies `Action::Up`/`Down` through [`move_selection`], and
/// feeds the resulting `(mode, action)` back. The reducer only decides
/// *what a key means*, which is what keeps it pure.
#[must_use]
pub fn reduce(mode: Mode, key: Key) -> (Mode, Action) {
    match mode {
        Mode::Focus => match key {
            Key::Prefix => (Mode::Nav, Action::EnterNav),
            // Everything else reaches the shell verbatim. Ctrl+C included:
            // it is a real signal the shell owns, and quitting is Nav's `q`.
            Key::Ctrl(c) => (mode, Action::WriteBytes(control_bytes(c))),
            // Alt is the ESC prefix — how a terminal has always sent it, and
            // what makes alt+b / alt+f move by word in a shell.
            Key::Alt(c) => {
                let mut bytes = vec![0x1b];
                bytes.extend_from_slice(encode(c).as_slice());
                (mode, Action::WriteBytes(bytes))
            }
            // Encoded as UTF-8, not cast to a byte: `c as u8` silently
            // mangles every accented character.
            Key::Char(c) => (mode, Action::WriteBytes(encode(c))),
            // A PTY in canonical mode expects DEL for backspace; BS (0x08)
            // is a different key that most shells do not treat as erase.
            Key::Backspace => (mode, Action::WriteBytes(vec![0x7f])),
            Key::Tab => (mode, Action::WriteBytes(vec![b'\t'])),
            Key::BackTab => (mode, Action::WriteBytes(b"\x1b[Z".to_vec())),
            Key::Enter => (mode, Action::WriteBytes(vec![b'\n'])),
            Key::Esc => (mode, Action::WriteBytes(b"\x1b".to_vec())),
            Key::Up => (mode, Action::WriteBytes(b"\x1b[A".to_vec())),
            Key::Down => (mode, Action::WriteBytes(b"\x1b[B".to_vec())),
            Key::Left => (mode, Action::WriteBytes(b"\x1b[D".to_vec())),
            Key::Right => (mode, Action::WriteBytes(b"\x1b[C".to_vec())),
            Key::Home => (mode, Action::WriteBytes(b"\x1b[H".to_vec())),
            Key::End => (mode, Action::WriteBytes(b"\x1b[F".to_vec())),
            Key::Insert => (mode, Action::WriteBytes(b"\x1b[2~".to_vec())),
            Key::Delete => (mode, Action::WriteBytes(b"\x1b[3~".to_vec())),
            Key::PgUp => (mode, Action::WriteBytes(b"\x1b[5~".to_vec())),
            Key::PgDn => (mode, Action::WriteBytes(b"\x1b[6~".to_vec())),
        },
        Mode::Nav => match key {
            Key::Up | Key::Char('k') => (mode, Action::Up),
            Key::Down | Key::Char('j') => (mode, Action::Down),
            Key::Enter | Key::Esc => (Mode::Focus, Action::FocusShell),
            // The second half of the `<prefix> [` sequence.
            Key::Char('[') => (Mode::Scroll, Action::EnterScroll),
            Key::Char('q') | Key::Ctrl('c') => (mode, Action::Quit),
            // Anything else is a swallowed prefix key.
            _ => (mode, Action::None),
        },
        Mode::Scroll => match key {
            Key::PgUp => (mode, Action::ScrollUp),
            Key::PgDn => (mode, Action::ScrollDown),
            Key::Char('q') | Key::Esc => (Mode::Focus, Action::ExitScroll),
            Key::Ctrl('c') => (mode, Action::Quit),
            _ => (mode, Action::None),
        },
    }
}

/// How many lines one wheel notch scrolls. Three is what a terminal
/// conventionally scrolls per notch — enough to feel like movement, small
/// enough that a flick does not overshoot the whole buffer.
const WHEEL_LINES: usize = 3;

/// Reduce one mouse wheel notch: `(mode, direction) -> (mode, action)`.
///
/// The mode never changes. A wheel notch is not a mode switch — the user can
/// glance up the scrollback and keep typing, and the driver returns the panel
/// to the live bottom as soon as they do.
#[must_use]
pub fn reduce_wheel(mode: Mode, direction: Direction) -> (Mode, Action) {
    (mode, Action::ScrollLines(direction, WHEEL_LINES))
}

/// Reduce one keystroke while the focused shell's process is gone.
///
/// [`Mode::Focus`] forwards keys to a PTY; when there is no longer one, every
/// key is swallowed — including the ones a user reaches for to get out. So a
/// dead shell rebinds Focus to the only two things left: `enter` (or `r`)
/// restarts it, `q` / `ctrl+c` / `ctrl+d` quits the view. The prefix still
/// opens nav, and the other modes are unchanged — they never talked to the
/// PTY in the first place.
#[must_use]
pub fn reduce_exited(mode: Mode, key: Key) -> (Mode, Action) {
    let Mode::Focus = mode else {
        return reduce(mode, key);
    };
    match key {
        Key::Prefix => (Mode::Nav, Action::EnterNav),
        Key::Enter | Key::Char('r') => (mode, Action::Restart),
        Key::Char('q') | Key::Ctrl('c') | Key::Ctrl('d') => (mode, Action::Quit),
        _ => (mode, Action::None),
    }
}

/// A character as the bytes a terminal sends for it.
fn encode(c: char) -> Vec<u8> {
    let mut buf = [0u8; 4];
    c.encode_utf8(&mut buf).as_bytes().to_vec()
}

/// The control byte for `Ctrl` + `c`: the ASCII rule of masking off all but
/// the low five bits, so `ctrl+c` is `0x03` and `ctrl+d` is `0x04`.
///
/// A non-ASCII character has no control byte; it is sent as itself rather
/// than dropped.
fn control_bytes(c: char) -> Vec<u8> {
    if c.is_ascii() {
        vec![(c.to_ascii_uppercase() as u8) & 0x1f]
    } else {
        encode(c)
    }
}

/// The selection after an `Up`/`Down` action, clamped to `row_count`.
/// Pure so tests can assert navigation without any state machinery.
#[must_use]
pub fn move_selection(selection: usize, direction: Direction, row_count: usize) -> usize {
    if row_count == 0 {
        return 0;
    }
    match direction {
        Direction::Up => selection.saturating_sub(1),
        Direction::Down => (selection + 1).min(row_count - 1),
    }
}

/// Which way the selection moves. Split out so [`move_selection`] stays a
/// plain function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

#[cfg(test)]
#[path = "../../tests/unit/tui/key_router.rs"]
mod tests;
