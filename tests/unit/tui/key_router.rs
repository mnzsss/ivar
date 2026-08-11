#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

// -- Focus: keys reach the shell, the prefix opens Nav --------------------

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
        reduce(Mode::Focus, Key::Ctrl('c')),
        (Mode::Focus, Action::WriteBytes(vec![0x03]))
    );
}

#[test]
fn ctrl_b_is_the_prefix_into_nav() {
    assert_eq!(
        reduce(Mode::Focus, Key::Prefix),
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
    assert_eq!(reduce(Mode::Nav, Key::Ctrl('c')), (Mode::Nav, Action::Quit));
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
        reduce(Mode::Scroll, Key::Ctrl('c')),
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

/// The keys a shell needs that the router used to drop on the floor. Every
/// one of these is a key the user pressed and nothing happened.
#[test]
fn focus_forwards_the_editing_keys_a_shell_needs() {
    // Backspace is DEL, not BS: this is the key that "did not erase".
    assert_eq!(
        reduce(Mode::Focus, Key::Backspace),
        (Mode::Focus, Action::WriteBytes(vec![0x7f]))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::Tab),
        (Mode::Focus, Action::WriteBytes(vec![b'\t']))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::Delete),
        (Mode::Focus, Action::WriteBytes(b"\x1b[3~".to_vec()))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::Home),
        (Mode::Focus, Action::WriteBytes(b"\x1b[H".to_vec()))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::End),
        (Mode::Focus, Action::WriteBytes(b"\x1b[F".to_vec()))
    );
}

/// `Ctrl` + a letter is a control byte, not the letter. Sending the letter
/// means `ctrl+d` types a `d` instead of ending input.
#[test]
fn focus_forwards_control_chords_as_control_bytes() {
    assert_eq!(
        reduce(Mode::Focus, Key::Ctrl('d')),
        (Mode::Focus, Action::WriteBytes(vec![0x04]))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::Ctrl('u')),
        (Mode::Focus, Action::WriteBytes(vec![0x15]))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::Ctrl('c')),
        (Mode::Focus, Action::WriteBytes(vec![0x03]))
    );
}

/// `alt` is the ESC prefix — what makes `alt+b` move back a word.
#[test]
fn focus_forwards_alt_chords_with_an_escape_prefix() {
    assert_eq!(
        reduce(Mode::Focus, Key::Alt('b')),
        (Mode::Focus, Action::WriteBytes(vec![0x1b, b'b']))
    );
}

/// A `char` is encoded, not cast: `c as u8` turns every accented character
/// into a different byte, which is a real problem for anyone typing a
/// language with accents.
#[test]
fn focus_encodes_non_ascii_characters_as_utf8() {
    assert_eq!(
        reduce(Mode::Focus, Key::Char('ç')),
        (Mode::Focus, Action::WriteBytes("ç".as_bytes().to_vec()))
    );
    assert_eq!(
        reduce(Mode::Focus, Key::Char('á')),
        (Mode::Focus, Action::WriteBytes("á".as_bytes().to_vec()))
    );
}

/// A wheel notch scrolls whatever the mode is, and changes none of them:
/// the user can glance up the scrollback and keep typing.
#[test]
fn a_wheel_notch_scrolls_without_switching_modes() {
    for mode in [Mode::Focus, Mode::Nav, Mode::Scroll] {
        assert_eq!(
            reduce_wheel(mode, Direction::Up),
            (mode, Action::ScrollLines(Direction::Up, 3))
        );
        assert_eq!(
            reduce_wheel(mode, Direction::Down),
            (mode, Action::ScrollLines(Direction::Down, 3))
        );
    }
}

/// With the shell's process gone, Focus keys have nowhere to go — so they
/// stop being shell keys and become the two things left to do.
#[test]
fn an_exited_shell_rebinds_focus_to_restart_and_quit() {
    assert_eq!(
        reduce_exited(Mode::Focus, Key::Enter),
        (Mode::Focus, Action::Restart)
    );
    assert_eq!(
        reduce_exited(Mode::Focus, Key::Char('r')),
        (Mode::Focus, Action::Restart)
    );
    for key in [Key::Char('q'), Key::Ctrl('c'), Key::Ctrl('d')] {
        assert_eq!(reduce_exited(Mode::Focus, key), (Mode::Focus, Action::Quit));
    }
    assert_eq!(
        reduce_exited(Mode::Focus, Key::Prefix),
        (Mode::Nav, Action::EnterNav),
        "the way out through nav still works"
    );
    assert_eq!(
        reduce_exited(Mode::Focus, Key::Char('x')),
        (Mode::Focus, Action::None),
        "and nothing else is written into a PTY that is gone"
    );
}

/// Nav and Scroll never talked to the PTY, so a dead shell changes nothing
/// about them.
#[test]
fn the_other_modes_are_unchanged_by_a_dead_shell() {
    for mode in [Mode::Nav, Mode::Scroll] {
        for key in [Key::Char('q'), Key::Esc, Key::PgUp, Key::Up] {
            assert_eq!(reduce_exited(mode, key), reduce(mode, key));
        }
    }
}
