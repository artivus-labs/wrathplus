//! The event log an agent reads: every UI event, numbered, in a bounded window. Written by the game
//! loop, read by HTTP threads that may long-poll for the next event, all of them or only those a
//! [`Filter`] names.

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

/// Which events a read wants, by name: `names` (any of them, or everything when empty) less
/// `exclude`. A pattern is an event's whole name (`PLAYER_DEAD`), or a prefix when it ends in `_`
/// or `*` (`AGENT_`, `CHAT_MSG_`, `UNIT_SPELLCAST*`).
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Filter {
    names: Vec<String>,
    exclude: Vec<String>,
}

impl Filter {
    /// From the comma-separated lists `GET /events?names=…&exclude=…` carries.
    pub(crate) fn parse(names: Option<&str>, exclude: Option<&str>) -> Self {
        let list = |text: Option<&str>| -> Vec<String> {
            text.unwrap_or("")
                .split(',')
                .map(|p| p.trim().to_ascii_uppercase())
                .filter(|p| !p.is_empty())
                .collect()
        };
        Self {
            names: list(names),
            exclude: list(exclude),
        }
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty() && self.exclude.is_empty()
    }

    pub(crate) fn matches(&self, event: &str) -> bool {
        let hit = |pattern: &String| match pattern.strip_suffix('*') {
            Some(prefix) => event.starts_with(prefix),
            None if pattern.ends_with('_') => event.starts_with(pattern.as_str()),
            None => event == pattern,
        };
        (self.names.is_empty() || self.names.iter().any(hit)) && !self.exclude.iter().any(hit)
    }

    fn wants(&self, entry: &Value) -> bool {
        self.is_empty() || self.matches(entry["event"].as_str().unwrap_or(""))
    }
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

