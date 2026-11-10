//! Build 12340's NPC conversation messages: gossip, the quest giver panels, the quest template and
//! the giver markers. Each decodes to the shared [`SessionEvent`]s the Vanilla decoder produces,
//! so the client's gossip, quest and marker consumers are unchanged. Layouts from AzerothCore
//! (`GossipDef.cpp`, `QuestPackets.cpp`, `QuestHandler.cpp`, `MiscHandler.cpp`).
//!
//! The messages whose body is unchanged since 1.12 (NPC text, gossip complete and POI, the quest
//! update notices, invalid/failed, the log-full notice, the push result) decode here too, strictly:
//! a body must end where its layout does. `SMSG_QUESTUPDATE_ADD_ITEM` is empty from AzerothCore
//! (`Player::SendQuestUpdateAddItem`), where 1.12 carried the item and count.

use std::io::{self, Cursor};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::events::SessionEvent;
use crate::types::{
    GossipOption, GossipPoi, NpcTextBlock, QuestComplete, QuestConfirmAccept, QuestDetails,
    QuestGiverList, QuestListEntry, QuestObjective, QuestOfferReward, QuestRequestItems,
    QuestRewardItem, QuestShareMsg, QuestTemplate, WrathQuestInfo,
};
use crate::wrath_session::{cstring, f32_le, u32_le, u64_le, u8_le};

pub const SMSG_QUEST_QUERY_RESPONSE: u16 = 0x005D;
pub const SMSG_GOSSIP_MESSAGE: u16 = 0x017D;
pub const SMSG_GOSSIP_COMPLETE: u16 = 0x017E;
pub const SMSG_NPC_TEXT_UPDATE: u16 = 0x0180;
pub const SMSG_QUESTGIVER_STATUS: u16 = 0x0183;
pub const SMSG_QUESTGIVER_QUEST_LIST: u16 = 0x0185;
pub const SMSG_QUESTGIVER_QUEST_DETAILS: u16 = 0x0188;
pub const SMSG_QUESTGIVER_REQUEST_ITEMS: u16 = 0x018B;
pub const SMSG_QUESTGIVER_OFFER_REWARD: u16 = 0x018D;
pub const SMSG_QUESTGIVER_QUEST_INVALID: u16 = 0x018F;
pub const SMSG_QUESTGIVER_QUEST_COMPLETE: u16 = 0x0191;
pub const SMSG_QUESTGIVER_QUEST_FAILED: u16 = 0x0192;
pub const SMSG_QUESTLOG_FULL: u16 = 0x0195;
pub const SMSG_QUESTUPDATE_FAILED: u16 = 0x0196;
pub const SMSG_QUESTUPDATE_FAILEDTIMER: u16 = 0x0197;
pub const SMSG_QUESTUPDATE_COMPLETE: u16 = 0x0198;
pub const SMSG_QUESTUPDATE_ADD_KILL: u16 = 0x0199;
pub const SMSG_QUESTUPDATE_ADD_ITEM: u16 = 0x019A;
pub const SMSG_QUEST_CONFIRM_ACCEPT: u16 = 0x019C;
pub const SMSG_GOSSIP_POI: u16 = 0x0224;
pub const MSG_QUEST_PUSH_RESULT: u16 = 0x0276;
pub const SMSG_QUESTGIVER_STATUS_MULTIPLE: u16 = 0x0418;

pub const CMSG_QUEST_QUERY: u32 = 0x005C;
pub const CMSG_GOSSIP_HELLO: u32 = 0x017B;
pub const CMSG_GOSSIP_SELECT_OPTION: u32 = 0x017C;
pub const CMSG_NPC_TEXT_QUERY: u32 = 0x017F;
pub const CMSG_QUESTGIVER_STATUS_QUERY: u32 = 0x0182;
pub const CMSG_QUESTGIVER_HELLO: u32 = 0x0184;
pub const CMSG_QUESTGIVER_QUERY_QUEST: u32 = 0x0186;
pub const CMSG_QUESTGIVER_ACCEPT_QUEST: u32 = 0x0189;
pub const CMSG_QUESTGIVER_COMPLETE_QUEST: u32 = 0x018A;
pub const CMSG_QUESTGIVER_REQUEST_REWARD: u32 = 0x018C;
pub const CMSG_QUESTGIVER_CHOOSE_REWARD: u32 = 0x018E;
pub const CMSG_QUESTLOG_REMOVE_QUEST: u32 = 0x0194;
pub const CMSG_QUEST_CONFIRM_ACCEPT: u32 = 0x019B;
pub const CMSG_PUSHQUESTTOPARTY: u32 = 0x019D;
pub const CMSG_QUESTGIVER_STATUS_MULTIPLE_QUERY: u32 = 0x0417;

/// The open gossip menu's id: 2.4 added it to `CMSG_GOSSIP_SELECT_OPTION`, where the client's
/// command still names only the option. Written on each `SMSG_GOSSIP_MESSAGE`; one session has
/// one gossip window.
static GOSSIP_MENU_ID: AtomicU32 = AtomicU32::new(0);

