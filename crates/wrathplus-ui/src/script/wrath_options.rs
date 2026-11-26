//! Wrath options-panel and UIParent globals (`AudioOptionsPanels`, `InterfaceOptionsPanels`,
//! `VideoOptionsPanels`, `UIParent.lua`), from build 12340's bindings.
//!
//! Voice: every voice binding goes through the voice device manager `[0xd37d24]`, which this
//! client never creates (no voice chat), so they answer its null arms: no capture device
//! (`0x7dac40` returns nothing), a nil output device (`0x7dab80`), selects that find nothing,
//! current devices `-1` (`0x9ec208`), and a silent loopback (`0x985e00`-`0x985e60`: level and
//! flags 0). `BNSetMatureLanguageFilter` (`0x5376c0`) does nothing without a Battle.net session.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// The stock Effects and Stereo panels' CVars (`VideoOptionsPanels.xml`), restored to defaults by
/// `RestoreVideoEffectsDefaults`/`RestoreVideoStereoDefaults` (`0x76a130(1)`/`(2)`). Unresolved,
/// labelled: the reference resets its CVar category; which CVars it holds is untraced, and the
/// panels' controls stand in.
const EFFECTS_CVARS: &[&str] = &[
    "farclip",
    "particleDensity",
    "extShadowQuality",
    "environmentDetail",
    "groundEffectDensity",
    "groundEffectDist",
    "BaseMip",
    "textureFilteringMode",
    "weatherDensity",
    "componentTextureLevel",
    "specular",
    "ffxGlow",
    "ffxDeath",
    "projectedTextures",
];
const STEREO_CVARS: &[&str] = &[
    "gxStereoEnabled",
    "gxStereoConvergence",
    "gxStereoSeparation",
];

impl super::UiScript {
    /// `SetUIVisibility`'s flag (`[0xac80a8]`), true until the script hides the UI.
    pub fn ui_visible(&self) -> bool {
        !self.model_ref().ui_hidden
    }
}

