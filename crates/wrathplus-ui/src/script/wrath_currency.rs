//! Build-12340 currency display state. No roster or balances are invented before a host push.
//! Contracts: Blizzard_TokenUI.lua:68,180,254,263; Blizzard_TokenUI.xml:331,370;
//! StaticPopup.lua:143-144. Display verbs do not stand in for server operations.
//!
//! The list is 12340's own build (`0x5b0030`), redone on a push and inside the three setters:
//! categories sorted by `0x5afc40` (flag bit 0 last, then name), the last one the Unused bucket;
//! each category's known tokens in table order, then sorted by item name (`0x5aff10`). The unused
//! and backpack marks are bits of `currencyTokensUnused1/2` and `currencyTokensBackpack1/2`
//! (`0x51f01a`-`0x51f08c`, flags `0x20`). Labelled: 12340 stores those per character; here they
//! are global, as every other such CVar is.

use super::{Model, UiScript};
use crate::strings::collate;
use mlua::{Lua, MultiValue, Value};
use std::collections::HashMap;

/// A `CurrencyTypes.dbc` row, in table order, with its item's display state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCurrencyEntry {
    /// The item's name; `None` while the item query is pending.
    pub name: Option<String>,
    /// Raw category identity; an id with no `CurrencyCategory.dbc` row never shows.
    pub category_id: u32,
    /// One-based bit in the known mask and in the CVar pairs.
    pub bit_index: u32,
    pub count: u32,
    /// TokenUI: 1 = arena, 2 = honor, 0 = ordinary item currency.
    pub extra_currency_type: u32,
    /// `None` while the item query is pending, never an invented texture.
    pub icon: Option<String>,
    pub item_id: u32,
}

/// A `CurrencyCategory.dbc` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCurrencyCategory {
    pub id: u32,
    pub flags: u32,
    pub name: String,
    /// 1 at load (`0x5afde2`); the VM keeps a fold by id across pushes and forgets it on the
    /// empty logout push. Labelled: 12340 reloads the table on entering the world (`0x5afd90`).
    pub is_expanded: bool,
}

/// One built row (`0xc1e960`, stride 0x18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// Index into the sorted categories.
    Header(usize),
    /// Index into the entries, and the backpack bit at the build (`+0x14`).
    Token(usize, bool),
}

/// The built list; the VM rebuilds it, and a host push leaves it empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCurrencyList(Vec<Row>);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCurrencySnapshot {
    pub categories: Vec<WrathCurrencyCategory>,
    pub entries: Vec<WrathCurrencyEntry>,
    /// `PLAYER_FIELD_KNOWN_CURRENCIES`, both words.
    pub known: u64,
    /// Native getters project a missing Player to numeric zero, not nil.
    pub honor_current: Option<u32>,
    pub arena_current: Option<u32>,
    /// The hidden token bag's stacks (slots 118-149) as `(entry, count)`, whatever the known
    /// bits: `GetItemCount`'s default walk counts them (`0x753a50`, bit `0x200`).
    pub token_stacks: Vec<(u32, u32)>,
    pub list: WrathCurrencyList,
}

/// A one-based bit as a mask; 0 and anything past 64 mark nothing.
fn bit(index: u32) -> u64 {
    index
        .checked_sub(1)
        .and_then(|i| 1u64.checked_shl(i))
        .unwrap_or(0)
}

const UNUSED: [&str; 2] = ["currencyTokensUnused1", "currencyTokensUnused2"];
const BACKPACK: [&str; 2] = ["currencyTokensBackpack1", "currencyTokensBackpack2"];

/// A CVar pair's 64 bits: each CVar's integer (`[cvar+0x30]`, `SStrToInt`), the second the high word.
fn marks(model: &Model, pair: [&str; 2]) -> u64 {
    let word = |name: &str| {
        let slot = model.cvars.get(&name.to_ascii_lowercase());
        slot.map_or(0, |s| super::cvars::sstr_to_int(&s.value) as u32)
    };
    u64::from(word(pair[0])) | (u64::from(word(pair[1])) << 32)
}

