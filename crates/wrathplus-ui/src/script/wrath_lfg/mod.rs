//! Build-12340 LFG metadata. Catalogs are immutable inputs, not queue/eligibility state.
use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::{
    binding_abi::{number_arg, tointeger_arg},
    Model, UiScript,
};

/// Localized native dungeon row. Raw ID/type remain distinct from a packed wire entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgDungeon {
    pub id: u32,
    pub name: String,
    pub min_level: i32,
    pub max_level: i32,
    pub target_level: i32,
    pub target_min_level: i32,
    pub target_max_level: i32,
    pub map_id: i32,
    pub difficulty: i32,
    pub flags: u32,
    pub dungeon_type: i32,
    pub faction: i32,
    pub texture: String,
    pub expansion: i32,
    pub order: i32,
    pub group_id: i32,
    pub description: String,
}

/// Source-ordered immutable projection; no sorting/deduplication of lookup tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgCatalog {
    pub dungeons: Vec<WrathLfgDungeon>,
    /// All eight raw LFGDungeonExpansion words. Native lookup matches words 1/2 and uses
    /// words 4..7 for min/max/target-min/target-max; word 3 remains uninterpreted.
    pub expansions: Vec<[u32; 8]>,
    /// `(map, difficulty, maximum players)`, retaining source order and signed bits.
    pub map_difficulties: Vec<(i32, i32, i32)>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    groups: Arc<Vec<WrathLfgGroup>>,
    catalog: Arc<WrathLfgCatalog>,
    expansion: Option<u8>,
    locks: Arc<WrathLfgLockSnapshot>,
    presentation: Arc<WrathLfgPresentation>,
    /// The native random list (`0x55aaf0`, count `0xbeaa74`) and best choice (`0x559ea0`,
    /// `0xbeaa24`) as full packed entries, rebuilt from the inputs when a lock block is present.
    derived: l1::Derived,
    /// Accepted explicit lock requests for App to send, in call order.
    requests: Vec<WrathLfgLockRequest>,
    /// Uncached reward-item lookups for App's completion queue, in call order.
    item_requests: Vec<WrathLfgItemRequest>,
    /// L2: the queue snapshot, L0 selection and verbs (`queue`).
    queue: queue::Queue,
    /// LFRFrame's raid browser (`raid_browser`).
    raid_browser: raid_browser::RaidBrowser,
}

impl UiScript {
    /// Install shared metadata and the native session expansion context. `None` uses base bounds,
    /// not expansion zero; it is an explicit unavailable-data policy until the feed is known.
    /// App owns startup/reload
    /// restoration; replacing metadata does not send requests or mutate queue state.
    pub fn set_wrath_lfg_catalog(&mut self, catalog: Arc<WrathLfgCatalog>, expansion: Option<u8>) {
        let mut model = self.model_mut();
        model.wrath_lfg.catalog = catalog;
        model.wrath_lfg.expansion = expansion;
        model.wrath_lfg.rederive();
    }
}

mod derived;
mod l1;
mod requests;
pub use requests::{set_wrath_lfg_request_clock, WrathLfgItemRequest, WrathLfgLockRequest};

mod locks;
pub use locks::{
    WrathLfgLock, WrathLfgLockSnapshot, WrathLfgMemberLocks, WrathLfgPlayerLocks,
    WrathLfgPresentation, WrathLfgPreview, WrathLfgRewardItem,
};

mod queue;
pub(crate) use queue::{boot_vote, start_backfill};

