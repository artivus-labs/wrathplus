//! Video settings that reach the window and the presentation path: 1.12's `gxWindow` and
//! `gxVSync` rows (`OptionsFrame.lua:14`, `:9`) and the `gxResolution` windowed size.
//!
//! Deviation: `gxWindow "0"` is borderless fullscreen, not 1.12's exclusive mode-set at
//! `gxResolution`, because Wayland has no client mode-setting (winit ignores
//! `Fullscreen::Exclusive` there), X11's XRandR changes the desktop's mode and a crash leaves it
//! changed, and macOS has no exclusive mode. Bevy's `WindowMode::Fullscreen` also panics on a live
//! change that cannot resolve a monitor (`bevy_winit::winit_windows:91`, `system.rs:333`).
//!
//! The `gx*` rows are latched until `RestartGx()`, as in 1.12; a restart re-asserts them against
//! the window rather than re-creating the device, since wgpu reconfigures the surface live.
//!
//! Uncapped is `AutoNoVsync`, never `Immediate`: on Metal `Immediate` takes ~1 s `nextDrawable`
//! stalls (`capture::probe_uncap_mode`).

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode, WindowResolution};
use wrathplus_ui::script::ScreenResolution;

/// `$WOW_NOVSYNC=1`: uncap for this session only; it never reaches `config.toml` (the
/// `session_owned` set in [`crate::cvars`]).
pub fn novsync_env() -> bool {
    std::env::var("WOW_NOVSYNC").as_deref() == Ok("1")
}

/// Whether this run sizes its own window (a capture, a background run) and so stays windowed.
/// Session-only: `gxWindow`/`gxResolution` are env-overridden while it holds, never saved over.
pub(crate) fn windowed_env() -> bool {
    crate::run_mode::showcase_active()
        || std::env::var_os("WOW_WIN").is_some()
        || std::env::var_os("WOW_CAPTURE").is_some()
        || std::env::var_os("WOW_CAPTURE_UI").is_some()
        || wrathplus_world::bgwin::background_run()
}

/// `$WOW_WIN=WxH` in logical px; the one parser, so the size asked for and the size checked agree.
pub fn requested_window_size() -> Option<UVec2> {
    let v = std::env::var("WOW_WIN").ok()?;
    let (w, h) = v.split_once('x')?;
    Some(UVec2::new(w.parse().ok()?, h.parse().ok()?))
}

/// `$WOW_DPI=<f32>`: render at another display's pixel grid, since text rasterizing and snapping
/// quantize in device pixels. It overrides the window's scale factor, so `WOW_WIN` then means
/// physical pixels and a capture is the framebuffer that display would show.
pub(crate) fn requested_dpi() -> Option<f32> {
    let v: f32 = std::env::var("WOW_DPI").ok()?.parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v)
}

/// Apply [`requested_dpi`] to a window resolution; the one place the knob is spent.
pub fn at_requested_dpi(res: WindowResolution) -> WindowResolution {
    match requested_dpi() {
        Some(dpi) => res.with_scale_factor_override(dpi),
        None => res,
    }
}

/// The display modes WrathPlus ships; neither is the reference's mode-setting fullscreen.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum DisplayMode {
    /// Borderless, filling the monitor: `gxWindow "0"`, the reference's default.
    #[default]
    Fullscreen,
    /// A window at [`VideoConfig::windowed`]. `gxWindow "1"`.
    Windowed,
}

/// The windowed size a fresh config gets.
pub const DEFAULT_WINDOWED: UVec2 = UVec2::new(1600, 900);

/// `gxWindow`'s value to the mode, by the reference's 0/1 parse (int, `!= 0`); shared by
/// [`crate::cvars`]'s arm and the boot read.
pub(crate) fn display_from_flag(v: f32) -> DisplayMode {
    if v != 0.0 {
        DisplayMode::Windowed
    } else {
        DisplayMode::Fullscreen
    }
}

/// `gxResolution`'s value (`"1280x800"`, parsed by the reference with `sscanf("%d%c%d")`) to a
/// size. A zero extent is refused rather than handed to the windowing system.
pub fn parse_resolution(value: &str) -> Option<UVec2> {
    let (w, h) = value.split_once(['x', 'X'])?;
    let size = UVec2::new(w.trim().parse().ok()?, h.trim().parse().ok()?);
    (size.x > 0 && size.y > 0).then_some(size)
}

