//! The reputation pane's twelve `ReputationFrame` globals: the app pushes a flat snapshot, and
//! the engine groups, sorts and folds it as the reference does (`[0x4d5200, 0x4d6d80)`); every
//! index the API takes or returns is 1-based into the visible rows. A row is a header by flag
//! `0x08` (`0x4d5acb`), which emulators name `INVISIBLE_FORCED`, and files under its
//! `Faction.dbc` `team`, or under `-1` while inactive (flag `0x20`); key `0` reads `FACTION_OTHER`
//! and `-1` `FACTION_INACTIVE`. A childless header is not a row (`GetNumFactions`, `0xb73764`).
//! Headers sort by name with the synthetic keys last, `0` first (`0x4d5dc0`); children sort by
//! name (`0x4d5e70`). The at-war toggle also refuses in combat and applies the -3000 floor only
//! toward peace (`0x4d5fd0`); only `canToggleAtWar`, what the pane reports, lives here.

use std::collections::{HashMap, HashSet};

use mlua::{Lua, MultiValue, Value};

use super::binding_abi;
use super::Model;

/// The synthetic header key for factions the player moved to the inactive bucket.
const KEY_INACTIVE: i64 = -1;
/// The synthetic header key for factions with no parent, the "Other" bucket.
const KEY_OTHER: i64 = 0;

/// One reputation faction, resolved app-side from `Faction.dbc` and the player's wire slot.
#[derive(Clone, Debug, PartialEq)]
pub struct FactionEntry {
    /// `Faction.dbc` id: the row's identity, and what a child's [`Self::parent_id`] names.
    pub faction_id: u32,
    /// `reputationIndex`: the slot every send addresses and the selection is held by.
    pub rep_list_id: u32,
    /// `Faction.dbc`'s `team`: the header key this row files under, `0` the "Other" bucket, and
    /// "Inactive" instead while [`Self::inactive`] is set.
    pub parent_id: u32,
    /// The localized name: the bar's label, or a header's text.
    pub name: String,
    /// The localized description, for the detail popup; often empty.
    pub description: String,
    /// The DBC race/class base plus the wire standing (`0x4d6370`): `barValue`, absolute.
    pub standing: i32,
    /// The rank, 1..=8 (`FACTION_STANDING_LABEL<n>`); `GetWatchedFactionInfo`'s `reaction` is this
    /// same scale (`0x4d68a0`), not the unit-reaction one.
    pub standing_id: u8,
    /// The rank's absolute bounds (`.rdata 0x80928c`), `barMin`/`barMax`; the stock pane
    /// normalizes them itself (`ReputationFrame.lua:80-82`).
    pub bar_min: i32,
    pub bar_max: i32,
    /// Flag `0x01`, the only flag gating list membership: off until the player meets the faction.
    pub visible: bool,
    /// Flag `0x08`: this row is a header.
    pub is_header: bool,
    /// Flag `0x02`; the toggle flips it here, unacked.
    pub at_war: bool,
    /// Flag `0x10` clear and `standing >= -3000`: what `GetFactionInfo` reports.
    pub can_toggle_at_war: bool,
    /// Flag `0x20`; flipped here like [`Self::at_war`], it moves the row under "Inactive" rather
    /// than hiding it.
    pub inactive: bool,
    /// The fitting race/class slot's `ReputationFlags & 0x80` (`0x5d0ce0`): a header row that
    /// carries its own bar (`GetFactionInfo`'s `hasRep`), as 3.3.5's sub-headers do.
    pub has_rep: bool,
}

/// A flat, unordered push of every faction the player has a slot for; the engine groups and sorts.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ReputationState {
    pub entries: Vec<FactionEntry>,
    /// `PLAYER_FIELD_WATCHED_FACTION_INDEX` as a slot, `None` for none: a server field with no
    /// client mirror, never written here.
    pub watched: Option<u32>,
}

/// One outbound reputation verb the pane queued, drained by the app into its `WorldWriter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReputationSend {
    /// `CMSG_SET_FACTION_ATWAR`.
    AtWar { rep_list_id: u32, at_war: bool },
    /// `CMSG_SET_FACTION_INACTIVE`.
    Inactive { rep_list_id: u32, inactive: bool },
    /// `CMSG_SET_WATCHED_FACTION`; `None` is the wire's `-1`, not slot 0.
    Watch(Option<u32>),
}

