//! The gossip window's Lua intents, sent: option selects, quest-row clicks and the close.

use bevy::prelude::*;
use wrathplus_ui::script::UiScript;

use super::GossipState;
use crate::net::{ClientCommand, NetCommands};
use crate::ui_quest::{row_is_active, row_is_one_click};

/// Drain the Lua intents: an option sends `CMSG_GOSSIP_SELECT_OPTION` with its wire `index` and,
/// coded, the code the binding took (its confirm and code prompts ran there, `0x58af10`), a quest
/// row its questgiver query, and `CloseGossip` clears locally, as there is no close packet.
pub(super) fn drain_gossip(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<GossipState>,
    commands: Res<NetCommands>,
) {
    let Some(mut script) = script else {
        return;
    };
    for (pos, code) in script.take_gossip_selects() {
        let Some(npc) = state.npc else { continue };
        // Refused while the text query is out, as the reference's `SelectGossipOption`
        // (`0x4e2320`) refuses under the latch `0xbbb670`.
        if state.text_pending() {
            debug!(
                "ui_gossip: SelectGossipOption({pos}) while the text query is in flight — refused"
            );
            continue;
        }
        // Lua position is 1-based; resolve it to the option's wire `index`.
        match pos
            .checked_sub(1)
            .and_then(|i| state.options.get(i as usize))
        {
            Some(opt) => {
                debug!("ui_gossip: select option {} (index {})", pos, opt.index);
                let _ = commands.0.send(ClientCommand::GossipSelectOption {
                    guid: npc,
                    option: opt.index,
                    code: code.filter(|_| opt.coded),
                });
            }
            None => debug!("ui_gossip: SelectGossipOption({pos}) out of range — ignored"),
        }
    }
    // Quest-row clicks: the 1-based row picks the quest.
    for pos in script.take_gossip_quest_selects() {
        let Some(npc) = state.npc else { continue };
        // Refused while pending too; the reference's latch is traced for option selects only.
        if state.text_pending() {
            debug!(
                "ui_gossip: SelectGossipQuest({pos}) while the text query is in flight — refused"
            );
            continue;
        }
        match pos
            .checked_sub(1)
            .and_then(|i| state.quests.get(i as usize))
        {
            Some((quest_id, icon, _level, _title)) => {
                let (quest, icon) = (*quest_id, *icon);
                let active = row_is_active(icon);
                // As on the quest greeting panel: an active row is a turn-in, and so is an
                // available one with its one-click flag.
                let cmd = if active || row_is_one_click(icon) {
                    ClientCommand::QuestgiverComplete { npc, quest }
                } else {
                    ClientCommand::QuestgiverQuery { npc, quest }
                };
                debug!("ui_gossip: quest row {pos} (quest {quest}, icon {icon}, active {active})");
                let _ = commands.0.send(cmd);
            }
            None => debug!("ui_gossip: SelectGossipQuest({pos}) out of range — ignored"),
        }
    }
    if script.take_gossip_close() {
        debug!("ui_gossip: client-side close (no packet)");
        state.clear();
    }
}
