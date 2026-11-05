//! Build-12340 currency display state. No roster or balances are invented before a host push.
//! Contracts: Blizzard_TokenUI.lua:68,180,254,263; Blizzard_TokenUI.xml:331,370;
//! StaticPopup.lua:143-144. Display verbs do not stand in for server operations.

use super::{Model, UiScript};
use mlua::{Lua, MultiValue, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCurrencyEntry {
    /// Missing while the host item query is pending; Lua sees an empty string.
    /// Installed TokenUI.lua:68-73 hides rows with absent/empty names.
    pub name: Option<String>,
    /// Raw category identity, including IDs absent from CurrencyCategory.dbc.
    pub category_id: u32,
    pub is_header: bool,
    pub is_expanded: bool,
    pub is_unused: bool,
    pub is_watched: bool,
    pub count: u32,
    /// TokenUI: 1 = arena, 2 = honor, 0 = ordinary item currency.
    pub extra_currency_type: u32,
    /// Pending item icon projects an empty texture path, never an invented texture.
    pub icon: Option<String>,
    pub item_id: u32,
}

/// Ordered headers followed by their entries, including children of collapsed headers.
/// Names/icons/item ids must come from the installed data; counts from host state; honor/arena caps are fixed native-client constants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCurrencySnapshot {
    pub entries: Vec<WrathCurrencyEntry>,
    /// Unavailable descriptor state remains explicit in the host snapshot.
    /// Native getters project a missing Player to numeric zero, not nil.
    pub honor_current: Option<u32>,
    pub arena_current: Option<u32>,
    /// The hidden token bag's stacks (slots 118-149) as `(entry, count)`, whatever the known
    /// bits: `GetItemCount`'s default walk counts them (`0x753a50`, bit `0x200`).
    pub token_stacks: Vec<(u32, u32)>,
}

