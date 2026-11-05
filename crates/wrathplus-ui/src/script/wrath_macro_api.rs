//! Build 12340's macro queries and setters beside `macros.rs`'s table: a macro's body, its bound
//! spell or item, the running macro, and the cursor's macro mode.
//!
//! A record carries a bound spell (`[rec+0x564]`, with its book at `[rec+0x56c]`) and a bound item
//! (`[rec+0x568]`). The body's derivation is the app's ([`super::MacroBinding`]);
//! `SetMacroSpell`/`SetMacroItem` write the pair explicitly and take the record off the re-derive
//! list (`[rec+0x574]` cleared, `0x565ad0`), so an explicit pair overrides the derived one until
//! the body is edited or the macro deleted (inferred from that unlink: the body setter `0x5663a0`
//! is not decompiled). The `[,target]` argument is the unit whose guid the reference stores at
//! `[rec+0x578]` (`0x60abf0`); nothing reads it but the setters' change tests and the re-derive's
//! (`0x5650c0`), so it is kept for the tests alone.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::cursor::CursorPayload;
use super::Model;

/// An explicit pair a setter wrote: `spell` 0 for none, -1 for a name that did not resolve, and
/// whether it came from the pet's book; `item` 0 for none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExplicitMacroBind {
    pub spell: i32,
    pub pet_book: bool,
    pub item: u32,
    /// The `[,target]` unit's guid (`[rec+0x578]`), 0 for none.
    pub target: u64,
}

type Explicit = ExplicitMacroBind;

/// The macro running now (`[0xbeaf4c]`, the record) and the button it was run with
/// (`[0xbeaf48]`), as `0x564db0` saves and restores them around a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunningMacro {
    /// The record's id; `None` for no run, or a `RunMacroText` run, whose scratch record has id 0.
    pub id: Option<u32>,
    pub button: Option<String>,
}

#[derive(Default)]
struct State {
    explicit: HashMap<u32, Explicit>,
    /// Bumped by every setter call: the app's signal to re-derive the bar's bound spells.
    generation: u64,
    running: RunningMacro,
}

fn state(lua: &Lua) -> mlua::AppDataRefMut<'_, State> {
    lua.app_data_mut::<State>().expect("macro api state")
}

/// Set the running macro's button (`[0xbeaf48]`), answering the old one: a `CLICK` binding sets
/// it to the click's mouse button around the click (`0x563150`).
pub(super) fn swap_running_button(lua: &Lua, button: Option<String>) -> Option<String> {
    std::mem::replace(&mut state(lua).running.button, button)
}

/// Drop macro `id`'s explicit pair: its body changed or it was deleted.
pub(super) fn forget_explicit(lua: &Lua, id: u32) {
    state(lua).explicit.remove(&id);
}

impl super::UiScript {
    /// Mark a macro run as `0x564db0` does before its lines: answer the previous run, which
    /// [`Self::end_running_macro`] restores, so a nested run unwinds to its caller's.
    pub fn begin_running_macro(&mut self, run: RunningMacro) -> RunningMacro {
        std::mem::replace(&mut state(&self.lua).running, run)
    }

    /// Restore the run [`Self::begin_running_macro`] answered.
    pub fn end_running_macro(&mut self, previous: RunningMacro) {
        state(&self.lua).running = previous;
    }

    /// The pairs `SetMacroSpell`/`SetMacroItem` wrote, by macro id, which override the body's
    /// derived spell.
    pub fn macro_explicit_binds(&self) -> HashMap<u32, ExplicitMacroBind> {
        state(&self.lua).explicit.clone()
    }

    /// Bumped by every `SetMacroSpell`/`SetMacroItem` call; re-derive the bound spells when it
    /// moves.
    pub fn macro_explicit_generation(&self) -> u64 {
        state(&self.lua).generation
    }
}

/// The record's `(spell, item)`: the explicit pair, else the body's derived spell and no item.
fn bound(lua: &Lua, model: &Model, id: u32) -> Explicit {
    if let Some(e) = state(lua).explicit.get(&id) {
        return *e;
    }
    let b = model.macro_bindings.get(&id).copied().unwrap_or_default();
    Explicit {
        spell: b.spell,
        pet_book: b.pet_book,
        ..Explicit::default()
    }
}

/// The `[,target]` argument's guid (`0x60abf0` with no default): a unit token the resolver knows,
/// else 0. Inferred: a name or a token it refuses reads 0.
fn target(lua: &Lua, model: &Model, args: &MultiValue) -> u64 {
    let Some(token) = args
        .get(2)
        .and_then(|v| super::binding_abi::optional_string(lua, v))
    else {
        return 0;
    };
    model.unit_guids.resolve(&token).ok().flatten().unwrap_or(0)
}

/// The macro record argument 1 names (`0x566df0`), as its id.
fn record(lua: &Lua, args: &MultiValue) -> Option<u32> {
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    super::macros::macro_arg(&model, args.front())
}