/// Sets or clears a token's bit in a CVar pair, written as `%d` (`0x766940`).
fn mark(model: &mut Model, pair: [&str; 2], index: u32, on: bool) {
    let mask = bit(index);
    if mask == 0 {
        return;
    }
    let all = marks(model, pair);
    let all = if on { all | mask } else { all & !mask };
    let (name, word) = if index > 32 {
        (pair[1], (all >> 32) as u32)
    } else {
        (pair[0], all as u32)
    };
    super::cvars::set_from_engine(model, name, (word as i32).to_string());
}

impl WrathCurrencySnapshot {
    /// `0x5b0030`: a category shows when one of its own rows is known, or, for the last, when any
    /// row is marked unused, known or not (`0x5afe30`). A folded header adds no rows.
    fn build(&self, unused: u64, backpack: u64) -> Vec<Row> {
        let mut rows = Vec::new();
        let last = self.categories.len().checked_sub(1);
        for (c, cat) in self.categories.iter().enumerate() {
            let is_last = Some(c) == last;
            let mine = |e: &WrathCurrencyEntry| e.category_id == cat.id;
            let shows = self.entries.iter().any(|e| {
                (mine(e) && self.known & bit(e.bit_index) != 0)
                    || (is_last && unused & bit(e.bit_index) != 0)
            });
            if !shows {
                continue;
            }
            rows.push(Row::Header(c));
            if !cat.is_expanded {
                continue;
            }
            let start = rows.len();
            for (i, e) in self.entries.iter().enumerate() {
                let mask = bit(e.bit_index);
                let unused = unused & mask != 0;
                let child = if is_last { unused } else { mine(e) && !unused };
                if self.known & mask != 0 && child {
                    rows.push(Row::Token(i, backpack & mask != 0));
                }
            }
            // `0x5aff10`: an item with no record or no name yet sorts last.
            rows[start..].sort_by(|a, b| {
                let name = |r: &Row| match r {
                    Row::Token(i, _) => self.entries[*i].name.as_deref(),
                    Row::Header(_) => None,
                };
                match (name(a), name(b)) {
                    (Some(a), Some(b)) => collate(a, b),
                    (a, b) => a.is_none().cmp(&b.is_none()),
                }
            });
        }
        rows
    }

    /// `0x5b0340`: the n-th row, in table order, both known and marked for the backpack. No cap:
    /// `MAX_WATCHED_TOKENS` is TokenUI's (`Blizzard_TokenUI.lua:3`).
    fn backpack(&self, backpack: u64, n: i64) -> Option<&WrathCurrencyEntry> {
        let n = usize::try_from(n).ok()?;
        let marked = |e: &&WrathCurrencyEntry| self.known & backpack & bit(e.bit_index) != 0;
        self.entries.iter().filter(marked).nth(n)
    }

    fn row(&self, index: i64) -> Option<Row> {
        self.list.0.get(usize::try_from(index).ok()?).copied()
    }
}

/// Rebuild the list from the snapshot and the CVars, as each of 12340's rebuild points does.
fn rebuild(model: &mut Model) {
    let (unused, backpack) = (marks(model, UNUSED), marks(model, BACKPACK));
    let list = model.wrath_currency.build(unused, backpack);
    model.wrath_currency.list = WrathCurrencyList(list);
}

impl UiScript {
    /// Refresh the tables and values; folds carry over by category id, and an empty push (logout)
    /// forgets them. No network request is emitted.
    pub fn set_wrath_currency_snapshot(&mut self, mut snapshot: WrathCurrencySnapshot) {
        let mut model = self.model_mut();
        let folds: HashMap<_, _> = (model.wrath_currency.categories.iter())
            .map(|c| (c.id, c.is_expanded))
            .collect();
        for cat in &mut snapshot.categories {
            cat.is_expanded = folds.get(&cat.id).copied().unwrap_or(cat.is_expanded);
        }
        snapshot.categories.sort_by(|a, b| {
            (a.flags & 1)
                .cmp(&(b.flags & 1))
                .then_with(|| collate(&a.name, &b.name))
        });
        model.wrath_currency = snapshot;
        rebuild(&mut model);
    }
}

/// A name or icon, nil while the item is unresolved (`0x5b06fe`, `0x5b08d5`).
fn text(lua: &Lua, t: Option<&str>) -> mlua::Result<Value> {
    t.map_or(Ok(Value::Nil), |t| lua.create_string(t).map(Value::String))
}