/// Whether this module decodes `opcode`.
pub const fn handles(opcode: u16) -> bool {
    matches!(
        opcode,
        SMSG_QUEST_QUERY_RESPONSE
            | SMSG_GOSSIP_MESSAGE
            | SMSG_GOSSIP_COMPLETE
            | SMSG_NPC_TEXT_UPDATE
            | SMSG_QUESTGIVER_STATUS
            | SMSG_QUESTGIVER_QUEST_LIST
            | SMSG_QUESTGIVER_QUEST_DETAILS
            | SMSG_QUESTGIVER_REQUEST_ITEMS
            | SMSG_QUESTGIVER_OFFER_REWARD
            | SMSG_QUESTGIVER_QUEST_INVALID
            | SMSG_QUESTGIVER_QUEST_COMPLETE
            | SMSG_QUESTGIVER_QUEST_FAILED
            | SMSG_QUESTLOG_FULL
            | SMSG_QUESTUPDATE_FAILED
            | SMSG_QUESTUPDATE_FAILEDTIMER
            | SMSG_QUESTUPDATE_COMPLETE
            | SMSG_QUESTUPDATE_ADD_KILL
            | SMSG_QUESTUPDATE_ADD_ITEM
            | SMSG_QUEST_CONFIRM_ACCEPT
            | SMSG_GOSSIP_POI
            | MSG_QUEST_PUSH_RESULT
            | SMSG_QUESTGIVER_STATUS_MULTIPLE
    )
}

/// Decode one of the [`handles`] opcodes.
pub fn decode(opcode: u16, body: &[u8]) -> io::Result<Vec<SessionEvent>> {
    let mut r = Cursor::new(body);
    Ok(match opcode {
        SMSG_GOSSIP_MESSAGE => gossip_message(&mut r)?,
        SMSG_QUESTGIVER_STATUS => {
            let npc = u64_le(&mut r)?;
            status_events(npc, u8_le(&mut r)?)
        }
        SMSG_QUESTGIVER_STATUS_MULTIPLE => {
            let count = u32_le(&mut r)?;
            let mut out = Vec::new();
            for _ in 0..count {
                let npc = u64_le(&mut r)?;
                out.extend(status_events(npc, u8_le(&mut r)?));
            }
            out
        }
        SMSG_QUESTGIVER_QUEST_LIST => vec![SessionEvent::QuestGreeting(quest_list(&mut r)?)],
        SMSG_QUESTGIVER_QUEST_DETAILS => vec![SessionEvent::QuestDetail(quest_details(&mut r)?)],
        SMSG_QUESTGIVER_REQUEST_ITEMS => vec![SessionEvent::QuestProgress(request_items(&mut r)?)],
        SMSG_QUESTGIVER_OFFER_REWARD => vec![SessionEvent::QuestOffer(offer_reward(&mut r)?)],
        SMSG_QUESTGIVER_QUEST_COMPLETE => vec![SessionEvent::QuestComplete(QuestComplete {
            quest_id: u32_le(&mut r)?,
            xp: u32_le(&mut r)?,
            money: u32_le(&mut r)?,
            items: Vec::new(),
        })],
        SMSG_QUEST_QUERY_RESPONSE => {
            vec![SessionEvent::QuestTemplate(Box::new(quest_template(
                &mut r,
            )?))]
        }
        SMSG_GOSSIP_COMPLETE => end(&r, vec![SessionEvent::GossipComplete])?,
        SMSG_NPC_TEXT_UPDATE => {
            let text_id = u32_le(&mut r)?;
            let blocks = npc_text_blocks(&mut r)?;
            end(&r, vec![SessionEvent::NpcGreeting { text_id, blocks }])?
        }
        SMSG_QUESTGIVER_QUEST_INVALID => {
            let reason = u32_le(&mut r)?;
            end(&r, vec![SessionEvent::QuestGiverInvalid { reason }])?
        }
        SMSG_QUESTGIVER_QUEST_FAILED => {
            let (quest_id, reason) = (u32_le(&mut r)?, u32_le(&mut r)?);
            end(
                &r,
                vec![SessionEvent::QuestGiverFailed { quest_id, reason }],
            )?
        }
        SMSG_QUESTLOG_FULL => end(&r, vec![SessionEvent::QuestLogFull])?,
        SMSG_QUESTUPDATE_FAILED | SMSG_QUESTUPDATE_FAILEDTIMER => {
            let quest_id = u32_le(&mut r)?;
            let timed = opcode == SMSG_QUESTUPDATE_FAILEDTIMER;
            end(&r, vec![SessionEvent::QuestFailed { quest_id, timed }])?
        }
        SMSG_QUESTUPDATE_COMPLETE => {
            let quest_id = u32_le(&mut r)?;
            end(&r, vec![SessionEvent::QuestObjectivesComplete { quest_id }])?
        }
        SMSG_QUESTUPDATE_ADD_KILL => {
            let (quest_id, entry, count, required) = (
                u32_le(&mut r)?,
                u32_le(&mut r)?,
                u32_le(&mut r)?,
                u32_le(&mut r)?,
            );
            // The killed unit; the toast keys on the entry.
            let _guid = u64_le(&mut r)?;
            end(
                &r,
                vec![SessionEvent::QuestObjectiveKill {
                    quest_id,
                    entry,
                    count,
                    required,
                }],
            )?
        }
        // Empty from AC; an item and a count when a server fills the optional payload.
        SMSG_QUESTUPDATE_ADD_ITEM => {
            let (item_id, count) = if body.is_empty() {
                (0, 0)
            } else {
                (u32_le(&mut r)?, u32_le(&mut r)?)
            };
            end(
                &r,
                vec![SessionEvent::QuestObjectiveItem { item_id, count }],
            )?
        }
        SMSG_QUEST_CONFIRM_ACCEPT => {
            let confirm = QuestConfirmAccept {
                quest_id: u32_le(&mut r)?,
                title: cstring(&mut r)?,
                sender: u64_le(&mut r)?,
            };
            end(&r, vec![SessionEvent::QuestConfirmAccept(confirm)])?
        }
        SMSG_GOSSIP_POI => {
            let flags = u32_le(&mut r)?;
            let pos = [f32_le(&mut r)?, f32_le(&mut r)?];
            let poi = GossipPoi {
                flags,
                pos,
                icon: u32_le(&mut r)?,
                data: u32_le(&mut r)?,
                name: cstring(&mut r)?,
            };
            end(&r, vec![SessionEvent::GossipPoi(poi)])?
        }
        MSG_QUEST_PUSH_RESULT => {
            let member = u64_le(&mut r)?;
            let msg = QuestShareMsg(u8_le(&mut r)?);
            end(&r, vec![SessionEvent::QuestPushResult { member, msg }])?
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an NPC opcode",
            ))
        }
    })
}

