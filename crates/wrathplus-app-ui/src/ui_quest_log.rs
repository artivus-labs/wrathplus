//! The quest log's app side: reads the player's `PLAYER_QUEST_LOG` slots every frame, resolves
//! each quest through the `SMSG_QUEST_QUERY_RESPONSE` cache and pushes the log, every quest with
//! its objective lines, firing `QUEST_LOG_UPDATE` on a change. An advanced objective also toasts
//! its line and fires `QUEST_WATCH_UPDATE`. Abandons, shares and header folds drain from here.

use crate::inventory::count_of;
use crate::inventory::InventoryScope;
use std::collections::HashSet;

use bevy::prelude::*;

use wrathplus_protocol::types::{
    quest_flags, quest_slot_state, QuestLogSlot, QuestObjective, QuestTemplate,
    QUEST_PVP_KILL_COUNTER,
};
use wrathplus_protocol::ObjectFields;
use wrathplus_ui::script::{
    QuestItemView, QuestLinkView, QuestLogDetail, QuestLogEntryView, QuestLogObjectiveView,
    QuestLogState, ScriptValue, UiScript,
};
use wrathplus_ui::strings::{fill, Arg};

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Spells;
use crate::entities::ItemDisplays;
use crate::items::Items;
use crate::names::NameCache;
use crate::net::{ClientCommand, Guid, NetCommands};
use crate::query_cache::QueryCache;

/// Marks a GameObject objective's entry on the wire (vmangos `Quest.cpp:512-516`).
const GO_OBJECTIVE_BIT: u32 = 0x8000_0000;

/// One occupied `PLAYER_QUEST_LOG` slot this frame.
struct Row {
    slot: u8,
    quest_id: u32,
    log_slot: QuestLogSlot,
}

/// The template cache, asked once per quest id, and this frame's quests as `(quest id, slot)`,
/// folded ones included: the reference's row table `0xbb71c0`, which an abandon searches.
#[derive(Resource, Default)]
pub struct QuestLog {
    templates: QueryCache<u32, QuestTemplate>,
    /// All cached quest rows, including folded rows, for the creature tooltip (`0x621f30`).
    pub(crate) tooltip_rows: Vec<wrathplus_ui::script::QuestRowLines>,
    row_slots: Vec<(u32, u8)>,
    /// The collapsed headers, by title.
    collapsed: HashSet<String>,
    /// Per pushed entry, folded rows included, the header's title, `None` for a quest: the fold
    /// drain's index map.
    header_keys: Vec<Option<String>>,
}

impl crate::query_cache::AskOnce for QuestLog {
    fn clear_pending(&mut self) {
        self.templates.clear_pending();
    }
}

impl QuestLog {
    /// A cached template's `ZoneOrSort`, never asking: the quest-log header the quest sits under.
    pub(crate) fn zone_or_sort(&self, quest_id: u32) -> Option<i32> {
        self.templates.get(quest_id).map(|t| t.zone_or_sort)
    }

    /// The template, asked once: a miss sends `CMSG_QUEST_QUERY` (once while in flight); an id
    /// the server does not know stays `None` without a re-ask.
    pub(crate) fn template(&self, quest_id: u32, commands: &NetCommands) -> Option<&QuestTemplate> {
        self.templates.get_or_ask(quest_id, || {
            debug!("ui_quest_log: asking quest template (quest {quest_id})");
            let _ = commands
                .0
                .send(ClientCommand::QuestQuery { quest: quest_id });
        })
    }

    /// Whether the server answered `quest_id` as unknown, not still pending.
    pub(crate) fn template_answered_unknown(&self, quest_id: u32) -> bool {
        self.templates.answered_unknown(quest_id)
    }

    /// Record a template answer (`SMSG_QUEST_QUERY_RESPONSE`).
    pub fn insert_template(&mut self, template: QuestTemplate) {
        self.templates.insert(template.quest_id, Some(template));
    }

    /// Disconnect: drops everything, the templates included.
    pub(crate) fn clear_session(&mut self) {
        self.templates.clear();
        self.tooltip_rows.clear();
        self.row_slots.clear();
        self.collapsed.clear();
        self.header_keys.clear();
    }
}

/// The quest log's packet handlers.
mod net {
    use bevy::prelude::*;
    use wrathplus_protocol::types::QuestTemplate;
    use wrathplus_protocol::{SessionEvent, SessionEventKind};

    use super::QuestLog;
    use crate::net::NetHandlerApp;

    /// Registers the template handler and the session-end listener.
    pub(super) fn register(app: &mut App) {
        use SessionEventKind as K;
        app.net_handler(K::QuestTemplate, on_template)
            .net_handler(K::Disconnected, on_session_end);
    }

    fn on_template(In(ev): In<SessionEvent>, mut quest_log: ResMut<QuestLog>) {
        if let SessionEvent::QuestTemplate(t) = ev {
            quest_template(t, &mut quest_log);
        }
    }

    /// The log's state dies with the socket: a second handler on the kind, after the bridge's own.
    fn on_session_end(In(_): In<SessionEvent>, mut quest_log: ResMut<QuestLog>) {
        quest_log.clear_session();
    }

    /// `SMSG_QUEST_QUERY_RESPONSE`, the answer to our `CMSG_QUEST_QUERY`, cached by quest id.
    fn quest_template(t: Box<QuestTemplate>, quest_log: &mut QuestLog) {
        debug!("net: quest template {} ({})", t.quest_id, t.title);
        quest_log.insert_template(*t);
    }
}

pub struct UiQuestLogPlugin;

impl Plugin for UiQuestLogPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        crate::ui_unit_quest_tooltip::register(app);
        crate::query_cache::register::<QuestLog>(app);
        app.init_resource::<QuestLog>()
            .add_systems(
                Startup,
                (
                    load_quest_header_names,
                    load_quest_tag_names,
                    crate::quest_rewards::load,
                )
                    .after(wrathplus_assets::AssetSet::Open),
            )
            .add_systems(
                Update,
                (
                    feed_quest_log.in_set(UnitFeed),
                    feed_quest_special_items
                        .in_set(UiFeed)
                        .after(feed_quest_log)
                        .run_if(crate::app_core::ui_state::ingame_ui_up),
                    // Before the script tick, so `QuestTimerFrame` reads this frame's clock.
                    feed_server_clock.in_set(UiFeed),
                    // Before the script tick, so a link tooltip rebuilds the frame its template lands.
                    feed_quest_links.in_set(UiFeed),
                    drain_quest_log_abandons.after(UiInput),
                    drain_quest_log_pushes.after(UiInput),
                    drain_quest_log_collapses.after(UiInput),
                ),
            );
    }
}

/// Quest link tooltips (`GameTooltip:SetHyperlink("quest:…")`, core `0x622960`): each id the VM
/// asks for goes through the template cache, asked once, and its view goes back once the
/// template is in, which rebuilds a tooltip still waiting on it (`0x61de20`). An id the server
/// does not know is dropped, so its tooltip stays as an uncached quest's.
///
/// Labelled: 12340 composes the requirement rows itself (`0x5e1300` counts, `0x5e1460` writes);
/// ours are the quest log's leaderboard lines ([`build_objectives`]), read off the player's log
/// slot when the quest is in it, else at zero, not traced against that composer.
#[allow(clippy::too_many_arguments)] // the template, the subject and the objective readers
fn feed_quest_links(
    script: Option<NonSendMut<UiScript>>,
    self_q: Query<(&ObjectStore, &Guid), With<SelfPlayer>>,
    quest_log: Res<QuestLog>,
    names: Res<NameCache>,
    objects: crate::net::Objects,
    items: Res<Items>,
    commands: Res<NetCommands>,
    states: Res<crate::app_core::state::WorldStates>,
    // The ids this VM asked for: a rebuilt VM has no tooltip waiting on them.
    mut waiting: Local<crate::app_core::vm_memo::VmMemo<Vec<u32>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let waiting = waiting.get(&script);
    for id in script.take_quest_link_requests() {
        if !waiting.contains(&id) {
            waiting.push(id);
        }
    }
    if waiting.is_empty() {
        return;
    }
    let Some((store, _)) = self_q.iter().next() else {
        return;
    };
    let player = crate::npc_text::player_identity(&self_q, &names, &commands);
    let macros = crate::npc_text::MacroContext {
        subject: player.as_ref(),
        states: &states,
    };
    let mut ready = Vec::new();
    {
        let get = |key: &str| {
            script
                .lua()
                .globals()
                .get::<String>(key)
                .ok()
                .filter(|t| !t.is_empty())
        };
        waiting.retain(|&id| {
            let Some(t) = quest_log.template(id, &commands) else {
                return !quest_log.template_answered_unknown(id);
            };
            let slot = (0..store.0.player_quest_log_slots())
                .filter_map(|s| store.0.player_quest_log(s))
                .find(|s| s.quest_id == id)
                .unwrap_or(QuestLogSlot {
                    quest_id: id,
                    counters: [0; 4],
                    state: 0,
                    timer: 0,
                });
            let requirements = build_objectives(
                t, &slot, &store.0, &objects, &items, &names, &commands, &get,
            )
            .into_iter()
            .map(|o| o.text)
            .collect();
            ready.push((
                id,
                QuestLinkView {
                    title: t.title.clone(),
                    objectives: crate::npc_text::substitute(&t.objectives_text, &macros),
                    requirements,
                },
            ));
            false
        });
    }
    for (id, view) in ready {
        script.set_quest_link_view(id, view);
    }
}

