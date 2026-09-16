//! Per-buffer occurrences (`M-s o`, issue 06): the same search pipeline
//! scoped to the current buffer — no walk at all. `Searcher::search_slice`
//! runs the (line-oriented) searcher over the buffer's bytes in memory
//! and streams the hits through the bus exactly like a project search
//! (one synthetic "file" group named after the buffer).

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::search::rg::{SearchEvent, StreamSink};

/// Spawn a per-buffer occur search on a worker thread; events flow to
/// `bus`. `display_name` is the buffer's results-view group label
/// (project-relative name, or `*scratch*`); `text` is the buffer content
/// (a copy — the search runs on a worker thread).
pub fn spawn_occur(
    display_name: String,
    text: String,
    pattern: String,
    cancel: Arc<AtomicBool>,
    bus_tx: mpsc::UnboundedSender<SearchEvent>,
    generation: usize,
) {
    std::thread::Builder::new()
        .name("redline-occur".into())
        .spawn(move || {
            let gen_id = generation;
            let matcher = match RegexMatcherBuilder::new()
                .case_smart(true) // smart case, like the project search
                .build(&pattern)
            {
                Ok(m) => m,
                Err(e) => {
                    let _ = bus_tx.send(SearchEvent::Error {
                        message: e.to_string(),
                        generation: gen_id,
                    });
                    return;
                }
            };
            let mut searcher = SearcherBuilder::new()
                .line_number(true)
                .binary_detection(BinaryDetection::quit(b'\0'))
                .build();
            let mut sink = StreamSink {
                file: display_name,
                tx: bus_tx.clone(),
                gen_id,
                cancel: cancel.clone(),
                literal: None, // a regex pattern: the column is undeterminable
                word: false,
                ranges: None,  // occur has no token-class filtering
                hits: 0,
            };
            if let Err(e) = searcher.search_slice(&matcher, text.as_bytes(), &mut sink) {
                tracing::warn!(error = %e, "occur: search_slice failed");
            }
            let _ = bus_tx.send(SearchEvent::Finished {
                cancelled: cancel.load(Ordering::Relaxed),
                generation: gen_id,
            });
        })
        .expect("spawn occur thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::rg::SearchBus;
    use std::time::Duration;

    fn drain_until_finished(
        rx: &mut mpsc::UnboundedReceiver<SearchEvent>,
    ) -> Vec<SearchEvent> {
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        loop {
            while let Ok(ev) = rx.try_recv() {
                if matches!(ev, SearchEvent::Finished { .. }) {
                    return out;
                }
                out.push(ev);
            }
            if start.elapsed() > Duration::from_secs(5) {
                return out;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// All in-file matches, with line numbers and a per-file (group) count.
    #[test]
    fn occur_lists_all_in_file_matches() {
        let text = "foo world\nbar foo\nfoo again\nbaz\n".to_string();
        let (bus, mut rx) = SearchBus::new();
        let tx = bus.sender();
        spawn_occur(
            "src/t.rs".to_string(),
            text.clone(),
            "foo".to_string(),
            Arc::new(AtomicBool::new(false)),
            tx,
            0,
        );
        let events = drain_until_finished(&mut rx);
        let hit_lines: Vec<u64> = events
            .iter()
            .filter_map(|ev| match ev {
                SearchEvent::Hit { line_no, line, .. } => Some((*line_no, line.clone())),
                _ => None,
            })
            .map(|(l, _)| l)
            .collect();
        assert_eq!(hit_lines, vec![1, 2, 3], "all in-file matches: {hit_lines:?}");
        // Per-file (group) count: FileDone carries the final hit count.
        let file_done = events
            .iter()
            .find_map(|ev| match ev {
                SearchEvent::FileDone { hits, .. } => Some(*hits),
                _ => None,
            })
            .expect("a FileDone event");
        assert_eq!(file_done, 3, "group count = 3");
        drop(bus);
    }

    /// Smart case: an all-lowercase pattern matches both cases.
    #[test]
    fn occur_smart_case() {
        let text = "Hello there\nHELLO again\n".to_string();
        let (bus, mut rx) = SearchBus::new();
        let tx = bus.sender();
        spawn_occur(
            "*scratch*".to_string(),
            text,
            "hello".to_string(),
            Arc::new(AtomicBool::new(false)),
            tx,
            0,
        );
        let events = drain_until_finished(&mut rx);
        let n = events
            .iter()
            .filter(|ev| matches!(ev, SearchEvent::Hit { .. }))
            .count();
        assert_eq!(n, 2, "smart case matches both: {events:?}");
        drop(bus);
    }

    /// A regex pattern works (no fixed-string assumption).
    #[test]
    fn occur_regex_pattern() {
        let text = "a1\nb2\na3\n".to_string();
        let (bus, mut rx) = SearchBus::new();
        let tx = bus.sender();
        spawn_occur(
            "x.txt".to_string(),
            text,
            "a\\d".to_string(),
            Arc::new(AtomicBool::new(false)),
            tx,
            0,
        );
        let events = drain_until_finished(&mut rx);
        let hit_lines: Vec<u64> = events
            .iter()
            .filter_map(|ev| match ev {
                SearchEvent::Hit { line_no, .. } => Some(*line_no),
                _ => None,
            })
            .collect();
        assert_eq!(hit_lines, vec![1, 3], "regex hits: {hit_lines:?}");
        drop(bus);
    }
}