/// The fold state; `pub(crate)` only because [`super::model::Model`] stores it. Per VM, so a
/// `/reload` resets it, where 12340's mask is a global only the full rebuild (`0x5d2bc0`) resets.
#[derive(Default)]
pub(crate) struct Folds {
    /// Folded headers by key (a `Faction.dbc` id, 0 "Other", -1 "Inactive").
    pub(crate) collapsed: HashSet<i64>,
    /// Every header key listed this session: 12340 never drops a header slot (`0xc23370`).
    pub(crate) seen: HashSet<i64>,
}

/// One header group; `pub(crate)` only because [`super::model::Model`] stores them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FactionGroup {
    /// A `Faction.dbc` id, or [`KEY_OTHER`] or [`KEY_INACTIVE`].
    key: i64,
    /// The header row's text; empty for the two synthetic keys, resolved against the VM's global
    /// strings when read.
    name: String,
    /// The group's members, sorted by [`collate`]d name: faction bars, and 3.3.5's sub-headers
    /// (a header faction whose own parent is a header) with their children.
    members: Vec<Member>,
}

/// A top-level group's member.
#[derive(Clone, Debug, PartialEq)]
enum Member {
    /// A faction bar, by position in [`ReputationState::entries`].
    Entry(usize),
    /// A sub-header (its entry) and its children, sorted by name.
    Sub(usize, Vec<usize>),
}

/// One visible row: a header (its group index), a sub-header or a faction bar (positions in
/// [`ReputationState::entries`]); a child of a sub-header is an `Entry` with `child` set.
#[derive(Clone, Copy)]
enum Row {
    Header(usize),
    SubHeader(usize),
    Entry { ei: usize, child: bool },
}

impl super::UiScript {
    /// Push the reputation snapshot and rebuild the tree. The folds survive it, as 12340's rebuild
    /// saves and restores its mask (`0x5d1a20`-`0x5d1aaf`); a header new to the session opens every
    /// one (`0x5cfe70` sets the mask at `0x5cfecd`), and only the first list folds "Inactive"
    /// (`0x5d2e0f`). The selection is an id, kept as 12340 keeps it.
    pub fn set_reputation(&mut self, state: ReputationState) {
        let mut model = self.model_mut();
        let groups = build_groups(&state.entries);
        let keys: HashSet<i64> = groups
            .iter()
            .flat_map(|g| {
                std::iter::once(g.key).chain(g.members.iter().filter_map(|m| match m {
                    Member::Sub(hi, _) => Some(i64::from(state.entries[*hi].faction_id)),
                    Member::Entry(_) => None,
                }))
            })
            .collect();
        if model.reputation_folds.seen.is_empty() {
            model.reputation_folds.collapsed = HashSet::from([KEY_INACTIVE]);
        } else if !keys.is_subset(&model.reputation_folds.seen) {
            model.reputation_folds.collapsed.clear();
        }
        if !keys.is_empty() {
            model.reputation_folds.seen.extend(keys);
            model.reputation_folds.seen.insert(KEY_INACTIVE);
        }
        model.reputation_groups = groups;
        model.reputation = state;
    }

    /// Drain the reputation verbs the pane queued since the last call.
    pub fn take_reputation_sends(&mut self) -> Vec<ReputationSend> {
        std::mem::take(&mut self.model_mut().reputation_sends)
    }
}

fn header_key(e: &FactionEntry) -> i64 {
    if e.inactive {
        KEY_INACTIVE
    } else {
        i64::from(e.parent_id)
    }
}

/// The VM's text for a synthetic header key; the enUS literal in a bare test VM.
fn synthetic_header(lua: &Lua, key: i64) -> String {
    let (global, fallback) = if key == KEY_INACTIVE {
        ("FACTION_INACTIVE", "Inactive")
    } else {
        ("FACTION_OTHER", "Other")
    };
    lua.globals()
        .get::<String>(global)
        .unwrap_or_else(|_| fallback.into())
}