/// An index as the five verbs read it: `isnumber`, else their `Usage:`; `__ftol2_sse`, less one.
fn index_arg(lua: &Lua, v: Option<&Value>, usage: &'static str) -> mlua::Result<i64> {
    let v = v.cloned().unwrap_or(Value::Nil);
    Ok(super::binding_abi::ftol_arg(lua, v, usage)?
        .wrapping_sub(1)
        .into())
}

/// `GetCurrencyListInfo`'s nine values for a built row (`0x5b0680`); nine nils off the list.
fn list_info(lua: &Lua, model: &Model, row: Option<Row>) -> mlua::Result<MultiValue> {
    let s = &model.wrath_currency;
    let Some(row) = row else {
        return Ok(MultiValue::from_vec(vec![Value::Nil; 9]));
    };
    let text = |t: Option<&str>| text(lua, t);
    let out = match row {
        Row::Header(c) => {
            let last = c + 1 == s.categories.len();
            vec![
                text(Some(&s.categories[c].name))?,
                Value::Boolean(true),
                Value::Boolean(s.categories[c].is_expanded),
                Value::Boolean(last),
                Value::Boolean(false),
                Value::Integer(0),
                Value::Integer(0),
                text(None)?,
                Value::Integer(0),
            ]
        }
        Row::Token(i, watched) => {
            let e = &s.entries[i];
            // `0x5b04f0`: the CVar bit of the first row naming the item, read at the call.
            let first = s.entries.iter().find(|r| r.item_id == e.item_id);
            let unused = marks(model, UNUSED) & bit(first.map_or(0, |r| r.bit_index)) != 0;
            vec![
                text(e.name.as_deref())?,
                Value::Boolean(false),
                Value::Boolean(false),
                Value::Boolean(unused),
                Value::Boolean(watched),
                Value::Integer(e.count.into()),
                Value::Integer(e.extra_currency_type.into()),
                text(e.icon.as_deref())?,
                Value::Integer(e.item_id.into()),
            ]
        }
    };
    Ok(MultiValue::from_vec(out))
}

