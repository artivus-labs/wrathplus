//! Build 12340's four binding modes and the globals that take a `mode` argument.
//!
//! Every key record of a set holds four commands, one per mode (`[rec+0x28+mode*0x10]`, its order
//! among the command's keys at `[rec+0x1c+mode*0x10]`, `0x55e470`/`0x55e4e0`). Lua names the
//! modes 1-4 and any other `mode` argument takes the call's default (`iVar - 1 < 4`). Mode 1 is
//! [`super::keybind::KeybindState`]'s base table; [`Layers`] holds modes 2-4 for each set. Mode
//! index 4, which only the defaults reach, is the resolved view: the highest of modes 2-4 that is
//! switched on and sets the key, else mode 1 (`0x55e470`'s walk). Dispatch reads it (`0x563150` is
//! always called with 4), as do `GetBindingKey` and `GetBindingAction` by default; `SetBinding`,
//! the `SetBinding*` forms and `GetBinding` default to mode 1.
//!
//! A switched-on mode is a bit of `[bindings+0x134]`, set by `0x55e550`, whose one caller
//! (`0x5206e0`) turns mode 2 on while the player is in combat (`UNIT_FLAG_IN_COMBAT`) or
//! auto-attacking (`[player+0x1858] & 2`, or an attack target at `[player+0xa20]`, `0x71af90`),
//! and off otherwise; nothing calls it for modes 3 and 4, so they are stored and read but never
//! resolve. A flip fires `UPDATE_BINDINGS` in the call (`0x55e5d0`).
//!
//! An empty command is stored as `"NONE"` (`0x562ed0`), so unbinding a key in a higher mode
//! shadows its mode-1 command while that mode is on. Deviation: in mode 1 an unbind drops the key,
//! as the base table always has; only `GetBindingKey("NONE", 1)` could tell the two apart.

use mlua::{Lua, MultiValue, Value};

use super::keybind::{bind_key, normalize_binding_key, KeyBinding, KeybindState, Row};
use super::Model;

/// The live set's index, after the defaults (0), the account set (1) and the character set (2).
pub(crate) const LIVE: usize = 3;

/// The resolved view's mode index, the default of `GetBindingKey`/`GetBindingAction`.
const RESOLVED: usize = 4;

/// What an empty command stores (`0x562ed0`).
const NONE: &str = "NONE";

/// Modes 2-4 of every set: `[set][mode - 1]`, each a key list in bind order, and the switched-on
/// mask (`[bindings+0x134]`, bit per mode index).
#[derive(Default)]
pub(crate) struct Layers {
    sets: [[Vec<KeyBinding>; 3]; 4],
    on: u32,
}

impl Layers {
    /// `0x562b80(from, to)`'s copy of the higher modes.
    pub(super) fn copy(&mut self, from: usize, to: usize) {
        self.sets[to] = self.sets[from].clone();
    }

    /// Empty a stored set's higher modes.
    pub(super) fn clear(&mut self, set: usize) {
        self.sets[set] = Default::default();
    }

    /// Bind `key` in mode index `mode` (1..=3) of `set`, an empty command storing `"NONE"`.
    fn bind(&mut self, set: usize, mode: usize, key: String, command: Option<&str>) {
        let command = command.filter(|c| !c.is_empty()).unwrap_or(NONE);
        bind_key(&mut self.sets[set][mode - 1], key, Some(command));
    }

    /// The live command `key` has in mode index `mode` (1..=3).
    fn slot(&self, mode: usize, key: &str) -> Option<&str> {
        self.sets[LIVE][mode - 1]
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, c)| c.as_str())
    }

    /// The mode index `key` resolves in: the highest switched-on mode that sets it, else 0.
    fn resolved_mode(&self, key: &str) -> usize {
        (1..=3)
            .rev()
            .find(|&m| self.on & (1 << m) != 0 && self.slot(m, key).is_some())
            .unwrap_or(0)
    }

    /// Every live key in record order (inferred: the base table's, then each higher mode's own
    /// keys in mode and bind order) with its resolved command.
    pub(super) fn effective(&self, base: &[KeyBinding]) -> Vec<KeyBinding> {
        let mut keys: Vec<String> = base.iter().map(|(k, _)| k.clone()).collect();
        for list in &self.sets[LIVE] {
            for (k, _) in list {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
        }
        keys.into_iter()
            .filter_map(|k| {
                let command = match self.resolved_mode(&k) {
                    0 => base.iter().find(|(b, _)| *b == k).map(|(_, c)| c.clone())?,
                    m => self.slot(m, &k)?.to_owned(),
                };
                Some((k, command))
            })
            .collect()
    }
}

impl KeybindState {
    /// The list mode index `mode` (0..=3) of the live set keeps.
    fn mode_list(&self, mode: usize) -> &[KeyBinding] {
        match mode {
            0 => &self.live,
            m => &self.layers.sets[LIVE][m - 1],
        }
    }

