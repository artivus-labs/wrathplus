//! Build 12340 bindings that answer constants: `GetMaxDailyQuests` (`0x58c470`, the f64 25 at
//! `0xa13578`), `GetMultiCastBarOffset` and `GetNumGlyphSockets` (one body, `0x5b71e0`: the f64 6
//! at `0xa0b708`), `GetNumMapDebugObjects` (`0x608560`, 0) and `HasDebugZoneMap` (`0x8e5250`,
//! no results).
//! Evidence: `/tmp/azb-app-mig/ghidra/audit{1,2}.log`.

use mlua::{Lua, MultiValue};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set("GetMaxDailyQuests", lua.create_function(|_, ()| Ok(25.0))?)?;
    g.set(
        "GetMultiCastBarOffset",
        lua.create_function(|_, ()| Ok(6.0))?,
    )?;
    // The glyph sockets share that body: rows `0xacfa80` and `0xacff54` both name `0x5b71e0`.
    g.set("GetNumGlyphSockets", lua.create_function(|_, ()| Ok(6.0))?)?;
    g.set(
        "GetNumMapDebugObjects",
        lua.create_function(|_, ()| Ok(0.0))?,
    )?;
    g.set(
        "HasDebugZoneMap",
        lua.create_function(|_, ()| Ok(MultiValue::new()))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn the_constants_read_as_build_12340_answers() {
        let s = UiScript::new().unwrap();
        assert!(s
            .eval::<bool>(
                "return GetMaxDailyQuests() == 25 and GetMultiCastBarOffset() == 6 \
                 and GetNumMapDebugObjects() == 0 and select('#', HasDebugZoneMap()) == 0"
            )
            .unwrap());
    }

    #[test]
    fn the_socket_count_is_the_constant_six() {
        let s = UiScript::new().unwrap();
        assert_eq!(
            s.eval::<(i64, i64)>("return select('#', GetNumGlyphSockets()), GetNumGlyphSockets()")
                .unwrap(),
            (1, 6)
        );
    }
}