/// Deviation: an in-flight name reads `...`, where the reference's reads `" "` (`0x82ee00`),
/// because a blank looks like a nameless objective.
const NAME_PLACEHOLDER: &str = "...";

/// One leaderboard line from its `%s`/`%d`/`%d` key. The keys are the leaderboard's own, not the
/// toast's `ERR_QUEST_ADD_*` twins that share their enUS sentences: `GetQuestLogLeaderBoard`
/// (`0x4e0110`) takes `QUEST_MONSTERS_KILLED` `0x84b61c` (creature), `QUEST_OBJECTS_FOUND`
/// `0x84b634` (override text, GameObject) and `QUEST_ITEMS_NEEDED` `0x84b5f8` (item). A key the
/// install lacks gives an empty line, not no line, as `GetNumQuestLeaderBoards` still counts it.
fn leaderboard_text(
    // The VM's own `GlobalStrings.lua`.
    get: &dyn Fn(&str) -> Option<String>,
    key: &str,
    name: &str,
    cur: u32,
    req: u32,
) -> String {
    get(key).map_or_else(String::new, |f| {
        fill(
            &f,
            &[Arg::S(name), Arg::D(i64::from(cur)), Arg::D(i64::from(req))],
        )
    })
}

/// One creature or GameObject line, for `ReqCreatureOrGOId[i] != 0` (`0x4e00c0`, `0x4e02d7`);
/// `counter` is the slot's native objective counter for index i. A non-empty `text` is the name and takes
/// `QUEST_OBJECTS_FOUND` (`0x4e03da`), while `type` stays `"monster"` (`0x4e04ca`).
fn creature_line(
    obj: &QuestObjective,
    counter: u16,
    creature_name: Option<&str>,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<QuestLogObjectiveView> {
    if obj.creature_or_go == 0 {
        return None;
    }
    let req = obj.required_count;
    let is_go = obj.creature_or_go & GO_OBJECTIVE_BIT != 0;
    let cur = u32::from(counter).min(req);
    // A GameObject takes `QUEST_OBJECTS_FOUND` on both legs (`0x4e031e`); with no GameObject
    // name cache here, one without an override wears the placeholder.
    let (key, name) = if !obj.text.is_empty() {
        ("QUEST_OBJECTS_FOUND", obj.text.as_str())
    } else if is_go {
        ("QUEST_OBJECTS_FOUND", NAME_PLACEHOLDER)
    } else {
        (
            "QUEST_MONSTERS_KILLED",
            creature_name.unwrap_or(NAME_PLACEHOLDER),
        )
    };
    Some(QuestLogObjectiveView {
        text: leaderboard_text(get, key, name, cur, req),
        kind: if is_go { "object" } else { "monster" }.into(),
        finished: cur >= req,
        cur,
        req,
        poi_bit: None,
    })
}

/// One item line, for `ReqItemId[i] > 0 && ReqItemId[i] != SrcItemId` (`0x4e04f4`-`0x4e04fd`).
/// `bag_count` is the live inventory count, as an item has no slot counter; the name comes from
/// the item cache alone, the override text being the creature branch's (`0x4e033f`, `0x4e03c9`).
fn item_line(
    obj: &QuestObjective,
    src_item_id: u32,
    item_name: Option<&str>,
    bag_count: u32,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<QuestLogObjectiveView> {
    if obj.item_id == 0 || obj.item_id == src_item_id {
        return None;
    }
    let req = obj.item_count;
    let cur = bag_count.min(req);
    let name = item_name.unwrap_or(NAME_PLACEHOLDER);
    Some(QuestLogObjectiveView {
        text: leaderboard_text(get, "QUEST_ITEMS_NEEDED", name, cur, req),
        kind: "item".into(),
        finished: cur >= req,
        cur,
        req,
        poi_bit: None,
    })
}

/// The PvP objective line, for a 3.3.5 template's `players_slain != 0`, counted in slot counter
/// [`QUEST_PVP_KILL_COUNTER`]; `QUEST_PLAYERS_KILLED` composes the same sentence as the
/// `ERR_QUEST_ADD_PLAYER_KILL_SII` toast. Unverified against the client: the line's place (last
/// here) and its `type` string, which no stock frame reads.
fn player_line(
    players_slain: u32,
    log_slot: &QuestLogSlot,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<QuestLogObjectiveView> {
    if players_slain == 0 {
        return None;
    }
    let cur = u32::from(log_slot.counters[QUEST_PVP_KILL_COUNTER]).min(players_slain);
    let text = get("QUEST_PLAYERS_KILLED").map_or_else(String::new, |f| {
        fill(
            &f,
            &[Arg::D(i64::from(cur)), Arg::D(i64::from(players_slain))],
        )
    });
    Some(QuestLogObjectiveView {
        text,
        kind: "player".into(),
        finished: cur >= players_slain,
        cur,
        req: players_slain,
        poi_bit: None,
    })
}

/// The template's `money` as (required, reward): a negative value is demanded at turn-in.
fn money_split(money: i32) -> (u32, u32) {
    if money < 0 {
        ((-money) as u32, 0)
    } else {
        (0, money as u32)
    }
}

/// One reward or choice `(item id, count)` as a [`QuestItemView`]. `SMSG_QUEST_QUERY_RESPONSE`
/// carries no display id, so the icon comes from the item template's `display_info_id`.
fn resolve_template_item(
    entry: u32,
    count: u32,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
) -> QuestItemView {
    let template = items.template(entry, 0, commands).cloned();
    let name = template.as_ref().map(|t| t.name.clone());
    let quality = template.as_ref().map(|t| t.quality).unwrap_or(1);
    let texture = template.as_ref().and_then(|t| {
        icons
            .and_then(|i| i.catalog.get(t.display_info_id))
            .and_then(|d| d.icon.clone())
    });
    // `GetQuestLogItemLink`'s payload; `None` until the template gives the name and quality.
    let link = name
        .as_ref()
        .map(|n| crate::ui_items::item_link(entry, n, quality));
    QuestItemView {
        name,
        texture,
        count,
        quality,
        item_id: entry,
        usable: true, // not computed; the stock frame tints unusable ones red
        link,
    }
}

/// One quest's objective lines, for every quest, as the watch frame reads any quest's
/// (`QuestLogFrame.lua:613-663`). The reference walks the array once per kind, `event`, creature
/// or GameObject, item, reputation (`0x4e0000`, `0x4e0110`), so one quad can give two lines. The
/// reputation line is not built; [`crate::npc_text::event_objective`] heads the list.
pub(crate) fn build_objectives(
    template: &QuestTemplate,
    log_slot: &QuestLogSlot,
    store: &ObjectFields,
    objects: &crate::net::Objects,
    items: &Items,
    names: &NameCache,
    commands: &NetCommands,
    // The VM's own `GlobalStrings.lua`, for the leaderboard's three format keys.
    get: &dyn Fn(&str) -> Option<String>,
) -> Vec<QuestLogObjectiveView> {
    let mut objectives = crate::npc_text::event_objective(template, log_slot.state);
    // Creature or GameObject: the counter index is the array index, empty elements included.
    for (i, obj) in template.objectives.iter().enumerate() {
        let is_go = obj.creature_or_go & GO_OBJECTIVE_BIT != 0;
        let creature_name = (obj.creature_or_go != 0 && !is_go)
            .then(|| {
                names
                    .resolve_creature(obj.creature_or_go, 0, commands)
                    .map(str::to_string)
            })
            .flatten();
        if let Some(line) = creature_line(obj, log_slot.counters[i], creature_name.as_deref(), get)
        {
            objectives.push(QuestLogObjectiveView {
                poi_bit: Some(i as u8),
                ..line
            });
        }
    }
    // Item.
    for (i, obj) in template.objectives.iter().enumerate() {
        if obj.item_id == 0 {
            continue;
        }
        let item_name = items
            .template(obj.item_id, 0, commands)
            .map(|t| t.name.clone());
        let bag_count = count_of(store, objects, obj.item_id, InventoryScope::QUEST_ITEMS);
        if let Some(line) = item_line(
            obj,
            template.src_item_id,
            item_name.as_deref(),
            bag_count,
            get,
        ) {
            objectives.push(QuestLogObjectiveView {
                poi_bit: Some(4 + i as u8),
                ..line
            });
        }
    }
    let players_slain = template.wrath.as_ref().map_or(0, |w| w.players_slain);
    objectives.extend(player_line(players_slain, log_slot, get));
    objectives
}

/// A quest's detail pane from its template: description, money, rewards and choices.
pub(crate) fn build_detail(
    template: &QuestTemplate,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
    macros: &crate::npc_text::MacroContext,
    spells: Option<&Spells>,
) -> QuestLogDetail {
    let (required_money, reward_money) = money_split(template.money);
    let rewards = template
        .rewards
        .iter()
        .filter(|&&(id, _)| id != 0)
        .map(|&(id, count)| resolve_template_item(id, count, items, icons, commands))
        .collect();
    let choices = template
        .choices
        .iter()
        .filter(|&&(id, _)| id != 0)
        .map(|&(id, count)| resolve_template_item(id, count, items, icons, commands))
        .collect();

    QuestLogDetail {
        // Quest text arrives with its `$N`, `$B`, `$G` and `$<n>w` macros unexpanded.
        description: crate::npc_text::substitute(&template.details, macros),
        objectives_text: crate::npc_text::substitute(&template.objectives_text, macros),
        required_money,
        reward_money,
        // Query honor addition/multiplier are retained; the level-dependent award needs
        // the honor table, so do not present the raw addition as a resolved reward.
        reward_talents: template.wrath.as_ref().map_or(0, |w| w.bonus_talents),
        reward_arena_points: template.wrath.as_ref().map_or(0, |w| w.arena_points),
        // Always a string, never nil: stock concatenates it (`WorldMapFrame.lua:1872`).
        completion_text: (template.wrath.is_some())
            .then(|| crate::npc_text::completion_text(template, macros)),
        choices,
        rewards,
        reward_spell: crate::ui_quest::reward_spell_view(template.reward_spell, spells),
        ..Default::default()
    }
}

/// `AbandonQuest`'s search (`0x4df070`): the slot (`+0x4`) of the non-header row of the 40 in
/// `0xbb71c0` that carries `quest_id`; with no such row, nothing is sent.
fn abandon_slot(quest_id: u32, row_slots: &[(u32, u8)]) -> Option<u8> {
    row_slots
        .iter()
        .find(|&&(id, _)| id == quest_id)
        .map(|&(_, slot)| slot)
}

/// Moves the 1-based selection to its quest's new index. A header is never a selection
/// (`QuestLog_GetFirstSelectableQuest` skips them), so one with no visible quest becomes 0 and
/// `QuestLog_Update` selects the first quest (`QuestLogFrame.lua:293-296`).
fn remap_selection(old: &[QuestLogEntryView], new: &[QuestLogEntryView], sel: u32) -> u32 {
    let Some(prev) = (sel as usize)
        .checked_sub(1)
        .and_then(|i| old.get(i))
        .filter(|e| !e.is_header)
    else {
        return 0;
    };
    new.iter()
        .position(|e| !e.is_header && e.quest_id == prev.quest_id)
        .map(|i| i as u32 + 1)
        .unwrap_or(0)
}

/// Pushes the server's wall clock every frame, for `GetQuestTimers()`; before the first
/// `SMSG_QUERY_TIME_RESPONSE` nothing is pushed and the bindings report no timer.
fn feed_server_clock(
    script: Option<NonSendMut<UiScript>>,
    clock: Res<crate::net::ServerWallClock>,
) {
    let (Some(mut script), Some(now)) = (script, clock.now_unix()) else {
        return;
    };
    script.set_server_unix_time(now);
}

/// A quest's log level (`0x5e0d20`): a template level of -1 answers the player's own level
/// (`0x5e0d7a`), which the row sort (`0x5e08f0`) and `GetQuestLogTitle` both read.
fn log_level(level: i32, player: u32) -> u32 {
    u32::try_from(level).unwrap_or(player)
}

/// The reference's header for `zoneOrSort` 0 (`0x84b4ec`). Only a cached template makes a header
/// (`0x4de6f6`), so it never stands for a template in flight.
const MISSING_HEADER: &str = "Missing header! (quest designers)";

/// One row's grouping and ordering inputs.
struct GroupRow {
    zone_or_sort: i32,
    /// [`MISSING_HEADER`] for 0, empty for an id neither `AreaTable` nor `QuestSort` names.
    header: String,
    level: u32,
    title: String,
    slot: u8,
}

/// The quest log's display order (rebuild `0x4de510`): the header groups, each with its rows.
/// Groups (`0x4de751`, comparator `0x4de8f0`) put id 0 first (`0x4de913`), then go by name
/// through the case-insensitive collator `0x64a4c0` (`_strnicmp 0x414310`), an unnamed id as `""`.
/// Rows (`0x4deac0`) go by level (`0x4decd9`), then title (`0x4decfb`), never by slot; the
/// reference's `qsort` is unstable, so a full tie has no order there, and ties break on slot here.
fn order_groups(rows: &[GroupRow]) -> Vec<(i32, String, Vec<usize>)> {
    let mut groups: Vec<(i32, String, Vec<usize>)> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        match groups.iter_mut().find(|(z, _, _)| *z == r.zone_or_sort) {
            Some((_, _, idxs)) => idxs.push(i),
            None => groups.push((r.zone_or_sort, r.header.clone(), vec![i])),
        }
    }
    groups.sort_by_key(|(zos, name, _)| {
        if *zos == 0 {
            (0u8, String::new())
        } else {
            (1, name.to_lowercase())
        }
    });
    for (_, _, idxs) in &mut groups {
        idxs.sort_by_key(|&i| {
            let r = &rows[i];
            (r.level, r.title.to_lowercase(), r.slot)
        });
    }
    groups
}

/// Reads the player's `PLAYER_QUEST_LOG` slots each frame and pushes a [`QuestLogState`] on a
/// change; also refreshes `row_slots` for the abandon drain.
#[allow(clippy::type_complexity)] // Bevy system parameters / a closure table, kept inline
pub(crate) fn feed_quest_log(
    script: Option<NonSendMut<UiScript>>,
    self_q: Query<(&ObjectStore, &Guid), With<SelfPlayer>>,
    mut quest_log: ResMut<QuestLog>,
    names: Res<NameCache>,
    // The bag walk behind an item objective's count.
    objects: crate::net::Objects,
    items: Res<Items>,
    icons: Option<Res<ItemDisplays>>,
    commands: Res<NetCommands>,
    header_names: Option<Res<QuestHeaderNamesRes>>,
    tag_names: Option<Res<QuestTagNamesRes>>,
    states: Res<crate::app_core::state::WorldStates>,
    (spells, rewards, factions, reputations): (
        Option<Res<Spells>>,
        Option<Res<crate::quest_rewards::QuestRewardTables>>,
        Option<Res<crate::app_core::state::Factions>>,
        Option<Res<crate::net::Reputations>>,
    ),
    // The completion toast is a message-catalog row, whose surface and sound the sink reads.
    mut sink: crate::ui_action::MessageSink,
    mut last: Local<crate::app_core::vm_memo::VmMemo<QuestLogState>>,
    // The accept line's queue.
    mut quest_lines: ResMut<crate::ui_quest::QuestLines>,
    mut prior_slots: Local<crate::app_core::vm_memo::VmMemo<Option<SlotStates>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let last = last.get(&script);
    let prior_slots = prior_slots.get(&script);
    let Some((store, _)) = self_q.iter().next() else {
        return;
    };

    let mut rows = Vec::new();
    let mut slots = Vec::with_capacity(usize::from(store.0.player_quest_log_slots()));
    for slot in 0..store.0.player_quest_log_slots() {
        let Some(log_slot) = store.0.player_quest_log(slot) else {
            slots.push((0, 0));
            continue;
        };
        slots.push((log_slot.quest_id, log_slot.state));
        if log_slot.quest_id == 0 {
            continue; // a slot an abandon or turn-in cleared
        }
        rows.push(Row {
            slot,
            quest_id: log_slot.quest_id,
            log_slot,
        });
    }

    // The quest-slot watcher's accept line, whose catalog row carries `QUESTADDED`. The first
    // snapshot after login is silent: the initial create fills the whole log.
    if let Some(prior) = prior_slots.as_ref() {
        for quest_id in accepted_quests(prior, &slots) {
            quest_lines.push(crate::ui_quest::QuestLine::Accepted(quest_id));
        }
    }
    *prior_slots = Some(slots);

    // The player's own `GlobalStrings.lua`, for the leaderboard's three format keys.
    let get = |key: &str| {
        script
            .lua()
            .globals()
            .get::<String>(key)
            .ok()
            .filter(|t| !t.is_empty())
    };

    // ── In-flight templates: only cached quests are listed ──────────────────────────────────────
    // The rebuild (`0x4de510`) skips a quest whose template is uncached, counting it in
    // `[0xbb7498]` (`0x4de67e`), and lists every other one, so `GetNumQuestLogEntries()` counts
    // cached quests. The reference then holds that list until the last template lands
    // (`0x4de545`, `0x4de8b0`); here each quest shows as its template lands. `template()` runs for
    // every row before the filter: the miss is what sends the query.
    let cached: Vec<bool> = rows
        .iter()
        .map(|r| quest_log.template(r.quest_id, &commands).is_some())
        .collect();
    let rows: Vec<Row> = rows
        .into_iter()
        .zip(cached)
        .filter_map(|(r, hit)| hit.then_some(r))
        .collect();

    // ── Section headers ─────────────────────────────────────────────────────────────────────────
    // One per `ZoneOrSort`: a positive id names an `AreaTable` zone, a negative a `QuestSort`.
    let player_level = store.0.unit_level().unwrap_or(0);
    let ordered = {
        let keyed: Vec<GroupRow> = rows
            .iter()
            .map(|r| {
                let t = quest_log.templates.get(r.quest_id);
                let zos = t.map(|t| t.zone_or_sort).unwrap_or(0);
                GroupRow {
                    zone_or_sort: zos,
                    header: if zos == 0 {
                        MISSING_HEADER.to_string()
                    } else {
                        header_names
                            .as_ref()
                            .and_then(|h| h.0.resolve(zos))
                            .unwrap_or_default()
                            .to_string()
                    },
                    level: t.map_or(0, |t| log_level(t.level, player_level)),
                    title: t.map(|t| t.title.clone()).unwrap_or_default(),
                    slot: r.slot,
                }
            })
            .collect();
        order_groups(&keyed)
    };
    let groups = ordered;

    let player = crate::npc_text::player_identity(&self_q, &names, &commands);
    let macros = crate::npc_text::MacroContext {
        subject: player.as_ref(),
        states: &states,
    };

    let mut entries: Vec<QuestLogEntryView> = Vec::new();
    let mut header_keys: Vec<Option<String>> = Vec::new();
    // Quests under a collapsed header stay in the row array after every visible row, as the
    // comparator `0x5e08f0` sorts them (`0x5e0a2a`-`0x5e0b63`); it ties two folded rows, which keep
    // their group, level and title order here.
    let mut folded: Vec<QuestLogEntryView> = Vec::new();
    let mut tooltip_rows = Vec::new();
    for (header, (_, name, row_idxs)) in (1..).zip(&groups) {
        let collapsed = quest_log.collapsed.contains(name);
        entries.push(QuestLogEntryView {
            quest_id: 0,
            title: name.clone(),
            level: 0,
            tag: None,
            is_header: true,
            collapsed,
            complete: 0,
            timer: 0,
            pushable: false,
            objectives: Vec::new(),
            detail: None,
            ..Default::default()
        });
        header_keys.push(Some(name.clone()));
        for &ri in row_idxs {
            let r = &rows[ri];
            let (title, level, tag, pushable, objectives, detail) =
                match quest_log.template(r.quest_id, &commands) {
                    Some(t) => (
                        t.title.clone(),
                        log_level(t.level, player_level),
                        // The tag: `Type` is a `QuestInfo.dbc` id, 0 naming none. No table gives
                        // `None`, never `""`: the row's Lua tests presence.
                        tag_names
                            .as_ref()
                            .and_then(|tags| tags.0.resolve(t.quest_type))
                            .map(str::to_string),
                        // `QUEST_FLAGS_SHARABLE` (vmangos `QuestDef.h:153`) is the client's test
                        // alone: vmangos pushes without it (`QuestHandler.cpp:403-459`) and checks
                        // it only at the accept (`QuestHandler.cpp:114`).
                        t.flags & quest_flags::SHARABLE != 0,
                        build_objectives(
                            t,
                            &r.log_slot,
                            &store.0,
                            &objects,
                            &items,
                            &names,
                            &commands,
                            &get,
                        ),
                        // Every row's: the bindings resolve the selection per call.
                        Some({
                            let mut detail = build_detail(
                                t,
                                &items,
                                icons.as_deref(),
                                &commands,
                                &macros,
                                spells.as_deref(),
                            );
                            if let (Some(w), Some(tables)) = (&t.wrath, rewards.as_deref()) {
                                tables.fill_log(
                                    &mut detail,
                                    w,
                                    t.level,
                                    store.0.unit_level(),
                                    macros.subject,
                                );
                            }
                            detail
                        }),
                    ),
                    // The in-flight filter above kept only cached rows.
                    None => unreachable!("the in-flight gate leaves only cached rows"),
                };
            let complete = if r.log_slot.state & quest_slot_state::COMPLETE != 0 {
                1
            } else if r.log_slot.state & quest_slot_state::FAIL != 0 {
                -1
            } else {
                0
            };
            tooltip_rows.push((
                r.quest_id,
                title.clone(),
                complete == 1,
                objectives
                    .iter()
                    .map(|o| (o.poi_bit, o.text.clone()))
                    .collect(),
            ));
            let entry = QuestLogEntryView {
                suggested_group: quest_log
                    .template(r.quest_id, &commands)
                    .and_then(|t| t.wrath.as_ref())
                    .map_or(0, |w| w.suggested_players),
                daily: quest_log
                    .template(r.quest_id, &commands)
                    .is_some_and(|t| t.wrath.is_some() && t.flags & 0x1000 != 0),
                quest_id: r.quest_id,
                title,
                level,
                tag,
                is_header: false,
                collapsed: false,
                complete,
                pushable,
                // Absolute unix seconds, 0 untimed; the bindings subtract the server clock.
                timer: r.log_slot.timer,
                objectives,
                detail,
                folded_under: if collapsed { header } else { 0 },
            };
            if collapsed {
                folded.push(entry);
            } else {
                entries.push(entry);
                header_keys.push(None);
            }
        }
    }
    entries.extend(folded);
    header_keys.resize(entries.len(), None);

    // ── Selection remap ─────────────────────────────────────────────────────────────────────────
    let sel = script.quest_log_selection();
    let new_sel = remap_selection(&last.entries, &entries, sel);
    if new_sel != sel {
        script.set_quest_log_selection(new_sel);
    }
    // Every cached row, folded or not: the table `AbandonQuest` searches.
    quest_log.row_slots = rows.iter().map(|r| (r.quest_id, r.slot)).collect();
    quest_log.header_keys = header_keys;
    quest_log.tooltip_rows = tooltip_rows;

    // Build 12340's quest reputation rewards (`0x58d050`): each quest's staged list, which
    // `ProcessQuestLogRewardFactions` stages for the selected quest.
    if let (Some(table), Some(factions)) = (
        rewards.as_deref().and_then(|r| r.faction_rewards()),
        factions.as_deref(),
    ) {
        let catalog = factions.catalog();
        let (race, class) = (
            store.0.unit_race().unwrap_or(0),
            store.0.unit_class().unwrap_or(0),
        );
        let standing = |f: u32| {
            catalog.faction(f).map_or(0, |info| {
                let wire = usize::try_from(info.rep_index)
                    .ok()
                    .and_then(|i| reputations.as_ref()?.0.get(i))
                    .map_or(0, |&(_, s)| s);
                info.base_for(race, class) + wire
            })
        };
        script.set_wrath_quest_reward_factions(
            rows.iter()
                .filter_map(|r| {
                    let w = quest_log.templates.get(r.quest_id)?.wrath.as_ref()?;
                    let staged = crate::quest_reputation::staged(
                        &w.reward_factions,
                        w.reward_rep_show_mask,
                        table,
                        catalog,
                        &standing,
                    );
                    Some((r.quest_id, staged))
                })
                .collect(),
        );
    }
    // Build 12340's quest item drops (`0x5e1ff0`, `0x5e20a0`): each quest's source item and its
    // objectives' drops, off the template.
    script.set_wrath_quest_item_drops(
        rows.iter()
            .filter_map(|r| {
                let t = quest_log.templates.get(r.quest_id)?;
                let drops = t
                    .wrath
                    .as_ref()
                    .map(|w| w.item_drops.to_vec())
                    .unwrap_or_default();
                Some((r.quest_id, (t.src_item_id, drops)))
            })
            .collect(),
    );
    let fresh = QuestLogState {
        entries,
        num_quests: rows.len() as u32,
    };
    if fresh == *last {
        return;
    }
    // Announced after the push, so a handler reads the new log. The reference announces from the
    // `SMSG_QUESTUPDATE_*` handler (`0x5e5ad0`), dropping a toast whose name is uncached; this
    // rides the log diff, folded quests included, as the handler is blind to the fold. A progress
    // toast is the leaderboard line, as the `ERR_QUEST_ADD_*_SII` keys compose the same sentence.
    let progressed = quests_with_progressed_objectives(last, &fresh);
    script.set_quest_log(fresh.clone());
    // `QUEST_WATCH_UPDATE` with the quest's 1-based row among all rows (`0xc237b0`)
    // goes first, from the `SMSG_QUESTUPDATE_*` handler, so the log update after it redraws the
    // tracker stock's 300 s auto-watch grew (`QuestLogFrame.lua:265-269`); the reference fires it
    // before the descriptor lands, here after the push.
    for quest in &progressed {
        let row = ScriptValue::Int(i64::from(quest.index));
        script.fire_event("QUEST_WATCH_UPDATE", vec![row]);
    }
    script.fire_event("QUEST_LOG_UPDATE", vec![]);
    if !progressed.is_empty() {
        for quest in progressed {
            for line in quest.changed_lines {
                script.fire_event("UI_INFO_MESSAGE", vec![ScriptValue::Str(line)]);
            }
            if quest.completed {
                // Catalog row `0xf8`, or `0xf9` with no text, never the turn-in's
                // `ERR_QUEST_COMPLETE_S`. The reference shows it only on
                // `SMSG_QUESTUPDATE_COMPLETE` (`0x5e5d12`), its `%s` the quest's end text
                // (`+0x129c`, `0x5e5d22`); this shows the title, on any completion flip.
                let line = if quest.title.is_empty() {
                    crate::ui_action::keyed_line(&script, "ERR_QUEST_UNKNOWN_COMPLETE")
                } else {
                    crate::ui_action::keyed_line_s(
                        &script,
                        "ERR_QUEST_OBJECTIVE_COMPLETE_S",
                        &[&quest.title],
                    )
                };
                crate::ui_action::show_messages(&mut script, &mut sink, "ui_quest_log", line);
            }
        }
    }
    *last = fresh;
}

/// `set_wrath_quest_special_items`: each logged quest's usable item, as the Lua `(bag, slot)` of
/// its first copy in the backpack or the four bags, which `GetQuestLogSpecialItemInfo`,
/// `UseQuestLogSpecialItem` and their kin read. Labelled inferred: the reference picks the item in
/// `0x5e0da0`, not traced; this takes the template's `SrcItemId`.
#[allow(clippy::type_complexity)] // Bevy system parameters / a closure table, kept inline
fn feed_quest_special_items(
    script: Option<NonSendMut<UiScript>>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    quest_log: Res<QuestLog>,
    objects: crate::net::Objects,
    commands: Res<NetCommands>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<Vec<(u32, i64, u32)>>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let Some(store) = self_q.iter().next() else {
        return;
    };
    let mut seen = HashSet::new();
    let items: Vec<(u32, i64, u32)> = (0..store.0.player_quest_log_slots())
        .filter_map(|slot| store.0.player_quest_log(slot))
        .filter(|s| s.quest_id != 0 && seen.insert(s.quest_id))
        .filter_map(|s| {
            let source = quest_log.template(s.quest_id, &commands)?.src_item_id;
            if source == 0 {
                return None;
            }
            let (bag, slot, _) = crate::inventory::find_item(
                &store.0,
                &objects,
                source,
                crate::inventory::ItemSearch::default(),
            )?;
            let (bag, slot) = crate::ui_items::lua_bag_pos(bag, slot)?;
            Some((s.quest_id, bag, slot))
        })
        .collect();
    let (last, reset) = last.get_reset(&script);
    if reset || last.as_ref() != Some(&items) {
        *last = Some(items.clone());
        script.set_wrath_quest_special_items(items);
    }
}

/// Every quest-log slot as `(quest id, state)`, the slot watcher's input.
type SlotStates = Vec<(u32, u8)>;

/// The quests the slot watcher `0x5ddd80` announces, per slot as `(quest id, state)`: a quest in a
/// slot that held none, or the same quest with its fail bit cleared (`0x5dde17`-`0x5dde34`). A slot
/// that swaps one quest for another announces nothing.
fn accepted_quests(prior: &[(u32, u8)], now: &[(u32, u8)]) -> Vec<u32> {
    now.iter()
        .zip(prior)
        .filter(|&(&(id, state), &(was_id, was_state))| {
            id != 0
                && (was_id == 0
                    || (was_id == id
                        && state & quest_slot_state::FAIL == 0
                        && was_state & quest_slot_state::FAIL != 0))
        })
        .map(|(&(id, _), _)| id)
        .collect()
}

/// Whether an objective advanced. The reference toasts only on the server's additive
/// `SMSG_QUESTUPDATE_ADD_KILL` and `_ADD_ITEM` (`0x5e5ad0`, `0x5dd060`), with no removal opcode,
/// so a count going down or a name landing late announces nothing. A turn-in destroys the items
/// (vmangos `Player.cpp:13252`) before the slot clears (`Player.cpp:13294`): a silent dip.
fn advanced(now: &QuestLogObjectiveView, was: &QuestLogObjectiveView) -> bool {
    now.cur > was.cur
}

/// One quest whose objectives advanced between two log states.
struct QuestProgress {
    /// The quest's 1-based row in the new log, `GetQuestLogTitle`'s index.
    index: u32,
    /// The COMPLETE toast's `%s` here; empty takes `ERR_QUEST_UNKNOWN_COMPLETE`.
    title: String,
    changed_lines: Vec<String>,
    /// The quest's COMPLETE state turned on in this diff.
    completed: bool,
}

/// The quests in both states with a same-position line that advanced, or whose COMPLETE state
/// turned on. A quest or a line appearing or leaving is not progress.
fn quests_with_progressed_objectives(
    old: &QuestLogState,
    new: &QuestLogState,
) -> Vec<QuestProgress> {
    fn entry_of(s: &QuestLogState, id: u32) -> Option<&QuestLogEntryView> {
        s.entries.iter().find(|e| !e.is_header && e.quest_id == id)
    }
    new.entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_header)
        .filter_map(|(i, e)| {
            let prev = entry_of(old, e.quest_id)?;
            let changed_lines: Vec<String> = e
                .objectives
                .iter()
                .zip(prev.objectives.iter())
                .filter(|(now, was)| advanced(now, was))
                .map(|(now, _)| now.text.clone())
                .collect();
            let completed = e.complete == 1 && prev.complete != 1;
            (!changed_lines.is_empty() || completed).then_some(QuestProgress {
                index: i as u32 + 1,
                title: e.title.clone(),
                changed_lines,
                completed,
            })
        })
        .collect()
}

/// The `ZoneOrSort` header names; without client data only id 0's header has a name.
#[derive(Resource)]
pub struct QuestHeaderNamesRes(pub wrathplus_formats::QuestHeaderNames);

/// Startup: reads the `AreaTable` and `QuestSort` names off the patch chain.
fn load_quest_header_names(
    mut commands: Commands,
    assets: Option<Res<wrathplus_assets::WorldAssets>>,
) {
    use wrathplus_assets::LockRecover;
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_quest_header_names(&mut chain)
    };
    match loaded {
        Ok(names) => commands.insert_resource(QuestHeaderNamesRes(names)),
        Err(e) => warn!("ui_quest_log: quest header names failed to load: {e:#}"),
    }
}