/// The current map and difficulty's `index`-th DungeonEncounter row (0-based) under `mask`:
/// name, icon, killed (`0x5538b0`, shared by the instance-lock popup).
pub(crate) fn lock_encounter(
    m: &Model,
    index: usize,
    mask: u32,
) -> Option<(String, Option<String>, bool)> {
    let (map, difficulty) = m.wrath_lfg.queue.location()?;
    let e = m
        .wrath_lfg
        .queue
        .encounters()
        .iter()
        .filter(|e| e.map == map && e.difficulty == difficulty)
        .nth(index)?;
    Some((
        e.name.clone(),
        e.icon.clone(),
        mask & (1 << (e.bit & 31)) != 0,
    ))
}
pub use queue::{
    WrathLfgBoot, WrathLfgCompletion, WrathLfgCompletionItem, WrathLfgEncounter, WrathLfgProposal,
    WrathLfgProposalRow, WrathLfgQueueRequest, WrathLfgQueueSnapshot, WrathLfgQueueStatus,
    WrathLfgRoleCheck, WrathLfgRoleCheckMember, WrathLfgUpdate,
};

mod raid_browser;
pub use raid_browser::{
    WrathRaidBrowserGroup, WrathRaidBrowserMember, WrathRaidBrowserRequest, WrathRaidBrowserStats,
    WrathRaidBrowserView,
};

mod choices;
mod order;
mod preferences;
pub use choices::WrathLfgGroup;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    preferences::install(lua)?;
    l1::install(lua)?;
    requests::install(lua)?;
    choices::install(lua)?;
    order::install(lua)?;
    queue::install(lua)?;
    raid_browser::install(lua)?;
    lua.globals().set(
        "GetLFGDungeonInfo",
        lua.create_function(|lua, id: Value| {
            // 0x555590 -> 0x5553f0: numeric strings accepted and rounded (`lua_tointeger` at
            // `0x5555bc`), missing row returns zero values.
            let id = tointeger_arg(lua, id, "Usage: GetLFGDungeonInfo(dungeonID)")? as u32;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_lfg;
            let Some(row) = state.catalog.dungeons.iter().find(|row| row.id == id) else {
                return Ok(MultiValue::new());
            };
            dungeon_info(lua, state, row)
        })?,
    )
}

/// `[min, max, targetMin, targetMax]`: the row's, or the session expansion's LFGDungeonExpansion
/// row when one matches.
fn level_bounds(state: &State, row: &WrathLfgDungeon) -> [i32; 4] {
    let id = row.id;
    // 0x6349f0: stop when the matching dungeon's contiguous group ends; first expansion
    // match wins. Never sort this table, or substitute a different expansion's row.
    let mut found_group = false;
    for entry in &state.catalog.expansions {
        if entry[1] != id {
            if found_group {
                break;
            }
            continue;
        }
        found_group = true;
        if state
            .expansion
            .is_some_and(|expansion| entry[2] == u32::from(expansion))
        {
            return [
                entry[4] as i32,
                entry[5] as i32,
                entry[6] as i32,
                entry[7] as i32,
            ];
        }
    }
    [
        row.min_level,
        row.max_level,
        row.target_min_level,
        row.target_max_level,
    ]
}