/// The `WindowMode` a display mode means, on a given monitor. `maximize` is `gxMaximize`, which
/// counts only while windowed: the reference's window rebuild (`0x58cf10`) gives a windowed,
/// maximized window the popup style `0x90000000`, no caption and no border, sized to the screen
/// (`GetSystemMetrics` 0 and 1) at its origin, which is a borderless window over the monitor.
pub(crate) fn window_mode(
    display: DisplayMode,
    maximize: bool,
    monitor: MonitorSelection,
) -> WindowMode {
    match (display, maximize) {
        (DisplayMode::Fullscreen, _) | (DisplayMode::Windowed, true) => {
            WindowMode::BorderlessFullscreen(monitor)
        }
        (DisplayMode::Windowed, false) => WindowMode::Windowed,
    }
}

/// The mode the primary window is born in, resolved before the `App` exists so a fullscreen launch
/// does not flash windowed until `Startup`. `MonitorSelection::Primary`, since `Current` has no
/// answer before the window exists (`bevy_winit::select_monitor`).
pub fn boot_window_mode() -> WindowMode {
    let flag = |name| crate::cvars::boot_cvar(name).and_then(|v| v.parse::<f32>().ok());
    // A run that sizes its own window owns both rows for the session.
    let (display, maximize) = if windowed_env() {
        (DisplayMode::Windowed, false)
    } else {
        (
            flag("gxWindow").map_or_else(DisplayMode::default, display_from_flag),
            flag("gxMaximize").is_some_and(|v| v != 0.0),
        )
    };
    window_mode(display, maximize, MonitorSelection::Primary)
}

/// The windowed size the primary window is born at, `gxResolution`, read as [`boot_window_mode`]
/// is; `bevy_winit` ignores it while fullscreen.
pub fn boot_windowed_size() -> UVec2 {
    crate::cvars::boot_cvar("gxResolution")
        .and_then(|v| parse_resolution(&v))
        .unwrap_or(DEFAULT_WINDOWED)
}

/// The video knobs a CVar write lands on. Defaults read only the environment; `load_config`
/// applies the file at `Startup`, which the boot window already matches.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub struct VideoConfig {
    pub vsync: bool,
    pub(crate) display: DisplayMode,
    /// `gxMaximize`: a windowed window fills the monitor, borderless ([`window_mode`]).
    pub maximize: bool,
    /// The windowed size, `gxResolution`. Kept while fullscreen so leaving it can restore it.
    pub(crate) windowed: UVec2,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            vsync: !novsync_env(),
            display: if windowed_env() {
                DisplayMode::Windowed
            } else {
                DisplayMode::default()
            },
            maximize: false,
            windowed: DEFAULT_WINDOWED,
        }
    }
}

/// The present mode a vsync setting means. On is `PresentMode::default()` (`Fifo`, never tears),
/// not `AutoVsync`, which resolves to `FifoRelaxed` and can tear on a late frame.
pub fn present_mode(vsync: bool) -> PresentMode {
    if vsync {
        PresentMode::default()
    } else {
        PresentMode::AutoNoVsync
    }
}

pub struct VideoPlugin;

