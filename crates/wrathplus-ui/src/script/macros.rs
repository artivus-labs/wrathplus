//! Player macros, as build 12340's macro manager keeps them (`UIMacros.cpp`): client-side records
//! with no opcode (`macros-cache.txt`), and `CreateMacro` returns the new index at once, so the
//! table lives here. The app seeds it ([`UiScript::set_macros`]) and saves it when
//! [`UiScript::take_macros_dirty`] says; the icon chooser's list is `wrath_macro_icons`'.
//!
//! Each macro carries a stable id ([`MacroView::id`]): account ids count up from 0 and character
//! ids from [`CHARACTER_MACRO_BASE`] (`0xa0f834`), each scope's counter its own (`0xbeaf58`). The
//! Lua index is a slot in a 72-entry table (`0xbeae20`), rebuilt on every change (`0x566240`):
//! the account macros fill slots 1..=36 and the character macros 37..=72, each scope sorted by
//! name, case-insensitively (`0x564a80`). So a create or a rename can move the other macros'
//! indices, and what holds a macro across that (the cursor, an action button) holds its id.
//! A macro's bound spell (`0x4e5a50`'s macro arm) is the app's to derive: it needs the spell
//! catalog and the player's book.

use mlua::{Lua, MultiValue, Value};

use super::cursor::{queue_cursor_update, CursorMacro, CursorPayload};
use super::Model;

/// The account scope's cap (`0xa0f82c`), `MAX_ACCOUNT_MACROS` in `Blizzard_MacroUI.lua:1`, and so
/// the character slots' base.
pub const MAX_ACCOUNT_MACROS: usize = 36;

/// The character scope's cap (`0xa0f830`), `MAX_CHARACTER_MACROS`.
pub const MAX_CHARACTER_MACROS: usize = 18;

/// The first character macro id's base (`0xa0f834`): a character id is `0x1000000 + n`, which an
/// action word carries as the `0x41` kind (`ACTION_BUTTON_CMACRO`) over the low 24 bits.
pub const CHARACTER_MACRO_BASE: u32 = 0x0100_0000;

/// The name copy's cap (`0x568086`, 0x40 bytes with the NUL), in bytes.
pub const MAX_MACRO_NAME: usize = 63;

/// The body cap, `MacroFrameText`'s `letters="255"`.
pub const MAX_MACRO_BODY: usize = 255;

/// The slot table's half width (`0x566240` steps 0x24 slots per scope).
const SCOPE_SLOTS: usize = 36;

/// One macro, as `GetMacroInfo` reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacroView {
    /// The stable id (`[rec+0x18]`), which the saved file's `MACRO %d` carries.
    pub id: u32,
    pub name: String,
    /// The icon's texture path, kept by name because the reference saves the icon by name.
    pub texture: Option<String>,
    /// The macro's lines, `\n`-separated.
    pub body: String,
}

/// A macro record's cached cast, the reference's `([rec+0x564], [rec+0x568])` pair (`0x4efe00`):
/// the bound spell and the book it resolved from, which the app derives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MacroBinding {
    /// The bound spell id; 0 for none, -1 for a `/cast` that did not resolve.
    pub spell: i32,
    /// The spell came from the pet's book (`0x4b3b51` writes 1), not the player's.
    pub pet_book: bool,
}

/// The macro table: the account and character lists, each sorted by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacroState {
    /// Slots 1..=36.
    pub account: Vec<MacroView>,
    /// Slots 37..=72 (37..=54 in use).
    pub character: Vec<MacroView>,
}

/// The macro id an action word names: kind `0x40` over the id, or `0x41` over a character id's
/// low 24 bits. `None` for any other kind.
pub fn macro_action_id(kind: u8, action: u32) -> Option<u32> {
    (kind & 0xfe == 0x40).then(|| (u32::from(kind & 1) << 24) | (action & 0x00ff_ffff))
}

/// The action word's `(kind, action)` for macro `id`, as `PlaceAction` packs it.
pub fn macro_action_word(id: u32) -> (u8, u32) {
    (0x40 | (id >> 24) as u8 & 1, id & 0x00ff_ffff)
}

/// A 1-based Lua macro index split into its scope and 0-based position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MacroIndex {
    per_character: bool,
    pos: usize,
}

