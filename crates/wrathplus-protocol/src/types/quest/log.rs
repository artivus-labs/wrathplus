//! Quest-log wire (vmangos `Server/Packets/Quest.cpp`): the quest query and response (92/93), log
//! swap, remove and full (403-405), and the `SMSG_QUESTUPDATE_*` progress pushes (406-410).

/// Objective quads the response always writes, the unused tail zero-filled (`QuestDef.h:34-43`).
pub const QUEST_OBJECTIVES_COUNT: u32 = 4;
/// `{itemId, count}` reward slots the response always writes.
pub const QUEST_REWARDS_COUNT: u32 = 4;
/// `{itemId, count}` reward-choice slots the response always writes.
pub const QUEST_REWARD_CHOICES_COUNT: u32 = 6;

/// One objective quad plus its text, which the packet sends after all the quads. `creature_or_go`
/// stays raw: a creature entry, or `(-id)|0x80000000` for a gameobject (`Quest.cpp:512-516`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestObjective {
    pub creature_or_go: u32,
    pub required_count: u32,
    pub item_id: u32,
    pub item_count: u32,
    pub text: String,
}

/// `QuestFlags` (vmangos `QuestDef.h:145-160`): the `QuestTemplate::flags` word, the only place
/// the client learns a quest's kind; the giver panels never carry it.
pub mod quest_flags {
    /// Not used by 1.12 data.
    pub const STAY_ALIVE: u32 = 0x0000_0001;
    /// Escort: one member accepting sends every other eligible one `SMSG_QUEST_CONFIRM_ACCEPT`.
    pub const PARTY_ACCEPT: u32 = 0x0000_0002;
    /// Not used by 1.12 data.
    pub const EXPLORATION: u32 = 0x0000_0004;
    /// Shareable. The client gates Share Quest on it: the server never tests it on the push, only
    /// on the receiver's accept (`Player::CanShareQuest`, `Player.cpp:13959`).
    pub const SHARABLE: u32 = 0x0000_0008;
    /// Not used by 1.12 data.
    pub const EPIC: u32 = 0x0000_0020;
    /// Not used by 1.12 data.
    pub const RAID: u32 = 0x0000_0040;
    /// Rewards ship only in `SMSG_QUESTGIVER_OFFER_REWARD`; DETAILS and this template zero them.
    pub const HIDDEN_REWARDS: u32 = 0x0000_0200;
    /// Granted and completed in one step; never appears in the client's quest log.
    pub const AUTO_REWARDED: u32 = 0x0000_0400;
    /// AzerothCore adds the quest as its detail shows (`QuestHandler.cpp:227`), and 12340's
    /// `QuestGetAutoAccept` (`0x58c430`) answers 1: stock's Accept then only hides the frame.
    pub const AUTO_ACCEPT: u32 = 0x0008_0000;
}

/// `SMSG_QUEST_QUERY_RESPONSE` (vmangos `Quest.cpp:393-528`, the `>1_9_4` branch): the quest
/// template behind the quest log's detail view. Its arrays are fixed-count with no display ids.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestTemplate {
    pub wrath: Option<super::giver::WrathQuestInfo>,
    pub quest_id: u32,
    pub method: u32,
    /// Signed: -1 is a quest at the player's own level (12340 `0x5e0d7a`).
    pub level: i32,
    pub zone_or_sort: i32,
    pub quest_type: u32,
    pub rep_objective_faction: u32,
    pub rep_objective_value: u32,
    pub next_quest_in_chain: u32,
    pub money: i32,
    pub money_max_level: u32,
    pub reward_spell: u32,
    pub src_item_id: u32,
    pub flags: u32,
    pub rewards: [(u32, u32); QUEST_REWARDS_COUNT as usize],
    pub choices: [(u32, u32); QUEST_REWARD_CHOICES_COUNT as usize],
    pub point_map_id: u32,
    pub point_x: f32,
    pub point_y: f32,
    pub point_opt: u32,
    pub title: String,
    pub objectives_text: String,
    pub details: String,
    pub end_text: String,
    pub objectives: [QuestObjective; QUEST_OBJECTIVES_COUNT as usize],
}

/// A creature template's quest links (`SMSG_CREATURE_QUERY_RESPONSE`'s `KillCredit1/2` and
/// `QuestItems[6]`): the unit tooltip's quest lines read them beside the entry itself (12340
/// `0x621070`, creature cache `+0x1c` and `+0x40`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreatureQuestLinks {
    pub kill_credit: [u32; 2],
    pub quest_items: [u32; 6],
}

// ── CMSG encoders ────────────────────────────────────────────────────────────────────────────────

// ── SMSG parsers ─────────────────────────────────────────────────────────────────────────────────
