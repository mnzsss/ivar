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
//!   keystroke; raw bytes flow straight through. Only the prefix key
//!   (`Ctrl+B`) is intercepted and switches to [`Mode::Nav`].
//! - [`Mode::Nav`] — navigate the sidebar: `j`/`k` (or the arrows) move the
//!   selection, `Enter` focuses the selected repo's shell, `[` opens
//!   [`Mode::Scroll`], `q` (or `Ctrl+C`) quits.
//! - [`Mode::Scroll`] — read the focused shell's scrollback: `PgUp`/`PgDn`
//!   scroll, `q` or `Esc` returns to [`Mode::Focus`].
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
    Char(char),
    CtrlC,
    /// The prefix key (`Ctrl+B`): the one key Focus forwards nowhere.
    CtrlB,
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
            Key::CtrlB => (Mode::Nav, Action::EnterNav),
            // Everything else reaches the shell. Ctrl+C is a real signal the
            // shell owns; quitting is Nav's `q`.
            Key::CtrlC => (mode, Action::WriteBytes(vec![0x03])),
            Key::Char(c) => (mode, Action::WriteBytes(vec![c as u8])),
            Key::Enter => (mode, Action::WriteBytes(vec![b'\n'])),
            Key::Esc => (mode, Action::WriteBytes(b"\x1b".to_vec())),
            Key::Up => (mode, Action::WriteBytes(b"\x1b[A".to_vec())),
            Key::Down => (mode, Action::WriteBytes(b"\x1b[B".to_vec())),
            Key::Left => (mode, Action::WriteBytes(b"\x1b[D".to_vec())),
            Key::Right => (mode, Action::WriteBytes(b"\x1b[C".to_vec())),
            Key::PgUp => (mode, Action::WriteBytes(b"\x1b[5~".to_vec())),
            Key::PgDn => (mode, Action::WriteBytes(b"\x1b[6~".to_vec())),
        },
        Mode::Nav => match key {
            Key::Up | Key::Char('k') => (mode, Action::Up),
            Key::Down | Key::Char('j') => (mode, Action::Down),
            Key::Enter | Key::Esc => (Mode::Focus, Action::FocusShell),
            // The second half of the `Ctrl+B [` sequence.
            Key::Char('[') => (Mode::Scroll, Action::EnterScroll),
            Key::Char('q') => (mode, Action::Quit),
            Key::CtrlC => (mode, Action::Quit),
            // Anything else is a swallowed prefix key.
            _ => (mode, Action::None),
        },
        Mode::Scroll => match key {
            Key::PgUp => (mode, Action::ScrollUp),
            Key::PgDn => (mode, Action::ScrollDown),
            Key::Char('q') | Key::Esc => (Mode::Focus, Action::ExitScroll),
            Key::CtrlC => (mode, Action::Quit),
            _ => (mode, Action::None),
        },
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    // -- Focus: keys reach the shell, Ctrl+B opens Nav ------------------------

    #[test]
    fn focus_forwards_characters_and_enter_as_bytes() {
        assert_eq!(
            reduce(Mode::Focus, Key::Char('x')),
            (Mode::Focus, Action::WriteBytes(vec![b'x']))
        );
        assert_eq!(
            reduce(Mode::Focus, Key::Enter),
            (Mode::Focus, Action::WriteBytes(vec![b'\n']))
        );
    }

    #[test]
    fn focus_forwards_arrows_and_page_keys_as_escape_sequences() {
        assert_eq!(
            reduce(Mode::Focus, Key::Up),
            (Mode::Focus, Action::WriteBytes(b"\x1b[A".to_vec()))
        );
        assert_eq!(
            reduce(Mode::Focus, Key::PgUp),
            (Mode::Focus, Action::WriteBytes(b"\x1b[5~".to_vec()))
        );
        assert_eq!(
            reduce(Mode::Focus, Key::PgDn),
            (Mode::Focus, Action::WriteBytes(b"\x1b[6~".to_vec()))
        );
    }

    #[test]
    fn focus_forwards_ctrl_c_to_the_shell() {
        assert_eq!(
            reduce(Mode::Focus, Key::CtrlC),
            (Mode::Focus, Action::WriteBytes(vec![0x03]))
        );
    }

    #[test]
    fn ctrl_b_is_the_prefix_into_nav() {
        assert_eq!(
            reduce(Mode::Focus, Key::CtrlB),
            (Mode::Nav, Action::EnterNav)
        );
    }

    // -- Nav: j/k move, enter focuses, q quits --------------------------------

    #[test]
    fn nav_arrows_and_j_k_move_the_selection() {
        assert_eq!(reduce(Mode::Nav, Key::Up), (Mode::Nav, Action::Up));
        assert_eq!(reduce(Mode::Nav, Key::Down), (Mode::Nav, Action::Down));
        assert_eq!(reduce(Mode::Nav, Key::Char('k')), (Mode::Nav, Action::Up));
        assert_eq!(reduce(Mode::Nav, Key::Char('j')), (Mode::Nav, Action::Down));
    }

    #[test]
    fn nav_enter_focuses_the_selected_shell() {
        assert_eq!(
            reduce(Mode::Nav, Key::Enter),
            (Mode::Focus, Action::FocusShell)
        );
        assert_eq!(
            reduce(Mode::Nav, Key::Esc),
            (Mode::Focus, Action::FocusShell)
        );
    }

    #[test]
    fn nav_open_bracket_enters_scroll_mode() {
        assert_eq!(
            reduce(Mode::Nav, Key::Char('[')),
            (Mode::Scroll, Action::EnterScroll)
        );
    }

    #[test]
    fn q_and_ctrl_c_quit_from_nav() {
        assert_eq!(reduce(Mode::Nav, Key::Char('q')), (Mode::Nav, Action::Quit));
        assert_eq!(reduce(Mode::Nav, Key::CtrlC), (Mode::Nav, Action::Quit));
    }

    #[test]
    fn nav_swallows_unknown_prefix_keys() {
        assert_eq!(reduce(Mode::Nav, Key::Char('x')), (Mode::Nav, Action::None));
    }

    // -- Scroll: PgUp/PgDn scroll, q/Esc returns to Focus ----------------------

    #[test]
    fn scroll_page_keys_scroll_and_q_or_esc_return_to_focus() {
        assert_eq!(
            reduce(Mode::Scroll, Key::PgUp),
            (Mode::Scroll, Action::ScrollUp)
        );
        assert_eq!(
            reduce(Mode::Scroll, Key::PgDn),
            (Mode::Scroll, Action::ScrollDown)
        );
        assert_eq!(
            reduce(Mode::Scroll, Key::Char('q')),
            (Mode::Focus, Action::ExitScroll)
        );
        assert_eq!(
            reduce(Mode::Scroll, Key::Esc),
            (Mode::Focus, Action::ExitScroll)
        );
    }

    #[test]
    fn scroll_ctrl_c_quits() {
        assert_eq!(
            reduce(Mode::Scroll, Key::CtrlC),
            (Mode::Scroll, Action::Quit)
        );
    }

    // -- selection -------------------------------------------------------------

    #[test]
    fn selection_moves_are_clamped() {
        assert_eq!(move_selection(0, Direction::Up, 5), 0);
        assert_eq!(move_selection(4, Direction::Down, 5), 4);
        assert_eq!(move_selection(2, Direction::Down, 5), 3);
        assert_eq!(move_selection(2, Direction::Up, 5), 1);
        assert_eq!(move_selection(0, Direction::Up, 0), 0);
    }
}
