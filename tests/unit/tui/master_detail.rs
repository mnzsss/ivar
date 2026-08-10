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
        Some(Key::CtrlC)
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
fn modified_letters_other_than_ctrl_b_and_ctrl_c_are_plain_chars() {
    assert_eq!(
        map_key(press(KeyCode::Char('x'), KeyModifiers::CONTROL)),
        Some(Key::Char('x'))
    );
}

#[test]
fn unmapped_keys_are_none() {
    assert_eq!(map_key(press(KeyCode::F(1), KeyModifiers::NONE)), None);
    assert_eq!(map_key(press(KeyCode::Tab, KeyModifiers::NONE)), None);
}
