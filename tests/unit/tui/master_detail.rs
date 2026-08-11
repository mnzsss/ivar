#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

/// The default prefix, `ctrl+o`.
fn prefix() -> Prefix {
    Prefix::default_prefix()
}

fn map(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
    map_key(press(code, modifiers), &prefix())
}

#[test]
fn ctrl_c_and_the_prefix_are_their_own_keys() {
    assert_eq!(
        map(KeyCode::Char('c'), KeyModifiers::CONTROL),
        Some(Key::Ctrl('c'))
    );
    assert_eq!(
        map(KeyCode::Char('o'), KeyModifiers::CONTROL),
        Some(Key::Prefix)
    );
}

/// The reason the prefix moved: `ctrl+b` is what tmux and Orca bind, so a
/// view running inside either never receives it. It must now reach the shell
/// like any other key.
#[test]
fn ctrl_b_is_no_longer_special() {
    assert_eq!(
        map(KeyCode::Char('b'), KeyModifiers::CONTROL),
        Some(Key::Ctrl('b'))
    );
}

#[test]
fn the_prefix_is_configurable() {
    let prefix = Prefix::parse("ctrl+a").expect("ctrl+a parses");
    assert_eq!(prefix.label(), "ctrl+a");
    assert_eq!(
        map_key(press(KeyCode::Char('a'), KeyModifiers::CONTROL), &prefix),
        Some(Key::Prefix)
    );
    // And the old default is just a control chord again.
    assert_eq!(
        map_key(press(KeyCode::Char('o'), KeyModifiers::CONTROL), &prefix),
        Some(Key::Ctrl('o'))
    );

    let prefix = Prefix::parse("F5").expect("f-keys parse, case-insensitively");
    assert_eq!(prefix.label(), "f5");
    assert_eq!(
        map_key(press(KeyCode::F(5), KeyModifiers::NONE), &prefix),
        Some(Key::Prefix)
    );
}

/// A typo in the env var must not cost the user their way out of the TUI.
#[test]
fn an_unparseable_prefix_is_refused_rather_than_guessed() {
    assert_eq!(Prefix::parse("ctrl+shift+z"), None);
    assert_eq!(Prefix::parse("meta+x"), None);
    assert_eq!(Prefix::parse("f13"), None);
    assert_eq!(Prefix::parse(""), None);
}

#[test]
fn plain_keys_map_to_their_codes() {
    assert_eq!(
        map(KeyCode::Char('q'), KeyModifiers::NONE),
        Some(Key::Char('q'))
    );
    assert_eq!(map(KeyCode::Enter, KeyModifiers::NONE), Some(Key::Enter));
    assert_eq!(map(KeyCode::Esc, KeyModifiers::NONE), Some(Key::Esc));
    assert_eq!(map(KeyCode::PageUp, KeyModifiers::NONE), Some(Key::PgUp));
    assert_eq!(map(KeyCode::PageDown, KeyModifiers::NONE), Some(Key::PgDn));
}

#[test]
fn modified_letters_other_than_the_prefix_reach_the_shell_as_chords() {
    assert_eq!(
        map(KeyCode::Char('x'), KeyModifiers::CONTROL),
        Some(Key::Ctrl('x'))
    );
    assert_eq!(
        map(KeyCode::Char('f'), KeyModifiers::ALT),
        Some(Key::Alt('f'))
    );
}

/// The editing keys must survive the trip from crossterm to the router —
/// an unmapped one is a key that does nothing in the view.
#[test]
fn the_editing_keys_are_mapped() {
    assert_eq!(
        map(KeyCode::Backspace, KeyModifiers::NONE),
        Some(Key::Backspace)
    );
    assert_eq!(map(KeyCode::Delete, KeyModifiers::NONE), Some(Key::Delete));
    assert_eq!(map(KeyCode::Tab, KeyModifiers::NONE), Some(Key::Tab));
    assert_eq!(map(KeyCode::Home, KeyModifiers::NONE), Some(Key::Home));
    assert_eq!(map(KeyCode::End, KeyModifiers::NONE), Some(Key::End));
}

#[test]
fn unmapped_keys_are_none() {
    assert_eq!(map(KeyCode::F(1), KeyModifiers::NONE), None);
    assert_eq!(map(KeyCode::CapsLock, KeyModifiers::NONE), None);
}
