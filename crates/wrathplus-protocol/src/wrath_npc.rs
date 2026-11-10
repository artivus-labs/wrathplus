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
        let box_money = u32_le(r)?;
        let message = cstring(r)?;
        let box_text = cstring(r)?;
        options.push(GossipOption {
            index,
            icon,
            coded,
            message,
            box_money,
            box_text,
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

/// `CMSG_GOSSIP_SELECT_OPTION`: `u64 guid, u32 menuId, u32 option`, then the code as a string
/// for a coded option only (12340 `0x58b023`-`0x58b030`); the menu id is the open menu's.
pub fn encode_gossip_select_option(guid: u64, option: u32, code: Option<&str>) -> Vec<u8> {
    let mut out = guid.to_le_bytes().to_vec();
    out.extend(GOSSIP_MENU_ID.load(Ordering::Relaxed).to_le_bytes());
    out.extend(option.to_le_bytes());
    if let Some(code) = code {
        out.extend(code.as_bytes());
        out.push(0);
    }
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
#[path = "wrath_npc_tests.rs"]
mod tests;