// Shared by the scalar getter and choice-info array builder (0x554e60).
fn dungeon_info(lua: &Lua, state: &State, row: &WrathLfgDungeon) -> mlua::Result<MultiValue> {
    let bounds = level_bounds(state, row);
    // 0x634950: exact map/difficulty lookup, no fallback. The getter exposes nil on miss.
    let maximum = state
        .catalog
        .map_difficulties
        .iter()
        .skip_while(|&&(map, _, _)| map != row.map_id)
        .take_while(|&&(map, _, _)| map == row.map_id)
        .find(|&&(_, difficulty, _)| difficulty == row.difficulty)
        .map(|&(_, _, maximum)| Value::Integer(maximum as i64))
        .unwrap_or(Value::Nil);
    let text = |s: &str| lua.create_string(s).map(Value::String);
    let number = |n: i32| Value::Integer(n as i64);
    Ok(MultiValue::from_vec(vec![
        text(&row.name)?,
        number(row.dungeon_type),
        number(bounds[0]),
        number(bounds[1]),
        number(row.target_level),
        number(bounds[2]),
        number(bounds[3]),
        number(row.expansion),
        number(row.group_id),
        text(&row.texture)?,
        number(row.difficulty),
        maximum,
        text(&row.description)?,
        Value::Boolean(row.flags & 4 != 0),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dungeon_tuple_abi_and_expansion_override() {
        let mut script = UiScript::new().unwrap();
        assert_eq!(
            script
                .eval::<u32>("return select('#', GetLFGDungeonInfo(42))")
                .unwrap(),
            0
        );
        let row = WrathLfgDungeon {
            id: 42,
            name: "Dungeon".into(),
            dungeon_type: 1,
            min_level: 10,
            max_level: 20,
            target_level: 15,
            target_min_level: 12,
            target_max_level: 18,
            map_id: 33,
            difficulty: 2,
            expansion: 1,
            group_id: -7,
            texture: "Texture".into(),
            description: "Description".into(),
            flags: 4,
            ..Default::default()
        };
        script.set_wrath_lfg_catalog(
            Arc::new(WrathLfgCatalog {
                dungeons: vec![row],
                expansions: vec![[1, 42, 2, 999, 30, 40, 32, 38]],
                map_difficulties: vec![(33, 2, 5)],
            }),
            Some(2),
        );
        assert!(script.eval::<bool>("local v={GetLFGDungeonInfo('41.5')} return #v==14 and v[1]=='Dungeon' and v[2]==1 and v[3]==30 and v[4]==40 and v[5]==15 and v[6]==32 and v[7]==38 and v[8]==1 and v[9]==-7 and v[10]=='Texture' and v[11]==2 and v[12]==5 and v[13]=='Description' and v[14]==true").unwrap());
        for args in ["", "nil", "false", "{}", "'bad'"] {
            assert!(script
                .eval::<MultiValue>(&format!("return GetLFGDungeonInfo({args})"))
                .unwrap_err()
                .to_string()
                .contains("Usage: GetLFGDungeonInfo(dungeonID)"));
        }
        assert_eq!(
            script
                .eval::<u32>("return select('#', GetLFGDungeonInfo(-1))")
                .unwrap(),
            0
        );
    }

    #[test]
    fn missing_map_join_and_expansion_do_not_invent_defaults() {
        let mut script = UiScript::new().unwrap();
        script.set_wrath_lfg_catalog(
            Arc::new(WrathLfgCatalog {
                dungeons: vec![WrathLfgDungeon {
                    id: 42,
                    min_level: -3,
                    ..Default::default()
                }],
                // Matching dungeon group ends before a later expansion match: native scan stops.
                expansions: vec![
                    [1, 42, 1, 0, 10, 20, 12, 18],
                    [2, 43, 2, 0, 0, 0, 0, 0],
                    [3, 42, 2, 0, 30, 40, 32, 38],
                ],
                map_difficulties: vec![(0, 1, 10), (1, 0, 20), (0, 0, 99)],
            }),
            Some(2),
        );
        assert!(script.eval::<bool>("local v={GetLFGDungeonInfo(42)} return select('#',GetLFGDungeonInfo(42))==14 and v[3]==-3 and v[12]==nil and v[13]=='' and v[14]==false").unwrap());
        let catalog = Arc::new(WrathLfgCatalog {
            dungeons: vec![WrathLfgDungeon {
                id: 42,
                min_level: 7,
                ..Default::default()
            }],
            expansions: vec![[1, 42, 0, 0, 99, 100, 99, 100]],
            ..Default::default()
        });
        script.set_wrath_lfg_catalog(catalog.clone(), None);
        assert_eq!(
            script
                .eval::<i32>("return select(3,GetLFGDungeonInfo(42))")
                .unwrap(),
            7
        );
        script.set_wrath_lfg_catalog(catalog, Some(0));
        assert_eq!(
            script
                .eval::<i32>("return select(3,GetLFGDungeonInfo(42))")
                .unwrap(),
            99
        );
        script.set_wrath_lfg_catalog(Arc::default(), None);
        assert_eq!(
            script
                .eval::<u32>("return select('#',GetLFGDungeonInfo(42))")
                .unwrap(),
            0
        );
    }
}
