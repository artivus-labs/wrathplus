//! The event log an agent reads: every UI event, numbered, in a bounded window. Written by the game
//! loop, read by HTTP threads that may long-poll for the next event.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Events kept for late readers; older ones are reported as missed.
const RETAINED: usize = 10_000;

pub(crate) struct EventLog {
    inner: Mutex<Inner>,
    arrived: Condvar,
}

struct Inner {
    /// The sequence number the next event gets; the first is 1.
    next: u64,
    entries: VecDeque<(u64, Value)>,
}

/// What a read returns: the events after `since`, the cursor for the next read, and how many
/// events fell out of the window before this reader saw them.
#[derive(Debug, PartialEq)]
pub(crate) struct Read {
    pub events: Vec<Value>,
    pub next: u64,
    pub missed: u64,
}

impl EventLog {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next: 1,
                entries: VecDeque::new(),
            }),
            arrived: Condvar::new(),
        }
    }

    /// Append events (each a JSON object without its `seq`), numbering them in order.
    pub(crate) fn push(&self, events: impl IntoIterator<Item = Value>) {
        let mut inner = self.inner.lock().expect("event log lock");
        let mut any = false;
        for mut event in events {
            let seq = inner.next;
            inner.next += 1;
            if let Some(map) = event.as_object_mut() {
                map.insert("seq".into(), json!(seq));
            }
            inner.entries.push_back((seq, event));
            if inner.entries.len() > RETAINED {
                inner.entries.pop_front();
            }
            any = true;
        }
        if any {
            self.arrived.notify_all();
        }
    }

    /// The events with `seq >= since`, at most `limit`, waiting up to `wait` for the first one.
    pub(crate) fn read(&self, since: u64, limit: usize, wait: Duration) -> Read {
        let deadline = Instant::now() + wait;
        let mut inner = self.inner.lock().expect("event log lock");
        while inner.next <= since {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            inner = self
                .arrived
                .wait_timeout(inner, left)
                .expect("event log lock")
                .0;
        }
        let oldest = inner.entries.front().map_or(inner.next, |(seq, _)| *seq);
        let missed = oldest.saturating_sub(since.max(1));
        let events: Vec<Value> = inner
            .entries
            .iter()
            .filter(|(seq, _)| *seq >= since)
            .take(limit)
            .map(|(_, e)| e.clone())
            .collect();
        let next = events
            .last()
            .and_then(|e| e["seq"].as_u64())
            .map_or(inner.next.min(since.max(oldest)), |s| s + 1);
        Read {
            events,
            next,
            missed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(n: u32) -> Vec<Value> {
        (0..n).map(|i| json!({"event": format!("E{i}")})).collect()
    }

    #[test]
    fn reads_resume_from_the_returned_cursor() {
        let log = EventLog::new();
        log.push(named(3));
        let first = log.read(0, 2, Duration::ZERO);
        assert_eq!(first.events.len(), 2);
        assert_eq!(first.events[0]["seq"], 1);
        assert_eq!(first.next, 3);
        let second = log.read(first.next, 10, Duration::ZERO);
        assert_eq!(second.events.len(), 1);
        assert_eq!(second.events[0]["event"], "E2");
        assert_eq!(second.next, 4);
        let empty = log.read(second.next, 10, Duration::ZERO);
        assert!(empty.events.is_empty());
        assert_eq!((empty.next, empty.missed), (4, 0));
    }

    #[test]
    fn a_reader_behind_the_window_is_told_what_it_missed() {
        let log = EventLog::new();
        log.push(named(RETAINED as u32 + 5));
        let read = log.read(1, 1, Duration::ZERO);
        assert_eq!(read.missed, 5);
        assert_eq!(read.events[0]["seq"], 6);
    }

    #[test]
    fn a_long_poll_wakes_on_the_next_event() {
        let log = std::sync::Arc::new(EventLog::new());
        let writer = log.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            writer.push(named(1));
        });
        let started = Instant::now();
        let read = log.read(1, 10, Duration::from_secs(5));
        handle.join().unwrap();
        assert_eq!(read.events.len(), 1);
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
