#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_blank_screen_is_all_empty_rows() {
    let screen = Screen::new(10, 3);
    assert_eq!(
        screen.rows(),
        &["".to_owned(), "".to_owned(), "".to_owned()]
    );
    assert_eq!(screen.as_text(), "\n\n");
}

#[test]
fn feeding_plain_text_puts_it_in_the_viewport() {
    let mut screen = Screen::new(80, 24);
    screen.feed(b"hello agent\n");
    assert_eq!(screen.rows().first().unwrap(), "hello agent");
}

#[test]
fn ansi_escape_codes_are_interpreted_not_passed_through() {
    let mut screen = Screen::new(80, 24);
    screen.feed(b"\x1b[31mred text\x1b[0m\n");
    // The colour escape is gone; the text remains.
    assert_eq!(screen.rows().first().unwrap(), "red text");
}

#[test]
fn feed_on_an_empty_screen_is_a_no_op() {
    let mut screen = Screen::new(0, 0);
    screen.feed(b"anything");
    assert!(screen.rows().is_empty());
}