/// The change callbacks of the reference's video-options registration block (`0x688470`); each
/// arm writes only its own resource.
pub fn on_cvar(
    ev: On<crate::cvars::CvarChanged>,
    mut cfg: ResMut<VideoConfig>,
    mut view: ResMut<wrathplus_world::view::ViewDistance>,
    mut msaa: ResMut<wrathplus_world::view::MsaaSetting>,
    msaa_formats: Res<wrathplus_world::view::MsaaFormats>,
    mut tex_filter: ResMut<wrathplus_assets::TexFilterSetting>,
    mut clutter: ResMut<wrathplus_world::clutter::ClutterConfig>,
    mut weather: ResMut<wrathplus_world::weather::WeatherState>,
    particles: Option<ResMut<wrathplus_world::particles::ParticleTuning>>,
    mut spell_effect_level: ResMut<SpellEffectLevel>,
    mut ffx: ResMut<wrathplus_world::ffx_glow::FfxSwitches>,
    mut console: Option<ResMut<crate::app_core::state::ConsoleEcho>>,
    mut cvars: ResMut<crate::cvars::Cvars>,
) {
    use wrathplus_world::view::{FARCLIP_RANGE, MSAA_RANGE};
    let v = ev.num();
    match ev.key().as_str() {
        // A value that is not a size is ignored with a warning.
        "gxresolution" => match parse_resolution(&ev.new) {
            Some(size) => cfg.windowed = size,
            None => warn!("cvar gxResolution: unparseable value '{}' ignored", ev.new),
        },
        "gxvsync" => cfg.vsync = ev.flag(),
        // The reference's polarity: `1` is windowed (the row is "Windowed Mode").
        "gxwindow" => cfg.display = display_from_flag(v),
        // Latched like `gxWindow`: the commit is `RestartGx`, which rebuilds the window
        // (`0x58cf10` reads `+0x09`); `apply_window_mode` is that rebuild.
        "gxmaximize" => cfg.maximize = ev.flag(),
        // The FFX pass's three switches, read every frame by the reference (`0x6cd8a6`, `0x6cc5a8`,
        // `0x6cdf10`), so a write shows on the next frame.
        "ffx" | "ffxglow" | "ffxdeath" => {
            match ev.key().as_str() {
                "ffx" => ffx.master = ev.flag(),
                "ffxglow" => ffx.glow = ev.flag(),
                _ => ffx.death = ev.flag(),
            }
            info!("video: full-screen effects {:?}", *ffx);
        }
        "farclip" => view.farclip = v.clamp(*FARCLIP_RANGE.start(), *FARCLIP_RANGE.end()),
        // Clamped, where the reference refuses an out-of-range write and keeps the value
        // (`0x688d90` echoes "NearClip must be in range 0.01 - 0.33" and returns 0).
        "nearclip" => view.set_nearclip(v),
        // The reference's `atoi` and clamp to `[1, 16]` (`0x63b250`), then this GPU's ceiling:
        // an unoffered count is a wgpu validation error on the first frame. The camera reads it
        // once, at spawn.
        "gxmultisample" => {
            let asked = (v as u32).clamp(*MSAA_RANGE.start(), *MSAA_RANGE.end());
            let granted = msaa_formats.clamp(asked);
            if granted != asked {
                warn!("cvar gxMultisample: this GPU does not offer {asked}x multisampling — using {granted}x");
            }
            msaa.samples = granted;
        }
        // Not applied until the next launch (the filter policy is published once at the end of
        // `CvarLoad`); the reference registers both with `flags = 1` and applies them live.
        // `anisotropic` takes the reference's clamp to `[1, 16]` (`0x689110`).
        "trilinear" => tex_filter.trilinear = ev.flag(),
        "anisotropic" => {
            tex_filter.aniso = (v as u32).clamp(
                *wrathplus_assets::ANISO_RANGE.start(),
                *wrathplus_assets::ANISO_RANGE.end(),
            )
        }
        // The panel's 0/1/2 is the density multiplier x1/x2/x3, clamped to the 1.12 slider's
        // range. `frillDensity` is the same knob, mirrored so `GetCVar` never answers two levels.
        "worlddetail" => {
            clutter.density = v.clamp(0.0, 2.0) + 1.0;
            cvars.mirror(
                wrathplus_ui::script::CVAR_FRILL_DENSITY,
                &clutter.frill_density().to_string(),
            );
            // The stop's other half, as `SetWorldDetail` writes it, so a stop set as a CVar keeps
            // `SmallCull` in step too.
            cvars.mirror(
                wrathplus_ui::script::CVAR_SMALL_CULL,
                &wrathplus_ui::script::small_cull_text(v.clamp(0.0, 2.0) as usize),
            );
        }
        // The same knob in the reference's cells per chunk, clamped to `[1, 256]`
        // (`ClutterConfig::set_frill_density`); the loaded tiles re-scatter off the change.
        "frilldensity" => {
            clutter.set_frill_density(v);
            cvars.mirror(
                wrathplus_ui::script::CVAR_WORLD_DETAIL,
                &(clutter.density - 1.0).to_string(),
            );
        }
        // Weather Intensity 0..3: the reference's callback `0x67b870` jumps (`0x67b8e8`) to the
        // quality cells {0.1, 0.33, 0.66, 1.0} at `[0x8680ec]`. Its off-grid handling is
        // untraced; this clamps.
        "weatherdensity" => weather.weather_density = v.trunc().clamp(0.0, 3.0) as u8,
        // Spell Detail (`0x689510`): `SStrToInt`, clamped to [0, 2] in the handler's own copy,
        // echoed, then the emission scalar 0.33, 0.66 or 1.0 through `0x7adfb0`, shared with
        // `particleDensity`. The reference runs it on every write, an unchanged one included; an
        // observer fires only on a change.
        "spelleffectlevel" => {
            spell_effect_level.0 = wrathplus_ui::script::sstr_to_int(&ev.new);
            let level = spell_effect_level.0.clamp(0, 2);
            if let Some(console) = console.as_mut() {
                console.print(format!("Spell effect level set to {level}."));
            }
            if let Some(mut particles) = particles {
                particles.set_density(spell_effect_scale(level));
            }
        }
        _ => {}
    }
}