impl WrathCurrencySnapshot {
    fn visible(&self) -> Vec<usize> {
        let mut expanded = true;
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                if row.is_header {
                    expanded = row.is_expanded;
                    Some(i)
                } else if expanded {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    }

    fn list_index(&self, index: i64) -> Option<usize> {
        let index = usize::try_from(index.checked_sub(1)?).ok()?;
        self.visible().get(index).copied()
    }
}

impl UiScript {
    /// Refresh authoritative values, preserving local flags by category/item identity.
    /// Removed identities are forgotten; an empty push clears logout state.
    /// No network request is emitted. Watch slots are bounded by TokenUI's MAX_WATCHED_TOKENS=3.
    pub fn set_wrath_currency_snapshot(&mut self, mut snapshot: WrathCurrencySnapshot) {
        let model = self.model_ref();
        let old = &model.wrath_currency;
        let headers: HashMap<_, _> = old
            .entries
            .iter()
            .filter(|r| r.is_header)
            .map(|r| (r.category_id, r.is_expanded))
            .collect();
        let tokens: HashMap<_, _> = old
            .entries
            .iter()
            .filter(|r| !r.is_header && r.item_id != 0)
            .map(|r| (r.item_id, (r.is_unused, r.is_watched)))
            .collect();
        drop(model);
        let mut watched = 0;
        for row in &mut snapshot.entries {
            if row.is_header {
                if let Some(expanded) = headers.get(&row.category_id) {
                    row.is_expanded = *expanded;
                }
                row.is_watched = false;
            } else {
                if let Some((unused, watched)) = tokens.get(&row.item_id) {
                    row.is_unused = *unused;
                    row.is_watched = *watched;
                }
            }
            if !row.is_header && row.is_watched {
                watched += 1;
                row.is_watched = watched <= 3;
            }
        }
        self.model_mut().wrath_currency = snapshot;
    }
}

fn flag(value: Value) -> mlua::Result<bool> {
    // Stock TokenUI passes numeric 0/1, not Lua's truthiness (where 0 is true).
    match value {
        Value::Boolean(b) => Ok(b),
        Value::Integer(n) => Ok(n != 0),
        Value::Number(n) => Ok(n != 0.0),
        Value::Nil => Ok(false),
        _ => Err(mlua::Error::external(
            "currency flag must be numeric or boolean",
        )),
    }
}

fn info(lua: &Lua, row: Option<&WrathCurrencyEntry>, backpack: bool) -> mlua::Result<MultiValue> {
    let Some(row) = row else {
        return Ok(MultiValue::from_vec(vec![Value::Nil]));
    };
    let mut out = vec![Value::String(
        lua.create_string(row.name.as_deref().unwrap_or_default())?,
    )];
    if !backpack {
        out.extend([
            Value::Boolean(row.is_header),
            Value::Boolean(row.is_expanded),
            Value::Boolean(row.is_unused),
            Value::Boolean(row.is_watched),
        ]);
    }
    out.extend([
        Value::Integer(row.count.into()),
        Value::Integer(row.extra_currency_type.into()),
        Value::String(lua.create_string(row.icon.as_deref().unwrap_or_default())?),
        Value::Integer(row.item_id.into()),
    ]);
    Ok(MultiValue::from_vec(out))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetCurrencyListSize",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<Model>()
                .expect("model")
                .wrath_currency
                .visible()
                .len())
        })?,
    )?;
    lua.globals().set(
        "GetCurrencyListInfo",
        lua.create_function(|lua, index: i64| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let snapshot = &model.wrath_currency;
            info(
                lua,
                snapshot.list_index(index).map(|i| &snapshot.entries[i]),
                false,
            )
        })?,
    )?;
    lua.globals().set(
        "GetBackpackCurrencyInfo",
        lua.create_function(|lua, index: i64| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let row = index
                .checked_sub(1)
                .and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < 3)
                .and_then(|i| {
                    model
                        .wrath_currency
                        .entries
                        .iter()
                        .filter(|r| !r.is_header && r.is_watched)
                        .nth(i)
                });
            info(lua, row, true)
        })?,
    )?;
    for name in [
        "ExpandCurrencyList",
        "SetCurrencyBackpack",
        "SetCurrencyUnused",
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, (index, value): (i64, Value)| {
                let enabled = flag(value)?;
                let mut model = lua.app_data_mut::<Model>().expect("model");
                let snapshot = &mut model.wrath_currency;
                let Some(i) = snapshot.list_index(index) else {
                    return Ok(());
                };
                match name {
                    "ExpandCurrencyList" if snapshot.entries[i].is_header => {
                        snapshot.entries[i].is_expanded = enabled
                    }
                    "SetCurrencyUnused" if !snapshot.entries[i].is_header => {
                        snapshot.entries[i].is_unused = enabled
                    }
                    "SetCurrencyBackpack" if !snapshot.entries[i].is_header => {
                        let watched = snapshot
                            .entries
                            .iter()
                            .filter(|r| !r.is_header && r.is_watched)
                            .count();
                        if !enabled || watched < 3 || snapshot.entries[i].is_watched {
                            snapshot.entries[i].is_watched = enabled;
                        }
                    }
                    _ => {}
                }
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
    // GameTooltip:SetCurrencyToken(index) (`0x630620`), over the list's visible rows, and
    // :SetBackpackToken(index) (`0x6307e0`), over the watched ones: the token's item tooltip. A
    // header (`+0xc`) or a row out of range shows nothing. Labelled: the reference shows the
    // carried instance from the currency slots (118-149) else the template; this shows the
    // template's link, as a token carries no instance state.
    let tooltip: mlua::Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    for (method, backpack) in [("SetCurrencyToken", false), ("SetBackpackToken", true)] {
        tooltip.set(
            method,
            lua.create_function(move |lua, (this, index): (mlua::Table, Value)| {
                let index = match index {
                    Value::Integer(n) => Some(n),
                    Value::Number(n) => Some(n.round() as i64),
                    v @ Value::String(_) => lua.coerce_number(v)?.map(|n| n.round() as i64),
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
                    let snapshot = &model.wrath_currency;
                    let row = if backpack {
                        usize::try_from(index - 1)
                            .ok()
                            .filter(|i| *i < 3)
                            .and_then(|i| {
                                snapshot
                                    .entries
                                    .iter()
                                    .filter(|r| !r.is_header && r.is_watched)
                                    .nth(i)
                            })
                    } else {
                        snapshot.list_index(index).map(|i| &snapshot.entries[i])
                    };
                    row.filter(|r| !r.is_header && r.item_id != 0)
                        .map(|r| r.item_id)
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
