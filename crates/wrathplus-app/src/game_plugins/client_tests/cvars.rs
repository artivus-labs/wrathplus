//! Whole-table tests of `crate::cvars`, from `cvars/mod.rs` (`tests`): they read every feature's
//! rows, resources and `on_cvar` observers, so they live in the root layer (APP-CRATE-SPLIT-PLAN,
//! item 4 follow-up). `cvars` keeps the tests of its own rows.

use std::collections::BTreeMap;
use std::time::Instant;

use bevy::prelude::*;
use wrathplus_ui::script::UiScript;

use crate::app_core::state::LootConfig;
use crate::app_core::ui_flags::UiScaleCvar;
use crate::chat_bubble::BubbleConfig;
use crate::cvars::*;
use crate::minimap::MinimapZoom;
use crate::nameplates::NameConfig;
use crate::player::camera::{FollowConfig, FollowStyle, LookConfig, FOLLOW_SPEED_RANGE};
use crate::player::camera_zoom::ZoomLimit;
use crate::portrait::PaneRate;
use crate::sound::SoundConfig;
use crate::target::ClickConfig;
use crate::ui_script::DEFAULT_UI_SCALE;
use crate::video::VideoConfig;
use crate::world_backdrop::{RenderScale, RENDER_SCALE_RANGE};
use wrathplus_ui::widget::MINIMAP_ZOOM_LEVELS;
use wrathplus_world::clutter::ClutterConfig;
use wrathplus_world::view::{MsaaSetting, ViewDistance, FARCLIP_RANGE, MSAA_RANGE};

/// Every row's [`Reference`] column holds, both ways: a `Same` row that drifted fails, and so
/// does a `Deviates` or `Overridden` row back in agreement. Numbers parse-compare, so "1" and
/// "1.0" agree.
#[test]
fn defaults_stand_where_the_reference_column_says() {
    /// Numeric when both parse, textual otherwise.
    fn agrees(ours: &str, theirs: &str) -> bool {
        match (ours.parse::<f32>(), theirs.parse::<f32>()) {
            (Ok(a), Ok(b)) => a == b,
            _ => ours == theirs,
        }
    }
    for row in crate::shipped_cvars::rows() {
        let name = row.name;
        match &row.reference {
            Reference::Same(value) => assert!(
                agrees(row.default, value),
                "{name}: the row claims the reference registers {value:?} and we ship the \
                 same, but our default is {:?}. If the reference really does differ, this is \
                 a `deviates` row and owes a reason.",
                row.default,
            ),
            Reference::Deviates { value, why } => {
                assert!(
                    !agrees(row.default, value),
                    "{name}: a `deviates` row that no longer deviates — our {:?} IS the \
                     reference's. Demote it to `same`; a stale deviation hides that we are \
                     faithful again.",
                    row.default,
                );
                assert!(!why.trim().is_empty(), "{name}: a deviation owes a reason");
            }
            Reference::Overridden { registered, why } => {
                assert!(
                    !agrees(row.default, registered),
                    "{name}: an `overridden` row whose default is just the registered string \
                     {registered:?} — that is `same`, and saying otherwise buries a real \
                     override behind a false one.",
                );
                assert!(
                    !why.trim().is_empty(),
                    "{name}: an override owes its mechanism"
                );
            }
            Reference::Ours(why) => assert!(
                !why.trim().is_empty(),
                "{name}: a CVar the reference does not have owes the reason it exists",
            ),
        }
    }
}

/// Every numeric registered default equals the code constant it mirrors; the string rows are
/// checked in [`the_string_valued_cvars_are_the_realm_and_the_windowed_size`].
#[test]
fn registered_defaults_mirror_the_code_truths() {
    let d: BTreeMap<&str, f32> = crate::shipped_cvars::rows()
        .filter_map(|r| r.default.parse::<f32>().ok().map(|f| (r.name, f)))
        .collect();
    let sound = SoundConfig::default();
    assert_eq!(d["Sound_MasterVolume"], sound.master);
    assert_eq!(d["Sound_SFXVolume"], sound.sfx);
    assert_eq!(d["Sound_MusicVolume"], sound.music);
    assert_eq!(d["Sound_AmbienceVolume"], sound.ambience);
    assert_eq!(d["Sound_EnableAllSound"] != 0.0, sound.enabled);
    assert_eq!(d["Sound_EnableSFX"] != 0.0, sound.sfx_enabled);
    assert_eq!(d["Sound_EnableMusic"] != 0.0, sound.music_enabled);
    assert_eq!(d["Sound_EnableAmbience"] != 0.0, sound.ambience_enabled);
    assert_eq!(d["Sound_EnableErrorSpeech"] != 0.0, sound.error_speech);
    assert_eq!(
        d["Sound_EnableSoundWhenGameIsInBG"] != 0.0,
        sound.background_sound
    );
    assert!(
        !sound.background_sound,
        "the reference goes quiet in the background and offers no way out"
    );
    assert_eq!(d["Sound_EnableReverb"] != 0.0, sound.reverb);
    assert_eq!(d["SoundOutputLimiter"] != 0.0, sound.limiter);
    assert!(sound.limiter, "the output limiter ships on");
    assert!(!sound.reverb, "zone reverb ships off");
    assert_eq!(d["uiScale"], DEFAULT_UI_SCALE);
    // `ViewDistance::default()` reads `$WOW_FARCLIP`, so this is the env-less literal.
    assert_eq!(d["farclip"], 350.0);
    assert_eq!(d["nearclip"], wrathplus_world::view::NEARCLIP_DEFAULT);
    assert_eq!(
        d["nearclip"],
        ViewDistance::default().nearclip,
        "the registered default and the resource's own must be one number"
    );
    // `MsaaSetting::default()` reads `$WOW_MSAA`, so this is the env-less literal.
    assert_eq!(d["gxMultisample"], 1.0);
    assert_eq!(
        d["deselectOnClick"] != 0.0,
        ClickConfig::default().deselect_on_click
    );
    assert_eq!(
        d["mouseInvertPitch"] != 0.0,
        LookConfig::default().invert_pitch
    );
    assert_eq!(d["mousespeed"], LookConfig::default().sensitivity);
    assert_eq!(d["cameraDistanceMax"], ZoomLimit::default().distance_max());
    assert_eq!(d["cameraDistanceMaxFactor"], ZoomLimit::default().factor());
    let follow = FollowConfig::default();
    let style = (d["cameraSmoothStyle"], d["cameraSmoothTrackingStyle"]);
    assert_eq!((style, follow.style), ((4.0, 4.0), FollowStyle::Smart));
    assert_eq!(d["cameraYawSmoothSpeed"], follow.yaw_speed);
    assert_eq!(d["autoLootDefault"] != 0.0, LootConfig::default().auto_loot);
    assert_eq!(
        d["showLootSpam"] != 0.0,
        LootConfig::default().show_loot_spam
    );
    assert_eq!(
        d["guildMemberNotify"] != 0.0,
        crate::guild::GuildMemberNotify::default().0
    );
    assert_eq!(d["guildMemberNotify"], 0.0, "the binary registers \"0\"");
    assert_eq!(
        d["BlockTrades"] != 0.0,
        crate::ui_trade::BlockTrades::default().0
    );
    assert_eq!(d["BlockTrades"], 0.0, "an unset BlockTrades allows trades");
    // Each `UnitName*` row's default is its bit of the mask's boot value.
    let names = NameConfig::default();
    for (row, value) in &d {
        if let Some(bit) = crate::nameplates::name_bit::of(&row.to_ascii_lowercase()) {
            assert_eq!(*value != 0.0, names.on(bit), "{row}");
        }
    }
    assert_eq!(
        names.0, 0x1bc,
        "the binary's \"1\" rows: guild, PvP title, both reactions' player and pet names"
    );
    let plates = crate::vplates::VPlateMode::default();
    assert_eq!(d["nameplateShowEnemies"] != 0.0, plates.enemies);
    assert_eq!(d["nameplateShowFriends"] != 0.0, plates.friends);
    assert_eq!(d["nameplateShowEnemyTotems"] != 0.0, plates.enemy_totems);
    assert_eq!(d["showVKeyCastbar"] != 0.0, plates.cast_bar);
    assert!(
        plates.cast_bar,
        "the binary registers showVKeyCastbar \"1\""
    );
    assert_eq!(d["threatWarning"], f32::from(plates.threat_warning));
    assert_eq!(
        plates.threat_warning, 3,
        "the binary registers threatWarning \"3\""
    );
    assert_eq!(
        d["nameplateShowFriendlyGuardians"] != 0.0,
        plates.friendly_guardians
    );
    assert!(
        !plates.enemies && !plates.friends && plates.enemy_pets && plates.friendly_totems,
        "the binary registers both categories \"0\" and every sub-gate \"1\""
    );
    let camera_opts = crate::player::camera_dynamics::CameraOptions::default();
    assert_eq!(d["cameraPivot"] != 0.0, camera_opts.pivot);
    assert!(camera_opts.pivot, "the binary registers cameraPivot \"1\"");
    assert_eq!(
        d["cameraWaterCollision"] != 0.0,
        camera_opts.water_collision
    );
    assert!(
        camera_opts.pivot && camera_opts.water_collision,
        "the binary registers cameraPivot and cameraWaterCollision both \"1\""
    );
    assert_eq!(d["cameraTerrainTilt"] != 0.0, camera_opts.terrain_tilt);
    assert!(
        !camera_opts.terrain_tilt,
        "the binary registers cameraTerrainTilt \"0\""
    );
    assert_eq!(
        d["cameraGroundSmoothSpeed"],
        camera_opts.ground_smooth_speed
    );
    assert_eq!(d["cameraTerrainTiltTimeMin"], camera_opts.tilt_time_min);
    assert_eq!(d["cameraTerrainTiltTimeMax"], camera_opts.tilt_time_max);
    assert_eq!(d["cameraBobbing"] != 0.0, camera_opts.bobbing);
    assert!(
        !camera_opts.bobbing && !camera_opts.terrain_tilt,
        "the binary registers cameraBobbing and cameraTerrainTilt both \"0\""
    );
    assert_eq!(d["cameraBobbingLRAmplitude"], camera_opts.bob_lr_amplitude);
    assert_eq!(d["cameraBobbingUDAmplitude"], camera_opts.bob_ud_amplitude);
    assert_eq!(d["cameraBobbingFrequency"], camera_opts.bob_frequency);
    assert_eq!(d["cameraBobbingSmoothSpeed"], camera_opts.bob_smooth_speed);
    assert_eq!(d["cameraPivotDXMax"], camera_opts.pivot_dx_max);
    assert_eq!(d["cameraPivotDYMin"], camera_opts.pivot_dy_min);
    assert_eq!(
        d["cameraTargetSmoothSpeed"],
        camera_opts.target_smooth_speed
    );
    assert_eq!(
        d["weatherDensity"],
        f32::from(wrathplus_world::weather::WeatherState::default().weather_density)
    );
    // `ClutterConfig::default()` reads `$WOW_CLUTTER_DENSITY`; stop 1 is its env-less ×2.
    assert_eq!(d["WorldDetail"], 1.0);
    assert_eq!(
        d["frillDensity"],
        (d["WorldDetail"] + 1.0) * wrathplus_formats::FRILL_DENSITY as f32
    );
    let bubbles = BubbleConfig::default();
    assert_eq!(d["ChatBubbles"] != 0.0, bubbles.all);
    assert_eq!(d["ChatBubblesParty"] != 0.0, bubbles.party);
    assert!(bubbles.all && bubbles.party, "12340 registers both 1");
    let zoom = MinimapZoom::default();
    assert_eq!(d["minimapZoom"], f32::from(zoom.outdoor));
    assert_eq!(d["minimapInsideZoom"], f32::from(zoom.inside));
    assert_eq!(zoom.outdoor, wrathplus_ui::widget::MINIMAP_DEFAULT_ZOOM);
    // `video::tests` welds `VideoConfig::vsync` to the window's boot present mode.
    assert_eq!(d["gxVSync"] != 0.0, VideoConfig::default().vsync);
    assert_eq!(d["boothHalfRate"] != 0.0, PaneRate::default().half);
    // Every visual golden assumes a 1:1 backdrop.
    assert_eq!(d["renderScale"], 1.0);
}