/// The `spellEffectLevel` record's integer (`rec+0x28`, `SStrToInt` of the value), unclamped: what
/// the dynamic-object shard emitter reads at spawn (`0x6eb967`), where the handler clamps only its
/// own copy.
#[derive(Resource)]
pub struct SpellEffectLevel(pub i32);

impl Default for SpellEffectLevel {
    /// The registered "2".
    fn default() -> Self {
        Self(2)
    }
}

/// The `spellEffectLevel` emission factor, 0.33, 0.66 or 1.0: the handler's f32 immediates
/// (`0x68956a` `0x3ea8f5c3`, `0x689588` `0x3f28f5c3`, `0x689561` `0x3f800000`) and the shard
/// emitter's `.rdata` pair (`0x808300`, `0x81199c`) are the same three. Any level but 0 or 1 is 1.0.
pub(crate) fn spell_effect_scale(level: i32) -> f32 {
    match level {
        0 => 0.33,
        1 => 0.66,
        _ => 1.0,
    }
}

/// `/console detailDoodadAlpha [0..255]`, the reference's console command (`0x6739a0`, registered
/// by `0x63f9e0` as a command, not a CVar, so it never persists): the ground-clutter cutout that
/// `texel.a x distance_ramp` is tested against, so at the default 128 grass ends ~61 yd out of
/// the 70 yd horizon. Out of range is rejected (`0x6739b9`) with a readout. A bare command also
/// prints the readout, where the reference reads an uninitialised stack slot.
fn detail_doodad_alpha(world: &mut World, args: &str) -> Vec<String> {
    let Some(mut clutter) = world.get_resource_mut::<wrathplus_world::clutter::ClutterConfig>()
    else {
        return vec!["detailDoodadAlpha: this run has no ground clutter".to_string()];
    };
    match args
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<u8>().ok())
    {
        Some(v) => {
            clutter.alpha_ref = f32::from(v) / 255.0;
            vec![format!("detailDoodadAlpha set to {v}")]
        }
        None => vec![format!(
            "detailDoodadAlpha is {} (usage: /console detailDoodadAlpha 0-255)",
            (clutter.alpha_ref * 255.0).round() as u32
        )],
    }
}