    /// The command `key` has in mode index `mode`, 4 the resolved one.
    fn command_of(&self, mode: usize, key: &str) -> Option<&str> {
        let mode = if mode == RESOLVED {
            self.layers.resolved_mode(key)
        } else {
            mode
        };
        self.mode_list(mode)
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, c)| c.as_str())
    }

    /// The keys bound to `command` in mode index `mode`, as `0x55e750` walks them: the key whose
    /// order is 0, then 1, and so on until no key has the next order. A key's order is its place
    /// among the command's keys in the mode it resolves in, so under mode 4 two modes' first keys
    /// share order 0 and only the first in record order is answered.
    pub(super) fn keys_in_mode(&self, mode: usize, command: &str) -> Vec<String> {
        let ordered = |m: usize| -> Vec<String> {
            self.mode_list(m)
                .iter()
                .filter(|(_, c)| c.eq_ignore_ascii_case(command))
                .map(|(k, _)| k.clone())
                .collect()
        };
        if mode != RESOLVED {
            return ordered(mode);
        }
        let ranked: Vec<(String, usize)> = self
            .layers
            .effective(&self.live)
            .into_iter()
            .filter(|(_, c)| c.eq_ignore_ascii_case(command))
            .filter_map(|(k, _)| {
                let order = ordered(self.layers.resolved_mode(&k))
                    .iter()
                    .position(|o| *o == k)?;
                Some((k, order))
            })
            .collect();
        (0..)
            .map_while(|n| ranked.iter().find(|(_, o)| *o == n).map(|(k, _)| k.clone()))
            .collect()
    }

    /// Whether `command` is one dispatch and `GetBindingAction` accept (`0x562140`, `0x562490`):
    /// a form (`SPELL `, `ITEM `, `MACRO `, `CLICK `, any case) or a declared command, the lookup
    /// in the table at `[bindings+0xc]` (inferred the command table: `"NONE"` is in neither).
    pub(super) fn runnable(&self, command: &str) -> bool {
        ["SPELL ", "ITEM ", "MACRO ", "CLICK "].iter().any(|p| {
            command
                .get(..p.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(p))
        }) || self.by_name.contains_key(&command.to_ascii_uppercase())
    }

    /// `SetBinding(key, command, mode)` on the live set (`0x562ed0(3, mode, …)`): refused only for
    /// a key the validator rejects.
    pub(super) fn set_in_mode(&mut self, mode: usize, key: &str, command: Option<&str>) -> bool {
        if mode == 0 {
            return self.set_binding(key, command);
        }
        let Some(key) = normalize_binding_key(key) else {
            return false;
        };
        self.layers.bind(LIVE, mode, key, command);
        self.generation += 1;
        true
    }

    /// `0x55e550(mode, on)`: switch a mode index on or off, answering whether the resolved view
    /// moved, the reference's test for firing `UPDATE_BINDINGS`.
    fn switch_mode(&mut self, mode: usize, on: bool) -> bool {
        let bit = 1 << mode;
        if mode == 0 || (self.layers.on & bit != 0) == on {
            return false;
        }
        self.layers.on ^= bit;
        self.generation += 1;
        true
    }
}

/// A mode argument (`0x84df20`, then `0x88b9c0`'s truncation): 1-4 name mode indexes 0-3, anything
/// else is `default`.
pub(super) fn mode_arg(lua: &Lua, v: Option<&Value>, default: usize) -> usize {
    let n = match v {
        Some(v @ (Value::Integer(_) | Value::Number(_) | Value::String(_))) => {
            lua.coerce_number(v.clone()).ok().flatten()
        }
        _ => None,
    };
    match n.map(f64::trunc) {
        Some(n) if (1.0..=4.0).contains(&n) => n as usize - 1,
        _ => default,
    }
}

/// `lua_isstring` (`0x84df60`) then its text.
fn string_at(lua: &Lua, args: &MultiValue, i: usize) -> Option<String> {
    match args.get(i) {
        Some(v @ (Value::String(_) | Value::Integer(_) | Value::Number(_))) => lua
            .coerce_string(v.clone())
            .ok()
            .flatten()
            .map(|s| s.to_string_lossy()),
        _ => None,
    }
}

/// Store one key in a mode of the live set and, on success, fire `UPDATE_BINDINGS` before
/// answering 1 (`0x563520`, and each `SetBinding*` form); nil for a refused key.
pub(super) fn store_and_announce(
    lua: &Lua,
    mode: usize,
    key: &str,
    command: Option<&str>,
) -> Value {
    let stored = lua
        .app_data_mut::<Model>()
        .expect("model app_data")
        .keybinds
        .set_in_mode(mode, key, command);
    if stored {
        super::event::fire_global(lua, "UPDATE_BINDINGS", &[]);
    }
    super::binding_abi::flag(stored)
}

/// The text values of `keys`, after `head`.
fn strings(lua: &Lua, head: &str, keys: Vec<String>) -> mlua::Result<MultiValue> {
    std::iter::once(head.to_owned())
        .chain(keys)
        .map(|k| Ok(Value::String(lua.create_string(k)?)))
        .collect()
}