/// The FFX pass's three switches reach it one by one (`ffx`, `ffxGlow`, `ffxDeath`).
#[test]
fn the_ffx_switches_reach_the_pass() {
    use wrathplus_world::ffx_glow::FfxSwitches;
    let mut app = cvar_app();
    let on = FfxSwitches::default();
    assert_eq!(*res::<FfxSwitches>(&app), on);
    apply(&mut app, "ffxGlow", "0");
    assert_eq!(*res::<FfxSwitches>(&app), FfxSwitches { glow: false, ..on });
    apply(&mut app, "ffxglow", "1");
    apply(&mut app, "ffxDeath", "0");
    assert_eq!(
        *res::<FfxSwitches>(&app),
        FfxSwitches { death: false, ..on }
    );
    apply(&mut app, "ffxDeath", "1");
    apply(&mut app, "ffx", "0");
    assert_eq!(
        *res::<FfxSwitches>(&app),
        FfxSwitches {
            master: false,
            ..on
        }
    );
}

/// `gxMaximize` registers latched (flags 3): a write is staged and moves nothing until the
/// `RestartGx` commit, where the window knob takes it.
#[test]
fn gx_maximize_waits_for_the_restart_commit() {
    let mut app = cvar_app();
    let outcome = app
        .world_mut()
        .resource_mut::<Cvars>()
        .set("gxMaximize", "1");
    assert_eq!(outcome, SetOutcome::Staged);
    let events = app.world_mut().resource_mut::<Cvars>().take_events();
    assert!(events.is_empty(), "a staged write fires no callback");
    assert!(!res::<VideoConfig>(&app).maximize);
    assert_eq!(app.world_mut().resource_mut::<Cvars>().commit_latched(), 1);
    let events = app.world_mut().resource_mut::<Cvars>().take_events();
    for event in events {
        app.world_mut().trigger(event);
    }
    assert!(res::<VideoConfig>(&app).maximize, "the commit applies it");
}

