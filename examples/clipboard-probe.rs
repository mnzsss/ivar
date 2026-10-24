//! Show what the terminal delivers for copy and paste chords, outside `ivar`.
//!
//! Usage:
//! ```sh
//! cargo run --example clipboard-probe
//! ```
//!
//! Press Ctrl+C, Ctrl+Shift+C, Ctrl+V and Ctrl+Shift+V: each prints the event
//! the terminal sent. A paste prints as `Paste`; a chord the terminal keeps
//! for itself prints nothing. Press `o` to write `ivar-osc52-probe` to the
//! clipboard through OSC 52, then paste it elsewhere to see whether the
//! terminal accepts it. `q` quits.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

const OSC52_PROBE: &[u8] = b"\x1b]52;c;aXZhci1vc2M1Mi1wcm9iZQ==\x07";

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    crossterm::execute!(out, EnableBracketedPaste)?;
    write!(out, "clipboard-probe: o = OSC 52 copy, q = quit\r\n")?;
    out.flush()?;

    let result = probe(&mut out);

    crossterm::execute!(out, DisableBracketedPaste)?;
    disable_raw_mode()?;
    result
}

fn probe(out: &mut io::Stdout) -> io::Result<()> {
    loop {
        if !crossterm::event::poll(Duration::from_millis(200))? {
            continue;
        }
        match crossterm::event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE {
                    return Ok(());
                }
                if key.code == KeyCode::Char('o') && key.modifiers == KeyModifiers::NONE {
                    out.write_all(OSC52_PROBE)?;
                    write!(
                        out,
                        "OSC 52 sent: paste to check for `ivar-osc52-probe`\r\n"
                    )?;
                } else {
                    write!(out, "Key {:?} {:?}\r\n", key.code, key.modifiers)?;
                }
            }
            Event::Paste(text) => write!(out, "Paste {text:?}\r\n")?,
            _ => {}
        }
        out.flush()?;
    }
}
