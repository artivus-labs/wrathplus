//! The tradeskill Lua globals over [`super::view`]'s visible rows and filters.

use mlua::{Lua, MultiValue, Value};

use crate::script::binding_abi::{self, number_arg};
use crate::script::item_stats::item_link;
use crate::script::Model;

use super::view::{
    build_groups, first_recipe_index, inv_slot_token, num_rows, present_inv_slots, recipe_at, rows,
    select, selected_visible_index, set_collapsed, Row,
};

/// An optional string as a Lua value (`nil` when absent).
fn opt_str(lua: &Lua, s: Option<&String>) -> mlua::Result<Value> {
    Ok(match s {
        Some(s) => Value::String(lua.create_string(s)?),
        None => Value::Nil,
    })
}

/// Register the tradeskill globals.
pub(in crate::script) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetTradeSkillLine(): ("UNKNOWN", 0, 0) with no window open, as the reference answers.
    g.set(
        "GetTradeSkillLine",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let (name, rank, max_rank) = match &model.trade_skill {
                Some(t) => (t.line_name.clone(), t.rank, t.max_rank),
                None => ("UNKNOWN".to_string(), 0, 0),
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&name)?),
                Value::Integer(i64::from(rank)),
                Value::Integer(i64::from(max_rank)),
            ]))
        })?,
    )?;

    // GetNumTradeSkills(): the visible rows, headers included.
    g.set(
        "GetNumTradeSkills",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(num_rows(&model) as i64)
        })?,
    )?;

    // GetTradeSkillItemLink(index) (`0x4ff410`): zero values on any miss (a header, no product, an
    // uncached template, which is never queried here), else the product's item link.
    g.set(
        "GetTradeSkillItemLink",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: GetTradeSkillItemLink(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let link = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|n| rows(&model).get(n).cloned())
                .and_then(|row| match row {
                    Row::Header { .. } => None,
                    Row::Entry(ei) => model.trade_skill.as_ref().map(|t| &t.recipes[ei]),
                })
                .filter(|r| r.product_item != 0)
                .and_then(|r| {
                    model
                        .item_templates
                        .get(&r.product_item)
                        .map(|t| item_link(r.product_item, &t.name, t.quality))
                });
            Ok(match link {
                Some(l) => MultiValue::from_vec(vec![Value::String(lua.create_string(&l)?)]),
                None => MultiValue::new(),
            })
        })?,
    )?;

    // GetTradeSkillReagentItemLink(index, reagentIndex) (`0x4ff800`): always one value, the link
    // or nil; `reagentIndex` counts the non-empty reagent slots, and the usage error's
    // `GetTradeReagentSkillItemLink` is the reference's own typo.
    g.set(
        "GetTradeSkillReagentItemLink",
        lua.create_function(|lua, (index, reagent): (Value, Value)| {
            let usage = "Usage: GetTradeReagentSkillItemLink(index, reagentIndex)";
            let index = number_arg(lua, index, usage)?;
            let reagent = number_arg(lua, reagent, usage)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let link = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|n| rows(&model).get(n).cloned())
                .and_then(|row| match row {
                    Row::Header { .. } => None,
                    Row::Entry(ei) => model.trade_skill.as_ref().map(|t| &t.recipes[ei]),
                })
                .zip(usize::try_from(reagent).ok().and_then(|r| r.checked_sub(1)))
                .and_then(|(r, ri)| r.reagents.get(ri))
                .and_then(|re| {
                    model
                        .item_templates
                        .get(&re.item)
                        .map(|t| item_link(re.item, &t.name, t.quality))
                });
            Ok(match link {
                Some(l) => Value::String(lua.create_string(&l)?),
                None => Value::Nil,
            })
        })?,
    )?;

    // GetTradeSkillInfo(index) (`0x5db2a0`), five values on every path: (name, "header", 0,
    // isExpanded, nil) for a header, (name, difficulty, numAvailable, nil, altVerb) for a recipe,
    // and (nil, nil, 0, nil, nil) for a miss. A non-number index raises.
    g.set(
        "GetTradeSkillInfo",
        lua.create_function(|lua, index: Value| {
            let index = lua
                .coerce_number(index)?
                .ok_or_else(|| mlua::Error::runtime("Usage: GetTradeSkillInfo(index)"))?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let miss = || {
                MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Integer(0),
                    Value::Nil,
                    Value::Nil,
                ])
            };
            let row = (index.round_ties_even() as i64)
                .checked_sub(1)
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|n| rows(&model).get(n).cloned());
            let Some(row) = row else {
                return Ok(miss());
            };
            match row {
                Row::Header { key, name } => {
                    let expanded = !model.trade_skill_collapsed.contains(&key);
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(&name)?),
                        Value::String(lua.create_string("header")?),
                        Value::Integer(0),
                        binding_abi::flag(expanded),
                        Value::Nil,
                    ]))
                }
                Row::Entry(ei) => {
                    let r = &model
                        .trade_skill
                        .as_ref()
                        .expect("rows() only yields Entry rows when trade_skill is Some")
                        .recipes[ei];
                    Ok(MultiValue::from_vec(vec![
                        Value::String(lua.create_string(&r.name)?),
                        Value::String(lua.create_string(r.difficulty.as_str())?),
                        Value::Integer(i64::from(r.num_available)),
                        Value::Nil,
                        match &r.alt_verb {
                            Some(verb) => Value::String(lua.create_string(verb)?),
                            None => Value::Nil,
                        },
                    ]))
                }
            }
        })?,
    )?;

    g.set(
        "GetFirstTradeSkill",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(first_recipe_index(&model)))
        })?,
    )?;

    // GetTradeSkillSubClasses(): the group names in order, unfiltered (`0x4ffb60`).
    g.set(
        "GetTradeSkillSubClasses",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::new();
            if let Some(t) = model.trade_skill.as_ref() {
                for grp in build_groups(&t.recipes) {
                    out.push(Value::String(lua.create_string(&grp.name)?));
                }
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // Expand/CollapseTradeSkillSubClass(i) (`0x5dd500`, `0x5dd460`): the group whose header is
    // visible row i, `<= 0` all; any other row raises the reference's own text, which names a skill
    // line for the one and a sub class for the other.
    for (verb, collapse, bad) in [
        (
            "ExpandTradeSkillSubClass",
            false,
            "Bad skill line in ExpandTradeSkillSubClass",
        ),
        (
            "CollapseTradeSkillSubClass",
            true,
            "Bad sub class in CollapseTradeSkillSubClass",
        ),
    ] {
        g.set(
            verb,
            lua.create_function(move |lua, id: i64| {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                if !set_collapsed(&mut model, id, collapse) {
                    return Err(mlua::Error::runtime(bad));
                }
                model.trade_skill_touched = true;
                Ok(())
            })?,
        )?;
    }

    g.set(
        "GetTradeSkillIcon",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            opt_str(lua, recipe_at(&model, index).and_then(|r| r.icon.as_ref()))
        })?,
    )?;

    g.set(
        "GetTradeSkillNumMade",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            // A miss answers 1, 1 (`0x5db7dd`).
            let (min_made, max_made) =
                recipe_at(&model, index).map_or((1, 1), |r| (r.min_made, r.max_made));
            Ok(MultiValue::from_vec(vec![
                Value::Integer(i64::from(min_made)),
                Value::Integer(i64::from(max_made)),
            ]))
        })?,
    )?;

    // GetTradeSkillCooldown(index): nil when ready, which the stock Lua tests for truthiness
    // (Blizzard_TradeSkillUI.lua:215).
    g.set(
        "GetTradeSkillCooldown",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(
                match recipe_at(&model, index).and_then(|r| r.cooldown_secs) {
                    Some(secs) => Value::Integer(secs as i64),
                    None => Value::Nil,
                },
            )
        })?,
    )?;

    g.set(
        "GetTradeSkillNumReagents",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(recipe_at(&model, index).map_or(0, |r| r.reagents.len()) as i64)
        })?,
    )?;

    // GetTradeSkillReagentInfo(index, reagentIndex): four nils on a miss (`0x5de4bb`); name and
    // icon are nil until the item template arrives.
    g.set(
        "GetTradeSkillReagentInfo",
        lua.create_function(|lua, (index, reagent_index): (usize, usize)| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(reagent) = recipe_at(&model, index)
                .and_then(|r| reagent_index.checked_sub(1).and_then(|n| r.reagents.get(n)))
            else {
                return Ok(MultiValue::from_vec(vec![Value::Nil; 4]));
            };
            Ok(MultiValue::from_vec(vec![
                opt_str(lua, reagent.name.as_ref())?,
                opt_str(lua, reagent.icon.as_ref())?,
                Value::Integer(i64::from(reagent.need)),
                Value::Integer(i64::from(reagent.have)),
            ]))
        })?,
    )?;

    // GetTradeSkillTools(index): alternating (name, has) pairs, which the stock Lua feeds to
    // `BuildColoredListString` (Blizzard_TradeSkillUI.lua:274).
    g.set(
        "GetTradeSkillTools",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::new();
            if let Some(r) = recipe_at(&model, index) {
                for (name, have) in &r.tools {
                    out.push(Value::String(lua.create_string(name)?));
                    out.push(binding_abi::flag(*have));
                }
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // GetTradeskillRepeatCount() (`0x5da270`): the Create All count, never below 1, with or
    // without a window. Labelled: 12340 answers it only while the repeating spell is selected; the
    // snapshot does not carry which spell repeats.
    g.set(
        "GetTradeskillRepeatCount",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(
                model
                    .trade_skill
                    .as_ref()
                    .map_or(0, |t| t.repeat_count)
                    .max(1),
            ))
        })?,
    )?;

    // SelectTradeSkill and GetTradeSkillSelectionIndex take and answer visible indices.
    g.set(
        "SelectTradeSkill",
        lua.create_function(|lua, index: u32| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            select(&mut model, index);
            Ok(())
        })?,
    )?;
    g.set(
        "GetTradeSkillSelectionIndex",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(selected_visible_index(&model)))
        })?,
    )?;

    // DoTradeSkill(index [, repeat]) (`0x5dac00`): queues the recipe's spell for `repeat` crafts
    // (default 1, truncated), which the app sends as one `CMSG_CAST_SPELL` each; a repeat at or
    // past numAvailable takes numAvailable, with no floor (`0x5dac90`-`0x5dac9c`).
    g.set(
        "DoTradeSkill",
        lua.create_function(|lua, (index, count): (Value, Value)| {
            let index = number_arg(lua, index, "Usage: DoTradeSkill(index [, repeat])")?;
            let count = lua.coerce_number(count)?.map_or(1, |n| n as i64 as u32);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let recipe = usize::try_from(index)
                .ok()
                .and_then(|i| recipe_at(&model, i));
            if let Some((spell_id, avail)) = recipe.map(|r| (r.spell_id, r.num_available)) {
                let n = if count >= avail { avail } else { count };
                model.trade_skill_dos.push((spell_id, n));
            }
            Ok(())
        })?,
    )?;

    // CloseTradeSkill(): client-side only, no packet; the app clears its state.
    g.set(
        "CloseTradeSkill",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.trade_skill_close = true;
            Ok(())
        })?,
    )?;

    // The subclass and inv-slot filters (Blizzard_TradeSkillUI.lua:314-414): index i is 1-based
    // into `GetTradeSkillSubClasses` (`0x4ffc70` bounds-checks it) or `GetTradeSkillInvSlots`, and
    // 0 is all. `Set*(i, 1, 1)` shows only i, as every menu click does; `Set*(i, 1)` shows i and
    // `Set*(i, 0)` hides it. Each set fires TRADE_SKILL_UPDATE through the touched flag
    // (`0x4fd710`), and `Get*(0)` asks whether everything is shown. The reference's per-header
    // subclass flag (`+0xc`, mask `0x84dd60`) is kept here as hidden group keys.
    // GetTradeSkillSubClassFilter(i) (`0x5da1b0`): `<= 0` asks whether every group shows; past the
    // groups raises.
    g.set(
        "GetTradeSkillSubClassFilter",
        lua.create_function(|lua, index: i64| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let groups = model
                .trade_skill
                .as_ref()
                .map_or_else(Vec::new, |t| build_groups(&t.recipes));
            let hidden = |k| model.trade_skill_subclass_hidden.contains(k);
            if index <= 0 {
                return Ok(binding_abi::flag(!groups.iter().any(|g| hidden(&g.key))));
            }
            let Some(g) = groups.get(index as usize - 1) else {
                return Err(mlua::Error::runtime(
                    "Bad sub class in GetTradeSkillSubClassFilter",
                ));
            };
            Ok(binding_abi::flag(!hidden(&g.key)))
        })?,
    )?;
    // SetTradeSkillSubClassFilter(index [, on, exclusive]) (`0x5dd0f0`): `<= 0` shows every group;
    // past the groups raises; `on` is required; `on` with a non-zero `exclusive` shows only i.
    g.set(
        "SetTradeSkillSubClassFilter",
        lua.create_function(|lua, (index, on, exclusive): (Value, Value, Value)| {
            const USAGE: &str = "Usage: SetTradeSkillSubClassFilter(index [, on\\off, exclusive])";
            let index = binding_abi::string_arg(lua, index, USAGE)?;
            let index = index.trim().parse::<f64>().map_or(0, |n| n as i64);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.trade_skill_touched = true;
            if index <= 0 {
                model.trade_skill_subclass_hidden.clear();
                return Ok(());
            }
            let keys: Vec<(u32, u32)> = model
                .trade_skill
                .as_ref()
                .map_or_else(Vec::new, |t| build_groups(&t.recipes))
                .into_iter()
                .map(|g| g.key)
                .collect();
            let Some(&key) = keys.get(index as usize - 1) else {
                return Err(mlua::Error::runtime(
                    "Bad sub class in SetTradeSkillSubClassFilter",
                ));
            };
            let Some(on) = lua.coerce_number(on)? else {
                return Err(mlua::Error::runtime(
                    "Missing on//off parameter in SetTradeSkillSubClassFilter",
                ));
            };
            let exclusive = lua.coerce_number(exclusive)?.is_some_and(|n| n as i64 != 0);
            if on as i64 == 0 {
                model.trade_skill_subclass_hidden.insert(key);
            } else if exclusive {
                model.trade_skill_subclass_hidden =
                    keys.into_iter().filter(|&k| k != key).collect();
            } else {
                model.trade_skill_subclass_hidden.remove(&key);
            }
            Ok(())
        })?,
    )?;
    // GetTradeSkillInvSlots(): the products' slot names in ascending bit order (`0xbde058`).
    g.set(
        "GetTradeSkillInvSlots",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::new();
            // A token (`0x84dd70`) missing from the string table answers "" rather than being
            // dropped: the filter verbs index this list by position.
            for bit in present_inv_slots(&model) {
                let word = inv_slot_token(bit)
                    .and_then(|k| crate::strings::global(lua, k))
                    .unwrap_or_default();
                out.push(Value::String(lua.create_string(word)?));
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;
    // The inv-slot pair works on the shown mask (`0x84dd64`); index i is the i-th present slot bit
    // (`0x4ffe60`).
    g.set(
        "GetTradeSkillInvSlotFilter",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            if model.trade_skill.is_none() {
                return Ok(binding_abi::flag(false));
            }
            let bits = present_inv_slots(&model);
            Ok(match index.checked_sub(1) {
                // Everything shown: (present & mask) == present (`0x4fffd0`).
                None => binding_abi::flag(
                    bits.iter()
                        .all(|&b| model.trade_skill_invslot_mask & (1 << b) != 0),
                ),
                Some(n) => binding_abi::flag(
                    bits.get(n)
                        .is_some_and(|&b| model.trade_skill_invslot_mask & (1 << b) != 0),
                ),
            })
        })?,
    )?;
    g.set(
        "SetTradeSkillInvSlotFilter",
        lua.create_function(
            |lua, (index, on, exclusive): (usize, Option<i64>, Option<i64>)| {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                if model.trade_skill.is_none() {
                    return Ok(());
                }
                let bits = present_inv_slots(&model);
                let on = on.unwrap_or(1) != 0;
                let exclusive = exclusive.unwrap_or(0) != 0;
                // The mask each path writes, as `0x4fd730` is called.
                match index.checked_sub(1) {
                    None => model.trade_skill_invslot_mask = u32::MAX,
                    Some(n) => {
                        let Some(&bit) = bits.get(n) else {
                            return Ok(());
                        };
                        if on && exclusive {
                            model.trade_skill_invslot_mask = 1 << bit;
                        } else if on {
                            model.trade_skill_invslot_mask |= 1 << bit;
                        } else {
                            model.trade_skill_invslot_mask &= !(1 << bit);
                        }
                    }
                }
                model.trade_skill_touched = true;
                Ok(())
            },
        )?,
    )?;

    Ok(())
}