/// Case-insensitive name order, raw bytes breaking a tie: the client's `stricmp`.
fn collate(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

/// Build the header groups: the visible non-header rows by [`header_key`], sorted by name. A group
/// is named by its pushed header row (the reference synthesizes a missing one, `0x4d5a70`), or left
/// empty for a synthetic key; a childless header is no group, as the client's count drops it.
fn build_groups(entries: &[FactionEntry]) -> Vec<FactionGroup> {
    let header_of: HashMap<i64, usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.is_header)
        .map(|(i, e)| (i64::from(e.faction_id), i))
        .collect();
    // A header filed under another header is a sub-header (3.3.5's middle level).
    let is_sub = |hi: usize| {
        let h = &entries[hi];
        !h.inactive && h.parent_id != 0 && header_of.contains_key(&i64::from(h.parent_id))
    };

    let mut by_key: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut subs: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if !e.visible || e.is_header {
            continue;
        }
        let key = header_key(e);
        match header_of
            .get(&key)
            .copied()
            .filter(|&hi| key > KEY_OTHER && is_sub(hi))
        {
            Some(hi) => subs.entry(hi).or_default().push(i),
            None => by_key.entry(key).or_default().push(i),
        }
    }
    // A sub-header with visible children joins its parent's group, even one with no bars of its
    // own.
    let mut sub_members: HashMap<i64, Vec<Member>> = HashMap::new();
    for (hi, mut children) in subs {
        children.sort_by(|&a, &b| collate(&entries[a].name, &entries[b].name));
        sub_members
            .entry(i64::from(entries[hi].parent_id))
            .or_default()
            .push(Member::Sub(hi, children));
    }
    let keys: std::collections::HashSet<i64> =
        by_key.keys().chain(sub_members.keys()).copied().collect();
    let name_key = |m: &Member| match m {
        Member::Entry(i) | Member::Sub(i, _) => entries[*i].name.clone(),
    };

    let mut groups: Vec<FactionGroup> = keys
        .into_iter()
        .map(|key| {
            let mut members: Vec<Member> = by_key
                .remove(&key)
                .unwrap_or_default()
                .into_iter()
                .map(Member::Entry)
                .collect();
            members.extend(sub_members.remove(&key).unwrap_or_default());
            members.sort_by(|a, b| collate(&name_key(a), &name_key(b)));
            FactionGroup {
                key,
                name: header_of
                    .get(&key)
                    .map_or(String::new(), |&hi| entries[hi].name.clone()),
                members,
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        let synthetic = |k: i64| k <= KEY_OTHER;
        match (synthetic(a.key), synthetic(b.key)) {
            // Larger raw key first among the synthetics: 0 "Other" before -1 "Inactive".
            (true, true) => b.key.cmp(&a.key),
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => collate(&a.name, &b.name),
        }
    });
    groups
}

fn rows(model: &Model) -> Vec<Row> {
    let mut out = Vec::new();
    for (gi, g) in model.reputation_groups.iter().enumerate() {
        out.push(Row::Header(gi));
        if model.reputation_folds.collapsed.contains(&g.key) {
            continue;
        }
        for m in &g.members {
            match m {
                Member::Entry(ei) => out.push(Row::Entry {
                    ei: *ei,
                    child: false,
                }),
                Member::Sub(hi, children) => {
                    out.push(Row::SubHeader(*hi));
                    let key = i64::from(model.reputation.entries[*hi].faction_id);
                    if !model.reputation_folds.collapsed.contains(&key) {
                        out.extend(children.iter().map(|&ei| Row::Entry { ei, child: true }));
                    }
                }
            }
        }
    }
    out
}

fn num_rows(model: &Model) -> usize {
    rows(model).len()
}

/// A 1-based visible row.
fn row_at(model: &Model, index: i32) -> Option<Row> {
    let n = usize::try_from(index).ok()?.checked_sub(1)?;
    rows(model).get(n).copied()
}

/// A row's id, 12340's `[row+4]`: the faction's id, or a synthetic header's key.
fn row_id(model: &Model, row: Row) -> i64 {
    match row {
        Row::Header(gi) => model.reputation_groups[gi].key,
        Row::SubHeader(ei) | Row::Entry { ei, .. } => {
            i64::from(model.reputation.entries[ei].faction_id)
        }
    }
}

/// The faction a visible row stands for: a bar, or a sub-header, which 3.3.5 selects, watches
/// and toggles like a bar (`0x5d07b0` and its siblings read the row's faction).
fn row_entry(row: Row) -> Option<usize> {
    match row {
        Row::Entry { ei, .. } | Row::SubHeader(ei) => Some(ei),
        Row::Header(_) => None,
    }
}

fn entry_at(model: &Model, index: i32) -> Option<&FactionEntry> {
    let ei = row_entry(row_at(model, index)?)?;
    model.reputation.entries.get(ei)
}