/// The quest tag names by `Type` (`QuestInfo.dbc`); without client data no quest has a tag.
#[derive(Resource)]
pub(crate) struct QuestTagNamesRes(pub wrathplus_formats::QuestTagNames);

/// Startup: reads the `QuestInfo.dbc` tag names off the patch chain.
fn load_quest_tag_names(
    mut commands: Commands,
    assets: Option<Res<wrathplus_assets::WorldAssets>>,
) {
    use wrathplus_assets::LockRecover;
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_quest_tag_names(&mut chain)
    };
    match loaded {
        Ok(names) => commands.insert_resource(QuestTagNamesRes(names)),
        Err(e) => warn!("ui_quest_log: quest tag names failed to load: {e:#}"),
    }
}

/// Drains `CollapseQuestHeader` and `ExpandQuestHeader`: a header row's index names that header
/// by its title; any other index, out of range or a quest row, is every header (`0x5e0c00`:
/// `0x5e0c06` and `0x5e0c18` jump to `0x5e0c87`, which writes the whole mask). The stock tracker
/// passes a quest row (`WatchFrame.lua:987`), so opening the log from it expands every header.
fn drain_quest_log_collapses(
    script: Option<NonSendMut<UiScript>>,
    mut quest_log: ResMut<QuestLog>,
) {
    let Some(mut script) = script else {
        return;
    };
    for (idx, collapse) in script.take_quest_log_collapses() {
        let header = (idx as usize)
            .checked_sub(1)
            .and_then(|i| quest_log.header_keys.get(i).cloned().flatten());
        match (header, collapse) {
            (Some(name), true) => {
                quest_log.collapsed.insert(name);
            }
            (Some(name), false) => {
                quest_log.collapsed.remove(&name);
            }
            (None, true) => {
                let all: Vec<String> = quest_log.header_keys.iter().flatten().cloned().collect();
                quest_log.collapsed.extend(all);
            }
            (None, false) => quest_log.collapsed.clear(),
        }
    }
}