/// Build 12340's additions (`/tmp/azb-app-mig/ghidra/ts*.log`): the description and recipe link,
/// the level, name and makeable filters, the repeat stop, and the linked-window queries.
///
/// The app builds `GetTradeSkillListLink`'s trade link (`0x5db060`) and the linked window
/// `IsTradeSkillLinked` reports; the recipe link's prefix is the open line's name.
pub(in crate::script) fn install_wrath(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let index_arg = |lua: &Lua, args: &MultiValue| -> Option<usize> {
        let n = lua.coerce_number(args.front()?.clone()).ok().flatten()?;
        usize::try_from(n.round() as i64).ok()
    };
    g.set(
        "GetTradeSkillDescription",
        lua.create_function(move |lua, args: MultiValue| {
            if args.front().is_none() {
                return Err(mlua::Error::runtime(
                    "Usage: GetTradeSkillDescription(index)",
                ));
            }
            let text = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                index_arg(lua, &args)
                    .and_then(|i| recipe_at(&model, i))
                    .and_then(|r| r.description.clone())
            };
            opt_str(lua, text.as_ref())
        })?,
    )?;
    g.set(
        "GetTradeSkillRecipeLink",
        lua.create_function(move |lua, args: MultiValue| {
            if args.front().is_none() {
                return Err(mlua::Error::runtime(
                    "Usage: GetTradeSkillRecipeLink(index)",
                ));
            }
            let link = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let line = model.trade_skill.as_ref().map(|t| t.line_name.clone());
                index_arg(lua, &args)
                    .and_then(|i| recipe_at(&model, i))
                    .map(|r| match &line {
                        Some(line) if !line.is_empty() => format!(
                            "|cffffd000|Henchant:{}|h[{}: {}]|h|r",
                            r.spell_id, line, r.name
                        ),
                        _ => format!("|cffffd000|Henchant:{}|h[{}]|h|r", r.spell_id, r.name),
                    })
            };
            Ok(match link {
                Some(l) => MultiValue::from_vec(vec![Value::String(lua.create_string(&l)?)]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    g.set(
        "GetTradeSkillListLink",
        lua.create_function(|lua, ()| {
            let link = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .trade_skill
                .as_ref()
                .and_then(|t| t.list_link.clone());
            opt_str(lua, link.as_ref())
        })?,
    )?;
    // IsTradeSkillLinked() (`0x5da310`): 1 and the linker's name, or nil, nil.
    g.set(
        "IsTradeSkillLinked",
        lua.create_function(|lua, ()| {
            let linked = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .trade_skill
                .as_ref()
                .and_then(|t| t.linked.clone());
            Ok(match linked {
                Some(name) => MultiValue::from_vec(vec![
                    Value::Integer(1),
                    Value::String(lua.create_string(&name)?),
                ]),
                None => MultiValue::from_vec(vec![Value::Nil, Value::Nil]),
            })
        })?,
    )?;
    // SetTradeSkillItemLevelFilter(minLevel, maxLevel) (`0x5dd090` → `0x5dd030`): a change
    // rebuilds the list and fires `TRADE_SKILL_UPDATE`.
    g.set(
        "SetTradeSkillItemLevelFilter",
        lua.create_function(|lua, args: MultiValue| {
            let num = |i: usize| {
                args.get(i)
                    .and_then(|v| lua.coerce_integer(v.clone()).ok().flatten())
            };
            let (Some(min), Some(max)) = (num(0), num(1)) else {
                return Err(mlua::Error::runtime(
                    "Usage: SetTradeSkillItemLevelFilter(minLevel, maxLevel)",
                ));
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.trade_skill_level_filter != (min, max) {
                model.trade_skill_level_filter = (min, max);
                model.trade_skill_touched = true;
            }
            Ok(())
        })?,
    )?;
    // SetTradeSkillItemNameFilter(text) (`0x5dd070` → `0x5dcf60`): nil or "" clears; a change
    // fires `TRADE_SKILL_FILTER_UPDATE`.
    g.set(
        "SetTradeSkillItemNameFilter",
        lua.create_function(|lua, args: MultiValue| {
            let text = match args.front() {
                Some(Value::String(s)) => Some(s.to_string_lossy()),
                Some(v @ (Value::Integer(_) | Value::Number(_))) => lua
                    .coerce_string(v.clone())
                    .ok()
                    .flatten()
                    .map(|s| s.to_string_lossy()),
                _ => None,
            }
            .filter(|t| !t.is_empty())
            .map(|t| t.to_lowercase());
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.trade_skill_name_filter != text {
                model.trade_skill_name_filter = text;
                model.trade_skill_filter_touched = true;
            }
            Ok(())
        })?,
    )?;
    // TradeSkillOnlyShowMakeable(flag) (`0x5dd3e0`).
    g.set(
        "TradeSkillOnlyShowMakeable",
        lua.create_function(|lua, args: MultiValue| {
            let on = binding_abi::bool_or_default(args.front(), false);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.trade_skill_only_makeable != on {
                model.trade_skill_only_makeable = on;
                model.trade_skill_touched = true;
            }
            Ok(())
        })?,
    )?;
    g.set(
        "StopTradeSkillRepeat",
        lua.create_function(|lua, ()| {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .trade_skill_stop_repeat = true;
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod wrath_tests {
    use super::super::tests::two_recipe_state;
    use crate::script::UiScript;

    fn err(s: &UiScript, call: &str) -> String {
        s.run(call).unwrap_err().to_string()
    }

    /// `GetTradeskillRepeatCount` is never below 1 (`0x5da2a0`); `DoTradeSkill` truncates and caps
    /// at numAvailable without a floor (`0x5dac90`).
    #[test]
    fn the_repeat_count_and_do_trade_skill_follow_12340() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(
            s.eval::<i64>("return GetTradeskillRepeatCount()").unwrap(),
            1
        );
        let mut state = two_recipe_state();
        state.repeat_count = 0;
        s.set_trade_skill(Some(state.clone()));
        assert_eq!(
            s.eval::<i64>("return GetTradeskillRepeatCount()").unwrap(),
            1
        );
        state.repeat_count = 3;
        s.set_trade_skill(Some(state));
        assert_eq!(
            s.eval::<i64>("return GetTradeskillRepeatCount()").unwrap(),
            3
        );
        s.run("DoTradeSkill(3, 9) DoTradeSkill(3, 2.7) DoTradeSkill(3, 0) DoTradeSkill(3)")
            .unwrap();
        assert_eq!(
            s.take_trade_skill_dos(),
            vec![(2963, 5), (2963, 2), (2963, 0), (2963, 1)]
        );
        assert!(err(&s, "DoTradeSkill()").contains("Usage: DoTradeSkill(index [, repeat])"));
    }

    /// The verbs that clear, miss in shape, or raise (`0x5da900`, `0x5db6e0`, `0x5de400`,
    /// `0x5dd460`/`0x5dd500`, `0x5da1b0`, `0x5dd0f0`).
    #[test]
    fn the_selection_misses_and_bad_indices_follow_12340() {
        let mut s = UiScript::new().unwrap();
        s.set_trade_skill(Some(two_recipe_state()));
        s.run("SelectTradeSkill(3) SelectTradeSkill(1)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetTradeSkillSelectionIndex()")
                .unwrap(),
            0,
            "a header clears the selection"
        );
        assert_eq!(
            s.eval::<(i64, i64)>("return GetTradeSkillNumMade(1)")
                .unwrap(),
            (1, 1)
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetTradeSkillReagentInfo(1, 1))")
                .unwrap(),
            4
        );
        assert!(err(&s, "CollapseTradeSkillSubClass(2)")
            .contains("Bad sub class in CollapseTradeSkillSubClass"));
        assert!(err(&s, "ExpandTradeSkillSubClass(3)")
            .contains("Bad skill line in ExpandTradeSkillSubClass"));
        s.run("CollapseTradeSkillSubClass(0) ExpandTradeSkillSubClass(-1)")
            .unwrap();
        assert!(err(&s, "GetTradeSkillSubClassFilter(5)")
            .contains("Bad sub class in GetTradeSkillSubClassFilter"));
        assert_eq!(
            s.eval::<i64>("return GetTradeSkillSubClassFilter(0)")
                .unwrap(),
            1
        );
        assert!(err(&s, "SetTradeSkillSubClassFilter(1)")
            .contains("Missing on//off parameter in SetTradeSkillSubClassFilter"));
        assert!(err(&s, "SetTradeSkillSubClassFilter(5, 1)")
            .contains("Bad sub class in SetTradeSkillSubClassFilter"));
        assert!(err(&s, "SetTradeSkillSubClassFilter({})")
            .contains("Usage: SetTradeSkillSubClassFilter(index [, on\\off, exclusive])"));
    }

    /// A recipe with no product files under the (-1, -1) header, which 12340 names by the line's
    /// alternate verb (`0x5db325`, `0x5daab4`).
    #[test]
    fn the_no_product_header_is_the_alternate_verb() {
        let mut s = UiScript::new().unwrap();
        let mut state = two_recipe_state();
        state.recipes[0].group = None;
        state.recipes[0].alt_verb = Some("Enchant".into());
        s.set_trade_skill(Some(state));
        let probe = "local n = GetTradeSkillInfo(3) return n .. '/' .. select(2, GetTradeSkillSubClasses())";
        assert_eq!(s.eval::<String>(probe).unwrap(), "Enchant/Enchant");
    }
}
