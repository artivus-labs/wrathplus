//! Test helpers that tests in other layers use: the root's `client_tests` and the `ui_native`
//! feeds. A `#[cfg(test)]` item cannot be seen from another crate, so once `net` is its own crate
//! these are compiled for its tests and, behind its `test-support` feature, for its dependents'
//! (`docs/APP-CRATE-SPLIT-PLAN.md`, the core and net move dry run).
// Under `test-support` outside a test build nothing in this crate calls them yet: their callers are
// other crates' tests once the split makes them `pub`.
#![cfg_attr(not(test), allow(dead_code))]
use bevy::prelude::*;
use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};

/// Decodes one build-12340 body through `adapter` and runs the session events through the
/// handler table, as the drain does.
pub fn dispatch_opcode(app: &mut App, adapter: &mut WrathEventAdapter, opcode: u16, body: &[u8]) {
    if let WrathDispatch::Handled { session_events, .. } =
        adapter.dispatch(opcode, body, 0).unwrap()
    {
        crate::net::handlers::dispatch(app.world_mut(), session_events);
    }
}

/// `SMSG 0x329`, the instance-control family's one receipt.
pub mod instance_control {
    use super::*;

    pub fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, body: &[u8]) {
        dispatch_opcode(app, adapter, 0x329, body);
    }
}

/// The item/inventory receipts.
pub mod inventory {
    use super::*;
    use wrathplus_protocol::wrath_inventory::Packet;

    pub fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, opcode: u16, body: &[u8]) {
        dispatch_opcode(app, adapter, opcode, body);
    }

    /// The capture FIFO, in order.
    pub fn captured(app: &App) -> Vec<Packet> {
        app.world()
            .resource::<crate::net::wrath_inventory_capture::NativeInventoryPackets>()
            .0
            .iter()
            .cloned()
            .collect()
    }
}

/// `SMSG 0x402`, the mirror-image reply.
pub mod mirror_image {
    use super::*;

    pub fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, body: &[u8]) {
        dispatch_opcode(app, adapter, 0x402, body);
    }
}

/// The death and corpse receipts.
pub mod death {
    use super::*;
    use wrathplus_protocol::wrath_death::Packet;

    pub fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, opcode: u16, body: &[u8]) {
        dispatch_opcode(app, adapter, opcode, body);
    }

    /// The capture FIFO, in order.
    pub fn captured(app: &App) -> Vec<Packet> {
        app.world()
            .resource::<crate::net::wrath_death_capture::NativeDeathPackets>()
            .0
            .iter()
            .cloned()
            .collect()
    }
}

/// `SMSG 0x501`, the completed-quest reply.
pub mod quests_completed {
    use super::*;

    pub fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, body: &[u8]) {
        dispatch_opcode(app, adapter, 0x501, body);
    }
}

/// `SMSG 0x4F7`, the UI timer reply.
pub mod ui_timer {
    use super::*;

    pub fn dispatch(app: &mut App, adapter: &mut WrathEventAdapter, body: &[u8]) {
        dispatch_opcode(app, adapter, 0x4f7, body);
    }
}

/// The vehicle fixtures' guids.
pub mod vehicle {
    /// The fixture vehicle we drive.
    pub const CAR: u64 = 0xF150_0000_0000_0007;
}

/// The native mover-order reader path (these orders bypass WrathEventAdapter).
/// Decode the raw body and publish the same event as the socket reader, without a socket.
pub fn dispatch_mover_order(app: &mut App, opcode: u16, body: &[u8]) {
    let order = wrathplus_protocol::wrath_authority::decode_mover_order(opcode, body).unwrap();
    let event = super::io::wrath::mover_order_event(&order).unwrap();
    super::handlers::dispatch(app.world_mut(), vec![event]);
}