impl super::UiScript {
    /// `0x5206e0`'s switch of mode 2: on while the player is in combat or auto-attacking. A flip
    /// fires `UPDATE_BINDINGS` in the call, and dispatch re-derives.
    pub fn set_combat_binding_mode(&mut self, on: bool) {
        let moved = self.model_mut().keybinds.switch_mode(1, on);
        if moved {
            super::event::fire_global(&self.lua, "UPDATE_BINDINGS", &[]);
        }
    }

    /// The live set's modes 2-4 as `(mode index, key, command)`, for the save.
    pub fn binding_layer_lines(&self) -> Vec<(u32, String, String)> {
        let model = self.model_mut();
        let layers = &model.keybinds.layers.sets[LIVE];
        (1..=3)
            .flat_map(|m| {
                layers[m - 1]
                    .iter()
                    .map(move |(k, c)| (m as u32, k.clone(), c.clone()))
            })
            .collect()
    }

    /// Seed set `set`'s modes 2-4 (0 the defaults, 1 account, 2 character): the defaults' own,
    /// then `lines` applied in order, a key the validator refuses dropped, as the `.wtf` reader
    /// `0x5641c0` copies the defaults in before the file's lines.
    pub fn seed_binding_layers(&mut self, set: usize, lines: Vec<(u32, String, String)>) {
        let mut model = self.model_mut();
        let layers = &mut model.keybinds.layers;
        if set != 0 {
            layers.copy(0, set);
        }
        for (mode, key, command) in lines {
            let mode = mode as usize;
            if let (1..=3, Some(key)) = (mode, normalize_binding_key(&key)) {
                layers.bind(set, mode, key, Some(&command));
            }
        }
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // `GetBinding(index[, mode])` (`0x55e8d0`): the row's name, then each key bound to it in the
    // mode (default 1). An index past the list answers the empty name alone.
    g.set(
        "GetBinding",
        lua.create_function(|lua, args: MultiValue| {
            let i = super::binding_abi::number_arg(
                lua,
                args.front().cloned().unwrap_or(Value::Nil),
                "Usage: GetBinding(index[, mode])",
            )?;
            let mode = mode_arg(lua, args.get(1), 0);
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let kb = &model.keybinds;
            let row = usize::try_from(i)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| kb.visible().nth(i));
            let name = match row {
                Some(Row::Header(h)) => h.as_str(),
                Some(Row::Command(c)) => kb.entries[*c].name.as_str(),
                None => "",
            };
            strings(lua, name, kb.keys_in_mode(mode, name))
        })?,
    )?;
    // `SetBinding("KEY"[, "COMMAND"][, mode])` (`0x563520`): 1 and `UPDATE_BINDINGS` for a stored
    // key, nil for a refused one.
    g.set(
        "SetBinding",
        lua.create_function(|lua, args: MultiValue| {
            let Some(key) = string_at(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: SetBinding(\"KEY\"[, \"COMMAND\"][, mode])",
                ));
            };
            let command = string_at(lua, &args, 1);
            let mode = mode_arg(lua, args.get(2), 0);
            Ok(store_and_announce(lua, mode, &key, command.as_deref()))
        })?,
    )?;
    // `GetBindingKey("COMMAND"[, mode])` (`0x55e9b0`): every key bound to the command, in order, in
    // the mode (default the resolved view).
    g.set(
        "GetBindingKey",
        lua.create_function(|lua, args: MultiValue| {
            let Some(command) = string_at(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetBindingKey(\"COMMAND\"[, mode])",
                ));
            };
            let mode = mode_arg(lua, args.get(1), RESOLVED);
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let keys = model.keybinds.keys_in_mode(mode, &command);
            keys.into_iter()
                .map(|k| Ok(Value::String(lua.create_string(k)?)))
                .collect::<mlua::Result<MultiValue>>()
        })?,
    )?;
    // `GetBindingAction("KEY"[, checkOverride][, mode])` (`0x562550` → `0x562490`): the key's
    // command in the mode (default the resolved view), `""` for none or for one neither a form nor
    // declared. With `checkOverride` an override of the key answers first (inferred: the priority
    // one before the normal one, as `0x55e5e0` reads the record's two entries); an override has
    // no higher modes, so under an explicit mode 2-4 it answers `""`.
    g.set(
        "GetBindingAction",
        lua.create_function(|lua, args: MultiValue| {
            let Some(key) = string_at(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetBindingAction(\"KEY\"[, checkOverride][, mode])",
                ));
            };
            let check = !matches!(args.get(1), None | Some(Value::Nil | Value::Boolean(false)));
            let mode = mode_arg(lua, args.get(2), RESOLVED);
            let key = key.to_ascii_uppercase();
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let kb = &model.keybinds;
            // The latest override of the key, a priority one before any other.
            let of_key = || kb.overrides.iter().rev().filter(|o| o.key == key);
            let overridden = check
                .then(|| of_key().find(|o| o.priority).or_else(|| of_key().next()))
                .flatten();
            let command = match overridden {
                Some(o) => (mode == 0 || mode == RESOLVED).then_some(o.command.as_str()),
                None => kb.command_of(mode, &key),
            };
            let command = command.filter(|c| kb.runnable(c)).unwrap_or("");
            lua.create_string(command)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_binding_modes_tests.rs"]
mod tests;