#[test]
fn the_observers_apply_every_arm() {
    let mut app = cvar_app();
    apply(&mut app, "Sound_MusicVolume", "0.7");
    assert_eq!(res::<SoundConfig>(&app).music, 0.7);
    // A string row reaches its knob; a value that is not an address leaves the knob alone.
    apply(&mut app, "realmList", "logon.example.org:3724");
    assert_eq!(
        res::<crate::realmlist::Realmlist>(&app).address(),
        "logon.example.org:3724"
    );
    apply(
        &mut app,
        "realmlist",
        r#"SET realmlist "elsewhere.example.org""#,
    );
    assert_eq!(
        res::<crate::realmlist::Realmlist>(&app).address(),
        "elsewhere.example.org"
    );
    apply(&mut app, "realmList", "not an address");
    assert_eq!(
        res::<crate::realmlist::Realmlist>(&app).address(),
        "elsewhere.example.org",
        "a known key with a bad value is consumed, and the resource keeps its truth",
    );
    // Clamps are the knob's own: volume to [0,1], farclip to FARCLIP_RANGE.
    apply(&mut app, "sound_mastervolume", "7");
    assert_eq!(res::<SoundConfig>(&app).master, 1.0);
    apply(&mut app, "farclip", "50");
    assert_eq!(res::<ViewDistance>(&app).farclip, *FARCLIP_RANGE.start());
    // `nearclip` clamps to the callback's `[0.01, 0.33]` (`0x688d90`).
    apply(&mut app, "nearclip", "0.001");
    assert_eq!(
        res::<ViewDistance>(&app).nearclip,
        0.01,
        "[0x8029d0], the callback's low bound"
    );
    apply(&mut app, "nearclip", "9");
    assert_eq!(
        res::<ViewDistance>(&app).nearclip,
        0.33,
        "[0x808300], its high bound"
    );
    apply(&mut app, "nearclip", "0.3");
    assert_eq!(res::<ViewDistance>(&app).nearclip, 0.3);
    // A sample count, clamped to the reference's [1, 16]; 1 is none.
    apply(&mut app, "gxMultisample", "4");
    assert_eq!(res::<MsaaSetting>(&app).samples, 4);
    apply(&mut app, "anisotropic", "99");
    assert_eq!(
        res::<wrathplus_assets::TexFilterSetting>(&app).aniso,
        *wrathplus_assets::ANISO_RANGE.end()
    );
    apply(&mut app, "anisotropic", "0");
    assert_eq!(
        res::<wrathplus_assets::TexFilterSetting>(&app).aniso,
        *wrathplus_assets::ANISO_RANGE.start()
    );
    // The knob ships on, so only the flip to 0 proves the arm.
    apply(&mut app, "trilinear", "0");
    assert!(!res::<wrathplus_assets::TexFilterSetting>(&app).trilinear);
    apply(&mut app, "trilinear", "1");
    assert!(res::<wrathplus_assets::TexFilterSetting>(&app).trilinear);
    // Clamped to the device's ceiling too: this GPU offers 4, and wgpu refuses a count it
    // lacks.
    apply(&mut app, "gxmultisample", "99");
    assert_eq!(res::<MsaaSetting>(&app).samples, 4);
    // The realistic route in: a config written where 8x exists, opened where it does not.
    apply(&mut app, "gxMultisample", "8");
    assert_eq!(
        res::<MsaaSetting>(&app).samples,
        4,
        "a device that stops at 4x must never be handed an 8"
    );
    apply(&mut app, "gxmultisample", "2");
    assert_eq!(res::<MsaaSetting>(&app).samples, 2);
    apply(&mut app, "gxmultisample", "0");
    assert_eq!(res::<MsaaSetting>(&app).samples, *MSAA_RANGE.start());
    apply(&mut app, "renderScale", "0.75");
    assert_eq!(res::<RenderScale>(&app).0, 0.75);
    apply(&mut app, "renderscale", "9");
    assert_eq!(res::<RenderScale>(&app).0, *RENDER_SCALE_RANGE.end());
    apply(&mut app, "renderscale", "0");
    assert_eq!(res::<RenderScale>(&app).0, *RENDER_SCALE_RANGE.start());
    assert!(!res::<crate::perf::FpsJournalSetting>(&app).0);
    apply(&mut app, "fpsJournal", "1");
    assert!(res::<crate::perf::FpsJournalSetting>(&app).0);
    apply(&mut app, "fpsjournal", "0");
    assert!(!res::<crate::perf::FpsJournalSetting>(&app).0);
    // Enable flags: any nonzero is on, zero is off (the client's int-parse + != 0).
    apply(&mut app, "Sound_EnableMusic", "0");
    assert!(!res::<SoundConfig>(&app).music_enabled);
    apply(&mut app, "sound_enableallsound", "1");
    assert!(res::<SoundConfig>(&app).enabled);
    apply(&mut app, "deselectonclick", "0");
    assert!(!res::<ClickConfig>(&app).deselect_on_click);
    apply(&mut app, "MouseInvertPitch", "1");
    assert!(res::<LookConfig>(&app).invert_pitch);
    apply(&mut app, "mousespeed", "1.4");
    assert_eq!(res::<LookConfig>(&app).sensitivity, 1.4);
    apply(&mut app, "mousespeed", "9");
    assert_eq!(res::<LookConfig>(&app).sensitivity, 1.5);
    // The engine's enum (0 Never, 1 Smart, 2 Always); 3, the validator's upper bound, is Never.
    apply(&mut app, "cameraSmoothStyle", "0");
    assert_eq!(res::<FollowConfig>(&app).style, FollowStyle::Never);
    apply(&mut app, "camerasmoothstyle", "2");
    assert_eq!(res::<FollowConfig>(&app).style, FollowStyle::Always);
    apply(&mut app, "cameraSmoothStyle", "3");
    assert_eq!(res::<FollowConfig>(&app).style, FollowStyle::Never);
    apply(&mut app, "cameraSmoothStyle", "1");
    assert_eq!(res::<FollowConfig>(&app).style, FollowStyle::Smart);
    apply(&mut app, "cameraSmoothTrackingStyle", "2");
    assert_eq!(
        res::<FollowConfig>(&app).tracking_style,
        FollowStyle::Always
    );
    assert_eq!(
        res::<FollowConfig>(&app).style,
        FollowStyle::Smart,
        "and only that one"
    );
    apply(&mut app, "cameraYawSmoothSpeed", "270");
    assert_eq!(res::<FollowConfig>(&app).yaw_speed, 270.0);
    apply(&mut app, "cameraYawSmoothSpeed", "9000");
    assert_eq!(
        res::<FollowConfig>(&app).yaw_speed,
        *FOLLOW_SPEED_RANGE.end()
    );
    // The max orbit lands as yards on the knob: `cameraDistanceMax` x the factor, held to
    // [0, 50]; the factor is not held to its slider's 1 to 2.
    apply(&mut app, "cameraDistanceMaxFactor", "1");
    assert_eq!(res::<ZoomLimit>(&app).max, 15.0);
    apply(&mut app, "cameradistancemaxfactor", "3");
    assert_eq!(res::<ZoomLimit>(&app).max, 45.0);
    apply(&mut app, "cameraDistanceMaxFactor", "1");
    apply(&mut app, "cameraDistanceMax", "25");
    assert_eq!(res::<ZoomLimit>(&app).max, 25.0);
    apply(&mut app, "cameraDistanceMaxFactor", "3");
    assert_eq!(res::<ZoomLimit>(&app).max, 50.0);
    apply(&mut app, "cameraDistanceMaxFactor", "-1");
    assert_eq!(res::<ZoomLimit>(&app).max, 0.0);
    apply(&mut app, "cameraDistanceMaxFactor", "nan");
    assert_eq!(res::<ZoomLimit>(&app).max, 50.0);
    // Out of `cameraDistanceMax`'s validator range: refused at the knob, 25 stands.
    apply(&mut app, "cameraDistanceMaxFactor", "1");
    apply(&mut app, "cameraDistanceMax", "60");
    assert_eq!(res::<ZoomLimit>(&app).max, 25.0);
    apply(&mut app, "autoLootDefault", "1");
    assert!(res::<LootConfig>(&app).auto_loot);
    apply(&mut app, "showLootSpam", "0");
    assert!(!res::<LootConfig>(&app).show_loot_spam);
    apply(&mut app, "guildMemberNotify", "1");
    assert!(res::<crate::guild::GuildMemberNotify>(&app).0);
    apply(&mut app, "BlockTrades", "1");
    assert!(res::<crate::ui_trade::BlockTrades>(&app).0);
    use crate::nameplates::name_bit;
    apply(&mut app, "UnitNameNPC", "0");
    assert!(!res::<NameConfig>(&app).on(name_bit::NPC));
    apply(&mut app, "unitnameown", "1");
    assert!(res::<NameConfig>(&app).on(name_bit::OWN));
    apply(&mut app, "UnitNameEnemyPlayerName", "0");
    assert!(!res::<NameConfig>(&app).on(name_bit::ENEMY_PLAYER));
    assert!(
        res::<NameConfig>(&app).on(name_bit::FRIENDLY_PLAYER),
        "one bit at a time"
    );
    apply(&mut app, "nameplateShowEnemies", "1");
    assert!(res::<crate::vplates::VPlateMode>(&app).enemies);
    apply(&mut app, "nameplateshowfriendlypets", "0");
    assert!(!res::<crate::vplates::VPlateMode>(&app).friendly_pets);
    apply(&mut app, "ChatBubbles", "0");
    assert!(!res::<BubbleConfig>(&app).all);
    apply(&mut app, "chatbubblesparty", "0");
    assert!(!res::<BubbleConfig>(&app).party);
    // WorldDetail: stop 0/1/2 is density ×1/×2/×3, clamped to the slider's range.
    apply(&mut app, "WorldDetail", "0");
    assert_eq!(res::<ClutterConfig>(&app).density, 1.0);
    apply(&mut app, "worlddetail", "7");
    assert_eq!(res::<ClutterConfig>(&app).density, 3.0);
    // `frillDensity` is the same field in cells per chunk, with its own clamp `[1, 256]`
    // (`0x688de0`); the stops round-trip through both spellings.
    apply(&mut app, "frillDensity", "48");
    assert_eq!(res::<ClutterConfig>(&app).density, 3.0);
    apply(&mut app, "frilldensity", "16");
    assert_eq!(res::<ClutterConfig>(&app).density, 1.0);
    // Past the top stop is honoured; pfUI's `hdgraphic` writes up to 256.
    apply(&mut app, "frillDensity", "256");
    assert_eq!(res::<ClutterConfig>(&app).density, 16.0);
    // `0` is not clutter-off: the callback pins it to 1.
    apply(&mut app, "frillDensity", "9000");
    assert_eq!(res::<ClutterConfig>(&app).density, 16.0);
    apply(&mut app, "frillDensity", "0");
    assert_eq!(res::<ClutterConfig>(&app).density, 1.0 / 16.0);
    apply(&mut app, "WorldDetail", "1");
    assert_eq!(res::<ClutterConfig>(&app).density, 2.0);
    for (wrote, want) in [
        ("0", 0u8),
        ("1", 1),
        ("2", 2),
        ("3", 3),
        ("2.9", 2),
        ("9", 3),
        ("-4", 0),
    ] {
        apply(&mut app, "weatherDensity", wrote);
        assert_eq!(
            res::<wrathplus_world::weather::WeatherState>(&app).weather_density,
            want,
            "weatherDensity {wrote}"
        );
    }
    // `gamma` is the ramp exponent: `SetGamma` applies `1 - v` before it arrives here. The
    // consumer clamps what the reference accepts unclamped (`SetGamma(5)` writes -4 there).
    for (wrote, want) in [("1.000000", 1.0), ("0.500000", 0.5), ("1.500000", 1.5)] {
        apply(&mut app, "gamma", wrote);
        assert_eq!(
            res::<crate::ui_gamma::DisplayGamma>(&app).0,
            want,
            "gamma {wrote}"
        );
    }
    apply(&mut app, "gamma", "-4.000000");
    assert_eq!(
        res::<crate::ui_gamma::DisplayGamma>(&app).0,
        *crate::ui_gamma::GAMMA_RANGE.start(),
        "a negative exponent clamps at the consumer, where it cannot blank the screen"
    );
    apply(&mut app, "gamma", "99");
    assert_eq!(
        res::<crate::ui_gamma::DisplayGamma>(&app).0,
        *crate::ui_gamma::GAMMA_RANGE.end()
    );
    assert_eq!(res::<ClutterConfig>(&app).frill_density(), 32.0);
    // Both spellings reach the field from a state neither holds; `$WOW_CLUTTER_DENSITY` takes
    // both for the session. Two values, since writing the sibling's mirrored value is a no-op.
    for (key, value) in [
        (wrathplus_ui::script::CVAR_WORLD_DETAIL, "2"),
        (wrathplus_ui::script::CVAR_FRILL_DENSITY, "16"),
    ] {
        app.world_mut().resource_mut::<ClutterConfig>().density = 0.5;
        assert_eq!(apply(&mut app, key, value), SetOutcome::Changed);
        assert_ne!(
            res::<ClutterConfig>(&app).density,
            0.5,
            "{key}: reached no knob"
        );
    }
    // `wrathplus-ui`'s `SetWorldDetail`/`GetWorldDetail` name these CVars by const and cannot see
    // this table, so both must stay registered and the stops must stay `frillDensity`'s unit
    // times n.
    assert!(crate::shipped_cvars::rows().any(|r| r.name == wrathplus_ui::script::CVAR_WORLD_DETAIL));
    assert!(
        crate::shipped_cvars::rows().any(|r| r.name == wrathplus_ui::script::CVAR_FRILL_DENSITY)
    );
    // The same for `GetGamma`/`SetGamma`, which read and write `1 - gamma`.
    assert!(crate::shipped_cvars::rows().any(|r| r.name == wrathplus_ui::script::CVAR_GAMMA));
    // `RestoreVideoDefaults`' row list lives in `wrathplus-ui` too; an unregistered row would be
    // skipped silently.
    for key in wrathplus_ui::script::VIDEO_DEFAULT_CVARS {
        assert!(
            crate::shipped_cvars::rows().any(|r| r.name.eq_ignore_ascii_case(key)),
            "{key}: RestoreVideoDefaults would restore it, and nothing registers it"
        );
    }
    // The video pairs and `SetWorldDetail`'s `smallCull` write by name; a missing row would
    // warn and store nothing.
    for key in wrathplus_ui::script::VIDEO_PAIR_CVARS
        .iter()
        .chain([&wrathplus_ui::script::CVAR_SMALL_CULL])
    {
        assert!(
            crate::shipped_cvars::rows().any(|r| r.name.eq_ignore_ascii_case(key)),
            "{key}: a video verb writes it, and nothing registers it"
        );
    }
    for (n, frill) in wrathplus_ui::script::WORLD_DETAIL_STOPS.iter().enumerate() {
        assert_eq!(
            *frill,
            wrathplus_formats::FRILL_DENSITY * (n as u32 + 1),
            "stop {n}: the reference's own 0x804518 entry must be this knob's unit times the stop"
        );
        apply(&mut app, "WorldDetail", &n.to_string());
        let by_stop = res::<ClutterConfig>(&app).density;
        // Off the stop first (the mirror already wrote this spelling), then the stop's cells.
        apply(&mut app, "frillDensity", "1");
        app.world_mut().resource_mut::<ClutterConfig>().density = 0.5;
        apply(&mut app, "frillDensity", &frill.to_string());
        assert_eq!(
            res::<ClutterConfig>(&app).density,
            by_stop,
            "stop {n}: the two spellings disagree"
        );
        assert_eq!(res::<ClutterConfig>(&app).frill_density(), *frill as f32);
    }
    apply(&mut app, "WorldDetail", "1");
    assert_eq!(res::<ClutterConfig>(&app).density, 2.0);
    // A stop set as a CVar keeps `SmallCull` in step, as `SetWorldDetail` writes it.
    apply(&mut app, "WorldDetail", "0");
    assert_eq!(
        res::<Cvars>(&app).get(wrathplus_ui::script::CVAR_SMALL_CULL),
        Some("0.070000")
    );
    apply(&mut app, "WorldDetail", "1");
    assert_eq!(
        res::<Cvars>(&app).get(wrathplus_ui::script::CVAR_SMALL_CULL),
        Some("0.040000")
    );
    // Spell Detail: `SStrToInt`, clamp, then the shared emission scalar (`0x689510`).
    for (level, scale) in [
        ("0", 0.33),
        ("1", 0.66),
        ("2", 1.0),
        ("-3", 0.33),
        ("7", 1.0),
    ] {
        apply(&mut app, "spellEffectLevel", level);
        assert_eq!(
            res::<wrathplus_world::particles::ParticleTuning>(&app).density(),
            scale,
            "spellEffectLevel {level}"
        );
        assert_eq!(
            res::<crate::video::SpellEffectLevel>(&app).0,
            level.parse::<i32>().unwrap(),
            "the record's integer, unclamped, for the shard emitter"
        );
    }
    apply(&mut app, "minimapZoom", "5");
    assert_eq!(res::<MinimapZoom>(&app).outdoor, 5);
    assert_eq!(
        res::<MinimapZoom>(&app).inside,
        3,
        "the two indices are independent"
    );
    apply(&mut app, "minimapinsidezoom", "9");
    assert_eq!(res::<MinimapZoom>(&app).inside, MINIMAP_ZOOM_LEVELS - 1);
    apply(&mut app, "minimapZoom", "-2");
    assert_eq!(res::<MinimapZoom>(&app).outdoor, 0);
    assert_eq!(apply(&mut app, "uiScale", "banana"), SetOutcome::Refused);
    assert_eq!(res::<UiScaleCvar>(&app).0, 0.9);
    assert_eq!(apply(&mut app, "bogus", "1"), SetOutcome::Unknown);
}