/// The CVar rows this feature owns ([`crate::cvars::RegisterCvars`]), each with the
/// reference's register site and its default's standing ([`crate::cvars::Registered`]).
pub const CVARS: &[crate::cvars::Registered] = {
    use crate::cvars::{deviates, same};
    &[
        // `gxApi` (`0x63a833`: name `0x842a64`, default `0x864f7c` "direct3d", flags 3, callback
        // `0x63b030`, record `[0xc4ea94]`): the reference builds D3D9 unless it reads "OpenGl"
        // (`0x63a3c4`, `0x842a5c`). Here it reports the wgpu backend (`wgpu::Backend::to_str`), pushed
        // from `RenderAdapterInfo` and owned by the session, so it is never persisted. pfUI's
        // `panel.lua:185` concatenates it.
        deviates(
            "gxApi",
            "",
            "direct3d",
            "descriptive, not a selector — WrathPlus renders through wgpu, which has no D3D9 \
             backend and no chooser; the value is the live adapter's own and is never persisted",
        )
        .latched(),
        // `gxVSync` (`0x63a859`, "1", flags 3), `OptionsFrame.lua:9`. The knob is
        // [`VideoConfig::vsync`], which the window's present mode follows at the
        // `RestartGx` commit. `$WOW_NOVSYNC=1` overrides it for the session.
        same("gxVSync", "1").latched(),
        // `gxWindow` (`0x63a889`): "0" on enUS; zhCN registers "1", as it does `gxMaximize`
        // (`0x63a8e0`), and koKR `AutoInteract` (`0x603390`). The knob is
        // [`VideoConfig::display`]. Deviation: "0" raises a borderless fullscreen window
        // instead of mode-setting the display, because Wayland and macOS offer no mode-set and X11's
        // leaves the desktop changed after a crash ([`crate::video`]).
        same("gxWindow", "0").latched(),
        // `gxResolution`, a string row parsed by `video::on_cvar`. Here it is only the windowed size:
        // fullscreen is the monitor's own and no mode list is offered.
        deviates(
            "gxResolution",
            "1600x900",
            "640x480",
            "narrowed to the WINDOWED size only — fullscreen is the monitor's own and we expose \
             no mode list, and 640x480 is not a window anyone would ship a client at",
        )
        .latched(),
        // `gxMultisample` (`0x63a950`, flags 3). The reference formats its default from field 21 of the
        // `VideoHardware.dbc` row `DetectHardware` (`0x641260`) matches, which is 1, no multisampling,
        // on every fallback row a modern GPU reaches. The knob is [`wrathplus_world::view::MsaaSetting`],
        // read once at the camera's spawn; `$WOW_MSAA` overrides it for the session.
        same("gxMultisample", "1").latched(),
        // `GetCurrentMultisampleFormat 0x48c580` looks up all three of the Video dropdown's values by
        // name, so these must exist. They describe the swapchain's own pair and steer nothing;
        // `SetMultisampleFormat` writes them as `0x48c640` does.
        deviates(
            "gxColorBits",
            "32",
            "16",
            "these describe, they do not steer — the pair is our swapchain's own, and every \
             format `MsaaFormats` publishes carries it",
        )
        .latched(),
        deviates(
            "gxDepthBits",
            "32",
            "16",
            "as `gxColorBits` — the depth half of the same descriptive pair",
        )
        .latched(),
        // `spellEffectLevel` (`0x688900`: name `0x869324`, default `0x843068` "2", flags 1, callback
        // `0x689510`, record `[0xc7f2a4]`), the Spell Detail slider (`OptionsFrame.lua:32`). The
        // callback's two effects here: the particle emission scalar ([`on_cvar`]) and
        // the dynamic-object shard rate, read at each emitter's spawn (`crate::entities`). Its third,
        // skipping one M2-scene ground decal below level 2 (`0x672b3a`), has no counterpart: WrathPlus
        // does not draw that projection.
        // Build 12340 registers "9" (`0x78e8f8`) on its 0..9 slider; the particle scalar here
        // follows 1.12's 0..2 scale ([`on_cvar`]), so 3.3.5's scale is not ported.
        deviates(
            "spellEffectLevel",
            "2",
            "9",
            "the particle scalar follows 1.12's 0..2 Spell Detail scale; 12340's 0..9 is not ported",
        ),
        // The latched `gx*` rows the stock window's boxes and refresh dropdown write (flags 3, committed
        // by `RestartGx`), each callback storing one field of the device-format record `0xc4eb98`.
        //
        // `gxMaximize` (`0x63a8e0`: name `0x864edc`, "0", "1" on zhCN like `gxWindow`, callback
        // `0x63b1f0` → `+0x09` `[0xc4eba1]`): the window rebuild `0x58cf10` makes a windowed, maximized
        // window a borderless popup (style `0x90000000`) at the screen's full size (`GetSystemMetrics`
        // 0 and 1). The knob is [`VideoConfig::maximize`].
        same("gxMaximize", "0").latched(),
        // `gxTripleBuffer` (`0x63a80d`: name `0x864f88`, "0", callback `0x63afe0` → `+0x18`), whose
        // callback refuses any value but 0 or 1 ("TripleBuffer must be 0 or 1", `0x8650b8`), a range
        // this registry has no way to refuse with. No effect here: wgpu's present modes offer no
        // triple-buffer choice.
        same("gxTripleBuffer", "0").latched(),
        // `gxCursor` (`0x63a906`: name `0x864eb8`, "1", callback `0x63b220` → `+0x05` `[0xc4eb9d]`),
        // Hardware Cursor. No effect here: WrathPlus always draws the OS cursor (`crate::cursor`) and has
        // no software cursor to fall back to.
        same("gxCursor", "1").latched(),
        // `gxFixLag` (`0x63a9c1`: name `0x864e40`, callback `0x63b310` → `+0x06` `[0xc4eb9e]`), Fix
        // Input Lag: its default is `"%d"` of `VideoHardware.dbc` field 20 (`[[0xc4e6e4]+0x50]`) of the
        // row `DetectHardware` matched, 1 on all three fallback rows (168, 169, 170) a modern GPU can
        // reach. No effect here: WrathPlus has no such switch in its frame pacing.
        same("gxFixLag", "1").latched(),
        // `gxRefresh` (`0x63a7e7`: name `0x864fa8`, "75", callback `0x63aed0` → `+0x28`), the refresh
        // dropdown (`OptionsFrame.lua:242-245`, `:300`); the callback accepts only the eleven rates at
        // `0x8645e0` (60 to 200). No effect here: fullscreen is borderless at the monitor's own rate,
        // and `GetRefreshRates` answers the reference's no-rates sentinel, which disables the dropdown.
        same("gxRefresh", "75").latched(),
    ]
};

