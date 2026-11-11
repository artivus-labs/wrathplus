//! The death state the descriptor does not carry, which the wire fills and the world and UI read:
//! the corpse point, the resurrection offer and the reclaim, spirit-healer and corpse-guid
//! latches. Moved here for the crate split (`https://github.com/artivus-labs/wrathplus/wiki/APP-CRATE-SPLIT-PLAN`, step 0 item 3, part
//! 2c); `crate::death` re-exports it and keeps its systems.

use bevy::prelude::*;

/// Where our corpse is, the `MSG_CORPSE_QUERY` answer, in raw WoW coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorpsePoint {
    /// The map to route toward; for a corpse inside an instance, the entrance's.
    pub display_map: i32,
    /// The corpse, or the dungeon entrance standing in for it.
    pub position: [f32; 3],
    /// The corpse's real map id, never adjusted.
    #[allow(dead_code)] // for the map markers
    pub corpse_map: u32,
}

/// A pending resurrection offer (`SMSG_RESURRECT_REQUEST`), the RESURRECT popup's data.
#[derive(Debug, Clone, PartialEq)]
pub struct ResurrectOffer {
    /// Answered in `CMSG_RESURRECT_RESPONSE`.
    pub caster: u64,
    /// Empty on the wire for a player caster, resolved through the name cache.
    pub name: String,
    /// Accepting applies resurrection sickness; picks the popup variant.
    pub sickness: bool,
    /// The reclaim-delay gate still holds: RESURRECT, not RESURRECT_NO_TIMER.
    pub has_timer: bool,
}

/// The death state the descriptor does not carry, filled from the wire and cleared at session
/// teardown.
#[derive(Resource, Default)]
pub struct DeathNet {
    /// When the corpse becomes reclaimable: `SMSG_CORPSE_RECLAIM_DELAY` anchored at arrival.
    pub reclaim_at: Option<f64>,
    /// Bumped per `SMSG_CORPSE_RECLAIM_DELAY`: the 1.12 client's handler re-fires the range
    /// events on each.
    pub reclaim_generation: u32,
    /// The last `MSG_CORPSE_QUERY` answer; a not-found drops it.
    pub corpse: Option<CorpsePoint>,
    /// A pending resurrection offer; cleared when answered or when the popup times out.
    pub resurrect: Option<ResurrectOffer>,
    /// Bumped per `SMSG_RESURRECT_REQUEST`: the offer announces per message (`0x5e7bc0` through
    /// `0x5ded50`), because the stock UI can hide the popup without answering it.
    pub resurrect_generation: u32,
    /// Our streamed corpse object's guid, which `CMSG_RECLAIM_CORPSE` carries like the 1.12 client.
    pub corpse_guid: Option<u64>,
    /// The spirit healer awaiting the XP-loss answer; Accept sends `CMSG_SPIRIT_HEALER_ACTIVATE`.
    pub spirit_healer: Option<u64>,
    /// Bumped per ask: a Cancel clears nothing, so only a fresh ask re-shows the dialog.
    pub confirm_generation: u32,
}

impl DeathNet {
    /// Arm the spirit healer's XP-loss question. The reference raises it on the click with no
    /// packet (`0x5df730`), and vmangos also pushes `SMSG_SPIRIT_HEALER_CONFIRM`; both come here.
    pub fn ask_spirit_healer(&mut self, npc: u64) {
        self.spirit_healer = Some(npc);
        self.confirm_generation = self.confirm_generation.wrapping_add(1);
    }
}