/// `/console spellEffectLevel 0` prints the handler's own line (`0x689537`-`0x689554`, format
/// `0x869f94`) with the level it clamped to; a Lua write logs it instead.
#[test]
fn the_spell_effect_level_handler_echoes_to_the_console() {
    let mut app = cvar_app();
    app.init_resource::<crate::app_core::state::ConsoleEcho>();
    assert_eq!(
        crate::console::execute(app.world_mut(), "spellEffectLevel 0"),
        ["Spell effect level set to 0."]
    );
    assert_eq!(
        crate::console::execute(app.world_mut(), "spellEffectLevel 9"),
        ["Spell effect level set to 2."]
    );
    assert_eq!(res::<Cvars>(&app).get("spellEffectLevel"), Some("9"));
    assert_eq!(
        crate::console::execute(app.world_mut(), "farclip 400"),
        Vec::<String>::new(),
        "a callback that prints nothing adds nothing"
    );
}

#[test]
fn compose_writes_the_diff_and_preserves_what_it_does_not_own() {
    let mut cvars = Cvars::with_rows(crate::shipped_cvars::FEATURE_ROWS);
    cvars.own_for_session("uiScale", Some("1.2")); // env-overridden this session
    cvars.load_file(
        [
            ("FutureKnob".to_string(), "3".to_string()), // a newer build's key: preserved
            ("uiScale".to_string(), "0.8".to_string()),  // the file's own, kept as found
            ("farclip".to_string(), "400".to_string()),  // will return to default
        ]
        .into(),
    );
    cvars.set("Sound_MusicVolume", "0.7"); // moved: written
    cvars.set("farclip", "350"); // back to default: removed
    let out = cvars.compose();
    assert_eq!(
        out.get("Sound_MusicVolume").map(String::as_str),
        Some("0.7")
    );
    assert!(
        !out.contains_key("Sound_MasterVolume"),
        "at default: absent"
    );
    assert_eq!(out.get("uiScale").map(String::as_str), Some("0.8"));
    assert!(!out.contains_key("farclip"));
    assert_eq!(out.get("FutureKnob").map(String::as_str), Some("3"));
}