/// The vsync and window levers: `$WOW_NOVSYNC`, and `$WOW_WIN`, a capture scenario or an
/// instrumented run, which owns the window geometry.
pub(crate) const SESSION_LEVERS: &[crate::cvars::SessionLever] = &[
    crate::cvars::SessionLever {
        name: "novsync_env",
        taken: novsync_env,
        read: |world| {
            let v = world.get_resource::<VideoConfig>();
            vec![("gxVSync", v.map(|c| crate::cvars::lever_flag(c.vsync)))]
        },
    },
    crate::cvars::SessionLever {
        name: "windowed_env",
        taken: windowed_env,
        read: |world| {
            let v = world.get_resource::<VideoConfig>();
            vec![
                (
                    "gxWindow",
                    v.map(|c| crate::cvars::lever_flag(c.display == DisplayMode::Windowed)),
                ),
                (
                    "gxMaximize",
                    v.map(|c| crate::cvars::lever_flag(c.maximize)),
                ),
                (
                    "gxResolution",
                    v.map(|c| format!("{}x{}", c.windowed.x, c.windowed.y)),
                ),
            ]
        },
    },
];

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        crate::cvars::RegisterCvars::register_cvars(app, CVARS);
        for &lever in SESSION_LEVERS {
            crate::cvars::RegisterCvars::register_session_lever(app, lever);
        }
        use crate::app_core::console::ConsoleCommandApp;
        app.init_resource::<SpellEffectLevel>();
        app.add_observer(on_cvar);
        app.console_command(
            "detailDoodadAlpha",
            "The ground-clutter cutout reference, 0-255 (128 = the default).",
            detail_doodad_alpha,
        );
        app.init_resource::<VideoConfig>()
            .init_resource::<GxRestarts>()
            .add_systems(Startup, (log_display_session, check_window_pinned).chain())
            .add_systems(
                Update,
                (
                    // After the tick and the CVar sync: the stock Okay handler calls `SetCVar` per
                    // row, then `RestartGx()`, so the staged rows are registered before the commit.
                    (drain_restart_gx, (apply_present_mode, apply_window_mode))
                        .chain()
                        .after(crate::app_core::schedule::UiInput)
                        .after(crate::cvars::sync_cvars),
                    // A push the tick may read (`GetScreenResolutions`): the feed phase.
                    publish_display_modes.in_set(crate::app_core::schedule::UiFeed),
                ),
            );
    }
}

/// How many `RestartGx()` calls the interface has made. A counter, not a flag: each applier keeps
/// its own last value, so one bump forces one re-assertion in each, in any order.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct GxRestarts(u32);

/// Carry `RestartGx()` calls into [`GxRestarts`] and commit the latched `gx*` rows (flags `3`),
/// where the reference's restart calls `CVar::Update` (`0x63e060`) on each; `GetCVar` answers the
/// applied value until then.
fn drain_restart_gx(
    script: Option<NonSendMut<wrathplus_ui::script::UiScript>>,
    mut restarts: ResMut<GxRestarts>,
    mut cvars: ResMut<crate::cvars::Cvars>,
    mut commands: Commands,
) {
    let Some(mut script) = script else {
        return;
    };
    let asks = script.take_restart_gx_asks();
    if asks == 0 {
        // A `ResMut` deref-mut is a change signal, so a quiet frame touches nothing.
        return;
    }
    restarts.0 = restarts.0.wrapping_add(asks);
    let committed = cvars.commit_latched();
    if cvars.has_events() {
        for event in cvars.take_events() {
            commands.trigger(event);
        }
    }
    info!("video: RestartGx — {committed} staged setting(s) committed; re-asserting the display mode and present mode");
}

/// The reference's three filters on the resolution list (`0x48bcfa` to `0x48bd18`): keep when
/// `w/h >= 1.248` (`[0x804570]`, just under 5:4, so square and portrait fail), `w >= 800` and
/// `h >= 600`.
fn offerable(r: ScreenResolution) -> bool {
    r.width >= 800 && r.height >= 600 && f64::from(r.width) / f64::from(r.height) >= 1.248
}

/// The reference's four hardcoded modes, in its append order (`0x48bda2`, `0x48bddf`, `0x48be43`,
/// `0x48bea7`), used when no enumerated mode survives [`offerable`]. The reference also takes this
/// path when `widescreen` (`0x63a747`, default `"1"`) is 0; WrathPlus does not register that CVar.
const SCREEN_FALLBACK: [ScreenResolution; 4] = [
    ScreenResolution {
        width: 800,
        height: 600,
    },
    ScreenResolution {
        width: 1024,
        height: 768,
    },
    ScreenResolution {
        width: 1280,
        height: 1024,
    },
    ScreenResolution {
        width: 1600,
        height: 1200,
    },
];

