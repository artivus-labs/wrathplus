//! The world map's visible quest order.

use mlua::{Lua, MultiValue, Value};

use crate::script::{Model, QuestLogEntryView};

/// The log quests with an icon on the displayed map (the first 25 log quests), in `0x5e3840`'s
/// three passes: the done quests (`0x5e0ea0`, objectives satisfied, or `0x5e10b0`, a quest with
/// nothing to collect or kill), then the watched ones in watch order (`0xc23680`), then the rest
/// in log order. Stock `WorldMapFrame_UpdateQuests` numbers its numeric POIs along this list and
/// `WatchFrame_DisplayTrackedQuests` along the watch list, so the shared quests carry the same
/// numbers; a done quest's swap POI takes the list index, and `QuestPOI_HideButtons(…,
/// numPOICompleteSwap + 1)` keeps only the leading ones. Labelled: `0x5e10b0`'s template checks
/// (money, spell, flags) read here as an in-progress quest with no objective lines.
pub(super) fn visible_quests(m: &Model) -> Vec<u32> {
    let done = |e: &&QuestLogEntryView| {
        e.complete == 1 || (e.complete == 0 && e.objectives.iter().all(|o| o.finished))
    };
    let quests: Vec<&QuestLogEntryView> = m
        .quest_log
        .entries
        .iter()
        .filter(|e| !e.is_header)
        .take(25)
        .filter(|e| super::icon(m, e.quest_id).is_some())
        .collect();
    let (first, rest): (Vec<_>, Vec<_>) = quests.into_iter().partition(done);
    let watched = &m.quest_watches.ids;
    let by_watch = watched
        .iter()
        .filter_map(|id| rest.iter().find(|e| e.quest_id == *id));
    let unwatched = rest.iter().filter(|e| !watched.contains(&e.quest_id));
    first
        .iter()
        .chain(by_watch)
        .chain(unwatched)
        .map(|e| e.quest_id)
        .collect()
}

/// `QuestPOIGetQuestIDByIndex(index)` (`0x5dfa80`): the quest in POI slot `index`, 1 to 25 of the
/// `0xc23b00` table, or nothing for an empty or out-of-range slot; a non-number raises. No stock
/// file calls it. Labelled: the slots are taken as the first 25 log quests in log order, as
/// [`visible_quests`] reads them; the table's fill (`0x5e6940`) is unread.
pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let by_index = lua.create_function(|lua, args: MultiValue| {
        let n = args
            .front()
            .cloned()
            .and_then(|v| lua.coerce_number(v).ok().flatten());
        let Some(n) = n else {
            return Err(mlua::Error::runtime(
                "Usage: QuestPOIGetQuestIDByIndex(index)",
            ));
        };
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let slot = usize::try_from(n as i64 - 1).ok().filter(|&i| i < 25);
        let id = slot.and_then(|i| m.quest_log.entries.iter().filter(|e| !e.is_header).nth(i));
        Ok(match id {
            Some(e) => MultiValue::from_vec(vec![Value::Integer(i64::from(e.quest_id))]),
            None => MultiValue::new(),
        })
    })?;
    lua.globals().set("QuestPOIGetQuestIDByIndex", by_index)
}

#[cfg(test)]
mod tests {
    use crate::script::{QuestLogEntryView, QuestLogState, UiScript};

    #[test]
    fn quest_poi_get_quest_id_by_index_reads_the_log_slots() {
        let mut s = UiScript::new().unwrap();
        let quest = |quest_id, is_header| QuestLogEntryView {
            quest_id,
            is_header,
            ..Default::default()
        };
        s.set_quest_log(QuestLogState {
            entries: vec![quest(0, true), quest(808, false), quest(826, false)],
            num_quests: 2,
        });
        let ask = |s: &UiScript, a: &str| {
            s.eval::<String>(&format!("return select('#', QuestPOIGetQuestIDByIndex({a}))..' '..tostring((QuestPOIGetQuestIDByIndex({a})))")).unwrap()
        };
        assert_eq!(ask(&s, "1"), "1 808");
        assert_eq!(ask(&s, "'2'"), "1 826");
        assert_eq!(ask(&s, "3"), "0 nil");
        assert_eq!(ask(&s, "0"), "0 nil");
        assert!(s.run("QuestPOIGetQuestIDByIndex()").is_err());
    }
}