#[test]
fn a_change_saves_after_one_quiet_second_with_no_exit() {
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-cvar-quiet-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _u = EnvGuard::unset("WOW_UI_SCALE");
    let _f = EnvGuard::unset("WOW_FARCLIP");
    let _d = EnvGuard::unset("WOW_CLUTTER_DENSITY");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());

    let mut app = cvar_app();
    app.update();
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(r#"SetCVar("Sound_MusicVolume", 0.75)"#)
        .unwrap();
    app.update();
    assert!(
        !tmp.join("config.toml").exists(),
        "inside the quiet second nothing is written"
    );

    // The change ages past the quiet second; no `AppExit` anywhere.
    {
        let mut cvars = app.world_mut().resource_mut::<Cvars>();
        assert!(cvars.dirty);
        cvars.last_change = Instant::now().checked_sub(SAVE_QUIET * 2);
    }
    app.update();
    let text = std::fs::read_to_string(tmp.join("config.toml")).unwrap();
    assert!(text.contains("MusicVolume = \"0.75\""), "{text}");
    assert!(!app.world().resource::<Cvars>().dirty);
    assert!(app.should_exit().is_none());
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn a_lua_setcvar_lands_in_config_toml_end_to_end() {
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-cvar-e2e-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _u = EnvGuard::unset("WOW_UI_SCALE");
    let _f = EnvGuard::unset("WOW_FARCLIP");
    let _d = EnvGuard::unset("WOW_CLUTTER_DENSITY");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());
    crate::local_state::write_atomic(&tmp.join("config.toml"), "[cvars]\nMusicVolume = \"0.1\"\n")
        .unwrap();

    let mut app = cvar_app();

    // Startup: the file's MusicVolume reaches the knob; Update: the VM table seeds from it.
    app.update();
    assert_eq!(app.world().resource::<SoundConfig>().music, 0.1);
    assert_eq!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .cvar("Sound_MusicVolume")
            .as_deref(),
        Some("0.1")
    );

    // The Lua write (what a settings slider will do) reaches the knob on the next frame…
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(r#"SetCVar("Sound_MusicVolume", 0.75)"#)
        .unwrap();
    app.update();
    assert_eq!(app.world().resource::<SoundConfig>().music, 0.75);

    // …and the exit flush writes the diff: the moved value, nothing at its default.
    app.world_mut().write_message(AppExit::Success);
    app.update();
    let text = std::fs::read_to_string(tmp.join("config.toml")).unwrap();
    assert!(text.contains("MusicVolume = \"0.75\""), "{text}");
    assert!(
        !text.contains("Sound_MasterVolume"),
        "defaults stay out:\n{text}"
    );
    let back: LocalConfig = toml::from_str(&text).unwrap();
    assert_eq!(back.cvars.len(), 1, "a diff, not a dump: {text}");
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn wrath_cvar_mirror_tracks_host_sync_and_vm_replacement() {
    let mut app = cvar_app();
    app.world_mut()
        .insert_non_send_resource(UiScript::new().unwrap());
    app.update();
    app.world_mut().resource_mut::<Cvars>().set("gamma", "1.2");
    app.update();
    let check = |app: &mut App| {
        let mut script = app.world_mut().non_send_resource_mut::<UiScript>();
        assert_eq!(
            script.eval::<String>("return GetCVar('GAMMA')").unwrap(),
            "1.2"
        );
        assert!(script.take_cvar_changes().is_empty());
        assert!(script.take_cvar_registrations().is_empty());
    };
    check(&mut app);
    assert_eq!(res::<crate::ui_gamma::DisplayGamma>(&app).0, 1.2);
    app.world_mut()
        .insert_non_send_resource(UiScript::new().unwrap());
    app.update();
    check(&mut app);
}

/// A client with a real CVar host: every knob resource an observer writes, every observer,
/// [`CvarPlugin`] and a VM for the mirror.
pub(super) fn cvar_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::MinimalPlugins)
        .insert_resource(SoundConfig::default())
        .insert_resource(UiScaleCvar(DEFAULT_UI_SCALE))
        .insert_resource(ViewDistance {
            farclip: 350.0,
            nearclip: wrathplus_world::view::NEARCLIP_DEFAULT,
        })
        .insert_resource(MsaaSetting { samples: 1 })
        // Literal: `TexFilterSetting::default()` reads `$WOW_TRILINEAR`/`$WOW_ANISO`.
        .insert_resource(wrathplus_assets::TexFilterSetting {
            trilinear: true,
            aniso: 1,
        })
        // A real-shaped device menu: `GetCurrentMultisampleFormat` needs rows to find.
        .insert_resource(wrathplus_world::view::MsaaFormats {
            formats: vec![(32, 32, 1), (32, 32, 2), (32, 32, 4)],
        })
        .init_resource::<LookConfig>()
        .init_resource::<crate::player::camera_dynamics::CameraOptions>()
        .init_resource::<crate::combat_text::DamageTextGates>()
        .init_resource::<crate::ui_chat::combat::LogPeriodicSpells>()
        .init_resource::<wrathplus_world::weather::WeatherState>()
        .init_resource::<wrathplus_world::particles::ParticleTuning>()
        .init_resource::<crate::video::SpellEffectLevel>()
        .init_resource::<crate::ui_gamma::DisplayGamma>()
        .init_resource::<ClickConfig>()
        .init_resource::<crate::target::AssistAttack>()
        .init_resource::<LootConfig>()
        .init_resource::<NameConfig>()
        .init_resource::<crate::vplates::VPlateMode>()
        .init_resource::<ClutterConfig>()
        .init_resource::<MinimapZoom>()
        .init_resource::<BubbleConfig>()
        .init_resource::<ZoomLimit>()
        .init_resource::<FollowConfig>()
        .init_resource::<VideoConfig>()
        .init_resource::<wrathplus_world::ffx_glow::FfxSwitches>()
        // Literal, not Default: RenderScale::default() reads $WOW_RENDER_SCALE.
        .insert_resource(RenderScale(1.0))
        // Literal: `Realmlist::default()` reads `$WOW_HOST`.
        .insert_resource(crate::realmlist::Realmlist::at(
            crate::realmlist::DEFAULT_REALMLIST,
        ))
        .init_resource::<PaneRate>()
        .init_resource::<crate::guild::GuildMemberNotify>()
        .init_resource::<crate::ui_trade::BlockTrades>()
        .init_resource::<crate::spell::AutoSelfCast>()
        .init_resource::<crate::perf::FpsJournalSetting>()
        .init_resource::<crate::text_filter::TextFilterSwitches>()
        .init_resource::<crate::game_tip::GameTipSetting>()
        .add_plugins(CvarPlugin);
    // No feature plugin here, so their rows join by hand, as each plugin would register them.
    for rows in crate::shipped_cvars::FEATURE_ROWS {
        app.register_cvars(rows);
    }
    for observer in ALL_OBSERVERS {
        observer(&mut app);
    }
    app.insert_non_send_resource(UiScript::new().unwrap());
    app
}