/// The list [`publish_display_modes`] last pushed and the current entry.
type PublishedModes = Option<(Vec<ScreenResolution>, Option<ScreenResolution>)>;

/// The host half of `GetScreenResolutions` / `GetCurrentResolution`: the monitors' mode sizes and
/// full sizes through [`offerable`], else [`SCREEN_FALLBACK`]; a pick sets the windowed size,
/// `gxResolution`. The engine adds the live window size when missing, which `CT_Viewport.lua:201`
/// needs. Deviation: sizes are logical px, because `gxResolution` is a logical inner size here.
/// The list is recomputed on change; the reference builds its list once and never invalidates it.
fn publish_display_modes(
    script: Option<NonSendMut<wrathplus_ui::script::UiScript>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    monitors: Query<&bevy::window::Monitor>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<PublishedModes>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let (offered, current) = display_modes(window, monitors.iter());
    // Keyed by VM: a `ReloadUI` VM has been pushed nothing, so a plain `Local` would skip it.
    let memo = last.get(&script);
    if memo.as_ref() == Some(&(offered.clone(), current)) {
        return;
    }
    *memo = Some((offered.clone(), current));
    script.set_screen_resolutions(offered, current);
}

// Shared by the load-time seed and the update feed: FrameXML reads the list in OnLoad.
fn display_modes<'a>(
    window: &Window,
    monitors: impl IntoIterator<Item = &'a bevy::window::Monitor>,
) -> (Vec<ScreenResolution>, Option<ScreenResolution>) {
    let res = &window.resolution;
    let current = Some(ScreenResolution {
        width: res.width() as u32,
        height: res.height() as u32,
    })
    .filter(|r| r.width > 0 && r.height > 0);
    let mut offered: Vec<ScreenResolution> = Vec::new();
    for m in monitors {
        // The monitor's own scale, not the window's: these rows describe the panel.
        let scale = if m.scale_factor > 0.0 {
            m.scale_factor
        } else {
            1.0
        };
        let logical = |size: UVec2| ScreenResolution {
            width: (size.x as f64 / scale).round() as u32,
            height: (size.y as f64 / scale).round() as u32,
        };
        offered.push(logical(m.physical_size()));
        offered.extend(m.video_modes.iter().map(|v| logical(v.physical_size)));
    }
    offered.retain(|r| offerable(*r));
    if offered.is_empty() {
        offered.extend(SCREEN_FALLBACK);
    }
    offered.sort_by_key(|r| (u64::from(r.width) * u64::from(r.height), r.width, r.height));
    offered.dedup();
    (offered, current)
}

pub fn seed_display_modes(world: &mut World, script: &mut wrathplus_ui::script::UiScript) {
    let mut windows = world.query_filtered::<&Window, With<PrimaryWindow>>();
    let Ok(window) = windows.single(world) else {
        return;
    };
    let window = window.clone();
    let mut monitors = world.query::<&bevy::window::Monitor>();
    let (offered, current) = display_modes(&window, monitors.iter(world));
    script.set_screen_resolutions(offered, current);
}

/// Check the window got the size `$WOW_WIN` asked for: the window manager may clamp it to the
/// display (macOS does). Fatal under a capture, whose diffs assume the scenario's size; a warning
/// otherwise.
fn check_window_pinned(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(want) = requested_window_size() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    // Under `$WOW_DPI`, `$WOW_WIN` is physical px, so compare the physical size.
    let res = &window.resolution;
    let got = match requested_dpi() {
        Some(_) => UVec2::new(res.physical_width(), res.physical_height()),
        None => UVec2::new(res.width() as u32, res.height() as u32),
    };
    if got == want {
        return;
    }
    let unit = if requested_dpi().is_some() {
        "physical"
    } else {
        "logical"
    };
    let capturing =
        std::env::var_os("WOW_CAPTURE").is_some() || std::env::var_os("WOW_CAPTURE_UI").is_some();
    if !capturing {
        warn!(
            "window: asked for {}x{} {unit}, got {}x{} — the window manager clamped it to the \
             display. Harmless here; it would invalidate a capture.",
            want.x, want.y, got.x, got.y
        );
        return;
    }
    error!(
        "window: REFUSING this capture — asked for {}x{} {unit}, got {}x{}. The window manager \
         clamped the request to the display this window opened on, so the image would not be the \
         size the scenario is denominated in and any diff against it would be meaningless. Use a \
         size that fits the current display (or move the window to a bigger one) and re-run.",
        want.x, want.y, got.x, got.y
    );
    exit.write(AppExit::error());
}

