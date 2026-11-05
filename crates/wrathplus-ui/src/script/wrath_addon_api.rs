//! Three build 12340 globals no stock FrameXML path had needed until the MultiBot census
//! (`azb-live/wire-a/azb-wire-ghidra-ql/multibot-api-decomp.log`): `IsLoggedIn` (`0x60a450`),
//! `GetItemIcon` (`0x517020`) and `GetCoinTextureString` (`0x510d00`). The host pushes the
//! `Item.dbc` icon join ([`UiScript::set_item_icons`]); the login flag follows `PLAYER_LOGIN`.

use std::collections::HashMap;
use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript};

/// `[0xbd0793]`, inverted: 12340 raises its byte at world init (`0x52a9e6`) and clears it just
/// before firing `PLAYER_LOGIN` (`0x52818d`, then event `0xfd`). A VM is one session, so it starts
/// logged out.
#[derive(Default)]
struct LoggedIn(bool);

/// `Item.dbc`'s `DisplayInfoID` joined through `ItemDisplayInfo.dbc`, by item entry: `Some` icon
/// path, or `None` for a row whose display names no icon.
#[derive(Default)]
struct ItemIcons(Arc<HashMap<u32, Option<Arc<str>>>>);

/// `0x70a910`'s fallback for a display with no icon.
const MISSING_ICON: &str = "Interface\\Icons\\INV_Misc_QuestionMark";

impl UiScript {
    /// Publish every `Item.dbc` entry's icon path, as `GetItemIcon` reads it.
    pub fn set_item_icons(&mut self, icons: Arc<HashMap<u32, Option<Arc<str>>>>) {
        self.lua.set_app_data(ItemIcons(icons));
    }
}

/// The `PLAYER_LOGIN` fire site: from here `IsLoggedIn()` answers 1.
pub(super) fn on_event(lua: &Lua, event: &str) {
    if event == "PLAYER_LOGIN" {
        lua.set_app_data(LoggedIn(true));
    }
}

/// `lua_isnumber`'s test: a number, or a string Lua converts to one.
fn number_arg(lua: &Lua, v: &Value) -> Option<f64> {
    match v {
        Value::Integer(_) | Value::Number(_) | Value::String(_) => {
            lua.coerce_number(v.clone()).ok().flatten()
        }
        _ => None,
    }
}

/// `GetCoinTextureString`'s body (`0x7e7e10`): zero is `COPPER_AMOUNT_TEXTURE` of 0; otherwise
/// each non-zero part, gold first, through its `*_AMOUNT_TEXTURE` with the amount and the font
/// height twice, joined by a space (`0x9e1050`). Labelled: the split `0x7e7c70` was not
/// decompiled; a negative amount answers as 0, as our `GetCoinText` does.
fn coin_texture_string(lua: &Lua, copper: u64, height: i64) -> mlua::Result<String> {
    let format = |key: &str, value: u64| -> mlua::Result<String> {
        let template = match lua.globals().get::<Value>(key)? {
            Value::String(v) => v.to_string_lossy(),
            _ => key.to_string(),
        };
        let string: mlua::Table = lua.globals().get("string")?;
        let format: mlua::Function = string.get("format")?;
        format.call((template, value, height, height))
    };
    if copper == 0 {
        return format("COPPER_AMOUNT_TEXTURE", 0);
    }
    let mut parts = Vec::new();
    for (key, value) in [
        ("GOLD_AMOUNT_TEXTURE", copper / 10_000),
        ("SILVER_AMOUNT_TEXTURE", copper / 100 % 100),
        ("COPPER_AMOUNT_TEXTURE", copper % 100),
    ] {
        if value > 0 {
            parts.push(format(key, value)?);
        }
    }
    Ok(parts.join(" "))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(LoggedIn::default());
    lua.set_app_data(ItemIcons::default());
    let globals = lua.globals();
    globals.set(
        "IsLoggedIn",
        lua.create_function(|lua, ()| {
            let on = lua.app_data_ref::<LoggedIn>().is_some_and(|l| l.0);
            Ok(super::binding_abi::flag(on))
        })?,
    )?;
    // GetExtendedItemInfo (`0x8e5250`) is the shared `xor eax, eax; ret` stub: nothing.
    globals.set(
        "GetExtendedItemInfo",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    // No `Item.dbc` row answers nothing; a row is an icon path, never nil.
    globals.set(
        "GetItemIcon",
        lua.create_function(|lua, args: MultiValue| {
            let arg = args.into_iter().next().unwrap_or(Value::Nil);
            let entry = super::item_arg::item_id_arg(
                lua,
                &lua.app_data_ref::<Model>().expect("model app_data"),
                &arg,
            );
            let icon = {
                let Some(icons) = lua.app_data_ref::<ItemIcons>() else {
                    return Ok(MultiValue::new());
                };
                match icons.0.get(&entry) {
                    Some(icon) => icon.as_deref().unwrap_or(MISSING_ICON).to_string(),
                    None => return Ok(MultiValue::new()),
                }
            };
            Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string(icon)?,
            )]))
        })?,
    )?;
    // GetCoinTextureString(amount [, fontHeight]): the height defaults to 14 and rounds as the
    // FPU does (`0x88b9c0`); the usage string is 12340's own, naming `GetCoinText`.
    globals.set(
        "GetCoinTextureString",
        lua.create_function(|lua, args: MultiValue| {
            let mut args = args.into_iter();
            let amount = args.next().unwrap_or(Value::Nil);
            let Some(amount) = number_arg(lua, &amount) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetCoinText(amount, fontHeight)",
                ));
            };
            let height = args
                .next()
                .and_then(|h| number_arg(lua, &h))
                .map_or(14, |h| h.round_ties_even() as i64);
            let copper = amount.round_ties_even().max(0.0) as u64;
            let text = coin_texture_string(lua, copper, height)?;
            Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string(text)?,
            )]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_addon_api_tests.rs"]
mod tests;