    /// The events with `seq >= since` that `filter` wants, at most `limit`, waiting up to `wait`
    /// for the first one (an event the filter drops does not end the wait). No `since` is "from
    /// now": nothing already logged, only what arrives during the wait. `next` is the cursor to
    /// read on from: past the last event returned when `limit` cut the read short, else past
    /// everything logged (the events the filter dropped included).
    pub(crate) fn read(
        &self,
        since: Option<u64>,
        limit: usize,
        wait: Duration,
        filter: &Filter,
    ) -> Read {
        let deadline = Instant::now() + wait;
        let mut inner = self.inner.lock().expect("event log lock");
        let since = since.unwrap_or(inner.next);
        // Everything below this has been looked at and is not wanted.
        let mut checked = since;
        loop {
            if inner.from(checked).any(|(_, e)| filter.wants(e)) {
                break;
            }
            checked = checked.max(inner.next);
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
            .from(since)
            .filter(|(_, e)| filter.wants(e))
            .take(limit)
            .map(|(_, e)| e.clone())
            .collect();
        let cut = events.len() == limit;
        let next = events
            .last()
            .and_then(|e| e["seq"].as_u64())
            .filter(|_| cut)
            .map_or(inner.next, |s| s + 1);
        Read {
            events,
            next,
            missed,
        }
    }
}

impl Inner {
    /// The retained entries with `seq >= since` (sequence numbers are contiguous).
    fn from(&self, since: u64) -> impl Iterator<Item = &(u64, Value)> {
        let oldest = self.entries.front().map_or(self.next, |(seq, _)| *seq);
        let skip = usize::try_from(since.saturating_sub(oldest)).unwrap_or(usize::MAX);
        self.entries.iter().skip(skip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(n: u32) -> Vec<Value> {
        (0..n).map(|i| json!({"event": format!("E{i}")})).collect()
    }

    fn events(names: &[&str]) -> Vec<Value> {
        names.iter().map(|n| json!({"event": n})).collect()
    }

    fn all() -> Filter {
        Filter::default()
    }

    #[test]
    fn reads_resume_from_the_returned_cursor() {
        let log = EventLog::new();
        log.push(named(3));
        let first = log.read(Some(0), 2, Duration::ZERO, &all());
        assert_eq!(first.events.len(), 2);
        assert_eq!(first.events[0]["seq"], 1);
        assert_eq!(first.next, 3);
        let second = log.read(Some(first.next), 10, Duration::ZERO, &all());
        assert_eq!(second.events.len(), 1);
        assert_eq!(second.events[0]["event"], "E2");
        assert_eq!(second.next, 4);
        let empty = log.read(Some(second.next), 10, Duration::ZERO, &all());
        assert!(empty.events.is_empty());
        assert_eq!((empty.next, empty.missed), (4, 0));
        // A cursor from the future (or an empty log) reads back the real one.
        let ahead = log.read(Some(1_000_000), 10, Duration::ZERO, &all());
        assert_eq!((ahead.events.len(), ahead.next, ahead.missed), (0, 4, 0));
        let fresh = EventLog::new().read(Some(0), 10, Duration::ZERO, &all());
        assert_eq!((fresh.events.len(), fresh.next, fresh.missed), (0, 1, 0));
    }

    #[test]
    fn a_reader_behind_the_window_is_told_what_it_missed() {
        let log = EventLog::new();
        log.push(named(RETAINED as u32 + 5));
        let read = log.read(Some(1), 1, Duration::ZERO, &all());
        assert_eq!(read.missed, 5);
        assert_eq!(read.events[0]["seq"], 6);
        // `since=0` is the whole buffer, and says what fell out of it too.
        let whole = log.read(Some(0), 1, Duration::ZERO, &all());
        assert_eq!((whole.missed, &whole.events[0]["seq"]), (5, &json!(6)));
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
        let read = log.read(Some(1), 10, Duration::from_secs(5), &all());
        handle.join().unwrap();
        assert_eq!(read.events.len(), 1);
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn no_cursor_reads_from_now() {
        let log = std::sync::Arc::new(EventLog::new());
        log.push(named(50));
        // Nothing already logged comes back, only the cursor to carry on from.
        let now = log.read(None, 100, Duration::ZERO, &all());
        assert_eq!((now.events.len(), now.next, now.missed), (0, 51, 0));
        // With a wait, what arrives during it does.
        let writer = log.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            writer.push(events(&["PLAYER_DEAD"]));
        });
        let read = log.read(None, 100, Duration::from_secs(5), &all());
        handle.join().unwrap();
        assert_eq!(read.events.len(), 1);
        assert_eq!(read.events[0]["event"], "PLAYER_DEAD");
        assert_eq!(read.next, 52);
    }

    #[test]
    fn patterns_are_whole_names_or_prefixes() {
        let f = Filter::parse(Some("AGENT_, chat_msg_*,PLAYER_DEAD,,"), Some("AGENT_LUA"));
        assert!(f.matches("AGENT_WALK_DONE"));
        assert!(f.matches("CHAT_MSG_SAY"));
        assert!(f.matches("PLAYER_DEAD"));
        assert!(!f.matches("AGENT_LUA"), "excluded");
        assert!(!f.matches("PLAYER_DEAD_AGAIN"), "a whole name is no prefix");
        assert!(!f.matches("UNIT_HEALTH"));
        // Exclusion alone keeps everything else.
        let f = Filter::parse(None, Some("COMBAT_LOG*,UNIT_"));
        assert!(f.matches("PLAYER_DEAD"));
        assert!(!f.matches("COMBAT_LOG_EVENT_UNFILTERED"));
        assert!(!f.matches("UNIT_AURA"));
        assert_eq!(Filter::parse(None, None), Filter::default());
    }

    #[test]
    fn a_filtered_read_skips_the_noise_and_moves_the_cursor_past_it() {
        let log = EventLog::new();
        log.push(events(&[
            "UNIT_HEALTH",
            "AGENT_WALK_DONE",
            "COMBAT_LOG_EVENT_UNFILTERED",
            "CHAT_MSG_SAY",
            "UNIT_AURA",
        ]));
        let f = Filter::parse(Some("AGENT_,CHAT_MSG_"), None);
        let read = log.read(Some(0), 100, Duration::ZERO, &f);
        let got: Vec<&str> = read
            .events
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect();
        assert_eq!(got, ["AGENT_WALK_DONE", "CHAT_MSG_SAY"]);
        assert_eq!(read.next, 6, "past the unwanted tail too");
        // Cut short by the limit: the cursor stops after the last event returned, so the next
        // read carries on with the rest.
        let first = log.read(Some(0), 1, Duration::ZERO, &f);
        assert_eq!(first.events[0]["event"], "AGENT_WALK_DONE");
        assert_eq!(first.next, 3);
        let rest = log.read(Some(first.next), 1, Duration::ZERO, &f);
        assert_eq!(rest.events[0]["event"], "CHAT_MSG_SAY");
        assert_eq!(rest.next, 5);
        let none = Filter::parse(Some("QUEST_"), None);
        let read = log.read(Some(0), 100, Duration::ZERO, &none);
        assert_eq!((read.events.len(), read.next), (0, 6));
    }

    #[test]
    fn only_a_wanted_event_ends_a_filtered_wait() {
        let log = std::sync::Arc::new(EventLog::new());
        let writer = log.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            writer.push(events(&["UNIT_HEALTH", "COMBAT_LOG_EVENT_UNFILTERED"]));
            std::thread::sleep(Duration::from_millis(150));
            writer.push(events(&["UNIT_AURA", "AGENT_COMBAT_DONE"]));
        });
        let f = Filter::parse(Some("AGENT_"), None);
        let started = Instant::now();
        let read = log.read(None, 100, Duration::from_secs(5), &f);
        handle.join().unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "the noise did not end the wait"
        );
        assert_eq!(read.events.len(), 1);
        assert_eq!(read.events[0]["event"], "AGENT_COMBAT_DONE");
        assert_eq!(read.next, 5);
        // Nothing wanted in time: an empty read, with the cursor past what was dropped.
        let quiet = log.read(Some(read.next), 100, Duration::from_millis(30), &f);
        assert_eq!((quiet.events.len(), quiet.next), (0, 5));
    }
}