fn entry_at_mut(model: &mut Model, index: i32) -> Option<&mut FactionEntry> {
    let ei = row_entry(row_at(model, index)?)?;
    model.reputation.entries.get_mut(ei)
}

/// Fold or unfold by 1-based visible index; `0` or any non-header index acts on every header, the
/// reference's own fall-through (`0x4d6a50`, `0x4d6aa0`).
fn set_collapsed(model: &mut Model, index: i32, collapse: bool) {
    let one = row_at(model, index).and_then(|row| match row {
        Row::Header(gi) => model.reputation_groups.get(gi).map(|g| g.key),
        Row::SubHeader(hi) => Some(i64::from(model.reputation.entries[hi].faction_id)),
        Row::Entry { .. } => None,
    });
    let keys: Vec<i64> = match one {
        Some(key) => vec![key],
        None => model
            .reputation_groups
            .iter()
            .flat_map(|g| {
                std::iter::once(g.key).chain(g.members.iter().filter_map(|m| match m {
                    Member::Sub(hi, _) => Some(i64::from(model.reputation.entries[*hi].faction_id)),
                    Member::Entry(_) => None,
                }))
            })
            .collect(),
    };
    for key in keys {
        if collapse {
            model.reputation_folds.collapsed.insert(key);
        } else {
            model.reputation_folds.collapsed.remove(&key);
        }
    }
}

/// `GetFactionInfo`'s miss (`0x5d0da0`'s no-row exit): thirteen values, standingID 1, which the
/// stock pane indexes `FACTION_BAR_COLORS` with unguarded.
fn unknown_faction() -> MultiValue {
    let mut out = vec![Value::Nil, Value::Nil, Value::Integer(1)];
    out.extend(std::iter::repeat_n(Value::Integer(0), 3));
    out.extend(std::iter::repeat_n(Value::Nil, 7));
    debug_assert_eq!(out.len(), 13, "the tuple is thirteen wide on every path");
    MultiValue::from_vec(out)
}