fn restore(lua: &Lua, names: &'static [&'static str]) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, _: MultiValue| {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        for name in names {
            let default = model
                .cvars
                .get(&name.to_ascii_lowercase())
                .map(|r| r.default.clone());
            if let Some(default) = default {
                super::cvars::set_from_engine(&mut model, name, default);
            }
        }
        Ok(())
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let noop = |lua: &Lua| lua.create_function(|_, _: MultiValue| Ok(()));
    // ── voice ──
    g.set(
        "VoiceEnumerateCaptureDevices",
        lua.create_function(|_, _: MultiValue| Ok(MultiValue::new()))?,
    )?;
    g.set(
        "VoiceEnumerateOutputDevices",
        lua.create_function(|_, v: Value| {
            Ok(match v {
                Value::Integer(_) | Value::Number(_) => MultiValue::from_vec(vec![Value::Nil]),
                _ => MultiValue::new(),
            })
        })?,
    )?;
    for name in ["VoiceSelectCaptureDevice", "VoiceSelectOutputDevice"] {
        g.set(name, noop(lua)?)?;
    }
    for name in [
        "VoiceGetCurrentCaptureDevice",
        "VoiceGetCurrentOutputDevice",
    ] {
        g.set(name, lua.create_function(|_, _: MultiValue| Ok(-1.0))?)?;
    }
    for name in [
        "VoiceChat_GetCurrentMicrophoneSignalLevel",
        "VoiceChat_IsPlayingLoopbackSound",
        "VoiceChat_IsRecordingLoopbackSound",
    ] {
        g.set(name, lua.create_function(|_, _: MultiValue| Ok(0))?)?;
    }
    for name in [
        "VoiceChat_PlayLoopbackSound",
        "VoiceChat_StopPlayingLoopbackSound",
        "VoiceChat_StopRecordingLoopbackSound",
        "BNSetMatureLanguageFilter",
        // `0x5f9790`: the WoW-mouse hardware probe; no such device is attached here.
        "DetectWowMouse",
    ] {
        g.set(name, noop(lua)?)?;
    }
    g.set(
        "VoiceChat_RecordLoopbackSound",
        lua.create_function(|_, v: Value| match v {
            Value::Integer(_) | Value::Number(_) => Ok(()),
            _ => Err(mlua::Error::runtime(
                "Usage: VoiceChat_RecordLoopbackSound(MaxRecordTimeInSeconds)",
            )),
        })?,
    )?;
    // ── interface / video ──
    g.set(
        "CanResetTutorials",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.wrath_tutorial.any_seen().then_some(1))
        })?,
    )?;
    // GetRefreshRates([resolution]) (`0x54f690`): the chosen mode's distinct rates, the first
    // mode's without a number; nothing past the list (`0x54f721`). No mode here reports a rate,
    // so a listed one answers the lone 0 stock greys the dropdown on (`VideoOptionsPanels.lua:277`).
    g.set(
        "GetRefreshRates",
        lua.create_function(|lua, args: MultiValue| {
            let pick = match args.front() {
                Some(v) if super::binding_abi::is_number(lua, Some(v)) => {
                    super::binding_abi::number_arg(lua, v.clone(), "")?.wrapping_sub(1) as u32
                }
                _ => 0,
            };
            let listed = lua
                .app_data_ref::<Model>()
                .expect("model")
                .screen_resolutions
                .len();
            Ok(if (pick as usize) < listed {
                MultiValue::from_vec(vec![Value::Integer(0)])
            } else {
                MultiValue::new()
            })
        })?,
    )?;
    g.set("RestoreVideoEffectsDefaults", restore(lua, EFFECTS_CVARS)?)?;
    g.set("RestoreVideoStereoDefaults", restore(lua, STEREO_CVARS)?)?;
    // ── UIParent ──
    // `0x512090` appends to the console screen, which is not built (`console.rs`): the line is
    // dropped after the usage check.
    g.set(
        "ConsoleAddMessage",
        lua.create_function(|_, v: Value| match v {
            Value::String(_) | Value::Integer(_) | Value::Number(_) => Ok(()),
            _ => Err(mlua::Error::runtime("Usage: ConsoleAddMessage(string)")),
        })?,
    )?;
    // GetBindingByKey("key"[, mode]) (`0x5625f0`): the action bound to the key, nothing when it
    // is unbound. Unresolved, labelled: the binding-set `mode` (1-4, default 4) reads the live set.
    g.set(
        "GetBindingByKey",
        lua.create_function(|lua, (key, _mode): (Value, Value)| {
            let Value::String(key) = key else {
                return Err(mlua::Error::runtime(
                    "Usage: GetBindingByKey(\"action\"[, mode])",
                ));
            };
            let action: String = lua
                .globals()
                .get::<mlua::Function>("GetBindingAction")?
                .call(key)?;
            Ok(match action {
                a if !a.is_empty() => {
                    MultiValue::from_vec(vec![Value::String(lua.create_string(a)?)])
                }
                _ => MultiValue::new(),
            })
        })?,
    )?;
    // `[0xac80a8]` from `0x815500(L, 1, 1)`, default true (`0x517c26`), then the refresh
    // `0x513cf0`. Labelled: what the refresh hides (world-attached UI) is not wired.
    g.set(
        "SetUIVisibility",
        lua.create_function(|lua, args: MultiValue| {
            let visible = super::binding_abi::bool_or_default(args.front(), true);
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .ui_hidden = !visible;
            Ok(())
        })?,
    )?;
    // The home group's counts (`[0xbd1998]`, `[0xbeb60c]`). Unresolved, labelled: a battleground
    // or instance group beside the home group is not modelled, so they are the group's counts.
    g.set(
        "GetRealNumPartyMembers",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<Model>()
                .expect("model")
                .party
                .members
                .len() as i64)
        })?,
    )?;
    g.set(
        "GetRealNumRaidMembers",
        lua.create_function(|lua, ()| {
            Ok(lua.app_data_ref::<Model>().expect("model").party.raid.len() as i64)
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn voice_answers_its_null_manager_and_ui_globals_their_state() {
        let mut s = UiScript::new().unwrap();
        s.run(
            r##"
            assert(select("#", VoiceEnumerateCaptureDevices(1)) == 0)
            assert(select("#", VoiceEnumerateOutputDevices(1)) == 1 and VoiceEnumerateOutputDevices(1) == nil)
            assert(VoiceGetCurrentCaptureDevice() == -1 and VoiceChat_IsPlayingLoopbackSound() == 0)
            assert(not pcall(VoiceChat_RecordLoopbackSound))
            assert(CanResetTutorials() == nil)
            assert(not pcall(ConsoleAddMessage) and pcall(ConsoleAddMessage, "hi"))
            assert(not pcall(GetBindingByKey) and GetBindingByKey("NOT-A-KEY") == nil)
            assert(GetRealNumPartyMembers() == 0 and GetRealNumRaidMembers() == 0)
            "##,
        )
        .unwrap();
        assert!(s.ui_visible());
        let rates = "return select('#', GetRefreshRates()), GetRefreshRates(1.9), select('#', GetRefreshRates(2))";
        assert_eq!(
            s.eval::<(i64, Option<i64>, i64)>(rates).unwrap(),
            (0, None, 0)
        );
        let mode = crate::script::ScreenResolution {
            width: 1024,
            height: 768,
        };
        s.set_screen_resolutions(vec![mode], None);
        assert_eq!(
            s.eval::<(i64, Option<i64>, i64)>(rates).unwrap(),
            (1, Some(0), 0),
            "`0x54f721`"
        );
        s.run("SetUIVisibility(false)").unwrap();
        assert!(!s.ui_visible());
        s.run("SetUIVisibility(true)").unwrap();
        assert!(s.ui_visible());
        s.run("SetUIVisibility(0)").unwrap();
        assert!(!s.ui_visible(), "0 is false to `0x815500`");
        s.run("SetUIVisibility()").unwrap();
        assert!(s.ui_visible(), "absent is the default, true");
    }
}
