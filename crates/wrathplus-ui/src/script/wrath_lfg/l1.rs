//! L1 lock, random and reward getters over the published lock snapshot, presentation and L0
//! catalog. Wow.exe 12340: GetLFDLockInfo 0x557930, GetLFDLockPlayerCount 0x557890,
//! GetLFDChoiceLockedState 0x5576b0, GetLFGDungeonRewards 0x557aa0, GetLFGDungeonRewardInfo
//! 0x557be0, GetLFGDungeonRewardLink 0x557d60, GetLFGRandomDungeonInfo 0x5555d0,
//! GetNumRandomDungeons 0x5553b0, GetRandomDungeonBestChoice 0x553120; derived list 0x55aaf0,
//! best choice 0x559ea0/0x559da0, unlocked test 0x559c90. Evidence: `/tmp/azb-ui-lfg-l1-*.log`.
//!
//! Unavailable is truthful: an invalid block answers what native answers for an invalid cache,
//! and a missing presentation key is unavailable (UNKNOWN name, fallback icon, no link). Uncached
//! reward getters do not yet register item callback interests: that FIFO is agreed with App first.

use mlua::{Lua, MultiValue, Value};

use super::derived::{self, Node};
use super::{dungeon_info, level_bounds, State, WrathLfgDungeon};
use crate::script::binding_abi::number_arg;
use crate::script::Model;

/// The native icon path when the display has no icon (`0x70a910`'s fallback under prefix 3).
pub(super) const FALLBACK_ICON: &str = "Interface\\Icons\\INV_Misc_QuestionMark";

/// Full packed entry: the row's type in the top byte, the id in the low 24 bits.
pub(super) fn full_entry(row: &WrathLfgDungeon) -> u32 {
    (row.dungeon_type as u32) << 24 | (row.id & 0xff_ffff)
}

/// The derived random list and best choice, both full entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Derived {
    pub random: Vec<u32>,
    pub best: Option<u32>,
}

/// `0x559c90`: unlocked unless the valid self block, or (outside raids, type 2) a valid party
/// member, holds an active lock (the node's `+0xd` byte) on the full entry.
fn joinable(
    self_valid: bool,
    player: &[Node],
    party: &Option<Vec<(u64, Vec<Node>)>>,
    entry: u32,
) -> bool {
    let locked = |nodes: &[Node]| nodes.iter().any(|n| n.entry == entry && n.active);
    if self_valid && locked(player) {
        return false;
    }
    if entry & 0xff00_0000 != 0x0200_0000 {
        if let Some(party) = party {
            if party.iter().any(|(_, nodes)| locked(nodes)) {
                return false;
            }
        }
    }
    true
}

impl State {
    fn player_nodes(&self) -> Option<Vec<Node>> {
        self.locks.player.as_ref().map(derived::player)
    }

    fn party_nodes(&self) -> Option<Vec<(u64, Vec<Node>)>> {
        self.locks.party.as_ref().map(|members| {
            members
                .iter()
                .map(|m| (m.guid, derived::locks(&m.locks)))
                .collect()
        })
    }

    pub(super) fn row(&self, id: u32) -> Option<&WrathLfgDungeon> {
        self.catalog.dungeons.iter().find(|r| r.id == id)
    }

    /// `0x55aaf0` then `0x559ea0`, run per receipt. Before any lock block has arrived (and
    /// after both are invalidated) the previous result stands, as the native globals do.
    pub(super) fn rederive(&mut self) {
        if self.locks.player.is_none() && self.locks.party.is_none() {
            return;
        }
        let player = self.player_nodes().unwrap_or_default();
        let party = self.party_nodes();
        // A row qualifies when random (type 6), or a holiday (flags & 4) whose self node exists
        // and is not an active lock (`0x559d40`).
        let random: Vec<u32> = self
            .catalog
            .dungeons
            .iter()
            .filter(|row| {
                row.dungeon_type == 6
                    || (row.flags & 4 != 0
                        && player
                            .iter()
                            .find(|n| n.entry == full_entry(row))
                            .is_some_and(|n| !n.active))
            })
            .map(full_entry)
            .collect();
        let unlocked = |entry: u32| joinable(self.locks.player.is_some(), &player, &party, entry);
        let mut best: Option<&WrathLfgDungeon> = None;
        for entry in &random {
            let Some(row) = self.row(entry & 0xff_ffff) else {
                continue;
            };
            if row.dungeon_type != 6 || !unlocked(row.id & 0xff_ffff | 0x0600_0000) {
                continue;
            }
            // `0x559da0`: signed strict comparisons, so an exact tie keeps the earlier one.
            let wins = match best {
                None => true,
                Some(b) if b.expansion != row.expansion => b.expansion < row.expansion,
                Some(b) => {
                    let (bb, cb) = (level_bounds(self, b), level_bounds(self, row));
                    let (bt, ct) = (bb[2].wrapping_add(bb[3]) / 2, cb[2].wrapping_add(cb[3]) / 2);
                    if bt != ct {
                        bt < ct
                    } else {
                        bb[0] < cb[0]
                    }
                }
            };
            if wins {
                best = Some(row);
            }
        }
        self.derived = Derived {
            best: best.map(full_entry),
            random,
        };
    }
}