/// The events of a body read to its end; trailing bytes are a layout this decoder does not know.
fn end(r: &Cursor<&[u8]>, events: Vec<SessionEvent>) -> io::Result<Vec<SessionEvent>> {
    if r.position() as usize != r.get_ref().len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "NPC message trailing bytes",
        ));
    }
    Ok(events)
}

/// `SMSG_NPC_TEXT_UPDATE`'s eight blocks (AC `QueryHandler.cpp:284-356`): probability, the two
/// gendered texts, the language and three `(delay, emote)` pairs each.
fn npc_text_blocks(r: &mut Cursor<&[u8]>) -> io::Result<Vec<NpcTextBlock>> {
    (0..crate::types::NPC_TEXT_BLOCKS)
        .map(|_| {
            let probability = f32_le(r)?;
            let male = cstring(r)?;
            let female = cstring(r)?;
            let _language = u32_le(r)?;
            for _ in 0..3 {
                let _delay = u32_le(r)?;
                let _emote = u32_le(r)?;
            }
            Ok(NpcTextBlock {
                probability,
                male,
                female,
            })
        })
        .collect()
}

/// Build 12340's `DIALOG_STATUS_*` onto 1.12's [`crate::types::dialog_status`] values. 3.3.5
/// adds low-level and repeatable tiers: a low-level offer shows no marker (1.12's `CHAT`), a
/// repeatable reward is the yellow `?`.
fn status_events(npc: u64, status: u8) -> Vec<SessionEvent> {
    vec![
        SessionEvent::QuestGiverStatus {
            npc,
            status: dialog_status(status),
        },
        SessionEvent::WrathQuestGiverStatus { npc, status },
    ]
}

fn dialog_status(status: u8) -> u32 {
    use crate::types::dialog_status as d;
    match status {
        1 => d::UNAVAILABLE,
        2 | 4 => d::CHAT,
        3 | 6 => d::REWARD_REP,
        5 => d::INCOMPLETE,
        7 | 8 => d::AVAILABLE,
        // AC QuestDef.h: REWARD2 (9) has no minimap dot; REWARD (10) does.
        // Both retain the yellow world question mark, but only Vanilla REWARD2 dots.
        9 => d::REWARD_OLD,
        10 => d::REWARD2,
        _ => d::NONE,
    }
}

/// `u64 npc, u32 menuId, u32 textId, u32 count` of `{u32 index, u8 icon, u8 coded, u32 boxMoney,
/// text, boxText}`, then `u32 count` of `{u32 quest, u32 icon, i32 level, u32 flags,
/// u8 repeatable, title}`.
fn gossip_message(r: &mut Cursor<&[u8]>) -> io::Result<Vec<SessionEvent>> {
    let npc = u64_le(r)?;
    GOSSIP_MENU_ID.store(u32_le(r)?, Ordering::Relaxed);
    let text_id = u32_le(r)?;
    let mut options = Vec::new();
    for _ in 0..u32_le(r)? {
        let index = u32_le(r)?;
        let icon = u8_le(r)?;
        let coded = u8_le(r)? != 0;
        let _box_money = u32_le(r)?;
        let message = cstring(r)?;
        let _box_text = cstring(r)?;
        options.push(GossipOption {
            index,
            icon,
            coded,
            message,
        });
    }
    let mut quests = Vec::new();
    let mut quest_info = Vec::new();
    for _ in 0..u32_le(r)? {
        let quest = u32_le(r)?;
        let icon = u32_le(r)?;
        let level = u32_le(r)? as i32;
        let flags = u32_le(r)?;
        let repeatable = u8_le(r)? != 0;
        quest_info.push(WrathQuestInfo {
            flags,
            repeatable,
            ..Default::default()
        });
        quests.push((quest, icon, level, cstring(r)?));
    }
    Ok(vec![SessionEvent::GossipMenu {
        npc,
        text_id,
        options,
        quests,
        quest_info,
    }])
}

/// `u64 npc, greeting, u32 emoteDelay, u32 emote, u8 count` of `{u32 quest, u32 icon,
/// i32 level, u32 flags, u8 repeatable, title}`.
fn quest_list(r: &mut Cursor<&[u8]>) -> io::Result<QuestGiverList> {
    let npc = u64_le(r)?;
    let greeting = cstring(r)?;
    let emote_delay = u32_le(r)?;
    let emote = u32_le(r)?;
    let mut quests = Vec::new();
    for _ in 0..u8_le(r)? {
        let quest_id = u32_le(r)?;
        let icon = u32_le(r)?;
        let level = u32_le(r)? as i32;
        let flags = u32_le(r)?;
        let repeatable = u8_le(r)? != 0;
        quests.push(QuestListEntry {
            wrath: Some(WrathQuestInfo {
                flags,
                repeatable,
                ..Default::default()
            }),
            quest_id,
            icon,
            level,
            title: cstring(r)?,
        });
    }
    Ok(QuestGiverList {
        npc,
        greeting,
        emote_delay,
        emote,
        quests,
    })
}