/// One boot line naming what the window got and, on Linux, the backend and any nested compositor
/// (mouse-look's `CursorGrabMode::Locked` is rejected on X11). Not dev-only: players paste it.
fn log_display_session(windows: Query<&Window, With<PrimaryWindow>>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let res = &window.resolution;
    info!(
        "video: {:?}, {}x{} logical / {}x{} physical (scale {}){}",
        window.mode,
        res.width(),
        res.height(),
        res.physical_width(),
        res.physical_height(),
        res.scale_factor(),
        display_session(),
    );
}

/// The display-server facts as a trailing clause; empty except on Linux/BSD, where the windowing
/// backend is chosen at runtime.
fn display_session() -> String {
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
    {
        let set = |k: &str| std::env::var_os(k).is_some();
        // winit prefers Wayland when `WAYLAND_DISPLAY` is set, else X11 (XWayland under a
        // Wayland compositor); `bevy`'s `default_platform` builds both.
        let backend = match (set("WAYLAND_DISPLAY"), set("DISPLAY")) {
            (true, _) => "wayland",
            (false, true) => "x11",
            (false, false) => "none",
        };
        // gamescope exports its socket name to children; the SteamOS session stamps the Deck.
        let nested = if set("GAMESCOPE_WAYLAND_DISPLAY") {
            ", gamescope"
        } else {
            ""
        };
        let deck = if set("SteamDeck") { ", steamdeck" } else { "" };
        format!(
            " [{backend}{nested}{deck}, XDG_SESSION_TYPE={}]",
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unset".into()),
        )
    }
    #[cfg(not(all(unix, not(any(target_os = "macos", target_os = "android")))))]
    String::new()
}

/// Push vsync to the window only when the setting moves or a `RestartGx()` asks: the capture
/// probes write `present_mode` directly, and their override must stick.
fn apply_present_mode(
    cfg: Res<VideoConfig>,
    restarts: Res<GxRestarts>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut last_restart: Local<u32>,
) {
    // A `RestartGx()` re-asserts even when nothing moved.
    let forced = std::mem::replace(&mut *last_restart, restarts.0) != restarts.0;
    if !cfg.is_changed() && !forced {
        return;
    }
    let want = present_mode(cfg.vsync);
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    // A spurious change to `Window` is a surface reconfigure.
    if window.present_mode != want {
        window.present_mode = want;
        info!(
            "video: vsync {} ({want:?})",
            if cfg.vsync { "on" } else { "off" }
        );
    }
}

/// Push the display mode to the window, gated as [`apply_present_mode`] is. The guard matches the
/// variant, not the monitor: birth uses `Primary` and a live toggle `Current`, so a `!=` compare
/// would re-assert fullscreen on every launch.
fn apply_window_mode(
    cfg: Res<VideoConfig>,
    restarts: Res<GxRestarts>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut last_restart: Local<u32>,
) {
    if crate::run_mode::showcase_active() {
        return;
    }
    // A `RestartGx()` re-asserts even when nothing moved, re-applying `gxResolution` to a window
    // that is already windowed.
    let forced = std::mem::replace(&mut *last_restart, restarts.0) != restarts.0;
    if !cfg.is_changed() && !forced {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let want = window_mode(cfg.display, cfg.maximize, MonitorSelection::Current);
    let already = matches!(
        (&window.mode, &want),
        (WindowMode::Windowed, WindowMode::Windowed)
            | (
                WindowMode::BorderlessFullscreen(_),
                WindowMode::BorderlessFullscreen(_)
            )
    );
    if already && !forced {
        return;
    }
    // Leaving fullscreen hands the size back: entering it overwrote `window.resolution` with the
    // monitor's (`bevy_window`'s documented behaviour). Both writes land in one frame, as
    // `bevy_winit::changed_windows` applies `mode` before `resolution`.
    if want == WindowMode::Windowed {
        window
            .resolution
            .set(cfg.windowed.x as f32, cfg.windowed.y as f32);
    }
    window.mode = want;
    info!(
        "video: display mode {:?}{} ({want:?})",
        cfg.display,
        if cfg.maximize && cfg.display == DisplayMode::Windowed {
            ", maximized"
        } else {
            ""
        }
    );
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;
