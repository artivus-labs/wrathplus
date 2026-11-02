//! `GetPlayerFacing()`: the player's facing in radians, 0 = north, increasing counter-clockwise
//! (the 3.3.5 addon API's own; the app pushes it with the world-map feed each frame).

use mlua::Lua;

use super::Model;

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetPlayerFacing",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model
                .worldmap
                .player_facing
                .rem_euclid(std::f32::consts::TAU))
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::super::UiScript;

    #[test]
    fn facing_reads_what_the_app_pushed() {
        let script = UiScript::new().unwrap();
        let facing: f32 = script.eval("return GetPlayerFacing()").unwrap();
        assert_eq!(facing, 0.0);
    }
}