fn int(n: i64) -> Value {
    Value::Integer(n)
}

/// The global string `key`, or the key itself when the strings are not loaded.
fn localized(lua: &Lua, key: &str) -> mlua::Result<Value> {
    let value: Value = lua.globals().get(key)?;
    Ok(if matches!(value, Value::String(_)) {
        value
    } else {
        Value::String(lua.create_string(key)?)
    })
}

/// The n-th (0-based) preview item of the valid self block's node for `entry` (`0x5574c0`).
pub(super) fn preview_item(
    state: &State,
    entry: u32,
    index: i32,
) -> Option<super::WrathLfgRewardItem> {
    let index = usize::try_from(index).ok()?;
    let nodes = state.player_nodes()?;
    let node = nodes.into_iter().find(|n| n.entry == entry)?;
    node.preview.items.get(index).cloned()
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // IsLFGDungeonJoinable(dungeonID) (`0x55a040`): a listed LFGDungeons row answers
    // `0x559c90(type << 24 | id)`; an unknown id answers nothing.
    g.set(
        "IsLFGDungeonJoinable",
        lua.create_function(|lua, id: Value| {
            let id = number_arg(lua, id, "Usage: IsLFGDungeonJoinable(dungeonID)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_lfg;
            let Some(row) = u32::try_from(id).ok().and_then(|id| state.row(id)) else {
                return Ok(MultiValue::new());
            };
            let player = state.player_nodes().unwrap_or_default();
            let party = state.party_nodes();
            let ok = joinable(
                state.locks.player.is_some(),
                &player,
                &party,
                full_entry(row),
            );
            Ok(MultiValue::from_vec(vec![Value::Boolean(ok)]))
        })?,
    )?;

    // GetLFDLockInfo(dungeonID, index) → name, reason. Index 1 is the player (self validity),
    // 2.. the party members in receipt order (party validity); invalid or out of range answers
    // nothing. The first node whose low 24 bits match the id answers its reason when active,
    // else 0; no node is 0.
    g.set(
        "GetLFDLockInfo",
        lua.create_function(|lua, (id, index): (Value, Value)| {
            const USAGE: &str = "Usage: GetLFDLockInfo(dungeonID, playerIndex)";
            let id = number_arg(lua, id, USAGE)?;
            let index = number_arg(lua, index, USAGE)?;
            if id < 0 {
                return Ok(MultiValue::new());
            }
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_lfg;
            let (guid, nodes) = if index == 1 {
                let Some(nodes) = state.player_nodes() else {
                    return Ok(MultiValue::new());
                };
                (state.presentation.self_guid, nodes)
            } else {
                let Some(party) = state.party_nodes() else {
                    return Ok(MultiValue::new());
                };
                let Some(member) = usize::try_from(index.wrapping_sub(2))
                    .ok()
                    .and_then(|i| party.into_iter().nth(i))
                else {
                    return Ok(MultiValue::new());
                };
                member
            };
            let name = match state.presentation.guid_names.get(&guid) {
                Some(name) => Value::String(lua.create_string(name)?),
                None => localized(lua, "UNKNOWN")?,
            };
            let reason = nodes
                .iter()
                .find(|n| n.entry & 0xff_ffff == id as u32)
                .map_or(0, |n| if n.active { n.reason as i32 } else { 0 });
            Ok(MultiValue::from_vec(vec![name, int(i64::from(reason))]))
        })?,
    )?;

    // GetLFDLockPlayerCount() → 0 with no valid self block, 1 with no valid party block, else
    // members + 1.
    g.set(
        "GetLFDLockPlayerCount",
        lua.create_function(|lua, _ignored: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let locks = &model.wrath_lfg.locks;
            Ok(match (&locks.player, &locks.party) {
                (None, _) => 0,
                (Some(_), None) => 1,
                (Some(_), Some(members)) => members.len() as i64 + 1,
            })
        })?,
    )?;

    // GetLFDChoiceLockedState([table]) → {[id] = reason}: the table reused (cleared) or a new
    // one; valid party members' active non-raid locks first, then the valid self block's active
    // locks, which overwrite.
    g.set(
        "GetLFDChoiceLockedState",
        lua.create_function(|lua, value: Value| {
            let out = match value {
                Value::Nil => lua.create_table()?,
                Value::Table(t) => {
                    t.clear()?;
                    t
                }
                _ => {
                    return Err(mlua::Error::runtime(
                        "Usage: GetLFDChoiceLockedState([table])",
                    ))
                }
            };
            let (party, player) = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                (
                    model.wrath_lfg.party_nodes(),
                    model.wrath_lfg.player_nodes(),
                )
            };
            for (_, nodes) in party.unwrap_or_default() {
                for n in nodes.iter().filter(|n| n.entry >> 24 != 2 && n.active) {
                    out.raw_set(i64::from(n.entry & 0xff_ffff), n.reason as i32)?;
                }
            }
            for n in player.unwrap_or_default().iter().filter(|n| n.active) {
                out.raw_set(i64::from(n.entry & 0xff_ffff), n.reason as i32)?;
            }
            Ok(out)
        })?,
    )?;

    // GetLFGDungeonRewards(dungeonID) → doneToday, moneyBase, moneyVar, experienceBase,
    // experienceVar, numRewards for the row's full entry; an unknown row answers nothing, no
    // preview false and five zeros.
    g.set(
        "GetLFGDungeonRewards",
        lua.create_function(|lua, id: Value| {
            let id = number_arg(lua, id, "Usage: GetLFGDungeonRewards(dungeonID)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_lfg;
            let Some(row) = state.row(id as u32) else {
                return Ok(MultiValue::new());
            };
            let entry = full_entry(row);
            let node = state
                .player_nodes()
                .and_then(|nodes| nodes.into_iter().find(|n| n.entry == entry));
            let p = node.map(|n| n.preview).unwrap_or_default();
            let signed = |v: u32| int(i64::from(v as i32));
            Ok(MultiValue::from_vec(vec![
                Value::Boolean(node_done(&p)),
                signed(p.money_base),
                signed(p.money_var),
                signed(p.xp_base),
                signed(p.xp_var),
                int(p.items.len() as i64),
            ]))
        })?,
    )?;

    // GetLFGDungeonRewardInfo(dungeonID, lootIndex) → name, texture, quantity, for random and
    // holiday rows only. The name is the cached template's, "" until it is cached; an uncached
    // call records one item request (the 0x552950 registration, `super::requests`).
    g.set(
        "GetLFGDungeonRewardInfo",
        lua.create_function(|lua, (id, index): (Value, Value)| {
            const USAGE: &str = "Usage: GetLFGDungeonRewardInfo(dungeonID, lootIndex)";
            let id = number_arg(lua, id, USAGE)?;
            let index = number_arg(lua, index, USAGE)?;
            let (out, uncached) = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let state = &model.wrath_lfg;
                let Some(row) = state.row(id as u32) else {
                    return Ok(MultiValue::new());
                };
                if row.dungeon_type != 6 && row.flags & 4 == 0 {
                    return Ok(MultiValue::new());
                }
                let Some(item) = preview_item(state, full_entry(row), index.wrapping_sub(1)) else {
                    return Ok(MultiValue::new());
                };
                let template = model.item_templates.get(&item.item);
                let name = template.map_or("", |t| t.name.as_str());
                let icon = state
                    .presentation
                    .display_icons
                    .get(&item.display)
                    .map_or(FALLBACK_ICON, String::as_str);
                let out = MultiValue::from_vec(vec![
                    Value::String(lua.create_string(name)?),
                    Value::String(lua.create_string(icon)?),
                    int(i64::from(item.count)),
                ]);
                (out, template.is_none().then_some(item.item))
            };
            if let Some(item) = uncached {
                super::requests::record_item_request(lua, item);
            }
            Ok(out)
        })?,
    )?;

    // GetLFGDungeonRewardLink(dungeonID, lootIndex) → link, always under the random type (6),
    // for a cached template with an App-built link; otherwise nothing.
    g.set(
        "GetLFGDungeonRewardLink",
        lua.create_function(|lua, (id, index): (Value, Value)| {
            const USAGE: &str = "Usage: GetLFGDungeonRewardLink(dungeonID, lootIndex)";
            let id = number_arg(lua, id, USAGE)?;
            let index = number_arg(lua, index, USAGE)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_lfg;
            let entry = id as u32 & 0xff_ffff | 0x0600_0000;
            let link = preview_item(state, entry, index.wrapping_sub(1))
                .filter(|item| model.item_templates.contains_key(&item.item))
                .and_then(|item| state.presentation.reward_links.get(&item.item));
            Ok(match link {
                Some(link) => MultiValue::from_vec(vec![Value::String(lua.create_string(link)?)]),
                None => MultiValue::new(),
            })
        })?,
    )?;

    g.set(
        "GetNumRandomDungeons",
        lua.create_function(|lua, _ignored: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.wrath_lfg.derived.random.len() as i64)
        })?,
    )?;

    // GetLFGRandomDungeonInfo(index) → id, then GetLFGDungeonInfo's values for that id.
    g.set(
        "GetLFGRandomDungeonInfo",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: GetLFGRandomDungeonInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = &model.wrath_lfg;
            let Some(entry) = usize::try_from(index.wrapping_sub(1))
                .ok()
                .and_then(|i| state.derived.random.get(i))
            else {
                return Ok(MultiValue::new());
            };
            let id = entry & 0xff_ffff;
            let mut out = vec![int(i64::from(id))];
            if let Some(row) = state.row(id) {
                out.extend(dungeon_info(lua, state, row)?);
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    g.set(
        "GetRandomDungeonBestChoice",
        lua.create_function(|lua, _ignored: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match model.wrath_lfg.derived.best {
                Some(entry) => MultiValue::from_vec(vec![int(i64::from(entry & 0xff_ffff))]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    Ok(())
}

/// The raw done byte as `lua_pushboolean` takes it.
fn node_done(p: &super::WrathLfgPreview) -> bool {
    p.done != 0
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::script::{
        ItemTemplateView, UiScript, WrathLfgCatalog, WrathLfgDungeon, WrathLfgLock,
        WrathLfgLockSnapshot, WrathLfgMemberLocks, WrathLfgPlayerLocks, WrathLfgPresentation,
        WrathLfgPreview, WrathLfgRewardItem,
    };

    fn row(
        id: u32,
        dungeon_type: i32,
        flags: u32,
        expansion: i32,
        levels: [i32; 3],
    ) -> WrathLfgDungeon {
        WrathLfgDungeon {
            id,
            name: format!("D{id}"),
            dungeon_type,
            flags,
            expansion,
            min_level: levels[0],
            target_min_level: levels[1],
            target_max_level: levels[2],
            ..Default::default()
        }
    }

    fn lock(entry: u32, reason: u32) -> WrathLfgLock {
        WrathLfgLock { entry, reason }
    }

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_lfg_catalog(
            Arc::new(WrathLfgCatalog {
                dungeons: vec![
                    row(7, 1, 2, 0, [15, 15, 21]),
                    row(9, 6, 0, 0, [15, 15, 25]),
                    row(10, 6, 0, 2, [70, 70, 80]),
                    row(11, 6, 0, 2, [75, 75, 80]),
                    row(12, 1, 4, 0, [80, 80, 80]),
                    row(13, 1, 4, 0, [80, 80, 80]),
                ],
                ..Default::default()
            }),
            Some(2),
        );
        s
    }

    fn publish(
        s: &mut UiScript,
        player: Option<WrathLfgPlayerLocks>,
        party: Option<Vec<WrathLfgMemberLocks>>,
    ) {
        s.set_wrath_lfg_lock_snapshot(Arc::new(WrathLfgLockSnapshot { player, party }));
    }

    #[test]
    fn lock_info_reads_self_then_members_with_truthful_invalid_and_unknown() {
        let mut s = vm();
        s.run("UNKNOWN = 'Unknown'").unwrap();
        assert_eq!(s.eval::<i64>("return GetLFDLockPlayerCount()").unwrap(), 0);
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFDLockInfo(7, 1))")
                .unwrap(),
            0
        );
        let mut names = WrathLfgPresentation {
            self_guid: 1,
            ..Default::default()
        };
        names.guid_names.insert(1, "Me".into());
        names.guid_names.insert(77, String::new());
        s.set_wrath_lfg_presentation(Arc::new(names));
        publish(
            &mut s,
            Some(WrathLfgPlayerLocks {
                locks: vec![lock(0x0100_0007, 5), lock(0x0600_0009, 0)],
                previews: vec![],
            }),
            None,
        );
        assert_eq!(s.eval::<i64>("return GetLFDLockPlayerCount()").unwrap(), 1);
        assert_eq!(
            s.eval::<(String, i64)>("return GetLFDLockInfo(7, 1)")
                .unwrap(),
            ("Me".into(), 5)
        );
        assert_eq!(
            s.eval::<(String, i64)>("return GetLFDLockInfo(99, 1)")
                .unwrap(),
            ("Me".into(), 0)
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFDLockInfo(7, 2))")
                .unwrap(),
            0,
            "no valid party block"
        );
        publish(
            &mut s,
            Some(WrathLfgPlayerLocks {
                locks: vec![lock(0x0100_0007, 5), lock(0x0600_0009, 0)],
                previews: vec![],
            }),
            Some(vec![
                WrathLfgMemberLocks {
                    guid: 77,
                    locks: vec![lock(0x0200_0008, 3), lock(0x0100_0007, 4)],
                },
                WrathLfgMemberLocks {
                    guid: 88,
                    locks: vec![],
                },
            ]),
        );
        assert_eq!(s.eval::<i64>("return GetLFDLockPlayerCount()").unwrap(), 3);
        assert_eq!(
            s.eval::<(String, i64)>("return GetLFDLockInfo(7, 2)")
                .unwrap(),
            (String::new(), 4)
        );
        assert_eq!(
            s.eval::<(String, i64)>("return GetLFDLockInfo(7, 3)")
                .unwrap(),
            ("Unknown".into(), 0)
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFDLockInfo(7, 4))")
                .unwrap(),
            0
        );
        assert!(s.run("GetLFDLockInfo(7)").is_err());
        // Party first without raid entries, then self overwrites; the table is reused.
        s.run(
            "T = {stale = 1} local r = GetLFDChoiceLockedState(T) assert(r == T) \
             assert(T.stale == nil and T[7] == 5 and T[9] == 0 and T[8] == nil)",
        )
        .unwrap();
        assert!(s.run("GetLFDChoiceLockedState(1)").is_err());
    }

    #[test]
    fn rewards_merge_duplicate_previews_and_answer_unavailable_presentation_truthfully() {
        let mut s = vm();
        let preview = |items: Vec<WrathLfgRewardItem>, done: u8| WrathLfgPreview {
            entry: 0x0600_0009,
            done,
            money_base: 100,
            xp_base: 200,
            money_var: 10,
            xp_var: 20,
            items,
        };
        publish(
            &mut s,
            Some(WrathLfgPlayerLocks {
                locks: vec![],
                previews: vec![
                    preview(
                        vec![WrathLfgRewardItem {
                            item: 11,
                            display: 5,
                            count: 2,
                        }],
                        0,
                    ),
                    preview(
                        vec![WrathLfgRewardItem {
                            item: 12,
                            display: 6,
                            count: 1,
                        }],
                        1,
                    ),
                ],
            }),
            None,
        );
        let mut p = WrathLfgPresentation::default();
        p.display_icons.insert(5, "Interface\\Icons\\X".into());
        p.reward_links.insert(11, "|link11|".into());
        p.reward_links.insert(12, "|link12|".into());
        s.set_wrath_lfg_presentation(Arc::new(p));
        s.set_item_template(
            11,
            ItemTemplateView {
                name: "Sword".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            s.eval::<(bool, i64, i64, i64, i64, i64)>("return GetLFGDungeonRewards(9)")
                .unwrap(),
            (true, 100, 10, 200, 20, 2),
            "the later duplicate overwrites scalars and appends items"
        );
        assert_eq!(
            s.eval::<(bool, i64, i64, i64, i64, i64)>("return GetLFGDungeonRewards(7)")
                .unwrap(),
            (false, 0, 0, 0, 0, 0)
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFGDungeonRewards(999))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<(String, String, i64)>("return GetLFGDungeonRewardInfo(9, 1)")
                .unwrap(),
            ("Sword".into(), "Interface\\Icons\\X".into(), 2)
        );
        assert_eq!(
            s.eval::<(String, String, i64)>("return GetLFGDungeonRewardInfo(9, 2)")
                .unwrap(),
            (String::new(), super::FALLBACK_ICON.into(), 1),
            "uncached name is empty; a missing icon falls back"
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFGDungeonRewardInfo(9, 3))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFGDungeonRewardInfo(7, 1))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<String>("return GetLFGDungeonRewardLink(9, 1)")
                .unwrap(),
            "|link11|"
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFGDungeonRewardLink(9, 2))")
                .unwrap(),
            0,
            "no link until the template is cached"
        );
        assert!(s.run("GetLFGDungeonRewardInfo(9)").is_err());
        // Item 12 was uncached at both of its RewardInfo calls; 11 was cached. Not deduplicated.
        s.run("GetLFGDungeonRewardInfo(9, 2)").unwrap();
        let asked: Vec<u32> = s
            .take_wrath_lfg_item_requests()
            .iter()
            .map(|r| r.item)
            .collect();
        assert_eq!(asked, vec![12, 12]);
        assert!(s.take_wrath_lfg_item_requests().is_empty());
    }

    #[test]
    fn random_list_and_best_choice_follow_the_native_derivation() {
        let mut s = vm();
        assert_eq!(
            s.eval::<i64>("return GetNumRandomDungeons()").unwrap(),
            0,
            "nothing before a receipt"
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetRandomDungeonBestChoice())")
                .unwrap(),
            0
        );
        publish(
            &mut s,
            Some(WrathLfgPlayerLocks {
                locks: vec![lock(0x0600_000b, 1001)],
                previews: vec![WrathLfgPreview {
                    entry: 0x0100_000c,
                    ..Default::default()
                }],
            }),
            Some(vec![]),
        );
        assert_eq!(s.eval::<i64>("return GetNumRandomDungeons()").unwrap(), 4);
        assert_eq!(
            s.eval::<(i64, String)>("return GetLFGRandomDungeonInfo(4)")
                .unwrap(),
            (12, "D12".into()),
            "a holiday with an inactive self node joins the list"
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetLFGRandomDungeonInfo(5))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<i64>("return GetRandomDungeonBestChoice()")
                .unwrap(),
            10,
            "the higher expansion wins; 11 is locked"
        );
        // Without the lock, 11 competes: same expansion, higher halved target levels.
        publish(&mut s, Some(WrathLfgPlayerLocks::default()), None);
        assert_eq!(
            s.eval::<i64>("return GetRandomDungeonBestChoice()")
                .unwrap(),
            11,
            "11's target levels are higher"
        );
        // Both blocks invalid: the previous derivation stands.
        publish(&mut s, None, None);
        assert_eq!(s.eval::<i64>("return GetNumRandomDungeons()").unwrap(), 3);
    }

    #[test]
    fn joinable_reads_self_then_party_locks_outside_raids() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_lfg_catalog(
            Arc::new(WrathLfgCatalog {
                dungeons: vec![
                    row(7, 1, 0, 0, [15, 15, 21]),
                    row(13, 1, 0, 0, [80, 80, 80]),
                    row(14, 2, 0, 2, [80, 80, 80]),
                ],
                ..Default::default()
            }),
            Some(2),
        );
        let joinable = |s: &UiScript, id: u32| {
            s.eval::<Option<bool>>(&format!("return IsLFGDungeonJoinable({id})"))
                .unwrap()
        };
        assert_eq!(
            s.eval::<i64>("return select('#', IsLFGDungeonJoinable(99))")
                .unwrap(),
            0
        );
        assert!(s.run("IsLFGDungeonJoinable()").is_err());
        assert_eq!(joinable(&s, 7), Some(true), "no lock block: joinable");
        publish(
            &mut s,
            Some(WrathLfgPlayerLocks {
                // A zero reason is still an active lock.
                locks: vec![lock(0x0100_0007, 0)],
                previews: vec![],
            }),
            Some(vec![WrathLfgMemberLocks {
                guid: 77,
                locks: vec![lock(0x0100_000d, 3), lock(0x0200_000e, 3)],
            }]),
        );
        assert_eq!(joinable(&s, 7), Some(false), "the player's own lock");
        assert_eq!(
            joinable(&s, 13),
            Some(false),
            "a party member's dungeon lock"
        );
        assert_eq!(
            joinable(&s, 14),
            Some(true),
            "party locks are not read for raids"
        );
        publish(&mut s, Some(WrathLfgPlayerLocks::default()), None);
        assert_eq!(joinable(&s, 7), Some(true));
        assert_eq!(joinable(&s, 13), Some(true), "no valid party block");
    }
}
