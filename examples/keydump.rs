// Bounded key-event dump probe (measurement, NOT a gate).
//
// Establishes the terminal event/char path the way the app sees it: enable
// raw mode (the TUI's input state — this is the state that decides how crossterm
// decodes the byte 0x0A), then poll crossterm events on a HARD total deadline
// and print each decoded KeyEvent. Feed it raw bytes (0x0A LF, 0x0D CR, plain
// chars) and read exactly what the terminal input layer decodes them into.
//
// Bounded by construction: fixed 4 s deadline + event cap, print-and-return,
// no blocking reads, no loops past the deadline.

use std::time::{Duration, Instant};

use crossterm::event::{self, Event};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

fn main() {
    enable_raw_mode().expect("enable raw mode (the TUI's input state)");
    let deadline = Duration::from_secs(4);
    let start = Instant::now();
    let mut seen = 0u32;
    loop {
        if event::poll(Duration::from_millis(100)).expect("poll") {
            match event::read().expect("read") {
                Event::Key(k) => {
                    seen += 1;
                    eprintln!(
                        "KEY code={:?} modifiers={:?} kind={:?}",
                        k.code, k.modifiers, k.kind
                    );
                }
                other => eprintln!("OTHER {other:?}"),
            }
        }
        if seen >= 50 || start.elapsed() >= deadline {
            break;
        }
    }
    let _ = disable_raw_mode();
    eprintln!("DONE events={seen}");
}