/// `lua_isstring`: a string or a number (`0x84df60`).
fn is_string(v: Option<&Value>) -> bool {
    matches!(
        v,
        Some(Value::String(_) | Value::Integer(_) | Value::Number(_))
    )
}

fn text(lua: &Lua, v: Option<&Value>) -> mlua::Result<String> {
    Ok(lua
        .coerce_string(v.cloned().unwrap_or(Value::Nil))?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

/// `SetMacroItem`'s item (`0x709de0`): the number after an `item:` anywhere in the text, else the
/// template whose name matches; 0 for neither. Labelled: the name leg (`0x53f3b0`) reads the
/// client's item cache, which the VM's templates stand in for.
fn item_id(model: &Model, query: &str) -> u32 {
    if let Some(at) = query.find("item:") {
        let digits: String = query[at + 5..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        return digits.parse().unwrap_or(0);
    }
    model
        .item_templates
        .iter()
        .find(|(_, t)| !query.is_empty() && t.name.eq_ignore_ascii_case(query))
        .map_or(0, |(id, _)| *id)
}

/// The link of a carried copy of `item` (`0x754a20(item, 0)`): the equipped slots in slot order,
/// then the backpack and the four bags, each in slot order. Inferred: that search order, and
/// that the bank is not searched.
fn carried_link(model: &Model, item: u32) -> Option<String> {
    if item == 0 {
        return None;
    }
    let worn = model.inventory_slots.iter().flatten();
    if let Some(link) = worn
        .filter(|s| s.item_id == item)
        .find_map(|s| s.link.clone())
    {
        return Some(link);
    }
    (0..=4).find_map(|bag| {
        let c = model.containers.get(&bag)?;
        let mut slots: Vec<_> = c.slots.iter().filter(|(_, s)| s.item_id == item).collect();
        slots.sort_by_key(|(slot, _)| **slot);
        slots.into_iter().find_map(|(_, s)| s.link.clone())
    })
}

/// The setters' tail (`0x567320`, `0x5670d0`): write the pair and, when `changed` (the setter's
/// own test: the other field was set, or its own field or the target moved), refresh the bar's
/// slots that show the macro (`0x5aa920`), each firing `ACTIONBAR_SLOT_CHANGED` with its action id
/// in slot order within the call; the app re-derives the bound spells off the generation. Answer
/// 1 when the setter's own field resolved (`0 < id`), else nil.
fn set(lua: &Lua, id: u32, pair: Explicit, changed: bool, resolved: i64) -> Value {
    {
        let mut st = state(lua);
        st.explicit.insert(id, pair);
        st.generation += 1;
    }
    if changed {
        let mut slots: Vec<u32> = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            model
                .actions
                .iter()
                .filter(|(&a, s)| {
                    a <= 144 && super::macros::macro_action_id(s.kind, s.action) == Some(id)
                })
                .map(|(&a, _)| a)
                .collect()
        };
        slots.sort_unstable();
        for a in slots {
            let arg = super::ScriptValue::Int(i64::from(a));
            super::event::fire_global(lua, "ACTIONBAR_SLOT_CHANGED", &[arg]);
        }
    }
    super::binding_abi::flag(resolved > 0)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let g = lua.globals();

    // `GetMacroBody(index or name)` (`0x566f40`): the body, or nothing for no macro.
    g.set(
        "GetMacroBody",
        lua.create_function(|lua, args: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let body = super::macros::macro_arg(&model, args.front())
                .and_then(|id| model.macros.by_id(id))
                .map(|m| m.body.clone());
            body.map(|b| Ok(Value::String(lua.create_string(b)?)))
                .into_iter()
                .collect::<mlua::Result<MultiValue>>()
        })?,
    )?;

    // `GetMacroSpell(index or name)` (`0x567450`): the bound spell's `Spell.dbc` name and rank
    // (`0x4cfd20`), two nils for a spell that does not resolve, nothing for no macro.
    g.set(
        "GetMacroSpell",
        lua.create_function(|lua, args: MultiValue| {
            let Some(id) = record(lua, &args) else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let row = u32::try_from(bound(lua, &model, id).spell)
                .ok()
                .and_then(|spell| model.spell_catalog.get(&spell));
            let (name, rank) = match row {
                Some(r) => (
                    Value::String(lua.create_string(&r.name)?),
                    Value::String(lua.create_string(r.rank.as_deref().unwrap_or(""))?),
                ),
                None => (Value::Nil, Value::Nil),
            };
            Ok(MultiValue::from_vec(vec![name, rank]))
        })?,
    )?;

    // `GetMacroItem(index or name)` (`0x567200`): the bound item's name and link, nothing for no
    // macro, no item or an uncached one (`0x67ca30`). A carried copy (`0x754a20`) answers its own
    // name and link, a random suffix included (`0x707c20`, `0x61e3a0`); else the template's
    // (`0x706d70`, `0x61e360`), as `GetItemInfo` builds them.
    g.set(
        "GetMacroItem",
        lua.create_function(|lua, args: MultiValue| {
            let (item, carried) = {
                let Some(id) = record(lua, &args) else {
                    return Ok(MultiValue::new());
                };
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let item = bound(lua, &model, id).item;
                (item, carried_link(&model, item))
            };
            if item == 0 {
                return Ok(MultiValue::new());
            }
            if let Some(link) = carried {
                let name = super::cursor::item_link_name(Some(&link));
                return Ok(MultiValue::from_vec(vec![
                    Value::String(lua.create_string(name)?),
                    Value::String(lua.create_string(link)?),
                ]));
            }
            let info: MultiValue = lua
                .globals()
                .get::<mlua::Function>("GetItemInfo")?
                .call(item)?;
            Ok(info.into_iter().take(2).collect())
        })?,
    )?;

    // `SetMacroSpell(macro, spell [,target])` (`0x567320`): bind the spell the player's book
    // resolves (`0x540200`, a leading `!` dropped), -1 for none, and clear the item. Answers 1 for
    // a resolved spell, else nil; nothing for no macro. `0x540200` writes the book it found the
    // spell in to `[rec+0x56c]`; inferred: the player's book is searched before the pet's.
    g.set(
        "SetMacroSpell",
        lua.create_function(|lua, args: MultiValue| {
            if !is_string(args.get(1)) {
                return Err(mlua::Error::runtime(
                    "Usage: SetMacroSpell(macro, spell [,target])",
                ));
            }
            let Some(id) = record(lua, &args) else {
                return Ok(MultiValue::new());
            };
            let query = text(lua, args.get(1))?;
            let query = query.strip_prefix('!').unwrap_or(&query);
            let (pair, changed) = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let pets = super::SpellBookState {
                    tabs: Vec::new(),
                    slots: model.pet_book.slots.clone(),
                };
                let found = |book| super::spellbook::resolve_spell_by_name(book, query);
                let (spell, pet_book) = match (found(&model.spellbook), found(&pets)) {
                    (Some(s), _) => (s.spell_id as i32, false),
                    (None, Some(s)) => (s.spell_id as i32, true),
                    (None, None) => (-1, false),
                };
                let pair = Explicit {
                    spell,
                    pet_book,
                    item: 0,
                    target: target(lua, &model, &args),
                };
                let old = bound(lua, &model, id);
                let changed = old.item > 0 || old.spell != spell || old.target != pair.target;
                (pair, changed)
            };
            let spell = pair.spell;
            let answer = set(lua, id, pair, changed, spell.into());
            Ok(MultiValue::from_vec(vec![answer]))
        })?,
    )?;

    // `SetMacroItem(macro, item [,target])` (`0x5670d0`): bind the item (`0x709de0`) and set the
    // spell to 0 for a found item, -1 for none. Answers 1 for a found item, else nil; nothing for
    // no macro.
    g.set(
        "SetMacroItem",
        lua.create_function(|lua, args: MultiValue| {
            if !is_string(args.get(1)) {
                return Err(mlua::Error::runtime(
                    "Usage: SetMacroItem(macro, item [,target])",
                ));
            }
            let Some(id) = record(lua, &args) else {
                return Ok(MultiValue::new());
            };
            let query = text(lua, args.get(1))?;
            let (pair, changed) = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let item = item_id(&model, &query);
                let pair = Explicit {
                    spell: if item != 0 { 0 } else { -1 },
                    pet_book: false,
                    item,
                    target: target(lua, &model, &args),
                };
                let old = bound(lua, &model, id);
                let changed = old.spell > 0 || old.item != item || old.target != pair.target;
                (pair, changed)
            };
            let item = pair.item;
            let answer = set(lua, id, pair, changed, item.into());
            Ok(MultiValue::from_vec(vec![answer]))
        })?,
    )?;

    // `GetRunningMacro()` (`0x564c10`): the running macro's index, nil outside a run and in a
    // `RunMacroText` run (its record's id is 0).
    g.set(
        "GetRunningMacro",
        lua.create_function(|lua, ()| {
            let Some(id) = state(lua).running.id else {
                return Ok(Value::Nil);
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            // `0x564ab0` answers -1 for a record off the table, so the index reads 0.
            Ok(Value::Integer(
                model.macros.index_of_id(id).map_or(0, |i| i as i64),
            ))
        })?,
    )?;

    // `GetRunningMacroButton()` (`0x564c70`): the button the running macro was run with, nil
    // without one.
    g.set(
        "GetRunningMacroButton",
        lua.create_function(|lua, ()| {
            let button = state(lua).running.button.clone();
            button.map(|b| lua.create_string(b)).transpose()
        })?,
    )?;

    // `CursorHasMacro()` (`0x515180`): 1 while the cursor holds a macro (mode 8), else nil.
    g.set(
        "CursorHasMacro",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(super::binding_abi::flag(matches!(
                model.cursor,
                Some(CursorPayload::Macro(_))
            )))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_macro_api_tests.rs"]
mod tests;
