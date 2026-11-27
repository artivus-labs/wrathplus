//! The skills-pane bindings behind the stock `SkillFrame`. The app pushes a flat snapshot of known
//! skill lines, resolved from `SkillLine.dbc` and `SkillLineCategory.dbc`, and the engine groups,
//! sorts and folds it as the client's list build (`0x4d2cb0`, comparator `0x4d3070`) does: one
//! header per category, by `category_order` (category id breaks a tie), and within a category the
//! untrained lines (rank 0) after the trained ones, each by name. A push re-expands every group.
//! Every Lua index is 1-based into 12340's whole row array (`0xc22b0c`): the visible rows, headers
//! plus the entries of expanded groups, then the folded groups' entries; `GetNumSkillLines` counts
//! the visible ones. The selection is held by skill id, so it survives a re-push, and selecting a
//! header clears it.
//! Which lines reach the engine at all is the app's half (`feed_skills`).
//!
//! `numTempPoints` answers 0: the model carries no temp points, and on 1.12's data nothing a player
//! holds moves the reference's field (`+0x10`), which only `AddSkillUp` increments (`0x4d345b`).
//! `stepCost` and `rankCost` are always nil, by the data: `SkillLine.skillCostsID` is 0 in every
//! row and no line a player holds carries the step-cost flags.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::binding_abi;
use super::Model;

/// One known skill line off the player's `PLAYER_SKILL_INFO` block, resolved by the app.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillEntry {
    pub skill_id: u32,
    /// The `SkillLine.dbc` name.
    pub name: String,
    /// The raw rank off the descriptor; 0 is untrained, which sorts below the trained lines.
    pub value: u32,
    /// The raw max rank off the descriptor; [`Self::mono`] overrides it at the API return.
    pub max: u32,
    /// `SkillRaceClassInfo.flags & 0x400` (`SKILL_FLAG_MONO_VALUE`): a single-rank line, whose
    /// `skillMaxRank` the client reports as 1 whatever the descriptor says (`0x4d3610`), so
    /// vmangos's 300/300 Beast Mastery reads as a proficiency.
    pub mono: bool,
    /// The temporary bonus, possibly negative: `skillModifier`, a signed read of the descriptor's
    /// temp half alone (`+0x850`).
    pub temp_bonus: i32,
    /// The permanent bonus (`+0x852`), possibly negative, folded into both ranks when they are
    /// nonzero (`0x4d380c`/`0x4d385d`).
    pub perm_bonus: i32,
    /// `SkillRaceClassInfo.reqLevel`, `GetSkillLineInfo`'s 11th return (`0x4d39ef`).
    pub min_level: u32,
    /// `SkillRaceClassInfo.skillCostID`; the 12th return is this plus one (`0x4d3a06`), added by
    /// the engine.
    pub cost_index: u32,
    /// `SkillLine.dbc` categoryId.
    pub category_id: u32,
    /// The category name, the header text.
    pub category_name: String,
    /// `SkillLineCategory` displayOrder, the group sort key.
    pub category_order: u32,
    /// `SkillLine.dbc` description (enUS column 12), `GetSkillLineInfo`'s 13th return.
    pub description: String,
    /// `isAbandonable`, the 8th return: a nonzero skill step and `SkillRaceClassInfo.flags & 0x20`
    /// (`SKILL_FLAG_UNLEARNABLE`), both checked by the client (`0x4d3953`–`0x4d3975`).
    pub abandonable: bool,
}

/// A flat, unordered push of every known skill line; the engine groups and sorts it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SkillsState {
    pub entries: Vec<SkillEntry>,
}

/// One category of the display tree, with its lines' positions in [`SkillsState::entries`].
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SkillGroup {
    category_id: u32,
    name: String,
    order: u32,
    /// Positions into [`SkillsState::entries`], in display order.
    entries: Vec<usize>,
}

