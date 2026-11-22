//! The quest lines a creature's unit tooltip draws (12340 `0x621070`, quest half), built from the
//! published quest log and each creature template's quest links, and handed to the VM by creature
//! entry ([`wrathplus_ui::script::UnitQuestBlock`]). Per quest row in log order, for a creature
//! of entry `E` with kill credits `K1`, `K2` and quest items `Q1..Q6`:
//! - the first creature slot counting `E`, then `K1`, then `K2` (`0x5e2370` per id, which answers
//!   the first matching slot): that slot's leaderboard line;
//! - for each quest item `Qi`, the first item slot needing it (`0x5e2630`; the quest's own source
//!   item only above a count of 1): that slot's line; and, while the row is not complete, the item's
//!   name for each of the quest's source drops that is `Qi` (template `+0x1c74`, gated on
//!   `0x5e0ea0`, read here as the row's `isComplete`, the strict form of that test).
//!
//! A quest with any line adds its title above them. The lines are the log's own leaderboard texts,
//! matched to their template slot by the POI bit (creature slot `i` is bit `i`, item slot `i` bit
//! `4 + i`).

use std::collections::HashMap;

use bevy::prelude::*;
use wrathplus_protocol::types::{CreatureQuestLinks, QuestTemplate};
use wrathplus_protocol::{SessionEvent, SessionEventKind};
use wrathplus_ui::script::{QuestRowLines, UiScript, UnitQuestBlock};

use crate::app_core::schedule::UiFeed;
use crate::items::Items;
use crate::net::{NetCommands, NetHandlerApp};
use crate::ui_quest_log::QuestLog;

/// The CVar the quest lines gate on.
pub(crate) const CVARS: &[crate::cvars::Registered] = {
    use crate::cvars::same;
    // Registered at `0x51dee1` with default `0x9e1464`, the string "1"; read at `0x621e6b`.
    &[same("showQuestTrackingTooltips", "1")]
};

/// Each creature template's quest links, by entry, as `SMSG_CREATURE_QUERY_RESPONSE` sent them.
#[derive(Resource, Default)]
pub struct CreatureQuestLinksCache(pub HashMap<u32, CreatureQuestLinks>);

/// Registers the CVar, the links cache's handler and the publish.
pub(crate) fn register(app: &mut App) {
    crate::cvars::RegisterCvars::register_cvars(app, CVARS);
    app.init_resource::<CreatureQuestLinksCache>()
        .net_handler(SessionEventKind::CreatureName, on_creature_name)
        .net_handler(SessionEventKind::Disconnected, on_session_end)
        .net_handler(SessionEventKind::LoggedOut, on_session_end)
        .add_systems(
            Update,
            publish
                .in_set(UiFeed)
                .after(crate::ui_quest_log::feed_quest_log),
        );
}

/// A second handler on the kind, beside the name cache's own: only the links.
fn on_creature_name(In(ev): In<SessionEvent>, mut cache: ResMut<CreatureQuestLinksCache>) {
    if let SessionEvent::CreatureName {
        entry,
        name,
        quest_links,
        ..
    } = ev
    {
        if name.is_some() {
            cache.0.insert(entry, quest_links);
        } else {
            cache.0.remove(&entry);
        }
    }
}

fn on_session_end(
    In(event): In<SessionEvent>,
    mut cache: ResMut<CreatureQuestLinksCache>,
    mut quest_log: ResMut<QuestLog>,
) {
    if matches!(
        event,
        SessionEvent::Disconnected { .. } | SessionEvent::LoggedOut
    ) {
        cache.0.clear();
        quest_log.tooltip_rows.clear();
    }
}

/// Rebuild the blocks every frame and hand them over when they change.
#[allow(clippy::type_complexity)] // Per-VM publication memo.
pub(crate) fn publish(
    script: Option<NonSendMut<UiScript>>,
    quest_log: Res<QuestLog>,
    cache: Res<CreatureQuestLinksCache>,
    items: Res<Items>,
    commands: Res<NetCommands>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<HashMap<u32, Vec<UnitQuestBlock>>>>>,
) {
    let Some(mut script) = script else { return };
    let rows = &quest_log.tooltip_rows;
    let templates: Vec<Option<&QuestTemplate>> = rows
        .iter()
        .map(|r| quest_log.template(r.0, &commands))
        .collect();
    let item_name = |id: u32| items.template(id, 0, &commands).map(|t| t.name.clone());
    let fresh = blocks_by_entry(rows, &templates, &cache.0, &item_name);
    let last = last.get(&script);
    if last.as_ref() != Some(&fresh) {
        script.set_unit_quest_lines(fresh.clone());
        *last = Some(fresh);
    }
}

