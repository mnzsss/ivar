//! Pure reducer: `(mode, key) -> (mode, action)`.
//!
//! The one place a keystroke becomes intent, and the one place that must
//! never touch the outside world — no reads, no writes, no clock. Two
//! renders of the same `(mode, key)` produce the same `(mode, action)`, so
//! this module is exhaustively testable without any harness at all.
//!
//! # Modes
//!
//! - [`Mode::Navigate`] — the master-detail view: arrows/j/k move the
//!   selection, Enter switches focus to the agent panel, `q` quits.
//! - [`Mode::Agent`] — the embedded PTY panel owns every keystroke; only
//!   `Esc` returns to [`Mode::Navigate`]. Raw bytes flow straight through.
//!
//! # Actions
//!
//! [`Action`] is intent, not I/O. The driver decides *how* to perform it
//! (which is exactly why the reducer stays pure).

/// What the user is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Master-detail navigation over the hall snapshot.
    Navigate,
    /// The agent's PTY has focus; keys flow to it.
    Agent,
}

/// A keystroke the reducer understands. Everything else — all raw bytes,
/// notably when in [`Mode::Agent`] — flows straight to the PTY uninterpreted.
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
}

/// What a keystroke means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Move the selection up one row (or first row).
    Up,
    /// Move the selection down one row (or last row).
    Down,
    /// Give the agent's PTY panel focus.
    FocusAgent,
    /// Leave the agent panel back to navigation.
    FocusNavigate,
    /// Quit the TUI.
    Quit,
    /// Bytes to write into the agent's PTY, verbatim.
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
        Mode::Navigate => match key {
            Key::Up => (mode, Action::Up),
            Key::Down => (mode, Action::Down),
            Key::Enter | Key::Esc => (Mode::Agent, Action::FocusAgent),
            Key::CtrlC => (Mode::Navigate, Action::Quit),
            Key::Char('q') => (mode, Action::Quit),
            // In navigation, nothing else reaches the agent.
            _ => (mode, Action::None),
        },
        Mode::Agent => match key {
            Key::Esc => (Mode::Navigate, Action::FocusNavigate),
            Key::CtrlC => (Mode::Navigate, Action::Quit),
            Key::Char(c) => (mode, Action::WriteBytes(vec![c as u8])),
            Key::Enter => (mode, Action::WriteBytes(vec![b'\n'])),
            Key::Up => (mode, Action::WriteBytes(b"\x1b[A".to_vec())),
            Key::Down => (mode, Action::WriteBytes(b"\x1b[B".to_vec())),
            Key::Left => (mode, Action::WriteBytes(b"\x1b[D".to_vec())),
            Key::Right => (mode, Action::WriteBytes(b"\x1b[C".to_vec())),
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

    #[test]
    fn navigation_arrow_keys_move_and_enter_focuses_the_agent() {
        assert_eq!(
            reduce(Mode::Navigate, Key::Up),
            (Mode::Navigate, Action::Up)
        );
        assert_eq!(
            reduce(Mode::Navigate, Key::Down),
            (Mode::Navigate, Action::Down)
        );
        assert_eq!(
            reduce(Mode::Navigate, Key::Enter),
            (Mode::Agent, Action::FocusAgent)
        );
    }

    #[test]
    fn q_and_ctrl_c_quit_from_navigation() {
        assert_eq!(
            reduce(Mode::Navigate, Key::Char('q')),
            (Mode::Navigate, Action::Quit)
        );
        assert_eq!(
            reduce(Mode::Navigate, Key::CtrlC),
            (Mode::Navigate, Action::Quit)
        );
    }

    #[test]
    fn esc_returns_from_agent_to_navigation() {
        assert_eq!(
            reduce(Mode::Agent, Key::Esc),
            (Mode::Navigate, Action::FocusNavigate)
        );
    }

    #[test]
    fn agent_keys_write_bytes_verbatim() {
        assert_eq!(
            reduce(Mode::Agent, Key::Char('x')),
            (Mode::Agent, Action::WriteBytes(vec![b'x']))
        );
        assert_eq!(
            reduce(Mode::Agent, Key::Enter),
            (Mode::Agent, Action::WriteBytes(vec![b'\n']))
        );
    }

    #[test]
    fn agent_arrow_keys_become_escape_sequences() {
        assert_eq!(
            reduce(Mode::Agent, Key::Up),
            (Mode::Agent, Action::WriteBytes(b"\x1b[A".to_vec()))
        );
    }

    #[test]
    fn selection_moves_are_clamped() {
        assert_eq!(move_selection(0, Direction::Up, 5), 0);
        assert_eq!(move_selection(4, Direction::Down, 5), 4);
        assert_eq!(move_selection(2, Direction::Down, 5), 3);
        assert_eq!(move_selection(2, Direction::Up, 5), 1);
        assert_eq!(move_selection(0, Direction::Up, 0), 0);
    }
}