impl super::UiScript {
    /// Replace the skills snapshot: rebuild the tree, re-expand every group (the client's rebuild
    /// stores `expandedMask = 0xFFFFFFFF` at `0x4d2ce2`) and keep the selection if its skill is
    /// still present.
    pub fn set_skills(&mut self, state: SkillsState) {
        let mut model = self.model_mut();
        let groups = build_groups(&state.entries);
        model.skills_collapsed.clear();
        if let Some(sid) = model.skills_selected {
            if !state.entries.iter().any(|e| e.skill_id == sid) {
                model.skills_selected = None;
            }
        }
        model.skills_groups = groups;
        model.skills = state;
    }

    /// Drain the skill ids `AbandonSkill` queued, one `CMSG_UNLEARN_SKILL` each; the removal comes
    /// back as a skill-field update.
    pub fn take_skill_abandons(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().skill_abandons)
    }
}

/// One visible row: a header (its group index) or an entry (its position in
/// [`SkillsState::entries`]).
#[derive(Clone, Copy)]
enum Row {
    Header(usize),
    Entry(usize),
}

/// The client's `stricmp`, approximated: case-insensitive, raw bytes as the tie-break.
fn collate(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Build the display tree by the module doc's grouping rule.
fn build_groups(entries: &[SkillEntry]) -> Vec<SkillGroup> {
    let mut map: HashMap<u32, SkillGroup> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        map.entry(e.category_id)
            .or_insert_with(|| SkillGroup {
                category_id: e.category_id,
                name: e.category_name.clone(),
                order: e.category_order,
                entries: Vec::new(),
            })
            .entries
            .push(i);
    }
    let mut groups: Vec<SkillGroup> = map.into_values().collect();
    for g in &mut groups {
        // Untrained below trained, then by name (`0x4d3070` compares the untrained byte, set at
        // `0x4d2e19`, before the `stricmp` at `0x4d318d`).
        g.entries.sort_by(|&a, &b| {
            (entries[a].value == 0)
                .cmp(&(entries[b].value == 0))
                .then_with(|| collate(&entries[a].name, &entries[b].name))
        });
    }
    groups.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then(a.category_id.cmp(&b.category_id))
    });
    groups
}

/// The visible rows; the Lua's 1-based index is a position in this list.
fn rows(model: &Model) -> Vec<Row> {
    let mut out = Vec::new();
    for (gi, g) in model.skills_groups.iter().enumerate() {
        out.push(Row::Header(gi));
        if !model.skills_collapsed.contains(&g.category_id) {
            for &ei in &g.entries {
                out.push(Row::Entry(ei));
            }
        }
    }
    out
}

fn num_rows(model: &Model) -> usize {
    rows(model).len()
}

/// Every row, 12340's whole array: the visible rows, then the folded groups' entries, which the
/// comparator `0x5cd4a0` sorts last. Labelled: 12340's order among those is qsort's, as they
/// compare equal (`0x5cd5b7`); here they keep the display order.
fn all_rows(model: &Model) -> Vec<Row> {
    let mut out = rows(model);
    for g in &model.skills_groups {
        if model.skills_collapsed.contains(&g.category_id) {
            out.extend(g.entries.iter().map(|&ei| Row::Entry(ei)));
        }
    }
    out
}

/// The row at a 1-based index, as the verbs' `index - 1 < total` unsigned test bounds it.
fn row_at(model: &Model, index: i32) -> Option<Row> {
    all_rows(model)
        .get(usize::try_from(index.wrapping_sub(1)).ok()?)
        .copied()
}

/// The entry at a 1-based index; `None` for a header.
fn entry_at(model: &Model, index: i32) -> Option<&SkillEntry> {
    match row_at(model, index)? {
        Row::Entry(ei) => model.skills.entries.get(ei),
        Row::Header(_) => None,
    }
}