/// A counted `{u32 item, u32 count, u32 displayId}` block.
fn item_block(r: &mut Cursor<&[u8]>) -> io::Result<Vec<QuestRewardItem>> {
    let mut out = Vec::new();
    for _ in 0..u32_le(r)? {
        out.push(QuestRewardItem {
            item_id: u32_le(r)?,
            count: u32_le(r)?,
            display_id: u32_le(r)?,
        });
    }
    Ok(out)
}

/// `u64 npc, u64 sharer, u32 quest`, title, details, objectives, `u8 autoAccept, u32 flags,
/// u32 suggestedPlayers, u8 isFinished`, the choice and reward blocks, `u32 money, u32 xp`, then
/// honor, titles, talents, reputations and emotes, which 1.12's panel does not show.
fn quest_details(r: &mut Cursor<&[u8]>) -> io::Result<QuestDetails> {
    let npc = u64_le(r)?;
    let _sharer = u64_le(r)?;
    let mut native = WrathQuestInfo::default();
    let quest_id = u32_le(r)?;
    let title = cstring(r)?;
    let details = cstring(r)?;
    let objectives = cstring(r)?;
    native.auto_accept = u8_le(r)? != 0;
    let auto_finish = 0;
    native.flags = u32_le(r)?;
    native.suggested_players = u32_le(r)?;
    native.finished = u8_le(r)? != 0;
    let choices = item_block(r)?;
    let rewards = item_block(r)?;
    let money = u32_le(r)? as i32;
    native.reward_xp = u32_le(r)?;
    native.reward_honor = u32_le(r)?;
    native.honor_multiplier = f32_le(r)?;
    let reward_spell = u32_le(r)?;
    let _spell_cast = u32_le(r)?;
    native.title_id = u32_le(r)?;
    native.bonus_talents = u32_le(r)?;
    native.arena_points = u32_le(r)?;
    Ok(QuestDetails {
        wrath: Some(native),
        npc,
        quest_id,
        title,
        details,
        objectives,
        auto_finish,
        choices,
        rewards,
        money,
        reward_spell,
    })
}

/// `u64 npc, u32 quest`, title, text, `u8 autoFinish, u32 flags, u32 suggestedPlayers`, a counted
/// run of `{u32 delay, u32 emote}`, the choice and reward blocks, `u32 money, u32 xp,
/// u32 honor, f32, u32 0x08, u32 spell`, then more 3.3.5 reward columns.
fn offer_reward(r: &mut Cursor<&[u8]>) -> io::Result<QuestOfferReward> {
    let npc = u64_le(r)?;
    let mut native = WrathQuestInfo::default();
    let quest_id = u32_le(r)?;
    let title = cstring(r)?;
    let offer_text = cstring(r)?;
    let auto_finish = u32::from(u8_le(r)?);
    let quest_flags = u32_le(r)?;
    native.flags = quest_flags;
    native.suggested_players = u32_le(r)?;
    for _ in 0..u32_le(r)? {
        let _delay = u32_le(r)?;
        let _emote = u32_le(r)?;
    }
    let choices = item_block(r)?;
    let rewards = item_block(r)?;
    let money = u32_le(r)? as i32;
    native.reward_xp = u32_le(r)?;
    native.reward_honor = u32_le(r)?;
    native.honor_multiplier = f32_le(r)?;
    let _unused = u32_le(r)?;
    let reward_spell = u32_le(r)?;
    let _spell_cast = u32_le(r)?;
    native.title_id = u32_le(r)?;
    native.bonus_talents = u32_le(r)?;
    native.arena_points = u32_le(r)?;
    Ok(QuestOfferReward {
        wrath: Some(native),
        npc,
        quest_id,
        title,
        offer_text,
        auto_finish,
        choices,
        rewards,
        money,
        quest_flags,
        reward_spell,
    })
}

/// `u64 npc, u32 quest`, title, text, `u32 0, u32 emote, u32 closeOnCancel, u32 flags,
/// u32 suggestedPlayers, u32 money`, the required items, then `u32 complete ? 3 : 0` and three
/// constant words.
fn request_items(r: &mut Cursor<&[u8]>) -> io::Result<QuestRequestItems> {
    let npc = u64_le(r)?;
    let mut native = WrathQuestInfo::default();
    let quest_id = u32_le(r)?;
    let title = cstring(r)?;
    let request_text = cstring(r)?;
    let _emote_delay = u32_le(r)?;
    let emote = u32_le(r)?;
    let close_on_cancel = u32_le(r)?;
    native.flags = u32_le(r)?;
    native.suggested_players = u32_le(r)?;
    let required_money = u32_le(r)?;
    let required_items = item_block(r)?;
    let is_complete = u32_le(r)? != 0;
    // AC GossipDef.cpp writes three required status words after completion.
    for _ in 0..3 {
        let _status = u32_le(r)?;
    }
    Ok(QuestRequestItems {
        wrath: Some(native),
        npc,
        quest_id,
        title,
        request_text,
        emote,
        close_on_cancel,
        required_money,
        required_items,
        is_complete,
    })
}