/// The blocks for every creature entry a quest objective names or the links cache knows.
pub(crate) fn blocks_by_entry(
    rows: &[QuestRowLines],
    templates: &[Option<&QuestTemplate>],
    links: &HashMap<u32, CreatureQuestLinks>,
    item_name: &dyn Fn(u32) -> Option<String>,
) -> HashMap<u32, Vec<UnitQuestBlock>> {
    let mut entries: Vec<u32> = links.keys().copied().collect();
    for t in templates.iter().flatten() {
        let creatures = t.objectives.iter().map(|o| o.creature_or_go);
        entries.extend(creatures.filter(|&c| c != 0 && (c as i32) > 0));
    }
    entries.sort_unstable();
    entries.dedup();
    let mut out = HashMap::new();
    for entry in entries {
        let link = links.get(&entry).copied().unwrap_or_default();
        let blocks: Vec<UnitQuestBlock> = rows
            .iter()
            .zip(templates)
            .filter_map(|(row, t)| block_for(entry, &link, row, (*t)?, item_name))
            .collect();
        if !blocks.is_empty() {
            out.insert(entry, blocks);
        }
    }
    out
}

/// One quest row's block for a creature of `entry`, `None` when the creature serves it nowhere.
fn block_for(
    entry: u32,
    link: &CreatureQuestLinks,
    (_, title, complete, lines): &QuestRowLines,
    template: &QuestTemplate,
    item_name: &dyn Fn(u32) -> Option<String>,
) -> Option<UnitQuestBlock> {
    let text_of = |bit: u8| {
        lines
            .iter()
            .find(|(b, _)| *b == Some(bit))
            .map(|(_, text)| text.clone())
    };
    let mut out = Vec::new();
    let ids = std::iter::once(entry).chain(link.kill_credit.into_iter().filter(|&k| k != 0));
    for id in ids {
        let slot = template
            .objectives
            .iter()
            .position(|o| o.creature_or_go == id);
        out.extend(slot.and_then(|i| text_of(i as u8)));
    }
    let drops = template
        .wrath
        .as_ref()
        .map(|w| w.item_drops)
        .unwrap_or_default();
    for item in link.quest_items.into_iter().take_while(|&i| i != 0) {
        let slot = template
            .objectives
            .iter()
            .position(|o| o.item_id == item && (item != template.src_item_id || o.item_count > 1));
        out.extend(slot.and_then(|i| text_of(4 + i as u8)));
        if !complete {
            let named = drops.iter().filter(|d| d.0 == item);
            out.extend(named.filter_map(|_| item_name(item)));
        }
    }
    (!out.is_empty()).then(|| UnitQuestBlock {
        title: title.clone(),
        lines: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::{QuestObjective, WrathQuestInfo};

    /// An AzerothCore-shaped template: `objectives` as `(creature, count, item, count)` per slot.
    fn quest(
        id: u32,
        title: &str,
        slots: &[(u32, u32, u32, u32)],
        drops: [(u32, u32); 4],
    ) -> QuestTemplate {
        let mut objectives: [QuestObjective; 4] = std::array::from_fn(|_| QuestObjective {
            creature_or_go: 0,
            required_count: 0,
            item_id: 0,
            item_count: 0,
            text: String::new(),
        });
        for (o, &(c, cn, i, inn)) in objectives.iter_mut().zip(slots) {
            (o.creature_or_go, o.required_count, o.item_id, o.item_count) = (c, cn, i, inn);
        }
        QuestTemplate {
            wrath: Some(WrathQuestInfo {
                item_drops: drops,
                ..Default::default()
            }),
            quest_id: id,
            method: 2,
            level: 5,
            zone_or_sort: 12,
            quest_type: 0,
            rep_objective_faction: 0,
            rep_objective_value: 0,
            next_quest_in_chain: 0,
            money: 0,
            money_max_level: 0,
            reward_spell: 0,
            src_item_id: 0,
            flags: 0,
            rewards: [(0, 0); 4],
            choices: [(0, 0); 6],
            point_map_id: 0,
            point_x: 0.0,
            point_y: 0.0,
            point_opt: 0,
            title: title.into(),
            objectives_text: String::new(),
            details: String::new(),
            end_text: String::new(),
            objectives,
        }
    }

    /// Quest 7 (kill Kobold Vermin, entry 6, ×10) and quest 33 (Tough Wolf Meat, item 750, ×8, a
    /// Young Wolf's quest item, entry 299), plus a source drop (item 1307) a Defias Thug carries.
    #[test]
    fn kill_item_credit_and_drop_lines_reach_their_creatures() {
        let kill = quest(7, "Kobold Camp Cleanup", &[(6, 10, 0, 0)], [(0, 0); 4]);
        let meat = quest(
            33,
            "Wolves Across the Border",
            &[(0, 0, 750, 8)],
            [(1307, 1), (0, 0), (0, 0), (0, 0)],
        );
        let rows: Vec<QuestRowLines> = vec![
            (
                7,
                "Kobold Camp Cleanup".into(),
                false,
                vec![(Some(0), "Kobold Vermin slain: 3/10".into())],
            ),
            (
                33,
                "Wolves Across the Border".into(),
                false,
                vec![(Some(4), "Tough Wolf Meat: 2/8".into())],
            ),
        ];
        let links = HashMap::from([
            (
                299,
                CreatureQuestLinks {
                    kill_credit: [0; 2],
                    quest_items: [750, 0, 0, 0, 0, 0],
                },
            ),
            // A creature that counts as Kobold Vermin (KillCredit1) and drops the source item.
            (
                40,
                CreatureQuestLinks {
                    kill_credit: [6, 0],
                    quest_items: [1307, 0, 0, 0, 0, 0],
                },
            ),
        ]);
        let name = |id: u32| (id == 1307).then(|| "Gold Pickup Schedule".to_string());
        let templates = [Some(&kill), Some(&meat)];
        let blocks = blocks_by_entry(&rows, &templates, &links, &name);

        let block = |title: &str, lines: &[&str]| UnitQuestBlock {
            title: title.into(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
        };
        assert_eq!(
            blocks[&6],
            [block("Kobold Camp Cleanup", &["Kobold Vermin slain: 3/10"])]
        );
        assert_eq!(
            blocks[&299],
            [block("Wolves Across the Border", &["Tough Wolf Meat: 2/8"])]
        );
        assert_eq!(
            blocks[&40],
            [
                block("Kobold Camp Cleanup", &["Kobold Vermin slain: 3/10"]),
                block("Wolves Across the Border", &["Gold Pickup Schedule"]),
            ],
            "a kill credit counts, and a source drop names its item"
        );

        // Counted out (COMPLETE), the source drop's line goes; the objectives stay, finished.
        let done: Vec<QuestRowLines> = rows
            .iter()
            .map(|r| (r.0, r.1.clone(), true, r.3.clone()))
            .collect();
        let blocks = blocks_by_entry(&done, &templates, &links, &name);
        assert_eq!(
            blocks[&40],
            [block("Kobold Camp Cleanup", &["Kobold Vermin slain: 3/10"])]
        );
        assert!(blocks[&6][0].lines[0].ends_with("3/10"));
    }
    #[test]
    fn first_slots_source_item_threshold_and_zero_terminated_links() {
        let mut t = quest(1, "Q", &[(6, 1, 750, 1), (6, 2, 750, 2)], [(1307, 1); 4]);
        t.src_item_id = 750;
        let row = (
            1,
            "Q".into(),
            false,
            vec![
                (Some(0), "first kill".into()),
                (Some(1), "second kill".into()),
                (Some(4), "source one".into()),
                (Some(5), "source two".into()),
            ],
        );
        let link = CreatureQuestLinks {
            kill_credit: [6, 6],
            quest_items: [750, 0, 1307, 0, 0, 0],
        };
        assert_eq!(
            block_for(6, &link, &row, &t, &|_| Some("drop".into()))
                .unwrap()
                .lines,
            ["first kill", "first kill", "first kill", "source two"]
        );
    }

    fn tooltip_script() -> UiScript {
        let s = UiScript::new().unwrap();
        s.run(
            "RegisterCVar('showQuestTrackingTooltips', '1'); CreateFrame('Frame', 'Owner'); \
               local tt = CreateFrame('GameTooltip', 'TT'); tt:SetOwner(Owner, 'ANCHOR_NONE')",
        )
        .unwrap();
        s
    }

    fn drawn(app: &mut App) -> Vec<String> {
        use wrathplus_ui::script::UnitState;
        let mut s = app.world_mut().non_send_resource_mut::<UiScript>();
        s.set_unit(
            "mouseover",
            Some(UnitState {
                exists: true,
                guid: 0xF130_0000_0600_1234,
                name: Some("Kobold".into()),
                ..Default::default()
            }),
        );
        s.run("TT:SetUnit('mouseover')").unwrap();
        s.eval(
            "local t={} for i=1,TT:NumLines() do t[i]=_G['TTTextLeft'..i]:GetText() end return t",
        )
        .unwrap()
    }

    #[test]
    fn publication_survives_vm_replacement_and_clears_stale_rows() {
        let mut app = App::new();
        let (tx, _rx) = crossbeam_channel::unbounded();
        app.insert_resource(NetCommands(tx));
        app.init_resource::<QuestLog>()
            .init_resource::<Items>()
            .init_resource::<CreatureQuestLinksCache>();
        app.world_mut()
            .resource_mut::<QuestLog>()
            .insert_template(quest(7, "Cleanup", &[(6, 10, 0, 0)], [(0, 0); 4]));
        app.world_mut().resource_mut::<QuestLog>().tooltip_rows = vec![(
            7,
            "Cleanup".into(),
            false,
            vec![(Some(0), "Kobolds: 3/10".into())],
        )];
        app.insert_non_send_resource(tooltip_script());
        app.add_systems(Update, publish);
        app.update();
        assert!(drawn(&mut app).contains(&" - Kobolds: 3/10".into()));
        // No intervening absent-VM frame: the exact old map must seed the replacement too.
        app.insert_non_send_resource(tooltip_script());
        app.update();
        assert!(drawn(&mut app).contains(&" - Kobolds: 3/10".into()));
        app.world_mut()
            .resource_mut::<QuestLog>()
            .tooltip_rows
            .clear();
        app.update();
        assert!(!drawn(&mut app).contains(&"Cleanup".into()));
    }

    #[test]
    fn disconnect_drops_creature_links_even_without_a_vm() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.init_resource::<QuestLog>();
        app.insert_resource(CreatureQuestLinksCache(HashMap::from([(
            6,
            CreatureQuestLinks::default(),
        )])));
        app.world_mut()
            .run_system_once_with(
                on_session_end,
                SessionEvent::Disconnected {
                    reason: "test".into(),
                    end: wrathplus_protocol::SessionEnd::Lost,
                },
            )
            .unwrap();
        assert!(app
            .world()
            .resource::<CreatureQuestLinksCache>()
            .0
            .is_empty());
    }
    #[test]
    fn real_creature_codec_lands_and_negative_answer_evicts_links() {
        use bevy::ecs::system::RunSystemOnce;
        use wrathplus_protocol::wrath_session::{decode, SMSG_CREATURE_QUERY_RESPONSE};
        let mut app = App::new();
        app.init_resource::<CreatureQuestLinksCache>();
        // AC QueryHandler.cpp: entry, six strings, flags/type/family/rank, credits, four
        // displays, health/mana modifiers, leader, six quest items, movement id.
        let mut body = 40u32.to_le_bytes().to_vec();
        for text in ["Thug", "", "", "", "", ""] {
            body.extend(text.as_bytes());
            body.push(0);
        }
        for v in [0u32, 7, 0, 0, 6, 9, 123, 0, 0, 0] {
            body.extend(v.to_le_bytes());
        }
        body.extend(1f32.to_le_bytes());
        body.extend(1f32.to_le_bytes());
        body.push(0);
        for v in [1307u32, 750, 0, 0, 0, 0, 0] {
            body.extend(v.to_le_bytes());
        }
        let event = decode(SMSG_CREATURE_QUERY_RESPONSE, &body, None)
            .unwrap()
            .remove(0);
        app.world_mut()
            .run_system_once_with(on_creature_name, event)
            .unwrap();
        assert_eq!(
            app.world().resource::<CreatureQuestLinksCache>().0[&40],
            CreatureQuestLinks {
                kill_credit: [6, 9],
                quest_items: [1307, 750, 0, 0, 0, 0]
            }
        );
        let missing = (40u32 | 0x8000_0000).to_le_bytes();
        let event = decode(SMSG_CREATURE_QUERY_RESPONSE, &missing, None)
            .unwrap()
            .remove(0);
        app.world_mut()
            .run_system_once_with(on_creature_name, event)
            .unwrap();
        assert!(!app
            .world()
            .resource::<CreatureQuestLinksCache>()
            .0
            .contains_key(&40));
    }
}
