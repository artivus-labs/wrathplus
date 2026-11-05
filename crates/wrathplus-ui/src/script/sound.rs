//! The sound bindings: `PlaySound`, `PlaySoundFile`, `PlayMusic` and `StopMusic` each queue a
//! plain request for the app to drain, since the engine does not touch the app's mixer.
//!
//! `PlaySound` (`0x9858b0`) takes a `SoundEntries` id when the argument passes `lua_isnumber`, a
//! numeric string included, else a kit name; it answers nothing. `PlaySoundFile` (`0x9859b0`) and
//! `PlayMusic` (`0x985950`) answer the number 1.
//!
//! The music pair drives its own slot: the reference gives Lua a stream of its own (`[0xb06ccc]`,
//! opened in `0x460450` at `0x4604a7`) beside the zone track's (`[0xb06cc4]`), one of the client's
//! two endlessly looping streams (`0x7a5592`). `StopMusic` (`0x985a10`) is `0x460450` with a null
//! name, so a [`MusicRequest`] is the argument, not the verb; it answers nothing.

use mlua::{Lua, Value};

use super::Model;

/// One queued sound for the app's kit player.
#[derive(Clone, Debug, PartialEq)]
pub enum SoundRequest {
    /// `PlaySound("name")`: a kit name.
    KitName(String),
    /// `PlaySound(id)`: a `SoundEntries` id.
    KitId(u32),
    /// `PlaySoundFile("path")`: a file path, no kit.
    File(String),
}

/// One queued `PlayMusic` or `StopMusic`, for the app's Lua music slot.
#[derive(Clone, Debug, PartialEq)]
pub enum MusicRequest {
    /// `PlayMusic("path")`: start the caller's looping stream.
    Play(String),
    /// `StopMusic()`: `0x460450` with no name.
    Stop,
}

impl super::UiScript {
    /// Queue a kit play from the app side, for the UI sounds the client fires from C++
    /// (`QUESTCOMPLETED` on the turn-in) that no Lua handler or message row owns.
    pub fn queue_sound_kit(&mut self, name: &str) {
        self.model_mut()
            .sound_queue
            .push(SoundRequest::KitName(name.to_string()));
    }

    /// Drain the queued sounds; the app plays each in 2D, as UI sounds have no world position.
    pub fn take_sounds(&mut self) -> Vec<SoundRequest> {
        std::mem::take(&mut self.model_mut().sound_queue)
    }

    /// Drain the music intents in call order: a play and a stop in one frame are a track or
    /// silence by their order.
    pub fn take_music(&mut self) -> Vec<MusicRequest> {
        std::mem::take(&mut self.model_mut().music_queue)
    }
}