/// `CollapseSkillHeader`/`ExpandSkillHeader` (`0x5ce3a0`/`0x5ce400`): an index at or below 0
/// folds or unfolds every group (`0x5ce3de`), any other row its own category, a skill row's too
/// (`0x5cd760` reads `[entry+4]`). Each re-sort signals `SKILL_LINES_CHANGED` (`0x5cd6fc`); a row
/// past the array does nothing.
fn set_collapsed(model: &mut Model, index: i32, collapse: bool) {
    let targets: Vec<u32> = if index <= 0 {
        model.skills_groups.iter().map(|g| g.category_id).collect()
    } else {
        match row_at(model, index) {
            Some(Row::Header(gi)) => vec![model.skills_groups[gi].category_id],
            Some(Row::Entry(ei)) => vec![model.skills.entries[ei].category_id],
            None => return,
        }
    };
    for c in targets {
        if collapse {
            model.skills_collapsed.insert(c);
        } else {
            model.skills_collapsed.remove(&c);
        }
    }
    let changed = ("SKILL_LINES_CHANGED".to_string(), Vec::new());
    model.pending_events.push(changed);
}

/// `GetSkillLineInfo`'s out-of-range answer, thirteen values (`0x4d3a2c`…`0x4d3a98`): the
/// reference's one unsigned bounds test (`0x4d3675`) sends index 0, a negative or past-the-end
/// index, no player and a missing DBC row here. Slots 4 and 5 must be numbers: stock
/// `SkillFrame.lua:194` adds them unguarded, and `SkillFrame_OnLoad` calls `SetSelectedSkill(0)`.
fn out_of_range_tuple(_lua: &Lua) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![
        Value::Nil,        // 1  skillName
        Value::Nil,        // 2  header
        Value::Nil,        // 3  isExpanded
        Value::Integer(0), // 4  skillRank
        Value::Integer(0), // 5  numTempPoints
        Value::Integer(0), // 6  skillModifier
        Value::Integer(0), // 7  skillMaxRank
        Value::Nil,        // 8  isAbandonable
        Value::Nil,        // 9  stepCost
        Value::Nil,        // 10 rankCost
        Value::Integer(0), // 11 minLevel
        Value::Integer(0), // 12 skillCostType
        Value::Nil,        // 13 skillDescription
    ]))
}

fn set_selected(model: &mut Model, index: i32) {
    model.skills_selected = entry_at(model, index).map(|e| e.skill_id);
}

/// `GetSelectedSkill()`: the selection's current visible index, 0 when none or folded away.
fn selected_index(model: &Model) -> u32 {
    let Some(sid) = model.skills_selected else {
        return 0;
    };
    rows(model)
        .iter()
        .position(|r| matches!(r, Row::Entry(ei) if model.skills.entries[*ei].skill_id == sid))
        .map_or(0, |p| (p + 1) as u32)
}

/// `GetSkillLineInfo`'s `(skillRank, skillMaxRank)`, in the client's order (`0x4d380c`–`0x4d38cb`):
///
/// ```text
/// skillRank    = rank > 0 ? rank + permBonus : rank
/// skillMaxRank = max  > 0 ? max  + permBonus : max
/// if mono { skillMaxRank = 1; if skillRank > 1 { skillRank = 1 } }
/// ```
///
/// Saturating at 0 keeps a negative permanent bonus from wrapping.
fn displayed_ranks(e: &SkillEntry) -> (i64, i64) {
    let fold = |v: u32| {
        if v > 0 {
            (i64::from(v) + i64::from(e.perm_bonus)).max(0)
        } else {
            0
        }
    };
    let (mut rank, mut max) = (fold(e.value), fold(e.max));
    if e.mono {
        max = 1;
        rank = rank.min(1);
    }
    (rank, max)
}