/// Every CVar observer in the crate; a new one is a line here and an `add_observer` in its
/// plugin.
const ALL_OBSERVERS: &[fn(&mut App)] = &[
    |app| {
        app.add_observer(crate::sound::on_cvar);
    },
    |app| {
        app.add_observer(crate::ui_script::on_cvar);
    },
    |app| {
        app.add_observer(crate::video::on_cvar);
    },
    |app| {
        app.add_observer(crate::player::camera::on_cvar);
    },
    |app| {
        app.add_observer(crate::player::camera_dynamics::on_cvar);
    },
    |app| {
        app.add_observer(crate::target::on_cvar);
    },
    |app| {
        app.add_observer(crate::spell::cast_target::on_cvar);
    },
    |app| {
        app.add_observer(crate::combat_text::on_cvar);
    },
    |app| {
        app.add_observer(crate::ui_chat::combat::on_cvar);
    },
    |app| {
        app.add_observer(crate::ui_loot::on_cvar);
    },
    |app| {
        app.add_observer(crate::nameplates::on_cvar);
    },
    |app| {
        app.add_observer(crate::vplates::on_cvar);
    },
    |app| {
        app.add_observer(crate::game_tip::on_cvar);
    },
    |app| {
        app.add_observer(crate::text_filter::on_cvar);
    },
    |app| {
        app.add_observer(crate::chat_bubble::on_cvar);
    },
    |app| {
        app.add_observer(crate::ui_guild::on_cvar);
    },
    |app| {
        app.add_observer(crate::ui_trade::on_cvar);
    },
    |app| {
        app.add_observer(crate::minimap::on_cvar);
    },
    |app| {
        app.add_observer(crate::portrait::on_cvar);
    },
    |app| {
        app.add_observer(crate::perf::on_cvar);
    },
    |app| {
        app.add_observer(crate::realmlist::on_cvar);
    },
    |app| {
        app.add_observer(crate::ui_gamma::on_cvar);
    },
    |app| {
        app.add_observer(crate::world_backdrop::on_cvar);
    },
];

/// A host write, its latch committed at once, and its observers run before this returns.
pub(super) fn apply(app: &mut App, name: &str, value: &str) -> SetOutcome {
    let world = app.world_mut();
    let (outcome, events) = {
        let mut cvars = world.resource_mut::<Cvars>();
        let outcome = cvars.set(name, value);
        cvars.commit_latched();
        (outcome, cvars.take_events())
    };
    for event in events {
        world.trigger(event);
    }
    outcome
}

pub(super) fn res<T: Resource>(app: &App) -> &T {
    app.world().resource::<T>()
}

/// The last character entered survives a quit: two launches over one `wrathplus-config/` with the
/// real [`CvarPlugin`] and [`crate::char_select`] systems, so the screen's host write must
/// reach the file through the registry's dirty/compose path.
#[test]
fn entering_the_world_survives_the_quit_and_comes_back_selected() {
    use crate::app_core::state::ClientState;
    use crate::app_core::state::Roster;
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-lastchar-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _u = EnvGuard::unset("WOW_UI_SCALE");
    let _f = EnvGuard::unset("WOW_FARCLIP");
    let _d = EnvGuard::unset("WOW_CLUTTER_DENSITY");
    let _w = EnvGuard::unset("WOW_CHAR");
    let _s = EnvGuard::unset("WOW_CHARSELECT_PICK");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());
    let roster = || {
        (1..=4)
            .map(|g| crate::char_select::test_character(g, &format!("Char{g}")))
            .collect::<Vec<_>>()
    };

    // ── Launch 1: the roster lands, and the player enters the world as the third row. ────
    let (tx, _rx) = crossbeam_channel::unbounded();
    let mut app = cvar_app();
    app.add_plugins(bevy::state::app::StatesPlugin);
    crate::char_select::add_test_systems(&mut app, tx);
    app.update(); // Startup loads the (absent) file; the first Update seeds the VM table
    app.world_mut().write_message(crate::net::CharListMessage {
        characters: roster(),
        realm: None,
    });
    app.update();
    assert_eq!(
        app.world().resource::<Roster>().selected(),
        Some(0),
        "nothing remembered yet, so the first row — the behaviour that was already right",
    );
    app.world_mut().resource_mut::<Roster>().pending_pick = Some(3); // guid 3 = row 2
    app.update();
    app.world_mut().write_message(AppExit::Success);
    app.update();

    let text = std::fs::read_to_string(tmp.join("config.toml")).unwrap();
    assert!(
        text.contains("lastCharacterIndex = \"2\""),
        "entering the world must reach the file, 0-based like Config.wtf:\n{text}"
    );

    // ── Launch 2: a fresh client over the same folder, and the roster arrives. ───────────
    let (tx, _rx2) = crossbeam_channel::unbounded();
    let mut app = cvar_app();
    app.add_plugins(bevy::state::app::StatesPlugin);
    crate::char_select::add_test_systems(&mut app, tx);
    app.update();
    app.world_mut().write_message(crate::net::CharListMessage {
        characters: roster(),
        realm: None,
    });
    app.update();

    assert_eq!(
        app.world().resource::<Roster>().selected(),
        Some(2),
        "the second launch must stand the SAME character on the stage — the whole report",
    );
    assert_eq!(
        *app.world().resource::<State<ClientState>>().get(),
        ClientState::CharSelect,
    );
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn a_minimap_setzoom_reaches_the_knob_and_the_file() {
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-mmzoom-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _u = EnvGuard::unset("WOW_UI_SCALE");
    let _f = EnvGuard::unset("WOW_FARCLIP");
    let _d = EnvGuard::unset("WOW_CLUTTER_DENSITY");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());
    // The previous session left the outdoor map zoomed right in.
    crate::local_state::write_atomic(&tmp.join("config.toml"), "[cvars]\nminimapZoom = \"5\"\n")
        .unwrap();

    let mut app = cvar_app();
    app.update();

    assert_eq!(app.world().resource::<MinimapZoom>().outdoor, 5);
    assert_eq!(app.world().resource::<MinimapZoom>().inside, 3);
    let seed = {
        let z = app.world().resource::<MinimapZoom>();
        (z.outdoor, z.inside)
    };
    {
        let mut script = app.world_mut().non_send_resource_mut::<UiScript>();
        assert_eq!(script.cvar("minimapZoom").as_deref(), Some("5"));
        // The widget is born first, then the persisted level is pushed into it: seeding a
        // widget that does not exist is a no-op.
        script.run(r#"m = CreateFrame("Minimap", "Mini")"#).unwrap();
        script.set_minimap_zoom(seed.0, seed.1);
        assert_eq!(script.eval::<u8>("return m:GetZoom()").unwrap(), 5);
        script.run("m:SetZoom(m:GetZoom() - 2)").unwrap();
    }
    app.update();
    assert_eq!(app.world().resource::<MinimapZoom>().outdoor, 3);

    app.world_mut().write_message(AppExit::Success);
    app.update();
    let text = std::fs::read_to_string(tmp.join("config.toml")).unwrap();
    assert!(
        !text.contains("minimapZoom"),
        "back at the registered default 3, so it leaves the diff entirely:\n{text}"
    );

    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("m:SetZoom(1)")
        .unwrap();
    app.update();
    app.world_mut().write_message(AppExit::Success);
    app.update();
    let text = std::fs::read_to_string(tmp.join("config.toml")).unwrap();
    assert!(text.contains("minimapZoom = \"1\""), "{text}");
    assert_eq!(app.world().resource::<MinimapZoom>().outdoor, 1);
    std::fs::remove_dir_all(&tmp).ok();
}

