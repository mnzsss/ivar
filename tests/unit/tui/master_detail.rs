#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn ctrl_c_and_ctrl_b_are_their_own_keys() {
    assert_eq!(
        map_key(press(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(Key::Ctrl('c'))
    );
    assert_eq!(
        map_key(press(KeyCode::Char('b'), KeyModifiers::CONTROL)),
        Some(Key::CtrlB)
    );
}

#[test]
fn plain_keys_map_to_their_codes() {
    assert_eq!(
        map_key(press(KeyCode::Char('q'), KeyModifiers::NONE)),
        Some(Key::Char('q'))
    );
    assert_eq!(
        map_key(press(KeyCode::Enter, KeyModifiers::NONE)),
        Some(Key::Enter)
    );
    assert_eq!(
        map_key(press(KeyCode::Esc, KeyModifiers::NONE)),
        Some(Key::Esc)
    );
    assert_eq!(
        map_key(press(KeyCode::PageUp, KeyModifiers::NONE)),
        Some(Key::PgUp)
    );
    assert_eq!(
        map_key(press(KeyCode::PageDown, KeyModifiers::NONE)),
        Some(Key::PgDn)
    );
}

#[test]
fn modified_letters_other_than_ctrl_b_reach_the_shell_as_chords() {
    assert_eq!(
        map_key(press(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        Some(Key::Ctrl('x'))
    );
    assert_eq!(
        map_key(press(KeyCode::Char('f'), KeyModifiers::ALT)),
        Some(Key::Alt('f'))
    );
}

/// The editing keys must survive the trip from crossterm to the router —
/// an unmapped one is a key that does nothing in the view.
#[test]
fn the_editing_keys_are_mapped() {
    assert_eq!(
        map_key(press(KeyCode::Backspace, KeyModifiers::NONE)),
        Some(Key::Backspace)
    );
    assert_eq!(
        map_key(press(KeyCode::Delete, KeyModifiers::NONE)),
        Some(Key::Delete)
    );
    assert_eq!(
        map_key(press(KeyCode::Tab, KeyModifiers::NONE)),
        Some(Key::Tab)
    );
    assert_eq!(
        map_key(press(KeyCode::Home, KeyModifiers::NONE)),
        Some(Key::Home)
    );
    assert_eq!(
        map_key(press(KeyCode::End, KeyModifiers::NONE)),
        Some(Key::End)
    );
}

#[test]
fn unmapped_keys_are_none() {
    assert_eq!(map_key(press(KeyCode::F(1), KeyModifiers::NONE)), None);
    assert_eq!(map_key(press(KeyCode::CapsLock, KeyModifiers::NONE)), None);
}
