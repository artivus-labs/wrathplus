//! Explicit L1 lock requests: `RequestLFDPlayerLockInfo` (0x5569d0) and
//! `RequestLFDPartyLockInfo` (0x556a50), each with its own process-lifetime gate.
//!
//! Native (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-LFG-L1-LIFECYCLE`): the counter (initialised to 2) is incremented before it
//! is tested; values <= 2 send. Above 2, the clock (OS milliseconds as unsigned 32-bit, times
//! the widened f32 0.001, `0x86d400`) is compared with the stored f32 timestamp: under 10.0
//! seconds rejects, still counting; otherwise the timestamp is stored, the counter reset to 0
//! and the request sent. Accepted pushes true, rejected false. Nothing invalidates the caches,
//! and there is no pending state or reply matching. The gates outlive VMs and sessions, so they
//! are process statics. App injects a boot-relative OS-monotonic clock (ms as wrapping u32); until
//! it does, a process-relative clock stands in (labelled): native's clock is OS uptime, so its first
//! call passes the 10 s test, where the stand-in rejects every request in the process's first 10 s.
//!
//! Item requests (agreed with App): each `GetLFGDungeonRewardInfo` call on an uncached template,
//! the native 0x552950 callback registration, appends one [`WrathLfgItemRequest`] to this VM's
//! outbox; cached calls append nothing and nothing is deduplicated. App drains it every frame and
//! owns the epoch-tagged queue, the item query and the completion: after publishing the item's
//! template and presentation it fires one `LFG_UPDATE_RANDOM_INFO` per pending request in order,
//! and drops them without events on failure. Requests a VM recorded but App had not drained when
//! that VM was replaced are lost (at most one frame's worth); the UI keeps no session policy.

use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use mlua::{Lua, MultiValue};

use crate::script::Model;

/// One accepted explicit request for App to send (opcodes 0x036E / 0x0371, empty body).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrathLfgLockRequest {
    Player,
    Party,
}

/// One uncached reward-item lookup, in call order; `seq` increases for the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrathLfgItemRequest {
    pub seq: u64,
    pub item: u32,
}

static NEXT_ITEM_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Append one item request to this VM's outbox.
pub(super) fn record_item_request(lua: &Lua, item: u32) {
    let seq = NEXT_ITEM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .wrath_lfg
        .item_requests
        .push(WrathLfgItemRequest { seq, item });
}

type Clock = Arc<dyn Fn() -> u32 + Send + Sync>;

/// One wrapper's counter and last accepted timestamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Gate {
    counter: i32,
    last: f32,
}

impl Gate {
    /// The PE `.data` initial counter of 2 and the zero-initialised timestamp.
    pub(super) const INITIAL: Gate = Gate {
        counter: 2,
        last: 0.0,
    };

    /// One explicit call at OS time `now_ms`; true sends.
    pub(super) fn request(&mut self, now_ms: u32) -> bool {
        self.counter = self.counter.wrapping_add(1);
        if self.counter <= 2 {
            return true;
        }
        // `0x009e1134`: the f32 0.001 widened, applied to the unsigned millisecond count.
        let now = f64::from(now_ms) * f64::from(0.001f32);
        if now - f64::from(self.last) < 10.0 {
            return false;
        }
        self.last = now as f32;
        self.counter = 0;
        true
    }
}

static GATES: Mutex<[Gate; 2]> = Mutex::new([Gate::INITIAL, Gate::INITIAL]);

fn clock_slot() -> &'static RwLock<Option<Clock>> {
    static CLOCK: RwLock<Option<Clock>> = RwLock::new(None);
    &CLOCK
}

fn now_ms() -> u32 {
    if let Some(clock) = clock_slot().read().ok().and_then(|c| c.clone()) {
        return clock();
    }
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u32
}

/// Install the OS-monotonic millisecond clock the request gates read, process-wide.
pub fn set_wrath_lfg_request_clock(clock: Arc<dyn Fn() -> u32 + Send + Sync>) {
    if let Ok(mut slot) = clock_slot().write() {
        *slot = Some(clock);
    }
}

impl crate::script::UiScript {
    /// Drain the accepted explicit lock requests, in call order.
    pub fn take_wrath_lfg_lock_requests(&mut self) -> Vec<WrathLfgLockRequest> {
        std::mem::take(&mut self.model_mut().wrath_lfg.requests)
    }

    /// Drain this VM's uncached reward-item requests, in call order.
    pub fn take_wrath_lfg_item_requests(&mut self) -> Vec<WrathLfgItemRequest> {
        std::mem::take(&mut self.model_mut().wrath_lfg.item_requests)
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    for (name, kind) in [
        ("RequestLFDPlayerLockInfo", WrathLfgLockRequest::Player),
        ("RequestLFDPartyLockInfo", WrathLfgLockRequest::Party),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, _ignored: MultiValue| {
                let now = now_ms();
                let accepted = {
                    let mut gates = GATES.lock().unwrap_or_else(|e| e.into_inner());
                    gates[kind as usize].request(now)
                };
                if accepted {
                    lua.app_data_mut::<Model>()
                        .expect("model app_data")
                        .wrath_lfg
                        .requests
                        .push(kind);
                }
                Ok(accepted)
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_counts_before_testing_and_resets_only_on_a_clock_acceptance() {
        let mut g = Gate::INITIAL;
        // The first call already tests the clock: 5 s of OS uptime is under 10.
        assert!(!g.request(5_000));
        assert_eq!(g.counter, 3, "a rejected call still counts");
        assert!(g.request(12_000), "12.0 - 0.0 >= 10.0");
        assert_eq!(g.counter, 0);
        assert_eq!(
            g.last,
            (12_000.0 * f64::from(0.001f32)) as f32,
            "quantised to f32"
        );
        // Two counter-only acceptances follow the reset, whatever the clock says.
        assert!(g.request(12_001));
        assert!(g.request(12_002));
        assert!(!g.request(21_999));
        assert!(g.request(22_000), "exactly 10.0 elapsed accepts");
    }

    #[test]
    fn the_clock_is_unsigned_milliseconds_stored_as_f32() {
        let mut g = Gate::INITIAL;
        assert!(
            g.request(u32::MAX),
            "unsigned, so the top bit is not negative"
        );
        assert_eq!(g.last, (f64::from(u32::MAX) * f64::from(0.001f32)) as f32);
    }

    #[test]
    fn independent_gates_answer_booleans_and_queue_requests() {
        use crate::script::UiScript;
        // Process-wide state: this is the only test that touches the statics.
        set_wrath_lfg_request_clock(Arc::new(|| 1_000_000));
        let mut s = UiScript::new().unwrap();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(s.eval::<bool>("return RequestLFDPlayerLockInfo()").unwrap());
        }
        assert_eq!(seen, [true, true, true, false]);
        assert!(s.eval::<bool>("return RequestLFDPartyLockInfo()").unwrap());
        assert_eq!(
            s.take_wrath_lfg_lock_requests(),
            [
                WrathLfgLockRequest::Player,
                WrathLfgLockRequest::Player,
                WrathLfgLockRequest::Player,
                WrathLfgLockRequest::Party,
            ]
        );
        // A new VM sees the same gate: the player gate is still throttled.
        let s2 = UiScript::new().unwrap();
        assert!(!s2
            .eval::<bool>("return RequestLFDPlayerLockInfo()")
            .unwrap());
    }
}