/// The string-valued rows, a closed list, each default checked on its own terms: `realmName`
/// (the reference's registered `""`, `0x882748`) and `gxApi` are empty until the session writes
/// them, and the stat panes' `playerStat*Dropdown` (`""`) until the sheet's per-class fill;
/// `gxResolution` and `realmList` pass their observers' parsers
/// ([`crate::video::parse_resolution`], [`crate::realmlist::normalize`]).
#[test]
fn the_string_valued_cvars_are_the_realm_and_the_windowed_size() {
    let mut strings: Vec<&str> = crate::shipped_cvars::rows()
        .filter(|r| r.default.parse::<f32>().is_err())
        .map(|r| r.name)
        .collect();
    strings.sort_unstable(); // the list is the claim, not where the rows sit in the table
    assert_eq!(
        strings.join(" "),
        "gxApi gxResolution lfdCollapsedHeaders lfdSelectedDungeons playerStatLeftDropdown playerStatRightDropdown realmList realmName"
    );
    let default_of = |name: &str| {
        crate::shipped_cvars::rows()
            .find(|r| r.name == name)
            .map(|r| r.default)
            .expect("registered")
    };
    assert_eq!(default_of("realmName"), "");
    assert_eq!(default_of("gxApi"), "");
    assert_eq!(
        crate::video::parse_resolution(default_of("gxResolution")),
        Some(crate::video::DEFAULT_WINDOWED)
    );
    // A default `realmlist::normalize` rejects would ship a client that cannot dial.
    assert_eq!(
        crate::realmlist::normalize(default_of(crate::realmlist::CVAR_REALMLIST)).as_deref(),
        Some(crate::realmlist::DEFAULT_REALMLIST),
    );
}

/// A registry loaded from `file`, with no disk.
fn registry(file: &[(&str, &str)]) -> Cvars {
    let mut cvars = Cvars::with_rows(crate::shipped_cvars::FEATURE_ROWS);
    cvars.load_file(
        file.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    );
    cvars
}

/// A latched row's write is staged: the applied value stands and nothing fires or dirties (the
/// reference's `Set 0x63df50` stores `latchedValue` and skips `InternalSet`) until the commit
/// (`0x63e060`) applies, fires, persists and mirrors it.
#[test]
fn a_latched_row_stages_the_write_until_the_boundary_commits_it() {
    let mut cvars = Cvars::with_rows(crate::shipped_cvars::FEATURE_ROWS);
    assert!(
        cvars.row("gxVSync").unwrap().latched,
        "the reference's flags=3 row"
    );
    assert_eq!(cvars.set("gxVSync", "0"), SetOutcome::Staged);
    assert_eq!(cvars.get("gxVSync"), Some("1"), "applied value stands");
    assert_eq!(cvars.row("gxVSync").unwrap().pending.as_deref(), Some("0"));
    assert!(!cvars.has_events(), "nothing fires before the boundary");
    assert!(!cvars.dirty, "nothing to save before the boundary");
    assert_eq!(
        cvars.set("gxVSync", "0"),
        SetOutcome::Unchanged,
        "same stage"
    );
    assert!(
        !cvars.compose().contains_key("gxVSync"),
        "a stage that is never committed never reaches the file"
    );
    assert_eq!(cvars.commit_latched(), 1);
    assert_eq!(cvars.get("gxVSync"), Some("0"));
    assert_eq!(cvars.row("gxVSync").unwrap().pending, None);
    assert_eq!(
        cvars.take_events(),
        vec![CvarChanged {
            name: "gxVSync".into(),
            old: "1".into(),
            new: "0".into()
        }]
    );
    assert!(cvars.dirty);
    assert_eq!(
        cvars.compose().get("gxVSync").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        cvars.take_outbox(),
        vec![("gxVSync".to_string(), "0".to_string())]
    );
    cvars.set("gxVSync", "1");
    assert_eq!(cvars.set("gxVSync", "0"), SetOutcome::Unchanged);
    assert_eq!(cvars.row("gxVSync").unwrap().pending, None);
    assert_eq!(cvars.commit_latched(), 0);
    // A latched row outside the gx set is not the video restart's to commit.
    assert_eq!(cvars.set("SoundBufferSize", "200"), SetOutcome::Staged);
    assert_eq!(cvars.commit_latched(), 0);
    assert_eq!(cvars.get("SoundBufferSize"), Some("100"));
    assert_eq!(
        cvars.row("SoundBufferSize").unwrap().pending.as_deref(),
        Some("200")
    );
    assert!(!cvars.compose().contains_key("SoundBufferSize"));
    assert_eq!(cvars.set("Sound_MusicVolume", "0.7"), SetOutcome::Changed);
    assert_eq!(cvars.get("Sound_MusicVolume"), Some("0.7"));
    assert_eq!(cvars.take_events().len(), 1);
}

#[test]
fn a_numeric_row_refuses_what_does_not_parse_and_corrects_the_mirror() {
    let mut cvars = Cvars::with_rows(crate::shipped_cvars::FEATURE_ROWS);
    assert_eq!(cvars.set_from_vm("uiScale", "banana"), SetOutcome::Refused);
    assert_eq!(cvars.get("uiScale"), Some("0.9"));
    assert!(!cvars.has_events());
    assert!(!cvars.dirty);
    assert_eq!(
        cvars.take_outbox(),
        vec![("uiScale".to_string(), "0.9".to_string())],
        "the VM stored 'banana' synchronously; the host writes the truth back"
    );
    assert_eq!(cvars.set("uiScale", "banana"), SetOutcome::Refused);
    assert!(
        cvars.take_outbox().is_empty(),
        "a host write has no mirror to correct"
    );
    assert_eq!(
        cvars.set("realmList", "not an address"),
        SetOutcome::Changed
    );
    assert_eq!(cvars.set("nosuchrow", "1"), SetOutcome::Unknown);
}

#[test]
fn an_addon_row_persists_like_the_clients_own() {
    let mut cvars = registry(&[("myAddonKnob", "3")]);
    assert_eq!(
        cvars.orphans(),
        vec![("myAddonKnob".to_string(), "3".to_string())],
        "unclaimed until the addon declares it — the VM's saved base"
    );
    cvars.learn_addon_row("myAddonKnob", "1");
    assert_eq!(
        cvars.get("myAddonKnob"),
        Some("3"),
        "starts at the saved value"
    );
    assert_eq!(cvars.default_of("myAddonKnob"), Some("1"));
    assert!(cvars.orphans().is_empty());
    cvars.learn_addon_row("myAddonKnob", "9");
    assert_eq!(
        cvars.default_of("myAddonKnob"),
        Some("1"),
        "a re-declaration is a no-op"
    );
    assert!(!cvars.dirty);
    assert_eq!(cvars.set_from_vm("myAddonKnob", "5"), SetOutcome::Changed);
    assert!(cvars.dirty, "an addon-only change is a change");
    assert_eq!(
        cvars.compose().get("myAddonKnob").map(String::as_str),
        Some("5")
    );
    cvars.set_from_vm("myAddonKnob", "1");
    assert!(
        !cvars.compose().contains_key("myAddonKnob"),
        "back at the addon's default, it leaves the diff"
    );
    let cvars = registry(&[("FutureKnob", "3")]);
    assert_eq!(
        cvars.compose().get("FutureKnob").map(String::as_str),
        Some("3")
    );
}

#[test]
fn a_session_owned_row_answers_the_env_and_never_reaches_the_file() {
    let mut cvars = Cvars::with_rows(crate::shipped_cvars::FEATURE_ROWS);
    cvars.own_for_session("uiScale", Some("1.2"));
    cvars.load_file(BTreeMap::from([("uiScale".to_string(), "0.8".to_string())]));
    assert_eq!(
        cvars.get("uiScale"),
        Some("1.2"),
        "the env's, not the file's"
    );
    assert!(
        !cvars.has_events(),
        "the knob already read the env — nothing to apply"
    );
    assert_eq!(cvars.set("uiScale", "1.4"), SetOutcome::Changed);
    assert_eq!(
        cvars.compose().get("uiScale").map(String::as_str),
        Some("0.8"),
        "the file keeps what it said"
    );
    // A lever with no resource still marks the row, so the file cannot apply over the env.
    cvars.own_for_session("farclip", None);
    cvars.load_file(BTreeMap::from([("farclip".to_string(), "500".to_string())]));
    assert_eq!(cvars.get("farclip"), Some("350"));
    assert!(cvars.is_session_owned("FARCLIP"));
}