/// `SMSG_QUEST_QUERY_RESPONSE`, AzerothCore `SendQuestQueryResponse`. 3.3.5 carries six required
/// items where 1.12's objectives carry four, one per objective row.
fn quest_template(r: &mut Cursor<&[u8]>) -> io::Result<QuestTemplate> {
    let mut native = WrathQuestInfo::default();
    let quest_id = u32_le(r)?;
    let method = u32_le(r)?;
    let level = u32_le(r)? as i32;
    let _min_level = u32_le(r)?;
    let zone_or_sort = u32_le(r)? as i32;
    let quest_type = u32_le(r)?;
    native.suggested_players = u32_le(r)?;
    let rep_objective_faction = u32_le(r)?;
    let rep_objective_value = u32_le(r)?;
    let _rep_objective_faction2 = u32_le(r)?;
    let _rep_objective_value2 = u32_le(r)?;
    let next_quest_in_chain = u32_le(r)?;
    native.reward_xp_id = u32_le(r)?;
    let money = u32_le(r)? as i32;
    let money_max_level = u32_le(r)?;
    let reward_spell = u32_le(r)?;
    let _reward_spell_cast = u32_le(r)?;
    native.reward_honor = u32_le(r)?;
    native.honor_multiplier = f32_le(r)?;
    let src_item_id = u32_le(r)?;
    let flags = u32_le(r)?;
    native.flags = flags;
    native.title_id = u32_le(r)?;
    native.players_slain = u32_le(r)?;
    native.bonus_talents = u32_le(r)?;
    native.arena_points = u32_le(r)?;
    native.reward_rep_show_mask = u32_le(r)?;
    let mut rewards = [(0, 0); 4];
    for slot in &mut rewards {
        *slot = (u32_le(r)?, u32_le(r)?);
    }
    let mut choices = [(0, 0); 6];
    for slot in &mut choices {
        *slot = (u32_le(r)?, u32_le(r)?);
    }
    for slot in &mut native.reward_factions {
        slot.0 = u32_le(r)?;
    }
    for slot in &mut native.reward_factions {
        slot.1 = u32_le(r)? as i32;
    }
    for slot in &mut native.reward_factions {
        slot.2 = u32_le(r)? as i32;
    }
    let point_map_id = u32_le(r)?;
    let point_x = f32_le(r)?;
    let point_y = f32_le(r)?;
    let point_opt = u32_le(r)?;
    let title = cstring(r)?;
    let objectives_text = cstring(r)?;
    let details = cstring(r)?;
    native.area_description = cstring(r)?;
    let end_text = cstring(r)?;
    let mut objectives: [QuestObjective; 4] = std::array::from_fn(|_| QuestObjective {
        creature_or_go: 0,
        required_count: 0,
        item_id: 0,
        item_count: 0,
        text: String::new(),
    });
    for (objective, drop) in objectives.iter_mut().zip(native.item_drops.iter_mut()) {
        objective.creature_or_go = u32_le(r)?;
        objective.required_count = u32_le(r)?;
        *drop = (u32_le(r)?, u32_le(r)?);
    }
    for i in 0..6 {
        let (item, count) = (u32_le(r)?, u32_le(r)?);
        if let Some(objective) = objectives.get_mut(i) {
            objective.item_id = item;
            objective.item_count = count;
        }
    }
    for objective in &mut objectives {
        objective.text = cstring(r)?;
    }
    Ok(QuestTemplate {
        wrath: Some(native),
        quest_id,
        method,
        level,
        zone_or_sort,
        quest_type,
        rep_objective_faction,
        rep_objective_value,
        next_quest_in_chain,
        money,
        money_max_level,
        reward_spell,
        src_item_id,
        flags,
        rewards,
        choices,
        point_map_id,
        point_x,
        point_y,
        point_opt,
        title,
        objectives_text,
        details,
        end_text,
        objectives,
    })
}

fn guid_quest(guid: u64, quest: u32) -> Vec<u8> {
    let mut out = guid.to_le_bytes().to_vec();
    out.extend(quest.to_le_bytes());
    out
}

/// `CMSG_GOSSIP_SELECT_OPTION`: `u64 guid, u32 menuId, u32 option`; the menu id is the open
/// menu's.
pub fn encode_gossip_select_option(guid: u64, option: u32) -> Vec<u8> {
    let mut out = guid.to_le_bytes().to_vec();
    out.extend(GOSSIP_MENU_ID.load(Ordering::Relaxed).to_le_bytes());
    out.extend(option.to_le_bytes());
    out
}

/// `CMSG_NPC_TEXT_QUERY`: `u32 textId, u64 guid`.
pub fn encode_npc_text_query(text_id: u32, guid: u64) -> Vec<u8> {
    let mut out = text_id.to_le_bytes().to_vec();
    out.extend(guid.to_le_bytes());
    out
}

/// `CMSG_QUESTGIVER_QUERY_QUEST`: `u64 guid, u32 quest, u8 0`.
pub fn encode_questgiver_query_quest(npc: u64, quest: u32) -> Vec<u8> {
    let mut out = guid_quest(npc, quest);
    out.push(0);
    out
}

/// `CMSG_QUESTGIVER_ACCEPT_QUEST`: `u64 guid, u32 quest, u32 0`.
pub fn encode_questgiver_accept_quest(npc: u64, quest: u32) -> Vec<u8> {
    let mut out = guid_quest(npc, quest);
    out.extend(0u32.to_le_bytes());
    out
}

