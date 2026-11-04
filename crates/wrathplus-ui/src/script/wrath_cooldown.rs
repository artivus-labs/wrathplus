//! The `Cooldown` kind's own methods (`FrameKind::Cooldown`), which a plain `Frame` lacks: the
//! engine draws the sweep from the model's record ([`super::super::QuadContent::CooldownSweep`]).

use mlua::{Lua, Table, Value};

use super::super::object::frame_handle_of;
use super::super::Model;

/// The kind's method table, which `kind_method_registries` links ahead of the frame methods.
pub(crate) const REG_COOLDOWN_METHODS: &str = "__wrathplus_cooldown_methods";

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let m = lua.create_table()?;
    // SetCooldown(start, duration): seconds, a non-number read as 0.
    m.set(
        "SetCooldown",
        lua.create_function(|lua, (this, start, duration): (Table, Value, Value)| {
            let h = frame_handle_of(lua, &this)?;
            let start = lua.coerce_number(start)?.unwrap_or(0.0);
            let duration = lua.coerce_number(duration)?.unwrap_or(0.0);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let reverse = model.wrath_cooldowns.get(&h).is_some_and(|c| c.2);
            model.wrath_cooldowns.insert(h, (start, duration, reverse));
            Ok(())
        })?,
    )?;
    // GetCooldownTimes(): start and duration in milliseconds.
    m.set(
        "GetCooldownTimes",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let (start, duration) = model
                .wrath_cooldowns
                .get(&h)
                .map_or((0.0, 0.0), |c| (c.0, c.1));
            Ok((start * 1000.0, duration * 1000.0))
        })?,
    )?;
    m.set(
        "SetReverse",
        lua.create_function(|lua, (this, reverse): (Table, Value)| {
            let h = frame_handle_of(lua, &this)?;
            let reverse = !matches!(reverse, Value::Nil | Value::Boolean(false));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .wrath_cooldowns
                .entry(h)
                .or_insert((0.0, 0.0, false))
                .2 = reverse;
            Ok(())
        })?,
    )?;
    m.set("SetDrawEdge", lua.create_function(|_, _: Value| Ok(()))?)?;
    lua.set_named_registry_value(REG_COOLDOWN_METHODS, m)
}
