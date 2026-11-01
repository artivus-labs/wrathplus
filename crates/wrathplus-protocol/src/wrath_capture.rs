//! Offline replay of a recorded build-12340 gameplay capture (`WOW_WRATH_PACKET_RECORD`, format in
//! `https://github.com/artivus-labs/wrathplus/wiki/WRATH-PACKET-RECORDING`): a strict reader and a per-opcode census that routes each
//! packet the way the live reader (`wrathplus-app` `net/io/wrath.rs`) does.
//!
//! Routes, in the live reader's order: `SMSG_MULTIPLE_MOVES` is split and its records replayed in
//! place; the session loop's own opcodes (logout, pong, login failure, transfers, client control,
//! mover orders, unset-can-fly, root/unroot) are decoded with their own decoders and produce no
//! events here, since their effects are the movement authority's; movement relays go through
//! `wrath_relay`; everything else through [`WrathEventAdapter`].
//!
//! Not modelled: the movement authority's state (barriers, the unsupported-movement guard, seat
//! tracking), so a relay that fails to decode is counted as a drop, which the live reader also
//! does for another unit outside the on-foot subset, and as fatal only for our own mover.
use std::collections::BTreeMap;
use std::io;

use crate::wrath_events::{WrathDispatch, WrathEventAdapter};
use crate::{SessionEvent, SessionEventKind};

/// `AZBWRPK1` then build 12340, little-endian.
pub const HEADER: &[u8; 12] = b"AZBWRPK1\x34\x30\x00\x00";

/// One recorded inbound packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Microseconds since the recorder opened.
    pub elapsed_us: u64,
    pub opcode: u16,
    pub body: Vec<u8>,
}

/// A read capture: every whole record, and whether an incomplete final record was cut off (an
/// interrupted write), which is reported, never invented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub records: Vec<Record>,
    pub truncated_tail: bool,
}

/// Read a capture file's bytes. A wrong header is an error; a short final record is reported.
pub fn read(bytes: &[u8]) -> io::Result<Capture> {
    if bytes.get(..HEADER.len()) != Some(HEADER.as_slice()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not an AZBWRPK1 build-12340 capture",
        ));
    }
    let mut at = HEADER.len();
    let mut records = Vec::new();
    while at < bytes.len() {
        let Some(head) = bytes.get(at..at + 14) else {
            return Ok(Capture {
                records,
                truncated_tail: true,
            });
        };
        let elapsed_us = u64::from_le_bytes(head[0..8].try_into().expect("8 bytes"));
        let opcode = u16::from_le_bytes([head[8], head[9]]);
        let len = u32::from_le_bytes(head[10..14].try_into().expect("4 bytes")) as usize;
        let Some(body) = bytes.get(at + 14..at + 14 + len) else {
            return Ok(Capture {
                records,
                truncated_tail: true,
            });
        };
        records.push(Record {
            elapsed_us,
            opcode,
            body: body.to_vec(),
        });
        at += 14 + len;
    }
    Ok(Capture {
        records,
        truncated_tail: false,
    })
}

/// Which path the live reader takes for an opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Route {
    /// Split into its records, replayed in place.
    Bundle,
    /// Decoded by the session loop itself; its effect is the movement authority's.
    Session,
    /// A movement relay (`wrath_relay`).
    Relay,
    /// The event adapter.
    Adapter,
}

/// What one opcode did across the capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpcodeTally {
    pub seen: usize,
    /// Decoded (adapter `Handled`, a relay event, a session-loop decode, a split bundle).
    pub handled: usize,
    /// The adapter returned `Unhandled`: no family claims the opcode.
    pub unhandled: usize,
    /// A decoder rejected the body (malformed or a layout mismatch), or the adapter errored.
    pub failed: usize,
    /// Session events produced.
    pub events: usize,
    pub route: Option<Route>,
}

/// One rejected packet, for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Index in replay order (bundled records counted).
    pub index: usize,
    pub opcode: u16,
    pub error: String,
}