/// Register the skills-pane globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetNumSkillLines() → the visible row count (0 before any push).
    g.set(
        "GetNumSkillLines",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(num_rows(&model) as i64)
        })?,
    )?;

    // GetSkillLineInfo(index) (`0x5cde20`, the index truncated) → 13 values on a skill row, 12 on
    // a header (`0x5cdf83`); past the array, the 13-value tuple, whose nil first slot drives stock
    // `SkillFrame.lua:211`'s early return.
    g.set(
        "GetSkillLineInfo",
        lua.create_function(|lua, index: Value| {
            let usage = "Usage: GetSkillLineInfo(index)";
            let index = binding_abi::number_arg(lua, index, usage)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(row) = row_at(&model, index) else {
                return out_of_range_tuple(lua);
            };
            match row {
                Row::Header(gi) => {
                    let grp = &model.skills_groups[gi];
                    let expanded = !model.skills_collapsed.contains(&grp.category_id);
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(&grp.name)?),
                        Value::Integer(1), // isHeader
                        binding_abi::flag(expanded),
                        Value::Integer(0), // skillRank
                        Value::Integer(0), // numTempPoints
                        Value::Integer(0), // skillModifier
                        Value::Integer(0), // skillMaxRank
                        Value::Nil,        // isAbandonable
                        Value::Nil,        // stepCost
                        Value::Nil,        // rankCost
                        Value::Integer(0), // minLevel
                        Value::Integer(0), // skillCostType, the last header return
                    ]))
                }
                Row::Entry(ei) => {
                    let e = &model.skills.entries[ei];
                    let (rank, max) = displayed_ranks(e);
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(&e.name)?),
                        Value::Nil, // isHeader
                        Value::Nil, // isExpanded
                        Value::Integer(rank),
                        Value::Integer(0),                       // numTempPoints
                        Value::Integer(i64::from(e.temp_bonus)), // skillModifier: temp only
                        Value::Integer(max),
                        binding_abi::flag(e.abandonable), // isAbandonable
                        // stepCost, rankCost: nil, since Lua reads `0` as true.
                        Value::Nil,
                        Value::Nil,
                        Value::Integer(i64::from(e.min_level)), // minLevel
                        // skillCostType: the cost index plus one (`0x4d3a06`).
                        Value::Integer(i64::from(e.cost_index) + 1),
                        Value::String(lua.create_string(&e.description)?), // skillDescription
                    ]))
                }
            }
        })?,
    )?;

    // Collapse/ExpandSkillHeader(index): `isnumber` or the usage, then `__ftol2` (`0x5ce3d4`).
    for (name, usage, collapse) in [
        (
            "CollapseSkillHeader",
            "Usage: CollapseSkillHeader(index)",
            true,
        ),
        (
            "ExpandSkillHeader",
            "Usage: ExpandSkillHeader(index)",
            false,
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let index = binding_abi::ftol_arg(lua, index, usage)?;
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                set_collapsed(&mut model, index, collapse);
                Ok(())
            })?,
        )?;
    }

    // SetSelectedSkill(index) (`0x5cd860`, truncated) / GetSelectedSkill(): held by skill id;
    // `GetSelectedSkill` (`0x5cd8f0`) answers its visible index, 0 when none or folded.
    g.set(
        "SetSelectedSkill",
        lua.create_function(|lua, index: Value| {
            let usage = "Usage: SetSelectedSkill(index)";
            let index = binding_abi::number_arg(lua, index, usage)?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            set_selected(&mut model, index);
            Ok(())
        })?,
    )?;
    g.set(
        "GetSelectedSkill",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(selected_index(&model)))
        })?,
    )?;

    // GetAdjustedSkillPoints() (`0x5ce540`): `PLAYER_CHARACTER_POINTS2`, the free professions
    // (`[desc+0xda4]`), less the points `AddSkillUp` holds (`0xc22b00`), none here.
    g.set(
        "GetAdjustedSkillPoints",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.talents.points.1))
        })?,
    )?;

    // AbandonSkill(index) (`0x5ce2c0`, truncated): queue the entry's skill id for
    // `CMSG_UNLEARN_SKILL`, changing nothing locally until the skill field moves. Labelled: a
    // header row sends skill 0 in 12340 (`0x5ce356`), which the server ignores; here nothing.
    g.set(
        "AbandonSkill",
        lua.create_function(|lua, index: Value| {
            let index = binding_abi::number_arg(lua, index, "Usage: AbandonSkill(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if let Some(id) = entry_at(&model, index).map(|e| e.skill_id) {
                model.skill_abandons.push(id);
            }
            Ok(())
        })?,
    )?;

    // CancelSkillUps() (`0x4d3e30`): zeroes every `numTempPoints` (`0x4d35c9`); the stock Close
    // button calls it (`SkillFrame.xml:352`). No temp points are modelled, so nothing resets.
    g.set("CancelSkillUps", lua.create_function(|_, ()| Ok(()))?)?;

    Ok(())
}

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