/// The camera reads `gxMultisample` once at spawn, so the file must be applied inside
/// [`CvarLoad`].
#[test]
fn the_loaded_file_reaches_a_knob_before_anything_ordered_after_cvar_load() {
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-cvar-load-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _u = EnvGuard::unset("WOW_UI_SCALE");
    let _f = EnvGuard::unset("WOW_FARCLIP");
    let _d = EnvGuard::unset("WOW_CLUTTER_DENSITY");
    let _m = EnvGuard::unset("WOW_MSAA");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());
    crate::local_state::write_atomic(
        &tmp.join("config.toml"),
        "[cvars]\nMusicVolume = \"0.1\"\ngxMultisample = \"4\"\n",
    )
    .unwrap();
    #[derive(Resource, Default)]
    struct SeenAtStartup(Option<(f32, u32)>);
    fn after_load(
        sound: Res<SoundConfig>,
        msaa: Res<MsaaSetting>,
        mut seen: ResMut<SeenAtStartup>,
    ) {
        seen.0 = Some((sound.music, msaa.samples));
    }
    let mut app = cvar_app();
    app.init_resource::<SeenAtStartup>()
        .add_systems(Startup, after_load.after(CvarLoad));
    app.update();
    assert_eq!(
        app.world().resource::<SeenAtStartup>().0,
        Some((0.1, 4)),
        "both the plain row and the latched one are applied by the time CvarLoad is over"
    );
    assert_eq!(
        app.world().resource::<Cvars>().get("gxMultisample"),
        Some("4"),
        "a file value is applied, not staged: the reference's LoadFile runs before Register"
    );
    std::fs::remove_dir_all(&tmp).ok();
}

/// Every registered row has a reader: a string literal naming it in this crate's or
/// `wrathplus-ui`'s code, or a `LUA_ONLY` entry naming its stock reader. Test files do not count.
#[test]
fn every_registered_row_has_a_reader_in_the_source() {
    /// Rows whose only reader is the stock interface.
    const LUA_ONLY: &[(&str, &str)] = &[
        (
            "UberTooltips",
            "GameTooltip's binding-line gate, stock and pfUI",
        ),
        ("gxApi", "pfUI's system tooltip names the backend"),
        (
            "useUiScale",
            "UIOptionsFrame.lua and OptionsFrame.lua branch on it to gate the uiScale slider",
        ),
        (
            "DesktopGamma",
            "OptionsFrame.lua's Use Desktop Gamma box, recorded at open and put back on close",
        ),
        (
            "pixelShaders",
            "OptionsFrame.lua's Enable All Shaders box, which its Okay copies into `ffx`",
        ),
        ("specular", "OptionsFrame.lua's Terrain Highlights box"),
        (
            "M2UseShaders",
            "OptionsFrame.lua's Vertex Animation Shaders box",
        ),
        ("M2UsePixelShaders", "OptionsFrame.lua's Phong Shading box"),
        ("lod", "OptionsFrame.lua's World LOD box"),
        (
            "movieSubtitle",
            "OptionsFrame.lua's Cinematic Subtitles box",
        ),
        (
            "useWeatherShaders",
            "OptionsFrame.lua's Weather Shaders box",
        ),
        ("gxTripleBuffer", "OptionsFrame.lua's Triple Buffering box"),
        ("gxCursor", "OptionsFrame.lua's Hardware Cursor box"),
        ("gxFixLag", "OptionsFrame.lua's Fix Input Lag box"),
        ("gxRefresh", "OptionsFrame.lua's refresh-rate dropdown"),
    ];
    let ui_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("wrathplus-ui")
        .join("src");
    let mut code = String::new();
    // The whole app (every layer crate after the split) but the registry, then the UI crate.
    let app_files = crate::app_core::source_tree::rust_files()
        .into_iter()
        .filter(|(rel, _)| !rel.starts_with("cvars/"))
        .map(|(_, path)| path);
    for file in app_files.chain(crate::test_support::rust_files(&ui_src)) {
        let rel = file.to_string_lossy().replace('\\', "/");
        if rel.contains("tests") {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("source is readable");
        for line in text.lines() {
            if !line.trim_start().starts_with("//") {
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    let mut orphans = Vec::new();
    for row in crate::shipped_cvars::rows() {
        if LUA_ONLY.iter().any(|(n, _)| *n == row.name) {
            continue;
        }
        let exact = format!("\"{}\"", row.name);
        let lower = format!("\"{}\"", row.name.to_ascii_lowercase());
        if !(code.contains(&exact) || code.contains(&lower)) {
            orphans.push(row.name);
        }
    }
    assert!(
        orphans.is_empty(),
        "registered rows nothing in the source reads (an observer arm, a `cvars.get`, or a \
         `LUA_ONLY` entry with its stock reader): {orphans:?}"
    );
    for (name, _) in LUA_ONLY {
        assert!(
            crate::shipped_cvars::rows().any(|r| r.name == *name),
            "{name}: named Lua-only but not registered"
        );
    }
}

/// The rows the reference latches (`flags` 2 or 3 at the register site): the sound-init row and
/// the `gx*` block `RestartGx` commits. `trilinear`, `anisotropic` and `farclip` register with
/// `flags = 1` and apply live.
#[test]
fn the_latched_rows_are_the_references_own() {
    let mut latched: Vec<&str> = crate::shipped_cvars::rows()
        .filter(|r| r.latched)
        .map(|r| r.name)
        .collect();
    latched.sort_unstable();
    assert_eq!(
        latched,
        vec![
            "SoundBufferSize",
            "gxApi",
            "gxColorBits",
            "gxCursor",
            "gxDepthBits",
            "gxFixLag",
            "gxMaximize",
            "gxMultisample",
            "gxRefresh",
            "gxResolution",
            "gxTripleBuffer",
            "gxVSync",
            "gxWindow",
        ]
    );
}

/// The stock 3.3.5 V, Shift-V and Ctrl-V bodies (`Bindings.xml`) write `nameplateShowEnemies` and
/// `nameplateShowFriends` through `SetCVar`, and the plate gate's [`crate::vplates::VPlateMode`]
/// follows through the CVar host. Build 12340 has no plate verbs.
#[test]
fn the_v_keys_move_the_plate_cvars_and_the_plates_follow() {
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
    wrathplus_formats::wow_data_or_skip!();
    let _l = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = std::env::temp_dir().join(format!("wrathplus-cvar-vkeys-{}", std::process::id()));
    std::fs::remove_dir_all(&tmp).ok();
    let _c = EnvGuard::unset("WOW_CAPTURE");
    let _u = EnvGuard::unset("WOW_UI_SCALE");
    let _f = EnvGuard::unset("WOW_FARCLIP");
    let _d = EnvGuard::unset("WOW_CLUTTER_DENSITY");
    let _h = EnvGuard::set("WRATHPLUS_HOME", tmp.to_str().unwrap());

    let mut app = cvar_app();
    app.update();
    crate::ui_script::load_stock_bindings(&mut app.world_mut().non_send_resource_mut::<UiScript>());
    let key = |app: &mut App, command: &str| {
        assert!(app
            .world()
            .non_send_resource::<UiScript>()
            .execute_binding(command, true)
            .unwrap());
        app.update();
    };
    let mode = |app: &App| {
        let m = res::<crate::vplates::VPlateMode>(app);
        (m.enemies, m.friends)
    };

    assert_eq!(mode(&app), (false, false), "both register \"0\"");
    key(&mut app, "NAMEPLATES");
    assert_eq!(mode(&app), (true, false), "V: enemies on");
    key(&mut app, "FRIENDNAMEPLATES");
    assert_eq!(
        mode(&app),
        (false, true),
        "Shift-V: friends on, enemies off"
    );
    key(&mut app, "NAMEPLATES");
    assert_eq!(
        mode(&app),
        (true, false),
        "V with friends showing: enemies on, friends off"
    );
    key(&mut app, "NAMEPLATES");
    assert_eq!(mode(&app), (false, false), "V again: off");
    key(&mut app, "ALLNAMEPLATES");
    assert_eq!(
        mode(&app),
        (true, true),
        "Ctrl-V with none showing: both on"
    );
    key(&mut app, "NAMEPLATES");
    assert_eq!(
        mode(&app),
        (true, false),
        "V with both on turns friends off"
    );
    key(&mut app, "ALLNAMEPLATES");
    assert_eq!(
        mode(&app),
        (false, false),
        "Ctrl-V with any showing: both off"
    );
    std::fs::remove_dir_all(&tmp).ok();
}
