//! The `MovieFrame` methods and build 12340's `GameMovieFinished`. WrathPlus has no movie playback
//! path, so `StartMovie` answers as the reference does when the file cannot open (`0x970660`:
//! `0x95eb30` fails, nil returned), and the stock `MovieFrame_PlayMovie` then hides the frame and
//! calls `GameMovieFinished` (`MovieFrame.lua:17-23`). Wow.exe pass: Experience's
//! `/tmp/azb-exp-misc-clean.log` and `/tmp/azb-wireb-ghidra/movie-decomp.log`.

use mlua::{Lua, Table, Value};

/// Registry key of the MovieFrame method table.
pub(super) const REG_MOVIEFRAME_METHODS: &str = "__wrathplus_movieframe_methods";

/// The movie module's VM state.
#[derive(Default)]
struct State {
    /// `EnableSubtitles(flag)`'s value (`frame+0x2a8`); no subtitle is ever shown without playback.
    subtitles: bool,
    /// `GameMovieFinished()` calls since the host's last drain.
    finished: u32,
}

impl super::UiScript {
    /// How many times `GameMovieFinished()` ran since the last call: the host ends a pending
    /// server movie (`0x518870`).
    pub fn take_game_movie_finished(&mut self) -> u32 {
        self.lua
            .app_data_mut::<State>()
            .map_or(0, |mut s| std::mem::take(&mut s.finished))
    }
}

fn receiver_name(this: &Table) -> String {
    mlua::ObjectLike::call_method::<Option<String>>(this, "GetName", ())
        .ok()
        .flatten()
        .unwrap_or_else(|| "<unnamed>".into())
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let m = lua.create_table()?;
    // StartMovie("filename", volume) (`0x970660`): a string and a number, else the usage error;
    // nil, as the reference answers a movie it cannot open.
    m.set(
        "StartMovie",
        lua.create_function(|_, (this, file, volume): (Table, Value, Value)| {
            let named = matches!(
                file,
                Value::String(_) | Value::Integer(_) | Value::Number(_)
            );
            let numbered = match &volume {
                Value::Integer(_) | Value::Number(_) => true,
                Value::String(s) => s.to_str().is_ok_and(|s| s.trim().parse::<f64>().is_ok()),
                _ => false,
            };
            if !(named && numbered) {
                return Err(mlua::Error::runtime(format!(
                    "Usage: {}:StartMovie(\"filename\", volume_0_to_255)",
                    receiver_name(&this)
                )));
            }
            Ok(Value::Nil)
        })?,
    )?;
    // StopMovie() (`0x970730` → `0x95eba0`): stops a playing movie; nothing plays here.
    m.set("StopMovie", lua.create_function(|_, _this: Table| Ok(()))?)?;
    // EnableSubtitles(flag) (`0x970770`): the flag, truthy as `0x815500` reads it.
    m.set(
        "EnableSubtitles",
        lua.create_function(|lua, (_this, flag): (Table, Value)| {
            let on = !matches!(flag, Value::Nil | Value::Boolean(false));
            if let Some(mut s) = lua.app_data_mut::<State>() {
                s.subtitles = on;
            }
            Ok(())
        })?,
    )?;
    lua.set_named_registry_value(REG_MOVIEFRAME_METHODS, m)?;
    Ok(())
}

/// Build 12340's `GameMovieFinished()` (`0x51d590` → `0x518870`), which `MovieFrame.lua` calls when
/// a movie ends or cannot start.
pub(super) fn install_wrath(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GameMovieFinished",
        lua.create_function(|lua, ()| {
            if let Some(mut s) = lua.app_data_mut::<State>() {
                s.finished += 1;
            }
            Ok(())
        })?,
    )
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn a_movie_that_cannot_play_finishes_through_the_stock_frame_path() {
        let mut s = UiScript::new().unwrap();
        s.run(
            r#"
            local f = CreateFrame("MovieFrame", "TestMovie", UIParent)
            assert(f:StartMovie("Interface\\Cinematics\\WOW_Intro", 150) == nil)
            f:StopMovie()
            f:EnableSubtitles(1)
            assert(not pcall(f.StartMovie, f))
            assert(not pcall(f.StartMovie, f, "x"))
            if not f:StartMovie("x", 100) then GameMovieFinished() end
        "#,
        )
        .unwrap();
        assert_eq!(s.take_game_movie_finished(), 1);
        assert_eq!(s.take_game_movie_finished(), 0);
    }
}