impl MacroIndex {
    /// `None` outside the 72 slots; not checked against the lists, which [`MacroState::get`] does.
    fn split(index: usize) -> Option<Self> {
        let zero = index.checked_sub(1)?;
        (zero < 2 * SCOPE_SLOTS).then_some(Self {
            per_character: zero >= SCOPE_SLOTS,
            pos: zero % SCOPE_SLOTS,
        })
    }

    fn lua_index(self) -> usize {
        self.pos + 1 + if self.per_character { SCOPE_SLOTS } else { 0 }
    }
}

impl MacroState {
    /// The macro at a 1-based Lua index.
    pub fn get(&self, index: usize) -> Option<&MacroView> {
        let at = MacroIndex::split(index)?;
        self.list(at.per_character).get(at.pos)
    }

    /// The macro with stable id `id` (`0x6f6020`'s hash lookup).
    pub fn by_id(&self, id: u32) -> Option<&MacroView> {
        self.account
            .iter()
            .chain(&self.character)
            .find(|m| m.id == id)
    }

    /// The 1-based Lua index of the macro with id `id` (`0x564ab0` plus one).
    pub fn index_of_id(&self, id: u32) -> Option<usize> {
        [false, true].into_iter().find_map(|per_character| {
            let pos = self.list(per_character).iter().position(|m| m.id == id)?;
            Some(MacroIndex { per_character, pos }.lua_index())
        })
    }

    fn list(&self, per_character: bool) -> &Vec<MacroView> {
        if per_character {
            &self.character
        } else {
            &self.account
        }
    }

    fn list_mut(&mut self, per_character: bool) -> &mut Vec<MacroView> {
        if per_character {
            &mut self.character
        } else {
            &mut self.account
        }
    }

    /// `GetMacroIndexByName` (`0x566620`): case-insensitive, account slots first; 0 on a miss.
    pub fn index_by_name(&self, name: &str) -> usize {
        [&self.account, &self.character]
            .into_iter()
            .flatten()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .and_then(|m| self.index_of_id(m.id))
            .unwrap_or(0)
    }

    /// The slot rebuild's sort (`0x566240` → `qsort` with `0x564a80`, `SStrCmpI` on the names).
    /// Deviation: `qsort` leaves equal names in no fixed order; this sort keeps their order.
    fn sort(&mut self) {
        for list in [&mut self.account, &mut self.character] {
            list.sort_by_key(|m| m.name.to_ascii_lowercase());
        }
    }
}

/// Truncate to `max` bytes at a char boundary, as the fixed-size copies do.
fn clamp_bytes(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Clamp the body to a count of characters, as its edit box does.
fn clamp_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

impl super::UiScript {
    /// Seed the whole table from the app's load, sorted as the slot rebuild sorts it, with each
    /// scope's id counter at its highest id (`0x567c10`). It does not mark the table dirty, or the
    /// load would trigger a save.
    pub fn set_macros(&mut self, mut state: MacroState) {
        state.sort();
        let mut model = self.model_mut();
        model.macro_counters = [
            state.account.iter().map(|m| m.id).max().unwrap_or(0),
            state
                .character
                .iter()
                .map(|m| m.id)
                .max()
                .unwrap_or(0)
                .max(CHARACTER_MACRO_BASE),
        ];
        model.macros = state;
        model.macros_dirty = false;
        model.macros_generation += 1;
    }

    /// The live table, for the app's save.
    pub fn macros(&self) -> MacroState {
        self.model_mut().macros.clone()
    }

    /// Push every macro's cached cast by macro id, replacing the last push.
    pub fn set_macro_bindings(&mut self, bindings: std::collections::HashMap<u32, MacroBinding>) {
        self.model_mut().macro_bindings = bindings;
    }

    /// Whether a script changed the table since the last call; the app then saves it and fires
    /// `UPDATE_MACROS` (`0x81b530(0x180)` at the rebuild).
    pub fn take_macros_dirty(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().macros_dirty)
    }

    /// Bumped by every seed and every change, so a per-frame reader such as the action bar compares
    /// a `u64` instead of diffing the table. Not [`Self::take_macros_dirty`]: that drain has one
    /// owner, the save, and a second reader would steal its edge.
    pub fn macros_generation(&self) -> u64 {
        self.model_mut().macros_generation
    }
}

