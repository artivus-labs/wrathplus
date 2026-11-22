//! Build 12340's quest done test, `0x5e0ea0(slot, strict)`, which `GetQuestLogTitle`'s
//! `isComplete` (`0x5e5cc0`, strict) and the POI mask `0x5e2950` (lenient) read. It works off the
//! client's own counts: the slot counters for creatures and GameObjects, the bags for items. The
//! slot's COMPLETE bit counts only for an exploration or event quest (`Flags & 6`, `0x5e0fe4`);
//! AzerothCore's item quests reach the turn-in here as soon as the bags hold the items.

use wrathplus_protocol::types::{quest_slot_state, QuestLogSlot, QuestTemplate};

/// The live state the test reads besides the template and the slot.
pub(crate) struct DoneInputs<'a> {
    /// An item's count in the bags (`0x754d00(item, 8)`).
    pub bag_count: &'a dyn Fn(u32) -> u32,
    /// A faction's standing (`0x5d05b0`). Labelled: read as base plus wire standing.
    pub standing: &'a dyn Fn(u32) -> i32,
    /// `PLAYER_FIELD_COINAGE` (`0x51a2b0`).
    pub money: u32,
}

/// `0x5e0ea0`. A failed slot is never done (`0x5e0f51`; the timed-out flag `0xc237bc` is not
/// modelled). `strict` (the second argument) refuses a quest with nothing to do: no exploration
/// flag, reputation, money, creature, item or PvP requirement (`0x5e0fc7`-`0x5e0fd7`).
pub(crate) fn quest_done(
    t: &QuestTemplate,
    slot: &QuestLogSlot,
    strict: bool,
    live: &DoneInputs,
) -> bool {
    if slot.state & quest_slot_state::FAIL != 0 {
        return false;
    }
    let native = t.wrath.as_ref();
    let event = t.flags & 6 != 0;
    let rep_max = native.map_or((0, 0), |w| w.rep_objective_max);
    let players_slain = native.map_or(0, |w| w.players_slain);
    let items: Vec<(u32, u32)> = match native {
        Some(w) => w.required_items.to_vec(),
        None => t
            .objectives
            .iter()
            .map(|o| (o.item_id, o.item_count))
            .collect(),
    };
    let creatures = t.objectives.iter().any(|o| o.creature_or_go != 0);
    let carries = items.iter().any(|&(id, _)| id != 0);
    let bare = !event && t.rep_objective_faction == 0 && rep_max.0 == 0 && t.money == 0;
    if bare && strict && !creatures && !carries && players_slain == 0 {
        return false;
    }
    if event && slot.state & quest_slot_state::COMPLETE == 0 {
        return false;
    }
    let rep_min_met = t.rep_objective_faction == 0
        || (live.standing)(t.rep_objective_faction) >= t.rep_objective_value as i32;
    let rep_max_met = rep_max.0 == 0 || (live.standing)(rep_max.0) <= rep_max.1;
    let money_met = t.money >= 0 || live.money >= t.money.unsigned_abs();
    let counters_met = t
        .objectives
        .iter()
        .zip(slot.counters)
        .all(|(o, c)| o.creature_or_go == 0 || u32::from(c) >= o.required_count);
    // Signed compare (`0x5e1081`).
    let items_met = items
        .iter()
        .all(|&(id, n)| id == 0 || (live.bag_count)(id) as i32 >= n as i32);
    // The PvP count is slot counter 0 (`0x5e1091`).
    rep_min_met
        && rep_max_met
        && money_met
        && counters_met
        && items_met
        && u32::from(slot.counters[0]) >= players_slain
}

#[cfg(test)]
#[path = "done_tests.rs"]
mod tests;
