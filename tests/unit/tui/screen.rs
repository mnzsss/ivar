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

/// The driver feeds whatever arrived since the last pump, so a shell's
/// output reaches the emulator in as many chunks as the PTY happened to
/// deliver. The viewport must be the whole conversation, not the last chunk.
#[test]
fn successive_feeds_accumulate_on_one_screen() {
    let mut screen = Screen::new(80, 24);
    // A PTY sends `\r\n`: bare `\n` is a line feed, which moves down without
    // returning to column zero. Feeding it the way a shell does is the point.
    screen.feed(b"hello ");
    screen.feed(b"agent\r\n");
    screen.feed(b"second line\r\n");
    assert_eq!(screen.rows().first().unwrap(), "hello agent");
    assert_eq!(screen.rows().get(1).unwrap(), "second line");
}

/// The cursor is emulator state too: a carriage return only means "back to
/// column zero" if the screen remembers where the cursor was.
#[test]
fn the_cursor_survives_between_feeds() {
    let mut screen = Screen::new(80, 24);
    screen.feed(b"progress: 10%");
    screen.feed(b"\rprogress: 99%");
    assert_eq!(screen.rows().first().unwrap(), "progress: 99%");
}