/// `u64 guid, u32 quest`: complete and request-reward.
pub fn encode_guid_quest(npc: u64, quest: u32) -> Vec<u8> {
    guid_quest(npc, quest)
}

/// `CMSG_QUESTGIVER_CHOOSE_REWARD`: `u64 guid, u32 quest, u32 choice`.
pub fn encode_questgiver_choose_reward(npc: u64, quest: u32, choice: u32) -> Vec<u8> {
    let mut out = guid_quest(npc, quest);
    out.extend(choice.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cstr(out: &mut Vec<u8>, s: &str) {
        out.extend(s.as_bytes());
        out.push(0);
    }

    #[test]
    fn a_gossip_menu_keeps_its_options_and_quests_and_its_id_rides_the_select() {
        let mut b = 0x42u64.to_le_bytes().to_vec();
        b.extend(7u32.to_le_bytes()); // menu id
        b.extend(100u32.to_le_bytes()); // text id
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend([3, 0]);
        b.extend(0u32.to_le_bytes());
        cstr(&mut b, "Train me.");
        cstr(&mut b, "");
        b.extend(1u32.to_le_bytes());
        b.extend(783u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.push(0);
        cstr(&mut b, "A Threat Within");
        let events = decode(SMSG_GOSSIP_MESSAGE, &b).unwrap();
        let [SessionEvent::GossipMenu {
            npc,
            text_id,
            options,
            quests,
            quest_info,
        }] = &events[..]
        else {
            panic!("{events:?}")
        };
        assert_eq!((*npc, *text_id), (0x42, 100));
        assert_eq!(options[0].message, "Train me.");
        assert_eq!(quest_info.len(), 1);
        assert_eq!(quest_info[0].flags, 0);
        assert_eq!(quests[0], (783, 2, 1, "A Threat Within".to_string()));
        let select = encode_gossip_select_option(0x42, 0);
        assert_eq!(&select[8..12], &7u32.to_le_bytes());
    }

    #[test]
    fn reward_statuses_keep_native_minimap_dot_distinction() {
        use crate::types::dialog_status as d;
        let guid = 0xf130_0001_2300_4567u64;
        for (native, expected) in [(9, d::REWARD_OLD), (10, d::REWARD2)] {
            let mut single = guid.to_le_bytes().to_vec();
            single.push(native);
            assert!(
                matches!(decode(SMSG_QUESTGIVER_STATUS, &single).unwrap()[0],
                SessionEvent::QuestGiverStatus { npc, status } if npc == guid && status == expected)
            );
            let mut multiple = 1u32.to_le_bytes().to_vec();
            multiple.extend(&single);
            assert!(
                matches!(decode(SMSG_QUESTGIVER_STATUS_MULTIPLE, &multiple).unwrap()[0],
                SessionEvent::QuestGiverStatus { npc, status } if npc == guid && status == expected)
            );
            for cut in 0..single.len() {
                assert!(decode(SMSG_QUESTGIVER_STATUS, &single[..cut]).is_err());
            }
            for cut in 0..multiple.len() {
                assert!(decode(SMSG_QUESTGIVER_STATUS_MULTIPLE, &multiple[..cut]).is_err());
            }
        }
    }

    #[test]
    fn giver_markers_map_onto_the_1_12_statuses() {
        use crate::types::dialog_status as d;
        let mut b = 2u32.to_le_bytes().to_vec();
        b.extend(1u64.to_le_bytes());
        b.push(8);
        b.extend(2u64.to_le_bytes());
        b.push(10);
        let events = decode(SMSG_QUESTGIVER_STATUS_MULTIPLE, &b).unwrap();
        assert!(
            matches!(events[0], SessionEvent::QuestGiverStatus { npc: 1, status } if status == d::AVAILABLE)
        );
        assert!(
            matches!(events[2], SessionEvent::QuestGiverStatus { npc: 2, status } if status == d::REWARD2)
        );
    }

    #[test]
    fn quest_details_read_through_the_3_3_5_header_to_money_and_spell() {
        let mut b = 0x42u64.to_le_bytes().to_vec();
        b.extend(0u64.to_le_bytes());
        b.extend(783u32.to_le_bytes());
        cstr(&mut b, "A Threat Within");
        cstr(&mut b, "details");
        cstr(&mut b, "objectives");
        b.push(0);
        b.extend(0u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.push(0);
        b.extend(0u32.to_le_bytes()); // choices
        b.extend(1u32.to_le_bytes()); // rewards
        b.extend([11u32, 1, 99].iter().flat_map(|v| v.to_le_bytes()));
        b.extend(25u32.to_le_bytes()); // money
        b.extend(40u32.to_le_bytes()); // xp
        b.extend(0u32.to_le_bytes()); // honor
        b.extend(0f32.to_le_bytes());
        b.extend(0u32.to_le_bytes()); // spell
        b.extend([0u32, 12, 3, 5].iter().flat_map(|v| v.to_le_bytes()));
        let events = decode(SMSG_QUESTGIVER_QUEST_DETAILS, &b).unwrap();
        let [SessionEvent::QuestDetail(d)] = &events[..] else {
            panic!("{events:?}")
        };
        assert_eq!((d.quest_id, d.money, d.rewards[0].item_id), (783, 25, 11));
        assert_eq!(d.objectives, "objectives");
        let native = d.wrath.as_ref().unwrap();
        assert_eq!(
            (
                native.reward_xp,
                native.title_id,
                native.bonus_talents,
                native.arena_points
            ),
            (40, 12, 3, 5)
        );
        for cut in 0..b.len() {
            assert!(decode(SMSG_QUESTGIVER_QUEST_DETAILS, &b[..cut]).is_err());
        }
    }
    #[test]
    fn greeting_retains_flags_repeatable_and_details_auto_accept_is_not_auto_finish() {
        let mut b = 1u64.to_le_bytes().to_vec();
        cstr(&mut b, "hello");
        b.extend([0u32, 0].iter().flat_map(|v| v.to_le_bytes()));
        b.push(1);
        b.extend([7u32, 4, 80, 0x1000].iter().flat_map(|v| v.to_le_bytes()));
        b.push(1);
        cstr(&mut b, "daily");
        let [SessionEvent::QuestGreeting(list)] =
            &decode(SMSG_QUESTGIVER_QUEST_LIST, &b).unwrap()[..]
        else {
            panic!()
        };
        let q = list.quests[0].wrath.as_ref().unwrap();
        assert_eq!(q.flags, 0x1000);
        assert!(q.repeatable);

        let mut b = 1u64.to_le_bytes().to_vec();
        b.extend(0u64.to_le_bytes());
        b.extend(7u32.to_le_bytes());
        for text in ["quest", "details", "objectives"] {
            cstr(&mut b, text);
        }
        b.push(1);
        b.extend([0x1000u32, 5].iter().flat_map(|v| v.to_le_bytes()));
        b.push(1);
        b.extend([0u32, 0, 10, 100, 250].iter().flat_map(|v| v.to_le_bytes()));
        b.extend(0.5f32.to_le_bytes());
        b.extend([12u32, 0, 13, 2, 3].iter().flat_map(|v| v.to_le_bytes()));
        let [SessionEvent::QuestDetail(d)] =
            &decode(SMSG_QUESTGIVER_QUEST_DETAILS, &b).unwrap()[..]
        else {
            panic!()
        };
        assert_eq!(d.auto_finish, 0);
        let q = d.wrath.as_ref().unwrap();
        assert!(q.auto_accept && q.finished);
        assert_eq!(
            (q.suggested_players, q.reward_honor, q.honor_multiplier),
            (5, 250, 0.5)
        );
    }
    #[test]
    fn offer_and_progress_retain_native_columns_at_their_own_offsets() {
        let mut b = 1u64.to_le_bytes().to_vec();
        b.extend(7u32.to_le_bytes());
        cstr(&mut b, "quest");
        cstr(&mut b, "reward");
        b.push(1);
        b.extend(
            [0x1000u32, 5, 1, 25, 4, 0, 0, 100, 900, 250]
                .iter()
                .flat_map(|v| v.to_le_bytes()),
        );
        b.extend(0.5f32.to_le_bytes());
        b.extend([8u32, 12, 0, 13, 2, 3].iter().flat_map(|v| v.to_le_bytes()));
        let [SessionEvent::QuestOffer(o)] = &decode(SMSG_QUESTGIVER_OFFER_REWARD, &b).unwrap()[..]
        else {
            panic!()
        };
        let q = o.wrath.as_ref().unwrap();
        assert_eq!(o.auto_finish, 1);
        assert_eq!(
            (o.money, o.reward_spell, q.flags, q.suggested_players),
            (100, 12, 0x1000, 5)
        );
        assert_eq!(
            (
                q.reward_xp,
                q.reward_honor,
                q.title_id,
                q.bonus_talents,
                q.arena_points
            ),
            (900, 250, 13, 2, 3)
        );
        for cut in 0..b.len() {
            assert!(decode(SMSG_QUESTGIVER_OFFER_REWARD, &b[..cut]).is_err());
        }

        let mut b = 1u64.to_le_bytes().to_vec();
        b.extend(7u32.to_le_bytes());
        cstr(&mut b, "quest");
        cstr(&mut b, "progress");
        b.extend(
            [0u32, 4, 1, 0x1000, 5, 100, 0, 3, 4, 8, 16]
                .iter()
                .flat_map(|v| v.to_le_bytes()),
        );
        let [SessionEvent::QuestProgress(p)] =
            &decode(SMSG_QUESTGIVER_REQUEST_ITEMS, &b).unwrap()[..]
        else {
            panic!()
        };
        assert!(p.is_complete);
        assert_eq!(p.required_money, 100);
        let q = p.wrath.as_ref().unwrap();
        assert_eq!((q.flags, q.suggested_players), (0x1000, 5));
        for cut in 0..b.len() {
            assert!(decode(SMSG_QUESTGIVER_REQUEST_ITEMS, &b[..cut]).is_err());
        }
    }

    #[test]
    fn a_quest_template_keeps_its_pvp_objective() {
        // The 26 header columns: id, method, level, ..., title id 21, players slain 22.
        let mut header = [0u32; 26];
        header[0] = 9_001;
        header[21] = 7;
        header[22] = 10;
        let mut b: Vec<u8> = header.iter().flat_map(|v| v.to_le_bytes()).collect();
        b.extend([0u8; 4 * (8 + 12 + 15 + 4)]); // rewards, choices, reputations, the POI point
        for s in ["Kill them", "objectives", "details", "", "end"] {
            cstr(&mut b, s);
        }
        b.extend([0u8; 4 * (16 + 12)]); // objective quads, required items
        for _ in 0..4 {
            cstr(&mut b, "");
        }
        let events = decode(SMSG_QUEST_QUERY_RESPONSE, &b).unwrap();
        let [SessionEvent::QuestTemplate(t)] = &events[..] else {
            panic!("one template")
        };
        assert_eq!((t.quest_id, t.title.as_str()), (9_001, "Kill them"));
        let native = t.wrath.as_ref().unwrap();
        assert_eq!((native.title_id, native.players_slain), (7, 10));
        for cut in 0..b.len() {
            assert!(decode(SMSG_QUEST_QUERY_RESPONSE, &b[..cut]).is_err());
        }
    }

    #[test]
    fn the_former_1_12_arms_decode_strictly_and_add_item_is_empty_from_ac() {
        macro_rules! same {
            ($a:expr, $b:expr) => {
                assert_eq!(format!("{:?}", $a), format!("{:?}", $b))
            };
        }
        use SessionEvent as E;
        let one = |op: u16, body: &[u8]| decode(op, body).unwrap();
        same!(one(SMSG_GOSSIP_COMPLETE, &[]), [E::GossipComplete]);
        assert!(decode(SMSG_GOSSIP_COMPLETE, &[0]).is_err());
        same!(one(SMSG_QUESTLOG_FULL, &[]), [E::QuestLogFull]);
        same!(
            one(SMSG_QUESTGIVER_QUEST_INVALID, &13u32.to_le_bytes()),
            [E::QuestGiverInvalid { reason: 13 }]
        );
        same!(
            one(
                SMSG_QUESTGIVER_QUEST_FAILED,
                &[5u32.to_le_bytes(), 4u32.to_le_bytes()].concat()
            ),
            [E::QuestGiverFailed {
                quest_id: 5,
                reason: 4
            }]
        );
        same!(
            one(SMSG_QUESTUPDATE_FAILEDTIMER, &5u32.to_le_bytes()),
            [E::QuestFailed {
                quest_id: 5,
                timed: true
            }]
        );
        same!(
            one(SMSG_QUESTUPDATE_FAILED, &5u32.to_le_bytes()),
            [E::QuestFailed {
                quest_id: 5,
                timed: false
            }]
        );
        same!(
            one(SMSG_QUESTUPDATE_COMPLETE, &5u32.to_le_bytes()),
            [E::QuestObjectivesComplete { quest_id: 5 }]
        );
        let mut kill = Vec::new();
        for v in [5u32, 0x8000_0010, 2, 6] {
            kill.extend(v.to_le_bytes());
        }
        kill.extend(0x99u64.to_le_bytes());
        same!(
            one(SMSG_QUESTUPDATE_ADD_KILL, &kill),
            [E::QuestObjectiveKill {
                quest_id: 5,
                entry: 0x8000_0010,
                count: 2,
                required: 6
            }]
        );
        assert!(
            decode(SMSG_QUESTUPDATE_ADD_KILL, &kill[..16]).is_err(),
            "no guid"
        );
        same!(
            one(SMSG_QUESTUPDATE_ADD_ITEM, &[]),
            [E::QuestObjectiveItem {
                item_id: 0,
                count: 0
            }]
        );
        same!(
            one(
                SMSG_QUESTUPDATE_ADD_ITEM,
                &[2589u32.to_le_bytes(), 3u32.to_le_bytes()].concat()
            ),
            [E::QuestObjectiveItem {
                item_id: 2589,
                count: 3
            }]
        );
        let mut confirm = 5u32.to_le_bytes().to_vec();
        cstr(&mut confirm, "Escort");
        confirm.extend(0x77u64.to_le_bytes());
        assert!(matches!(
            &one(SMSG_QUEST_CONFIRM_ACCEPT, &confirm)[..],
            [E::QuestConfirmAccept(c)] if (c.quest_id, c.title.as_str(), c.sender) == (5, "Escort", 0x77)
        ));
        let mut poi = 99u32.to_le_bytes().to_vec();
        poi.extend(1.5f32.to_le_bytes());
        poi.extend((-2.0f32).to_le_bytes());
        poi.extend(6u32.to_le_bytes());
        poi.extend(0u32.to_le_bytes());
        cstr(&mut poi, "Bank");
        assert!(matches!(
            &one(SMSG_GOSSIP_POI, &poi)[..],
            [E::GossipPoi(p)] if p.pos == [1.5, -2.0] && p.icon == 6 && p.name == "Bank"
        ));
        let push = [&0x66u64.to_le_bytes()[..], &[3]].concat();
        assert!(matches!(
            &one(MSG_QUEST_PUSH_RESULT, &push)[..],
            [E::QuestPushResult { member: 0x66, msg }] if msg.0 == 3
        ));
        let mut text = 77u32.to_le_bytes().to_vec();
        for i in 0..8 {
            text.extend((if i == 0 { 1.0f32 } else { 0.0 }).to_le_bytes());
            cstr(&mut text, if i == 0 { "Hello $N" } else { "" });
            cstr(&mut text, "");
            text.extend([0u8; 4 + 24]);
        }
        let events = one(SMSG_NPC_TEXT_UPDATE, &text);
        let [E::NpcGreeting {
            text_id: 77,
            blocks,
        }] = &events[..]
        else {
            panic!("one greeting")
        };
        same!((blocks.len(), blocks[0].male.as_str()), (8, "Hello $N"));
        assert!(decode(SMSG_NPC_TEXT_UPDATE, &[&text[..], &[0]].concat()).is_err());
    }
}
