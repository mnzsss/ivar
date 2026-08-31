#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_blank_screen_is_all_empty_rows() {
    let screen = Screen::new(10, 3);
    assert_eq!(
        screen.rows(),
        &["".to_owned(), "".to_owned(), "".to_owned()]
    );
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

/// The reason the view was monochrome: the emulator's colours have to reach
/// the widget, and `rows()` throws them away by design. `styled_rows()` is
/// the path that keeps them.
#[test]
fn colours_and_attributes_survive_into_the_styled_rows() {
    let mut screen = Screen::new(40, 3);
    screen.feed(b"\x1b[31mred\x1b[0m plain");

    let line = screen.styled_rows().first().unwrap();
    let red = line.spans.first().unwrap();
    assert_eq!(red.content, "red");
    assert_eq!(red.style.fg, Some(Color::Indexed(1)));

    // The reset really resets: what follows is not red.
    let rest: String = line
        .spans
        .iter()
        .skip(1)
        .map(|s| s.content.clone())
        .collect();
    assert_eq!(rest, " plain");
    assert!(
        line.spans
            .iter()
            .skip(1)
            .all(|s| s.style.fg != Some(Color::Indexed(1))),
        "the reset must end the red run"
    );

    // And the plain-text view is unchanged — both are still available.
    assert_eq!(screen.rows().first().unwrap(), "red plain");
}

#[test]
fn bold_and_rgb_reach_the_style() {
    let mut screen = Screen::new(40, 3);
    screen.feed(b"\x1b[1;38;2;10;20;30mloud");

    let span = screen.styled_rows().first().unwrap().spans.first().unwrap();
    assert_eq!(span.content, "loud");
    assert_eq!(span.style.fg, Some(Color::Rgb(10, 20, 30)));
    assert!(span.style.add_modifier.contains(Modifier::BOLD));
}

/// A row must not paint its full width: the trailing blanks are trimmed, or
/// every line carries a background all the way to the panel's edge.
#[test]
fn trailing_blanks_are_trimmed_from_a_styled_row() {
    let mut screen = Screen::new(40, 3);
    screen.feed(b"hi");

    let line = screen.styled_rows().first().unwrap();
    assert_eq!(line.width(), 2, "was: {line:?}");
}