fn changed(model: &mut Model) {
    model.macros.sort();
    model.macros_dirty = true;
    model.macros_generation += 1;
}

/// `0x568020`: a new record under the scope's next id, `None` with the scope full (`0xa0f82c`).
fn create_macro(
    model: &mut Model,
    name: &str,
    texture: Option<String>,
    body: &str,
    per_character: bool,
) -> Option<usize> {
    let scope = usize::from(per_character);
    let cap = [MAX_ACCOUNT_MACROS, MAX_CHARACTER_MACROS][scope];
    if model.macros.list(per_character).len() >= cap {
        return None;
    }
    model.macro_counters[scope] += 1;
    let id = model.macro_counters[scope];
    model.macros.list_mut(per_character).push(MacroView {
        id,
        name: clamp_bytes(name, MAX_MACRO_NAME),
        texture,
        body: clamp_chars(body, MAX_MACRO_BODY),
    });
    changed(model);
    model.macros.index_of_id(id)
}

/// The record a macro argument names (`0x566df0`): a number is a slot, else a string a name.
pub(super) fn macro_arg(model: &Model, v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::String(s) if s.to_str().ok()?.trim().parse::<f64>().is_err() => {
            let index = model.macros.index_by_name(&s.to_string_lossy());
            model.macros.get(index).map(|m| m.id)
        }
        v => {
            let n = match v {
                Value::Integer(i) => *i as f64,
                Value::Number(n) => *n,
                Value::String(s) => s.to_str().ok()?.trim().parse().ok()?,
                _ => return None,
            };
            let index = usize::try_from(n.round_ties_even() as i64).ok()?;
            model.macros.get(index).map(|m| m.id)
        }
    }
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Integer(i) => Some(*i as f64),
        Value::Number(n) => Some(*n),
        Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
        _ => None,
    }
}

fn string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.to_string_lossy()),
        Value::Integer(i) => Some(i.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The chooser's icon at a 1-based `iconIndex` (`0x564d70` under `0x565c20`'s count) as a texture
/// path; `None` out of range, which leaves the record's icon empty (`0x5681c3`).
fn chooser_icon(lua: &Lua, v: Option<&Value>) -> Option<String> {
    let n = number(v)?.round_ties_even() as i64;
    super::wrath_macro_icons::spell_icon(lua, n).map(|name| format!("Interface\\Icons\\{name}"))
}

