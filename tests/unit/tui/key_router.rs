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