/// The replay census.
#[derive(Debug, Clone, Default)]
pub struct Census {
    /// Packets replayed, bundled records included.
    pub packets: usize,
    pub opcodes: BTreeMap<u16, OpcodeTally>,
    /// Session-event kinds produced, by name.
    pub events: BTreeMap<String, usize>,
    /// Every rejection, in order.
    pub failures: Vec<Failure>,
}

const SMSG_CHARACTER_LOGIN_FAILED: u16 = 0x0041;
const SMSG_CLIENT_CONTROL_UPDATE: u16 = 0x0159;
const SMSG_PONG: u16 = 0x01dd;

fn session_decode(opcode: u16, body: &[u8]) -> Option<Result<(), String>> {
    use crate::wrath_authority as a;
    let check = |r: Result<(), String>| Some(r);
    match opcode {
        SMSG_CHARACTER_LOGIN_FAILED | SMSG_PONG => {
            let want = if opcode == SMSG_PONG { 4 } else { 1 };
            check(
                (body.len() == want)
                    .then_some(())
                    .ok_or_else(|| format!("{} byte body, want {want}", body.len())),
            )
        }
        SMSG_CLIENT_CONTROL_UPDATE => {
            let mut r = body;
            let res = crate::wire::read_packed_guid(&mut r)
                .map_err(|e| e.to_string())
                .and_then(|_| {
                    (r.len() == 1)
                        .then_some(())
                        .ok_or_else(|| "client control: one byte after the guid".to_string())
                });
            check(res)
        }
        0x003e..=0x0040 => check(
            crate::wrath_transfer::decode(opcode, body)
                .map(|_| ())
                .map_err(|e| e.to_string()),
        ),
        o if crate::wrath_logout::handles(o) => check(
            crate::wrath_logout::decode(o, body)
                .map(|_| ())
                .map_err(|e| e.to_string()),
        ),
        o if a::is_mover_order(o) => check(
            a::decode_mover_order(o, body)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        a::SMSG_MOVE_UNSET_CAN_FLY => check(
            a::decode_unset_can_fly(body)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        a::SMSG_FORCE_MOVE_ROOT | a::SMSG_FORCE_MOVE_UNROOT => check(
            a::decode_force_root(body)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        _ => None,
    }
}

impl Census {
    fn tally(&mut self, opcode: u16, route: Route) -> &mut OpcodeTally {
        let t = self.opcodes.entry(opcode).or_default();
        t.seen += 1;
        t.route = Some(route);
        t
    }

    fn fail(&mut self, opcode: u16, error: String) {
        self.failures.push(Failure {
            index: self.packets,
            opcode,
            error,
        });
    }

    fn emit(&mut self, event: SessionEvent, out: &mut Vec<SessionEvent>) {
        let kind = format!("{:?}", SessionEventKind::from(&event));
        *self.events.entry(kind).or_default() += 1;
        out.push(event);
    }

    /// Opcodes no family claims, most frequent first.
    pub fn unhandled(&self) -> Vec<(u16, usize)> {
        let mut v: Vec<(u16, usize)> = self
            .opcodes
            .iter()
            .filter(|(_, t)| t.unhandled > 0)
            .map(|(&o, t)| (o, t.unhandled))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
}

/// Replay `records` in order. Returns the census and the session events in order, for an app-side
/// drain.
pub fn census(records: &[Record]) -> (Census, Vec<SessionEvent>) {
    let mut c = Census::default();
    let mut adapter = WrathEventAdapter::new();
    let mut events = Vec::new();
    let mut queue: std::collections::VecDeque<(u16, Vec<u8>, u32)> = records
        .iter()
        .map(|r| (r.opcode, r.body.clone(), (r.elapsed_us / 1000) as u32))
        .collect();
    while let Some((opcode, body, ms)) = queue.pop_front() {
        if opcode == crate::wrath_movement_misc::SMSG_MULTIPLE_MOVES {
            match crate::wrath_movement_misc::split_multiple_moves(&body) {
                Ok(inner) => {
                    c.tally(opcode, Route::Bundle).handled += 1;
                    for (o, b) in inner.into_iter().rev() {
                        queue.push_front((o, b, ms));
                    }
                }
                Err(e) => {
                    c.tally(opcode, Route::Bundle).failed += 1;
                    c.fail(opcode, e.to_string());
                }
            }
            c.packets += 1;
            continue;
        }
        if let Some(result) = session_decode(opcode, &body) {
            match result {
                Ok(()) => c.tally(opcode, Route::Session).handled += 1,
                Err(e) => {
                    c.tally(opcode, Route::Session).failed += 1;
                    c.fail(opcode, e);
                }
            }
        } else if crate::wrath_relay::handles(opcode) {
            match crate::wrath_relay::decode(opcode, &body) {
                Ok(event) => {
                    let t = c.tally(opcode, Route::Relay);
                    t.handled += 1;
                    t.events += 1;
                    c.emit(event, &mut events);
                }
                Err(e) => {
                    c.tally(opcode, Route::Relay).failed += 1;
                    c.fail(
                        opcode,
                        format!("relay (dropped live for another unit): {e:?}"),
                    );
                }
            }
        } else {
            match adapter.dispatch(opcode, &body, ms) {
                Ok(WrathDispatch::Handled { session_events, .. }) => {
                    let t = c.tally(opcode, Route::Adapter);
                    t.handled += 1;
                    t.events += session_events.len();
                    for event in session_events {
                        c.emit(event, &mut events);
                    }
                }
                Ok(WrathDispatch::Unhandled { .. }) => {
                    c.tally(opcode, Route::Adapter).unhandled += 1;
                }
                Err(e) => {
                    c.tally(opcode, Route::Adapter).failed += 1;
                    c.fail(opcode, e.to_string());
                }
            }
        }
        c.packets += 1;
    }
    (c, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut out = 7u64.to_le_bytes().to_vec();
        out.extend(opcode.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    #[test]
    fn reads_records_and_reports_a_cut_tail() {
        let mut bytes = HEADER.to_vec();
        bytes.extend(record(0x1c8, &[]));
        bytes.extend(record(0x277, &11u32.to_le_bytes()));
        let whole = read(&bytes).unwrap();
        assert_eq!(whole.records.len(), 2);
        assert!(!whole.truncated_tail);
        assert_eq!(whole.records[1].opcode, 0x277);
        // An interrupted final write: keep what is whole.
        let cut = read(&bytes[..bytes.len() - 2]).unwrap();
        assert_eq!((cut.records.len(), cut.truncated_tail), (1, true));
        assert!(read(b"AZBWRPK1\x00\x00\x00\x00").is_err(), "wrong build");
        assert!(read(b"short").is_err());
    }

    #[test]
    fn the_census_routes_and_counts_like_the_live_reader() {
        let records = vec![
            Record {
                elapsed_us: 0,
                opcode: 0x1c8,
                body: vec![],
            },
            Record {
                elapsed_us: 10,
                opcode: 0x277,
                body: 11u32.to_le_bytes().to_vec(),
            },
            // Malformed: music needs four bytes.
            Record {
                elapsed_us: 20,
                opcode: 0x277,
                body: vec![1],
            },
            // Nothing claims it.
            Record {
                elapsed_us: 30,
                opcode: 0x7fff,
                body: vec![],
            },
            Record {
                elapsed_us: 40,
                opcode: SMSG_PONG,
                body: 5u32.to_le_bytes().to_vec(),
            },
        ];
        let (c, events) = census(&records);
        assert_eq!(c.packets, 5);
        assert_eq!(c.opcodes[&0x1c8].handled, 1);
        assert_eq!(c.opcodes[&0x277].seen, 2);
        assert_eq!(c.opcodes[&0x277].handled, 1);
        // The adapter answers a malformed body as unhandled rather than an error.
        assert_eq!(c.opcodes[&0x277].unhandled + c.opcodes[&0x277].failed, 1);
        assert_eq!(c.opcodes[&0x7fff].unhandled, 1);
        assert_eq!(c.opcodes[&SMSG_PONG].route, Some(Route::Session));
        assert_eq!(c.opcodes[&SMSG_PONG].handled, 1);
        assert_eq!(c.events["FishNotHooked"], 1);
        assert_eq!(c.events["PlayMusic"], 1);
        assert_eq!(events.len(), 2);
        assert!(c.unhandled().iter().any(|&(o, n)| o == 0x7fff && n == 1));
    }
}
