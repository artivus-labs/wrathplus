//! Gossip and NPC text, the dialog a friendly NPC opens (vmangos `Npc.cpp`, `GossipDef.cpp`).

/// One `SMSG_GOSSIP_MESSAGE` option: `index` is echoed as `gossipListId` on select, `icon` is a
/// `GOSSIP_ICON_*` (0 chat, 1 vendor, 2 taxi, 3 trainer, …), `coded` asks for a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipOption {
    pub index: u32,
    pub icon: u8,
    pub coded: bool,
    pub message: String,
}

/// One quest in `SMSG_GOSSIP_MESSAGE`'s quest list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestOption {
    pub quest_id: u32,
    pub icon: u32,
    pub level: i32,
    pub title: String,
}

/// `CMSG_NPC_TEXT_QUERY` body: the text id, then the NPC's guid (`Npc.cpp:8-12`).
pub fn npc_text_query(text_id: u32, guid: u64) -> Vec<u8> {
    let mut body = Vec::with_capacity(12);
    body.extend_from_slice(&text_id.to_le_bytes());
    body.extend_from_slice(&guid.to_le_bytes());
    body
}

/// One of the eight greeting variants of an `SMSG_NPC_TEXT_UPDATE` record, both gender columns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NpcTextBlock {
    /// The block's draw weight, not a rank; vmangos's fallback record sets all eight to 0.
    pub probability: f32,
    /// `text0`, shown when the NPC is male or genderless.
    pub male: String,
    /// `text1`, shown when the NPC is female.
    pub female: String,
}

impl NpcTextBlock {
    fn column(&self, female: bool) -> &str {
        if female {
            &self.female
        } else {
            &self.male
        }
    }
}

/// The greeting blocks in every `SMSG_NPC_TEXT_UPDATE` record.
pub const NPC_TEXT_BLOCKS: usize = 8;

/// `SMSG_GOSSIP_POI`, the map flag a guard drops when asked for directions; the server sends it
/// unasked from a gossip option (`Player::OnGossipSelect`). The 1.12 client treats it as an
/// `AreaPOI.dbc` record in minimap blip slot 1 (`0x4e2840`), hence the DBC field names.
#[derive(Debug, Clone, PartialEq)]
pub struct GossipPoi {
    /// As `AreaPOI.dbc` `Flags`: bit 0 makes it a candidate, bit 1 draws the in-range icon; every
    /// vmangos row sends 99.
    pub flags: u32,
    /// World x and y; the wire carries no z.
    pub pos: [f32; 2],
    /// The `POIIcons.blp` cell (8×8 grid, drawn only below 64); every vmangos row uses 6, the red
    /// flag (`GossipDef.h:113`).
    pub icon: u32,
    /// vmangos `points_of_interest.data`, 0 in every row.
    pub data: u32,
    /// The destination's name, the marker's tooltip.
    pub name: String,
}

/// Pick the greeting as the 1.12 client does (`0x4e2010`): a weighted draw over the blocks with
/// text in the NPC's gender column (female only at 1), never falling back to the other column.
/// `roll` is uniform in `[1.0, 2.0)`, `<=` makes an all-zero record pick block 0, and the sums
/// are `f64` as on the client's x87 stack. `None` is its "Missing gossip text!".
pub fn select_greeting(blocks: &[NpcTextBlock], npc_gender: u8, roll: f32) -> Option<&str> {
    let female = npc_gender == 1;
    let drawn = || blocks.iter().filter(|b| !b.column(female).is_empty());
    let sum: f64 = drawn().map(|b| b.probability as f64).sum();
    let threshold = (2.0 - roll as f64) * sum;
    let mut acc = 0.0f64;
    for block in drawn() {
        acc += block.probability as f64;
        if threshold <= acc {
            return Some(block.column(female));
        }
    }
    None
}