/// Register the macro globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetNumMacros",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok((
                model.macros.account.len() as i64,
                model.macros.character.len() as i64,
            ))
        })?,
    )?;

    // `GetMacroInfo(index or name)` (`0x566eb0`): name, texture and body; nothing for no macro.
    // Labelled: the dynamic icon of a `INV_Misc_QuestionMark` macro (`0x566ac0`'s bound spell or
    // item legs) is not modelled.
    g.set(
        "GetMacroInfo",
        lua.create_function(|lua, args: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(m) = macro_arg(&model, args.front()).and_then(|id| model.macros.by_id(id))
            else {
                return Ok(MultiValue::new());
            };
            let texture = match &m.texture {
                Some(t) => Value::String(lua.create_string(t)?),
                None => Value::Nil,
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&m.name)?),
                texture,
                Value::String(lua.create_string(&m.body)?),
            ]))
        })?,
    )?;

    g.set(
        "GetMacroIndexByName",
        lua.create_function(|lua, args: MultiValue| {
            let Some(name) = string(args.front()) else {
                return Err(mlua::Error::runtime("Usage: GetMacroIndexByName(name)"));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.macros.index_by_name(&name) as i64)
        })?,
    )?;

    // `CreateMacro(name, iconIndex, body, perCharacter)` (`0x568160`): the new index, or the
    // usage and failure errors. Labelled: the protected-call gate `0x5191c0(0xc)` has no
    // counterpart in this trusted UI.
    g.set(
        "CreateMacro",
        lua.create_function(|lua, args: MultiValue| {
            let a: Vec<Value> = args.into_iter().collect();
            let (Some(name), Some(_)) = (string(a.first()), number(a.get(1))) else {
                return Err(mlua::Error::runtime(
                    "Usage: CreateMacro(name, iconIndex, body, perCharacter)",
                ));
            };
            if name.is_empty() {
                return Err(mlua::Error::runtime(
                    "CreateMacro() failed, no name specified",
                ));
            }
            let texture = chooser_icon(lua, a.get(1));
            let body = string(a.get(2)).unwrap_or_default();
            let per_character = flag(a.get(3));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            match create_macro(&mut model, &name, texture, &body, per_character) {
                Some(i) => Ok(i as i64),
                // The count is the constant 36 for either scope (`0x56823d push 0x24`).
                None => Err(mlua::Error::runtime(
                    "CreateMacro() failed, already have 36 macros",
                )),
            }
        })?,
    )?;

    // `EditMacro(index or name, name, iconIndex, body)` (`0x566fa0`): an omitted argument leaves
    // its field alone, an out-of-range icon too; the answer is the macro's index after the
    // re-sort. Labelled: a missing macro answers 0, where the reference's search for id 0 answers
    // the first empty slot.
    g.set(
        "EditMacro",
        lua.create_function(|lua, args: MultiValue| {
            let a: Vec<Value> = args.into_iter().collect();
            let icon = chooser_icon(lua, a.get(2));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(id) = macro_arg(&model, a.first()) else {
                return Ok(0);
            };
            let (name, body) = (string(a.get(1)), string(a.get(3)));
            let macros = &mut model.macros;
            let Some(m) = macros
                .account
                .iter_mut()
                .chain(&mut macros.character)
                .find(|m| m.id == id)
            else {
                return Ok(0);
            };
            if let Some(name) = name {
                m.name = clamp_bytes(&name, MAX_MACRO_NAME);
            }
            if icon.is_some() {
                m.texture = icon;
            }
            if let Some(body) = body {
                super::wrath_macro_api::forget_explicit(lua, id);
                m.body = clamp_chars(&body, MAX_MACRO_BODY);
            }
            changed(&mut model);
            Ok(model.macros.index_of_id(id).map_or(0, |i| i as i64))
        })?,
    )?;

    // `DeleteMacro(index or name)` (`0x566f70` → `0x566d50`): no results. Labelled: its action-bar
    // sweep (`0x5aadd0(id)`) is the app's.
    g.set(
        "DeleteMacro",
        lua.create_function(|lua, args: MultiValue| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if let Some(id) = macro_arg(&model, args.front()) {
                super::wrath_macro_api::forget_explicit(lua, id);
                model.macros.account.retain(|m| m.id != id);
                model.macros.character.retain(|m| m.id != id);
                changed(&mut model);
            }
            Ok(())
        })?,
    )?;

    // `PickupMacro(index or name)` (`0x5674f0` → `0x520be0`): the cursor holds the macro's id,
    // mode 8. No results. Refused while the cursor holds anything; the reference clears it first
    // (`0x519280(1, 1)`).
    g.set(
        "PickupMacro",
        lua.create_function(|lua, args: MultiValue| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.cursor.is_some() {
                return Ok(());
            }
            let Some(m) = macro_arg(&model, args.front()).and_then(|id| model.macros.by_id(id))
            else {
                return Ok(());
            };
            let payload = CursorMacro {
                id: m.id,
                texture: m.texture.clone(),
            };
            model.cursor = Some(CursorPayload::Macro(payload));
            queue_cursor_update(&mut model);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// `GetCursorInfo`'s macro answer: `"macro"` and the held id's index now. Labelled: 12340's
/// mode-8 answer is untraced.
pub(super) fn cursor_info(
    lua: &Lua,
    macros: &MacroState,
    m: &CursorMacro,
) -> mlua::Result<Vec<Value>> {
    let index = macros.index_of_id(m.id).map_or(0, |i| i as i64);
    Ok(vec![
        Value::String(lua.create_string("macro")?),
        Value::Integer(index),
    ])
}

/// `perCharacter`, read with `GetBoolOrDefault` (`0x815500`): `nil`, `false` and `0` are false.
fn flag(v: Option<&Value>) -> bool {
    v.is_some_and(super::action::truthy_nonzero)
}

#[cfg(test)]
#[path = "macros_tests.rs"]
mod tests;