/// Register the sound and music globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "PlaySoundFile",
        // `0x9859b0`: `lua_isstring`, so a number is a file name, else the literal at `0xaa31c4`;
        // it answers the number 1 whatever the play did (`0x9859f4`).
        lua.create_function(|lua, args: mlua::MultiValue| {
            let path = super::binding_abi::string_arg(
                lua,
                args.front().cloned().unwrap_or(Value::Nil),
                "Usage: PlaySoundFile(\"soundfile\")",
            )?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.sound_queue.push(SoundRequest::File(path));
            Ok(1)
        })?,
    )?;
    lua.globals().set(
        "PlaySound",
        // `0x9858b0`: `lua_isnumber` first, numeric strings included, truncated to an id
        // (`0x9858d4`); else `lua_isstring`, a kit name; else the literal at `0xaa3188`. Both arms
        // play through `0x4c8fc0`; no return value on any path.
        lua.create_function(|lua, args: mlua::MultiValue| {
            let arg = args.front().cloned().unwrap_or(Value::Nil);
            let request = match lua.coerce_number(arg.clone())? {
                Some(id) => SoundRequest::KitId(id as i64 as u32),
                None => SoundRequest::KitName(super::binding_abi::string_arg(
                    lua,
                    arg,
                    "Usage: PlaySound(\"sound\")",
                )?),
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // During the UI load a play is dropped before any other gate (1.12's `0x458046`;
            // labelled: its 12340 site is untraced, and both arms share `0x4c8fc0`).
            if model.sound_suppression == 0 {
                model.sound_queue.push(request);
            }
            Ok(())
        })?,
    )?;
    lua.globals().set(
        "PlayMusic",
        lua.create_function(|lua, args: mlua::MultiValue| {
            // `0x985950`: `lua_isstring` (`0x84df60`), then `lua_tolstring` (`0x84e0e0`), so a
            // number becomes a file name and anything else raises this literal (`0xaa31a4`,
            // `0x985966`); the path goes to the file layer verbatim.
            let path = super::binding_abi::string_arg(
                lua,
                args.front().cloned().unwrap_or(Value::Nil),
                "Usage: PlayMusic(\"musicfile\")",
            )?;
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .music_queue
                .push(MusicRequest::Play(path));
            Ok(1) // the number 1 (`0x985994`)
        })?,
    )?;
    lua.globals().set(
        "StopMusic",
        // Reads no argument (`0x458770`), so an extra one is ignored.
        lua.create_function(|lua, ()| {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .music_queue
                .push(MusicRequest::Stop);
            Ok(())
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::{MusicRequest, SoundRequest};
    use crate::script::UiScript;

    /// `PlaySound` answers nothing (`0x9858b0`); a number, or a numeric string, is a
    /// `SoundEntries` id, truncated, and anything else a kit name.
    #[test]
    fn playsound_queues_by_id_or_name_and_answers_nothing() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(s.arity("PlaySound(1234)").unwrap(), 0);
        assert_eq!(s.arity(r#"PlaySound("GAMEOBJECT_DOOROPEN")"#).unwrap(), 0);
        // Further arguments are never read.
        s.run(r#"PlaySound(841.7, "SFX", true) PlaySound("850")"#)
            .unwrap();
        assert_eq!(
            s.take_sounds(),
            vec![
                SoundRequest::KitId(1234),
                SoundRequest::KitName("GAMEOBJECT_DOOROPEN".into()),
                SoundRequest::KitId(841),
                SoundRequest::KitId(850),
            ]
        );
        assert_eq!(
            s.eval::<(i64, bool)>(
                r#"local n = table.getn({PlaySoundFile("Sound\\Doodad\\BellTollHorde.wav")})
                   return n, PlaySoundFile(12) == 1"#
            )
            .unwrap(),
            (1, true)
        );

        assert_eq!(
            s.take_sounds(),
            vec![
                SoundRequest::File("Sound\\Doodad\\BellTollHorde.wav".into()),
                SoundRequest::File("12".into()),
            ]
        );
        assert!(s.take_sounds().is_empty());

        for (call, usage) in [
            ("PlaySound()", r#"Usage: PlaySound("sound")"#),
            ("PlaySound({})", r#"Usage: PlaySound("sound")"#),
            ("PlaySound(true)", r#"Usage: PlaySound("sound")"#),
            ("PlaySoundFile()", r#"Usage: PlaySoundFile("soundfile")"#),
            ("PlaySoundFile(nil)", r#"Usage: PlaySoundFile("soundfile")"#),
            ("PlayMusic({})", r#"Usage: PlayMusic("musicfile")"#),
        ] {
            let e = s.run(call).expect_err(call).to_string();
            assert!(e.contains(usage), "{call}: {e}");
        }
        // `lua_pushfstring` and `lua_error` (`0x985930`), no `luaL_where`: no position even when
        // a Lua function is the caller.
        assert_eq!(
            s.eval::<String>("return select(2, pcall(function() PlaySound() end))")
                .unwrap(),
            r#"Usage: PlaySound("sound")"#
        );
        assert!(s.take_sounds().is_empty(), "a raise queues nothing");
    }

    /// `PlayMusic` answers the number 1 (`0x985994`), `StopMusic` nothing (`0x985a15`).
    #[test]
    fn the_music_pair_queues_in_call_order() {
        let mut s = UiScript::new().unwrap();
        let (played, stopped): (String, usize) = s
            .eval(
                r#"return strjoin(",", tostringall(PlayMusic("Sound\\Music\\x.mp3"))),
                          select('#', StopMusic())"#,
            )
            .unwrap();
        assert_eq!((played.as_str(), stopped), ("1", 0));
        s.run(r#"PlayMusic("Sound\\Music\\ZoneMusic\\Elwynn\\DayElwynn01.mp3")"#)
            .unwrap();
        s.run("StopMusic()").unwrap();
        assert_eq!(
            s.take_music(),
            vec![
                MusicRequest::Play("Sound\\Music\\x.mp3".into()),
                MusicRequest::Stop,
                MusicRequest::Play("Sound\\Music\\ZoneMusic\\Elwynn\\DayElwynn01.mp3".into()),
                MusicRequest::Stop,
            ]
        );
        assert!(s.take_music().is_empty());
        assert!(s.take_sounds().is_empty());
    }

    #[test]
    fn playmusic_takes_a_string_or_a_number_and_stopmusic_takes_anything() {
        let mut s = UiScript::new().unwrap();
        assert!(s.run("PlayMusic()").is_err());
        assert!(s.run("PlayMusic(nil)").is_err());
        assert!(s.run("PlayMusic(true)").is_err());
        assert!(s.run("PlayMusic({})").is_err());
        assert!(s.take_music().is_empty(), "a raise queues nothing");

        // The reference asks the file layer for `"42"` and plays nothing.
        s.run("PlayMusic(42)").unwrap();
        s.run("StopMusic(1, 2)").unwrap();
        assert_eq!(
            s.take_music(),
            vec![MusicRequest::Play("42".into()), MusicRequest::Stop]
        );
    }
}