/// `GetBackpackCurrencyInfo`'s five values (`0x5b0a90`); five nils off the walk or while the
/// item is unresolved (`0x5b0b0d`).
fn backpack_info(lua: &Lua, e: Option<&WrathCurrencyEntry>) -> mlua::Result<MultiValue> {
    let Some(e) = e.filter(|e| e.name.is_some()) else {
        return Ok(MultiValue::from_vec(vec![Value::Nil; 5]));
    };
    let text = |t: Option<&str>| text(lua, t);
    Ok(MultiValue::from_vec(vec![
        text(e.name.as_deref())?,
        Value::Integer(e.count.into()),
        Value::Integer(e.extra_currency_type.into()),
        text(e.icon.as_deref())?,
        Value::Integer(e.item_id.into()),
    ]))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetCurrencyListSize",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(model.wrath_currency.list.0.len())
        })?,
    )?;
    lua.globals().set(
        "GetCurrencyListInfo",
        lua.create_function(|lua, args: MultiValue| {
            let index = index_arg(lua, args.front(), "Usage: GetCurrencyListInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model");
            list_info(lua, &model, model.wrath_currency.row(index))
        })?,
    )?;
    lua.globals().set(
        "GetBackpackCurrencyInfo",
        lua.create_function(|lua, args: MultiValue| {
            let index = index_arg(lua, args.front(), "Usage: GetBackpackCurrencyInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model");
            let backpack = marks(&model, BACKPACK);
            backpack_info(lua, model.wrath_currency.backpack(backpack, index))
        })?,
    )?;
    // Both arguments `isnumber`-gated and read by `__ftol2_sse`, so a boolean or nil raises.
    for (name, usage) in [
        (
            "ExpandCurrencyList",
            "Usage: ExpandCurrencyList(index,expand)",
        ),
        (
            "SetCurrencyBackpack",
            "Usage: SetCurrencyBackpack(index,backpack)",
        ),
        (
            "SetCurrencyUnused",
            "Usage: SetCurrencyUnused(index,unused)",
        ),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let index = index_arg(lua, args.front(), usage)?;
                let value = args.get(1).cloned().unwrap_or(Value::Nil);
                let enabled = super::binding_abi::ftol_arg(lua, value, usage)? != 0;
                let mut model = lua.app_data_mut::<Model>().expect("model");
                let row = model.wrath_currency.row(index);
                match (name, row) {
                    ("ExpandCurrencyList", Some(Row::Header(c))) => {
                        model.wrath_currency.categories[c].is_expanded = enabled;
                    }
                    (_, Some(Row::Token(i, _))) if name != "ExpandCurrencyList" => {
                        let item = model.wrath_currency.entries[i].item_id;
                        let entries = &model.wrath_currency.entries;
                        let first = entries.iter().find(|r| r.item_id == item);
                        let index = first.map_or(0, |r| r.bit_index);
                        let pair = if name == "SetCurrencyUnused" {
                            UNUSED
                        } else {
                            BACKPACK
                        };
                        mark(&mut model, pair, index, enabled);
                    }
                    _ => {}
                }
                // Each setter rebuilds whatever the index (`0x5b098f`, `0x5b09ff`, `0x5b0a6f`).
                rebuild(&mut model);
                Ok(())
            })?,
        )?;
    }
    // Build 12340: 0x60FC40/0x60FCC0 push current (or FLDZ without Player),
    // then fixed image doubles at 0xA23358 (75000) / 0xA1AD58 (10000).
    // Neither getter reads a server-supplied maximum; both return two numbers.
    for (name, honor) in [("GetHonorCurrency", true), ("GetArenaCurrency", false)] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, ()| {
                let model = lua.app_data_ref::<Model>().expect("model");
                Ok(if honor {
                    (model.wrath_currency.honor_current.unwrap_or(0), 75_000u32)
                } else {
                    (model.wrath_currency.arena_current.unwrap_or(0), 10_000u32)
                })
            })?,
        )?;
    }
    // GetMaxArenaCurrency() (`0x5d6ff0`): the same fixed double (`0xa1ad58`).
    lua.globals().set(
        "GetMaxArenaCurrency",
        lua.create_function(|_, ()| Ok(10_000.0f64))?,
    )?;
    // GameTooltip:SetCurrencyToken(index) (`0x630620`), over the list's built rows, and
    // :SetBackpackToken(index) (`0x6307e0`), over the backpack walk: the token's item tooltip. A
    // header or a row out of range shows nothing. Labelled: the reference shows the carried
    // instance from the currency slots (118-149) else the template; this shows the template's
    // link, as a token carries no instance state.
    let tooltip: mlua::Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    for (method, backpack) in [("SetCurrencyToken", false), ("SetBackpackToken", true)] {
        tooltip.set(
            method,
            lua.create_function(move |lua, (this, index): (mlua::Table, Value)| {
                // `0x6306bb`, `0x63087b`: `fistp` under chop, so the index truncates.
                let index = match index {
                    Value::Integer(n) => Some(n),
                    Value::Number(n) => Some(n as i64),
                    v @ Value::String(_) => lua.coerce_number(v)?.map(|n| n as i64),
                    _ => None,
                };
                let Some(index) = index else {
                    let name: Option<String> = mlua::ObjectLike::call_method(&this, "GetName", ())?;
                    return Err(mlua::Error::runtime(format!(
                        "Usage: {}:{method}(index)",
                        name.unwrap_or_else(|| "<unnamed>".into())
                    )));
                };
                let item = {
                    let model = lua.app_data_ref::<Model>().expect("model");
                    let s = &model.wrath_currency;
                    let i = index.saturating_sub(1);
                    let entry = if backpack {
                        s.backpack(marks(&model, BACKPACK), i)
                    } else {
                        match s.row(i) {
                            Some(Row::Token(e, _)) => s.entries.get(e),
                            _ => None,
                        }
                    };
                    entry.map(|e| e.item_id).filter(|&id| id != 0)
                };
                if let Some(item) = item {
                    mlua::ObjectLike::call_method::<()>(
                        &this,
                        "SetHyperlink",
                        format!("item:{item}"),
                    )?;
                }
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