/// `QuestLogPushQuest()`: the quest id taken at the click goes out; the server finds the party.
fn drain_quest_log_pushes(script: Option<NonSendMut<UiScript>>, commands: Res<NetCommands>) {
    let Some(mut script) = script else {
        return;
    };
    for quest in script.take_quest_log_pushes() {
        debug!("ui_quest_log: sharing quest {quest} with the party");
        let _ = commands.0.send(ClientCommand::PushQuestToParty { quest });
    }
}

fn drain_quest_log_abandons(
    script: Option<NonSendMut<UiScript>>,
    quest_log: Res<QuestLog>,
    commands: Res<NetCommands>,
) {
    let Some(mut script) = script else {
        return;
    };
    for quest in script.take_quest_log_abandons() {
        match abandon_slot(quest, &quest_log.row_slots) {
            Some(slot) => {
                debug!("ui_quest_log: abandon quest {quest} → slot {slot}");
                let _ = commands.0.send(ClientCommand::QuestlogRemove { slot });
            }
            None => debug!("ui_quest_log: abandon quest {quest} is not in the log — ignored"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::{Field, PLAYER_QUEST_LOG_SLOTS};

    /// Quest-id dword in a Vanilla fixture; production reads packed slots through ObjectFields.
    fn quest_id_field(slot: u8) -> u16 {
        let span = Field::PlayerQuestLog.span().unwrap();
        let stride = span.width / u16::from(PLAYER_QUEST_LOG_SLOTS);
        assert!(slot < PLAYER_QUEST_LOG_SLOTS);
        span.index + stride * u16::from(slot)
    }

    #[test]
    fn semantic_quest_fixture_preserves_empty_slots_and_last_slot() {
        let fields = ObjectFields::from_pairs(&[
            (quest_id_field(0), 783),
            (quest_id_field(PLAYER_QUEST_LOG_SLOTS - 1), 7),
        ]);
        assert_eq!(fields.player_quest_log(0).unwrap().quest_id, 783);
        assert!(fields.player_quest_log(1).is_none());
        assert_eq!(
            fields
                .player_quest_log(PLAYER_QUEST_LOG_SLOTS - 1)
                .unwrap()
                .quest_id,
            7
        );
    }

    // ── order_groups ────────────────────────────────────────────────────────────────────────────

    fn grow(zos: i32, header: &str, level: u32, title: &str, slot: u8) -> GroupRow {
        GroupRow {
            zone_or_sort: zos,
            header: header.into(),
            level,
            title: title.into(),
            slot,
        }
    }

    #[test]
    fn groups_put_id_zero_first_then_unnamed_then_case_folded_names() {
        let rows = vec![
            grow(12, "Elwynn Forest", 1, "a", 0),
            grow(-3, "warlock", 1, "b", 1), // a byte compare sorts it after "Westfall"
            grow(0, MISSING_HEADER, 1, "c", 2),
            grow(40, "Westfall", 1, "d", 3),
            grow(999, "", 1, "e", 4), // an id neither table names
        ];
        let order: Vec<i32> = order_groups(&rows).into_iter().map(|(z, _, _)| z).collect();
        assert_eq!(order, vec![0, 999, 12, -3, 40]);
    }

    #[test]
    fn the_zero_header_is_the_references_literal() {
        let rows = vec![grow(0, MISSING_HEADER, 1, "a", 0)];
        let groups = order_groups(&rows);
        assert_eq!(groups[0].1, "Missing header! (quest designers)");
    }

    #[test]
    fn within_a_group_it_is_level_then_title_not_slot_order() {
        // Seeded in the reverse of the expected order by slot.
        let rows = vec![
            grow(12, "Elwynn Forest", 9, "Zzz", 0),
            grow(12, "Elwynn Forest", 3, "beta", 1),
            grow(12, "Elwynn Forest", 3, "Alpha", 2),
            grow(12, "Elwynn Forest", 1, "Anything", 3),
        ];
        let groups = order_groups(&rows);
        assert_eq!(groups.len(), 1);
        let titles: Vec<&str> = groups[0]
            .2
            .iter()
            .map(|&i| rows[i].title.as_str())
            .collect();
        assert_eq!(titles, vec!["Anything", "Alpha", "beta", "Zzz"]);
    }

    #[test]
    fn title_order_is_case_insensitive() {
        let rows = vec![
            grow(12, "Zone", 5, "apple", 0),
            grow(12, "Zone", 5, "Banana", 1),
        ];
        let groups = order_groups(&rows);
        let titles: Vec<&str> = groups[0]
            .2
            .iter()
            .map(|&i| rows[i].title.as_str())
            .collect();
        assert_eq!(titles, vec!["apple", "Banana"]);
    }

    // ── remap_selection ─────────────────────────────────────────────────────────────────────────

    fn header(title: &str) -> QuestLogEntryView {
        QuestLogEntryView {
            detail: None,
            quest_id: 0,
            title: title.into(),
            level: 0,
            tag: None,
            is_header: true,
            collapsed: false,
            complete: 0,
            timer: 0,
            pushable: false,
            objectives: Vec::new(),
            ..Default::default()
        }
    }

    fn quest(id: u32) -> QuestLogEntryView {
        QuestLogEntryView {
            detail: None,
            quest_id: id,
            title: format!("Quest {id}"),
            level: 1,
            tag: None,
            is_header: false,
            collapsed: false,
            complete: 0,
            timer: 0,
            pushable: false,
            objectives: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn selection_follows_its_quest_across_reindexing() {
        let old = vec![header("A"), quest(7), quest(9)];
        // A new header lands above: quest 9 moves from entry 3 to entry 5.
        let new = vec![header("A"), quest(7), header("B"), header("C"), quest(9)];
        assert_eq!(remap_selection(&old, &new, 3), 5);
    }

    #[test]
    fn a_quest_leaving_the_log_resets_the_selection_to_zero_never_the_header() {
        // A header is never a selection: 0 lets `QuestLog_SetFirstValidSelection` pick a quest.
        // A fold keeps the quest in the rows, so only an abandon or a turn-in gets here.
        let old = vec![header("A"), quest(7)];
        let new = vec![header("A")];
        assert_eq!(remap_selection(&old, &new, 2), 0);
    }

    #[test]
    fn a_header_selection_is_evicted_not_preserved() {
        let old = vec![header("A"), quest(7)];
        let new = vec![header("A"), quest(7)];
        assert_eq!(remap_selection(&old, &new, 1), 0);
    }

    #[test]
    fn zero_and_stale_selections_stay_zero() {
        let old = vec![header("A"), quest(7)];
        let new = vec![header("A"), quest(7)];
        assert_eq!(remap_selection(&old, &new, 0), 0);
        assert_eq!(remap_selection(&old, &new, 99), 0); // out-of-range: nothing to follow
    }

    fn obj(
        creature_or_go: u32,
        required_count: u32,
        item_id: u32,
        item_count: u32,
        text: &str,
    ) -> QuestObjective {
        QuestObjective {
            creature_or_go,
            required_count,
            item_id,
            item_count,
            text: text.into(),
        }
    }

    // ── creature_line / item_line ───────────────────────────────────────────────────────────────

    /// Templates that name their key, as enUS gives each leaderboard key the same sentence as its
    /// `ERR_QUEST_ADD_*` twin.
    fn probe_strings(key: &str) -> Option<String> {
        match key {
            "QUEST_MONSTERS_KILLED" => Some("<KILLED %s %d of %d>".into()),
            "QUEST_PLAYERS_KILLED" => Some("<PLAYERS %d of %d>".into()),
            "QUEST_OBJECTS_FOUND" => Some("<FOUND %s %d of %d>".into()),
            "QUEST_ITEMS_NEEDED" => Some("<NEEDED %s %d of %d>".into()),
            _ => None,
        }
    }

    #[test]
    fn creature_objective_with_resolved_name() {
        let o = obj(100, 10, 0, 0, "");
        let line = creature_line(&o, 3, Some("Kobold Vermin"), &probe_strings).unwrap();
        assert_eq!(line.text, "<KILLED Kobold Vermin 3 of 10>");
        assert_eq!(line.kind, "monster");
        assert!(!line.finished);
    }

    #[test]
    fn creature_objective_in_flight_shows_a_placeholder() {
        let o = obj(100, 10, 0, 0, "");
        let line = creature_line(&o, 0, None, &probe_strings).unwrap();
        assert_eq!(line.text, "<KILLED ... 0 of 10>");
    }

    /// The GameObject key is fetched above the cache test (`0x4e031e`), so a nameless one keeps it.
    #[test]
    fn go_objective_falls_back_without_a_name_cache() {
        let go_id = ((-57i32) as u32) | 0x8000_0000;
        let o = obj(go_id, 1, 0, 0, "");
        let line = creature_line(&o, 1, None, &probe_strings).unwrap();
        assert_eq!(line.text, "<FOUND ... 1 of 1>");
        assert_eq!(line.kind, "object");
        assert!(line.finished); // cur (1) >= req (1)
    }

    #[test]
    fn go_objective_prefers_its_custom_text_over_the_fallback() {
        let go_id = ((-57i32) as u32) | 0x8000_0000;
        let o = obj(go_id, 4, 0, 0, "Destroy the barricade");
        let line = creature_line(&o, 1, None, &probe_strings).unwrap();
        assert_eq!(line.text, "<FOUND Destroy the barricade 1 of 4>");
        assert_eq!(line.kind, "object");
    }

    #[test]
    fn item_objective_counts_bags_and_clamps_to_required() {
        let o = obj(0, 0, 2000, 5, "");
        // 8 carried, 5 required: the line clamps.
        let line = item_line(&o, 0, Some("Kobold Ear"), 8, &probe_strings).unwrap();
        assert_eq!(line.text, "<NEEDED Kobold Ear 5 of 5>");
        assert_eq!(line.kind, "item");
        assert!(line.finished);
    }

    #[test]
    fn item_objective_in_flight_shows_a_placeholder() {
        let o = obj(0, 0, 2000, 5, "");
        let line = item_line(&o, 0, None, 2, &probe_strings).unwrap();
        assert_eq!(line.text, "<NEEDED ... 2 of 5>");
    }

    /// An override takes `QUEST_OBJECTS_FOUND` (`0x84b634` at `0x4e03e1`); `type` is `"monster"`.
    #[test]
    fn custom_text_overrides_the_creature_auto_line() {
        let o = obj(100, 10, 0, 0, "Slay the vermin");
        let line = creature_line(&o, 4, Some("Kobold Vermin"), &probe_strings).unwrap();
        assert_eq!(line.text, "<FOUND Slay the vermin 4 of 10>");
        assert_eq!(line.kind, "monster");
    }

    /// Against the real `GlobalStrings.lua`; skips without client data.
    #[test]
    fn the_leaderboard_keys_resolve_in_the_real_global_strings() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let src = chain
            .read_file("Interface\\FrameXML\\GlobalStrings.lua")
            .expect("GlobalStrings.lua in the chain");
        let vm = UiScript::new().expect("VM");
        vm.run(&String::from_utf8_lossy(&src)).expect("runs clean");
        let get = |key: &str| {
            vm.lua()
                .globals()
                .get::<String>(key)
                .ok()
                .filter(|t| !t.is_empty())
        };
        for key in [
            "QUEST_MONSTERS_KILLED",
            "QUEST_OBJECTS_FOUND",
            "QUEST_ITEMS_NEEDED",
            "QUEST_PLAYERS_KILLED",
        ] {
            assert!(get(key).is_some(), "{key} missing");
        }
        let o = obj(100, 10, 0, 0, "");
        let line = creature_line(&o, 3, Some("Kobold Vermin"), &get).unwrap();
        assert_eq!(line.text, "Kobold Vermin slain: 3/10");
    }

    /// A line's `finished` is `cur >= req` alone: `GetQuestLogLeaderBoard` reads the COMPLETE bit
    /// only for the `event` line (`0x4e02a2`); the turn-in predicate `0x4df580` reads both.
    #[test]
    fn a_complete_quest_does_not_mark_a_short_objective_finished() {
        let o = obj(100, 10, 0, 0, "");
        // 1 of 10: not finished, whatever the quest's state.
        let line = creature_line(&o, 1, Some("Kobold"), &probe_strings).unwrap();
        assert_eq!(line.text, "<KILLED Kobold 1 of 10>");
        assert!(!line.finished);
    }

    #[test]
    fn empty_objective_slot_emits_no_line() {
        let o = obj(0, 0, 0, 0, "");
        assert!(creature_line(&o, 0, None, &probe_strings).is_none());
        assert!(item_line(&o, 0, None, 0, &probe_strings).is_none());
    }

    // ── build_objectives ────────────────────────────────────────────────────────────────────────

    fn quest_template(objectives: [QuestObjective; 4]) -> QuestTemplate {
        QuestTemplate {
            wrath: None,
            quest_id: 1,
            method: 0,
            level: 1,
            zone_or_sort: 0,
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
            title: "A Threat Within".into(),
            objectives_text: String::new(),
            details: String::new(),
            end_text: String::new(),
            objectives,
        }
    }

    #[test]
    fn build_objectives_reads_every_occupied_quad_via_the_slots_counters_and_skips_empties() {
        let template = quest_template([
            obj(100, 10, 0, 0, ""), // creature objective, counter[0]
            obj(0, 0, 0, 0, ""),    // unused quad: no line
            obj(0, 0, 2000, 5, ""), // item objective (counted from the bags, not a counter)
            obj(0, 0, 0, 0, ""),    // unused quad
        ]);
        let log_slot = QuestLogSlot {
            quest_id: 1,
            counters: [3, 0, 0, 0],
            state: 0,
            timer: 0,
        };
        let store = ObjectFields::default(); // empty bags: the item objective reads 0/5
        let items = Items::default();
        let mut objs = crate::items::TestObjects::new();
        let objects = objs.get();
        let names = NameCache::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);

        let objectives = build_objectives(
            &template,
            &log_slot,
            &store,
            &objects,
            &items,
            &names,
            &commands,
            &probe_strings,
        );

        // The unused quads emit nothing; no names are seeded, so both lines wear the placeholder.
        assert_eq!(objectives.len(), 2);
        assert_eq!(objectives[0].text, "<KILLED ... 3 of 10>");
        assert_eq!(objectives[0].kind, "monster");
        assert!(!objectives[0].finished);
        assert_eq!(objectives[1].text, "<NEEDED ... 0 of 5>");
        assert_eq!(objectives[1].kind, "item");
    }

    /// A 3.3.5 PvP objective reads slot counter 0 and closes the list.
    #[test]
    fn a_pvp_objective_counts_in_slot_counter_zero_and_finishes_at_its_target() {
        let mut template = quest_template([
            obj(0, 0, 2000, 5, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
        ]);
        template.wrath = Some(wrathplus_protocol::types::WrathQuestInfo {
            players_slain: 10,
            ..Default::default()
        });
        let mut log_slot = QuestLogSlot {
            quest_id: 1,
            counters: [4, 0, 0, 0],
            state: 0,
            timer: 0,
        };
        let store = ObjectFields::default();
        let items = Items::default();
        let mut objs = crate::items::TestObjects::new();
        let objects = objs.get();
        let names = NameCache::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);
        let build = |template: &QuestTemplate, slot: &QuestLogSlot| {
            build_objectives(
                template,
                slot,
                &store,
                &objects,
                &items,
                &names,
                &commands,
                &probe_strings,
            )
        };
        let lines = build(&template, &log_slot);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (
                lines[1].text.as_str(),
                lines[1].kind.as_str(),
                lines[1].finished
            ),
            ("<PLAYERS 4 of 10>", "player", false)
        );
        log_slot.counters[0] = 12;
        let lines = build(&template, &log_slot);
        assert_eq!((lines[1].cur, lines[1].finished), (10, true));
        // A Vanilla-shaped or PvP-free template has no such line.
        template.wrath = None;
        assert_eq!(build(&template, &log_slot).len(), 1);
    }

    /// Creatures (`0x4e02d7`) and items (`0x4e04f4`) are separate passes, so a quad carrying both
    /// gives two lines. The fixture is quest 358's shape: creature 1941 ×8 and item 2834 ×8 at
    /// index 0, a second creature at 1.
    #[test]
    fn a_quad_carrying_both_a_creature_and_an_item_emits_both_lines() {
        let template = quest_template([
            obj(1941, 8, 2834, 8, ""), // both kinds, one index
            obj(1675, 5, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
        ]);
        let log_slot = QuestLogSlot {
            quest_id: 358,
            counters: [8, 1, 0, 0],
            state: 0,
            timer: 0,
        };
        let store = ObjectFields::default();
        let items = Items::default();
        // Empty bags: every item objective reads 0.
        let mut objs = crate::items::TestObjects::new();
        let objects = objs.get();
        let names = NameCache::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);

        let objectives = build_objectives(
            &template,
            &log_slot,
            &store,
            &objects,
            &items,
            &names,
            &commands,
            &probe_strings,
        );

        // Pass order: both creatures, then the item.
        assert_eq!(objectives.len(), 3, "{objectives:#?}");
        assert_eq!(objectives[0].text, "<KILLED ... 8 of 8>");
        assert_eq!(objectives[1].text, "<KILLED ... 1 of 5>");
        assert_eq!(objectives[2].text, "<NEEDED ... 0 of 8>");
        assert_eq!(objectives[2].kind, "item");
    }

    /// The override text is the creature branch's (`0x4e03da`), never the item line's.
    #[test]
    fn a_shared_quads_objective_text_names_the_creature_not_the_item() {
        let template = quest_template([
            obj(1941, 8, 2834, 8, "Graverobbers routed"),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
        ]);
        let log_slot = QuestLogSlot {
            quest_id: 358,
            counters: [8, 0, 0, 0],
            state: 0,
            timer: 0,
        };
        let store = ObjectFields::default();
        let items = Items::default();
        // Empty bags: every item objective reads 0.
        let mut objs = crate::items::TestObjects::new();
        let objects = objs.get();
        let names = NameCache::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);

        let objectives = build_objectives(
            &template,
            &log_slot,
            &store,
            &objects,
            &items,
            &names,
            &commands,
            &probe_strings,
        );
        assert_eq!(objectives.len(), 2);
        // The override is the creature's name, under `QUEST_OBJECTS_FOUND` (`0x4e03e1`).
        assert_eq!(objectives[0].text, "<FOUND Graverobbers routed 8 of 8>");
        // The item line wears the in-flight placeholder, never the creature's text.
        assert_eq!(objectives[1].text, "<NEEDED ... 0 of 8>");
    }

    /// `ReqItemId[i] != SrcItemId` (`0x4e00ce`, `0x4e04fa`).
    #[test]
    fn the_quests_own_source_item_is_not_a_leaderboard_line() {
        let mut template = quest_template([
            obj(0, 0, 2834, 8, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
        ]);
        template.src_item_id = 2834;
        let log_slot = QuestLogSlot {
            quest_id: 1,
            counters: [0; 4],
            state: 0,
            timer: 0,
        };
        let store = ObjectFields::default();
        let items = Items::default();
        // Empty bags: every item objective reads 0.
        let mut objs = crate::items::TestObjects::new();
        let objects = objs.get();
        let names = NameCache::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);

        let objectives = build_objectives(
            &template,
            &log_slot,
            &store,
            &objects,
            &items,
            &names,
            &commands,
            &probe_strings,
        );
        assert!(objectives.is_empty(), "{objectives:#?}");
    }

    #[test]
    fn a_complete_slot_state_does_not_finish_a_short_line() {
        let template = quest_template([
            obj(100, 10, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
        ]);
        let log_slot = QuestLogSlot {
            quest_id: 1,
            counters: [1, 0, 0, 0],            // 1 of the 10 required
            state: quest_slot_state::COMPLETE, // yet the quest is complete
            timer: 0,
        };
        let store = ObjectFields::default();
        let items = Items::default();
        // Empty bags: every item objective reads 0.
        let mut objs = crate::items::TestObjects::new();
        let objects = objs.get();
        let names = NameCache::default();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);

        let objectives = build_objectives(
            &template,
            &log_slot,
            &store,
            &objects,
            &items,
            &names,
            &commands,
            &probe_strings,
        );
        assert_eq!(objectives.len(), 1);
        assert!(
            !objectives[0].finished,
            "the counter is 1/10 — the quest's COMPLETE bit is not this line's business"
        );
        assert_eq!(objectives[0].text, "<KILLED ... 1 of 10>");
    }

    // ── money_split ──────────────────────────────────────────────────────────────────────────────

    #[test]
    fn money_sign_splits_into_required_or_reward() {
        assert_eq!(money_split(-500), (500, 0));
        assert_eq!(money_split(500), (0, 500));
        assert_eq!(money_split(0), (0, 0));
    }

    // ── abandon_slot ────────────────────────────────────────────────────────────────────────────

    #[test]
    fn abandon_maps_quest_id_to_slot_across_a_gap() {
        // Slots 0 and 2, slot 1 empty: each id finds its own slot.
        let row_slots = vec![(783u32, 0u8), (7, 2)];
        assert_eq!(abandon_slot(783, &row_slots), Some(0));
        assert_eq!(abandon_slot(7, &row_slots), Some(2));
        assert_eq!(abandon_slot(8, &row_slots), None); // not in the log: nothing to remove
        assert_eq!(abandon_slot(0, &row_slots), None); // 0 is never a quest
    }

    // ── The abandon mark is a quest id ──────────────────────────────────────────────────────────

    /// `SetAbandonQuest` (`0x4dfb50`) copies the selection `0xbb7480`, a quest id (`0x4def30`,
    /// `0x4def5d`), into the mark `0xbb7484`, and `AbandonQuest` (`0x4dfe00`, `0x4df070`) finds its
    /// row in `0xbb71c0` again, so a fold while the popup is up cannot retarget the abandon.
    #[test]
    fn a_fold_between_mark_and_confirm_does_not_retarget_the_abandon() {
        // Z1 = zone 0 ("Missing header!", forced first) holding A; Z2 = zone 7 holding B and C.
        // Entries: [Z1, A, Z2, B, C]; B is row 4.
        const A: u32 = 101;
        const B: u32 = 201;
        const C: u32 = 202;
        let quests = [(0u8, A, 0i32, "A"), (1, B, 7, "B"), (2, C, 7, "C")];

        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(NetCommands(tx));
        app.init_resource::<crate::net::GuidIndex>()
            .init_resource::<NameCache>()
            .init_resource::<Items>()
            .init_resource::<crate::app_core::state::WorldStates>()
            .init_resource::<crate::app_core::chat::ChatLog>()
            .init_resource::<crate::sound::MessageSounds>()
            .init_resource::<crate::ui_quest::QuestLines>();
        let mut log = QuestLog::default();
        let mut pairs = Vec::new();
        for (slot, id, zos, title) in quests {
            let mut t = quest_template(std::array::from_fn(|_| obj(0, 0, 0, 0, "")));
            t.quest_id = id;
            t.zone_or_sort = zos;
            t.title = title.into();
            // A scales with the player: its log level is the player's 10 (`0x5e0d7a`).
            t.level = if id == A { -1 } else { 1 };
            log.insert_template(t);
            pairs.push((quest_id_field(slot), id));
        }
        pairs.push((Field::UnitLevel.span().unwrap().index, 10));
        app.insert_resource(log);
        app.world_mut().spawn((
            ObjectStore(ObjectFields::from_pairs(&pairs)),
            Guid(0x2A),
            SelfPlayer,
        ));
        app.insert_non_send_resource(UiScript::new().unwrap());
        app.init_resource::<crate::ui_unit_quest_tooltip::CreatureQuestLinksCache>();
        app.add_systems(
            Update,
            (
                drain_quest_log_collapses,
                feed_quest_log,
                crate::ui_unit_quest_tooltip::publish,
                drain_quest_log_abandons,
            )
                .chain(),
        );
        let run = |app: &mut App, lua: &str| {
            app.world_mut()
                .non_send_resource_mut::<UiScript>()
                .run(lua)
                .unwrap();
        };

        app.update();
        assert_eq!(app.world().resource::<QuestLog>().tooltip_rows.len(), 3);
        let eval = |app: &App, lua: &str| {
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<i64>(lua)
                .unwrap()
        };
        assert_eq!(eval(&app, "return select(2, GetQuestLogTitle(2))"), 10);
        // Folded, A moves after every visible row and stays selected there: numEntries drops
        // (`0x5e0b80`) and a collapse never writes the selection `[0xc23ad8]`.
        run(&mut app, "SelectQuestLogEntry(2); CollapseQuestHeader(1)");
        app.update();
        assert_eq!(
            eval(
                &app,
                "return GetNumQuestLogEntries() * 10 + GetQuestLogSelection()"
            ),
            45
        );
        // A quest row, folded A's here, expands every header (`0x5e0c18` → `0x5e0c87`).
        run(&mut app, "ExpandQuestHeader(5)");
        app.update();
        assert_eq!(eval(&app, "return GetNumQuestLogEntries()"), 5);
        run(&mut app, "SelectQuestLogEntry(4); SetAbandonQuest()");
        assert_eq!(
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<String>("return GetAbandonQuestName()")
                .unwrap(),
            "B"
        );
        // With the popup up, fold Z1: B moves to row 3 and C takes row 4.
        run(&mut app, "CollapseQuestHeader(1)");
        app.update();
        assert_eq!(
            app.world().resource::<QuestLog>().tooltip_rows.len(),
            3,
            "folding a header must not hide its quest from the creature tooltip"
        );
        run(&mut app, "AbandonQuest()");
        app.update();

        let sent: Vec<_> = rx
            .try_iter()
            .filter(|c| matches!(c, ClientCommand::QuestlogRemove { .. }))
            .collect();
        assert!(
            matches!(sent.as_slice(), [ClientCommand::QuestlogRemove { slot: 1 }]),
            "the abandon removes B's slot (1), never C's (2): {sent:?}"
        );
        // Out of range, a negative index collapses every header (`0x5e0c06` → `0x5e0c87`).
        run(&mut app, "CollapseQuestHeader(-2.5)");
        app.update();
        assert_eq!(eval(&app, "return GetNumQuestLogEntries()"), 2);
    }

    // ── The accept line ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_slot_watcher_announces_a_filled_slot_and_a_cleared_failure_only() {
        use quest_slot_state::FAIL;
        let prior = [(0, 0), (7, 0), (8, FAIL), (9, FAIL), (10, 0), (11, 0)];
        let now = [(783, 0), (7, 0), (8, 0), (9, FAIL), (12, 0), (0, 0)];
        assert_eq!(
            accepted_quests(&prior, &now),
            [783, 8],
            "an empty slot filled, and the same quest with its fail bit cleared; a kept quest, a \
             kept failure, a slot swapping quests and a cleared slot say nothing"
        );
    }

    /// The watcher runs on the log's slots, not its rows: an accepted quest is announced before its
    /// template lands, and the login snapshot is silent.
    #[test]
    fn a_quest_entering_the_log_queues_its_accept_line() {
        use crate::ui_quest::{QuestLine, QuestLines};

        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(NetCommands(tx));
        app.init_resource::<crate::net::GuidIndex>()
            .init_resource::<NameCache>()
            .init_resource::<Items>()
            .init_resource::<crate::app_core::state::WorldStates>()
            .init_resource::<crate::app_core::chat::ChatLog>()
            .init_resource::<crate::sound::MessageSounds>()
            .init_resource::<QuestLines>()
            .init_resource::<QuestLog>();
        let player = app
            .world_mut()
            .spawn((
                ObjectStore(ObjectFields::from_pairs(&[(quest_id_field(0), 7)])),
                Guid(0x2A),
                SelfPlayer,
            ))
            .id();
        app.insert_non_send_resource(UiScript::new().unwrap());
        app.add_systems(Update, feed_quest_log);

        app.update();
        assert!(
            app.world().resource::<QuestLines>().queued().is_empty(),
            "the login snapshot fills the log silently"
        );

        app.world_mut()
            .entity_mut(player)
            .insert(ObjectStore(ObjectFields::from_pairs(&[
                (quest_id_field(0), 7),
                (quest_id_field(1), 783),
            ])));
        app.update();
        assert_eq!(
            app.world().resource::<QuestLines>().queued(),
            [QuestLine::Accepted(783)]
        );
        assert!(
            app.world_mut()
                .non_send_resource_mut::<UiScript>()
                .take_sounds()
                .is_empty(),
            "the sound is the accept row's, played with its line"
        );
    }

    mod progress;

    /// A quest link asks the template once; when it lands, the waiting tooltip fills with its
    /// title, objectives text and a requirement row per objective line.
    #[test]
    fn a_quest_link_asks_once_then_fills_from_the_template() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.insert_resource(NetCommands(tx));
        app.init_resource::<crate::net::GuidIndex>()
            .init_resource::<NameCache>()
            .init_resource::<Items>()
            .init_resource::<crate::app_core::state::WorldStates>()
            .init_resource::<QuestLog>();
        app.world_mut().spawn((
            ObjectStore(ObjectFields::from_pairs(&[])),
            Guid(0x2A),
            SelfPlayer,
        ));
        let s = UiScript::new().unwrap();
        s.run(
            r#"
            local a = CreateFrame("Button", "LinkOwner"); a:SetPoint("CENTER", 0, 0)
            a:SetWidth(10); a:SetHeight(10)
            local tt = CreateFrame("GameTooltip", "TT")
            tt:SetOwner(a, "ANCHOR_RIGHT")
            QUEST_TOOLTIP_REQUIREMENTS = "Requirements:"
            QUEST_MONSTERS_KILLED = "%s slain: %d/%d"
            QUEST_OBJECTS_FOUND = "%s: %d/%d"
            TT:SetHyperlink("|cffffff00|Hquest:1:1|h[A Threat Within]|h|r")
            "#,
        )
        .unwrap();
        app.insert_non_send_resource(s);
        app.add_systems(Update, feed_quest_links);

        app.update();
        app.update();
        // The quest asks; the unnamed test player's own name query rides along.
        let quest_queries = |rx: &crossbeam_channel::Receiver<ClientCommand>| {
            rx.try_iter()
                .filter(|c| matches!(c, ClientCommand::QuestQuery { .. }))
                .collect::<Vec<_>>()
        };
        let asked = quest_queries(&rx);
        assert!(
            matches!(asked[..], [ClientCommand::QuestQuery { quest: 1 }]),
            "asked once: {asked:?}"
        );

        let mut template = quest_template([
            obj(0x8000_0000 | 33, 2, 0, 0, "Gnoll Totem"),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
            obj(0, 0, 0, 0, ""),
        ]);
        template.objectives_text = "Burn the totems.".into();
        app.world_mut()
            .resource_mut::<QuestLog>()
            .insert_template(template);
        app.update();
        let lines: Vec<String> = app
            .world()
            .non_send_resource::<UiScript>()
            .eval(
                "local t = {} for i = 1, TT:NumLines() do \
                 t[i] = _G['TTTextLeft' .. i]:GetText() or '' end return t",
            )
            .unwrap();
        assert_eq!(lines.first().map(String::as_str), Some("A Threat Within"));
        assert!(lines.iter().any(|l| l == "Burn the totems."), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with(" - ") && l.contains("0/2")),
            "a requirement row at zero progress: {lines:?}"
        );
        assert!(quest_queries(&rx).is_empty(), "no second ask");
    }
}