/// A faction row's thirteen (`0x5d0da0`): the bar's eight, then isHeader, isCollapsed, hasRep
/// (headers only), isWatched and isChild.
fn faction_tuple(
    lua: &Lua,
    model: &Model,
    e: &FactionEntry,
    header: Option<bool>,
    child: bool,
) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![
        Value::String(lua.create_string(&e.name)?),
        Value::String(lua.create_string(&e.description)?),
        Value::Integer(i64::from(e.standing_id)),
        Value::Integer(i64::from(e.bar_min)),
        Value::Integer(i64::from(e.bar_max)),
        Value::Integer(i64::from(e.standing)),
        binding_abi::flag(e.at_war),
        binding_abi::flag(e.can_toggle_at_war),
        binding_abi::flag(header.is_some()),
        binding_abi::flag(header == Some(true)),
        binding_abi::flag(header.is_some() && e.has_rep),
        binding_abi::flag(model.reputation.watched == Some(e.rep_list_id)),
        binding_abi::flag(child),
    ]))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetNumFactions() → the visible row count (0 before any push).
    g.set(
        "GetNumFactions",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(num_rows(&model) as i64)
        })?,
    )?;

    // GetFactionInfo(index) → ReputationFrame.lua:158's thirteen: name, description, standingID,
    // barMin, barMax, barValue, atWarWith, canToggleAtWar, isHeader, isCollapsed, hasRep,
    // isWatched, isChild; thirteen wide on every path, a miss included (`0x5d1150` → `0x5d0da0`).
    // It, `GetFactionInfoByID`, `FactionToggleAtWar` and `Set*FactionActive` truncate by an inline
    // `fistp` qword (`0x5d119c`), so `number_arg`; their `ftol_arg` siblings call `__ftol2`.
    g.set(
        "GetFactionInfo",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::number_arg(lua, index, "Usage: GetFactionInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(row) = row_at(&model, index) else {
                return Ok(unknown_faction());
            };
            match row {
                Row::Header(gi) => {
                    let grp = &model.reputation_groups[gi];
                    let collapsed = model.reputation_folds.collapsed.contains(&grp.key);
                    let header = model
                        .reputation
                        .entries
                        .iter()
                        .find(|e| e.is_header && i64::from(e.faction_id) == grp.key);
                    if let Some(e) = header {
                        return faction_tuple(lua, &model, e, Some(collapsed), false);
                    }
                    // A synthetic header ("Other", "Inactive"): standing 1, bar 0/0/0
                    // (`0x5d1076`-`0x5d1146`).
                    let name = synthetic_header(lua, grp.key);
                    let mut out = vec![
                        Value::String(lua.create_string(&name)?),
                        Value::String(lua.create_string("")?),
                    ];
                    out.extend([1, 0, 0, 0].map(Value::Integer));
                    out.extend([
                        Value::Nil,
                        Value::Nil,
                        Value::Integer(1),
                        binding_abi::flag(collapsed),
                        Value::Nil,
                        Value::Nil,
                        Value::Nil,
                    ]);
                    Ok(MultiValue::from_vec(out))
                }
                Row::SubHeader(hi) => {
                    let e = &model.reputation.entries[hi];
                    let collapsed = model
                        .reputation_folds
                        .collapsed
                        .contains(&i64::from(e.faction_id));
                    faction_tuple(lua, &model, e, Some(collapsed), false)
                }
                Row::Entry { ei, child } => {
                    faction_tuple(lua, &model, &model.reputation.entries[ei], None, child)
                }
            }
        })?,
    )?;
    // GetFactionInfoByID(id) (`0x5d11e0` → `0x5d0da0`): the same thirteen for a Faction.dbc id,
    // listed or not; a faction the player has no row for answers the miss.
    g.set(
        "GetFactionInfoByID",
        lua.create_function(|lua, id: Value| {
            let id = binding_abi::number_arg(lua, id, "Usage: GetFactionInfoByID(id)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let all = rows(&model);
            let found = all.iter().find_map(|r| match *r {
                Row::Header(gi) => {
                    let key = model.reputation_groups[gi].key;
                    (key == i64::from(id)).then_some((
                        None,
                        Some(model.reputation_folds.collapsed.contains(&key)),
                        false,
                    ))
                }
                Row::SubHeader(hi) => (i64::from(model.reputation.entries[hi].faction_id)
                    == i64::from(id))
                .then(|| {
                    (
                        Some(hi),
                        Some(model.reputation_folds.collapsed.contains(&i64::from(id))),
                        false,
                    )
                }),
                Row::Entry { ei, child } => (i64::from(model.reputation.entries[ei].faction_id)
                    == i64::from(id))
                .then_some((Some(ei), None, child)),
            });
            let entry = |ei: Option<usize>| {
                ei.map(|i| &model.reputation.entries[i]).or_else(|| {
                    model
                        .reputation
                        .entries
                        .iter()
                        .find(|e| i64::from(e.faction_id) == i64::from(id))
                })
            };
            match found {
                Some((ei, header, child)) => match entry(ei) {
                    Some(e) => faction_tuple(lua, &model, e, header, child),
                    None => Ok(unknown_faction()),
                },
                None => match entry(None) {
                    Some(e) => faction_tuple(lua, &model, e, e.is_header.then_some(false), false),
                    None => Ok(unknown_faction()),
                },
            }
        })?,
    )?;

    // GetSelectedFaction() → the selected row's visible index, or 0 for none, as
    // `ReputationFrame.lua:146` tests; re-found by id each call (`0x5d0820` → `0x5d01b0`).
    g.set(
        "GetSelectedFaction",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(id) = model.reputation_selected else {
                return Ok(0i64);
            };
            let found = rows(&model).iter().position(|r| row_id(&model, *r) == id);
            Ok(found.map_or(0, |i| i as i64 + 1))
        })?,
    )?;

    // SetSelectedFaction(index): any visible row's id, a header's too (`0x5d07b0`); out of range
    // clears it.
    g.set(
        "SetSelectedFaction",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::ftol_arg(lua, index, "Usage: SetSelectedFaction(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.reputation_selected = row_at(&model, index).map(|r| row_id(&model, r));
            Ok(())
        })?,
    )?;

    // CollapseFactionHeader(index) / ExpandFactionHeader(index): a visible header index. Both fire
    // `UPDATE_FACTION`, as the reference's do (`0x4d6a50`, `0x4d6aa0` → `0x4d6400` → `0x4d5c40`,
    // fire site `0x4d5dab`): the stock header's `OnClick` only calls the binding
    // (`ReputationFrame.xml:9-15`), and the pane repaints on `UPDATE_FACTION` alone.
    g.set(
        "CollapseFactionHeader",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::ftol_arg(lua, index, "Usage: CollapseFactionHeader(index)")?;
            {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                set_collapsed(&mut model, index, true);
            }
            super::event::fire_global(lua, "UPDATE_FACTION", &[]);
            Ok(())
        })?,
    )?;
    g.set(
        "ExpandFactionHeader",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::ftol_arg(lua, index, "Usage: ExpandFactionHeader(index)")?;
            {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                set_collapsed(&mut model, index, false);
            }
            super::event::fire_global(lua, "UPDATE_FACTION", &[]);
            Ok(())
        })?,
    )?;

    // FactionToggleAtWar(index): flip the at-war bit and queue the send; `0x4d6950` writes locally
    // first, and nothing acks it.
    g.set(
        "FactionToggleAtWar",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::number_arg(lua, index, "Usage: FactionToggleAtWar(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(e) = entry_at_mut(&mut model, index) else {
                return Ok(());
            };
            if !e.can_toggle_at_war {
                return Ok(());
            }
            e.at_war = !e.at_war;
            let send = ReputationSend::AtWar {
                rep_list_id: e.rep_list_id,
                at_war: e.at_war,
            };
            model.reputation_sends.push(send);
            Ok(())
        })?,
    )?;

    // IsFactionInactive / SetFactionInactive / SetFactionActive (`0x4d69b0`, `0x4d6a00`): the flip
    // is local first too, and moves the row under "Inactive", so the tree regroups here
    // (`0x4d69b0` calls `0x4d5c40`); the folds survive, as only a server rebuild resets them.
    g.set(
        "IsFactionInactive",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::ftol_arg(lua, index, "Usage: IsFactionInactive(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(binding_abi::flag(
                entry_at(&model, index).is_some_and(|e| e.inactive),
            ))
        })?,
    )?;
    for (name, usage, inactive) in [
        (
            "SetFactionInactive",
            "Usage: SetFactionInactive(index)",
            true,
        ),
        ("SetFactionActive", "Usage: SetFactionActive(index)", false),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let index = binding_abi::number_arg(lua, index, usage)?;
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let Some(e) = entry_at_mut(&mut model, index) else {
                    return Ok(());
                };
                e.inactive = inactive;
                let send = ReputationSend::Inactive {
                    rep_list_id: e.rep_list_id,
                    inactive,
                };
                model.reputation_sends.push(send);
                let groups = build_groups(&model.reputation.entries);
                model.reputation_groups = groups;
                Ok(())
            })?,
        )?;
    }

    // GetWatchedFactionInfo() → name, reaction, min, max, value; `reaction` is the standingID
    // scale, and a nil name is `ReputationFrame.lua:167`'s nothing-watched gate.
    g.set(
        "GetWatchedFactionInfo",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let watched = model.reputation.watched.and_then(|slot| {
                model
                    .reputation
                    .entries
                    .iter()
                    .find(|e| e.rep_list_id == slot)
            });
            let Some(e) = watched else {
                // Five values here too, `nil, 0, 0, 0, 0`: `0x4d6820`'s four guards share one exit
                // and none raises. `0x4d5620` maps the out-of-range sentinel to 0, so it is refused
                // at the null-row guard, not by a range check.
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                ]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&e.name)?),
                Value::Integer(i64::from(e.standing_id)),
                Value::Integer(i64::from(e.bar_min)),
                Value::Integer(i64::from(e.bar_max)),
                Value::Integer(i64::from(e.standing)),
            ]))
        })?,
    )?;

    // SetWatchedFactionIndex(index): a visible index, 0 to stop watching; the row's own faction,
    // a header's too, sent only when it differs from the watched field (`0x5d1420` → `0x5d0ba0`).
    // Not local first: the bar moves when the descriptor update returns. The app sends none as
    // `-1`, since slot 0 is a real faction.
    g.set(
        "SetWatchedFactionIndex",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::ftol_arg(lua, index, "Usage: SetWatchedFactionIndex(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let slot = row_at(&model, index).and_then(|r| {
                let id = row_id(&model, r);
                let e = model.reputation.entries.iter();
                e.filter(|e| i64::from(e.faction_id) == id)
                    .map(|e| e.rep_list_id)
                    .next()
            });
            if slot != model.reputation.watched {
                model.reputation_sends.push(ReputationSend::Watch(slot));
            }
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
#[path = "reputation_tests.rs"]
mod tests;
