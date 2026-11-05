//! The UI-engine bridge: hosts [`wrathplus_ui::script::UiScript`] and feeds its
//! [`extract`](wrathplus_ui::script::UiScript::extract) output into the quad pass
//! ([`crate::ui_pass::UiQuads`]) every frame. The script side is WoW UI space (y-up, origin
//! bottom-left, a screen `768/uiScale` units tall); [`seam_scale`] carries quads ×s out and the
//! mouse ÷s in, and extraction flips to y-down window px.

use bevy::prelude::*;

use wrathplus_ui::script::UiScript;

/// The addon folder: discovery, manifests, the enable file and the load walk.
pub mod addons;
mod content;
pub(crate) mod extract;
pub(crate) mod glue;
mod token_player;
pub(crate) use token_player::{watched_inputs, SelfTokens};
pub mod glue_create;
mod input;
mod load_log;
mod manifest;
#[cfg(test)]
mod production_load;
#[cfg(test)]
mod stock_menus_tests;
mod wrath_addons;
#[cfg(test)]
mod wrath_framexml_tests;
mod wrath_native;
mod xp_diagnostics;

/// The stock FrameXML this client runs off the player's own patch chain; its header is the rule.
mod reference_ui;
pub use reference_ui::initialize_asset_overrides;

/// The feed gate: a UI feed's input-side early-out, audited by `WOW_FEED_GATE_CHECK=1`.
pub(crate) mod gate;

// Not test-only: the addon harness loads the whole shipped interface under each addon.
pub(crate) use manifest::default_bindings;
pub use manifest::load_default_ui;
#[cfg(any(test, feature = "test-support"))]
pub use manifest::{load_stock_bindings, stock_bindings_file};
pub(crate) use reference_ui::default_binding_layers;

pub(crate) use crate::app_core::state::{CursorPayloadHeld, PointerOverUi};
/// A synthetic pointer owns the mouse this frame (the drag probe's gesture), so
/// [`input::feed_ui_input`] skips its mouse half and never feeds the real cursor.
#[derive(Resource, Default)]
pub struct SyntheticPointer(pub bool);

/// A capture never reads the OS pointer: set for a whole `$WOW_CAPTURE`/`$WOW_CAPTURE_UI` run, so
/// its pixels never depend on where the real cursor rests.
#[derive(Resource, Default)]
pub(crate) struct CapturePointerPinned(pub(crate) bool);

/// The egui dev overlay's half of the pointer arbitration, written by `track_pointer_over_ui`;
/// defined here so a build without dev overlays still has the type.
#[derive(Resource, Default)]
pub struct EguiPointerOver(pub bool);

pub(crate) use crate::app_core::ui_flags::{
    PlayerUiClickConsumed, PlayerUiHover, UiClock, UiKeyboardCapture, UiScaleCvar,
};
/// One frame's UI-pass phase split in μs, as the `[ui-cost]` line prints it; owned by the
/// producer so it exists whether or not its reader, `hover_log`, is compiled in.
#[derive(Resource, Default, Clone)]
pub struct UiFrameCost {
    /// FontStrings the layout had the font engine shape this frame, and the first few by name.
    pub measured: usize,
    pub measured_texts: Vec<String>,
    pub tick: u128,
    pub resolve: u128,
    pub measure: u128,
    pub extract: u128,
    pub convert: u128,
    pub diff: u128,
    pub quads: usize,
    pub solves: u64,
    /// Full layout-graph derivations this frame, each about 30× a solve; expected zero.
    pub derives: u64,
    pub skipped: bool,
    /// Entries the splice re-converted this frame; 0 on a settled or full-conversion frame.
    pub spliced: usize,
    /// Entries the splice dropped (drawn last frame, not now); `[ui-cost] dropped=` prints it.
    pub dropped: usize,
}

/// Whether [`UiFrameCost`] is wanted this run: `WOW_UI_COST=1` or the hover recorder.
#[derive(Resource, Default)]
pub struct UiCostWanted(pub bool);

/// What [`extract::tick_script`] hands [`extract::paint_script`]; with `live` false (no VM or
/// window) the paint does nothing.
#[derive(Resource, Default)]
pub(crate) struct UiPassState {
    pub(crate) live: bool,
    /// The 768-virtual seam scale this frame was ticked under.
    pub(crate) seam: f32,
    pub(crate) dpi: f32,
    /// The meter's first three phases, so one `[ui-cost]` row still describes one frame.
    pub(crate) us_tick: u128,
    pub(crate) us_resolve: u128,
    pub(crate) us_measure: u128,
    /// The layout counters before the tick, so the row's `solves` and `derives` cover both halves.
    pub(crate) solves_before: u64,
    pub(crate) derives_before: u64,
}

/// UI chrome under the pointer: [`PointerOverUi`] minus the nameplates, which take the mouse but
/// not the wheel. The reference has no such bit: a wheel notch walks the strata and falls through
/// any frame without `OnMouseWheel` to `CGWorldFrame`.
#[derive(Resource, Default)]
pub(crate) struct PointerOverUiPanel(pub(crate) bool);

/// A wheel notch a UI `OnMouseWheel` handler took this frame: the reference's wheel walk stops at
/// the handler, so the notch never reaches `CGWorldFrame`'s binding. A chat frame takes the wheel
/// without the mouse (`enableMouse="false"`, `FloatingChatFrame.xml:505`), which
/// [`PointerOverUiPanel`] cannot see. Set by the UI feed, read by the binding pass after it.
#[derive(Resource, Default)]
pub(crate) struct UiWheelEaten(pub(crate) bool);

pub use crate::app_core::schedule::UiFeed;

/// The player-UI paint pass, after the camera so world-anchored widgets use this frame's view.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct UiPaint;

pub use crate::app_core::schedule::UiInput;

/// The FrameXML digest this process loads, which the corpus harness stamps on each report.
pub fn framexml_digest() -> String {
    content::digest()
}

/// Run a Lua chunk and log a failure: the form for any run whose `Result` is not consumed.
pub fn run_or_warn(script: &wrathplus_ui::script::UiScript, chunk: &str) {
    if let Err(e) = script.run(chunk) {
        warn!("ui_script: chunk failed: {e}");
    }
}

/// The shipped `uiScale`. Deviation: a flat 0.9, because 1.0 reads oversized. With `useUiScale`
/// off, its default (`0x8430c0`), the reference sets `max(768/H, 0.9)` above 768 px tall and 1.0
/// at or below (`0x492f70`, on a mode set and from `0x4908ad`), so the two agree from ~853 px up.
pub const DEFAULT_UI_SCALE: f32 = 0.9;

/// `WOW_UI_SCALE=` if set, clamped to the dial's range, else [`DEFAULT_UI_SCALE`].
fn default_ui_scale() -> f32 {
    std::env::var("WOW_UI_SCALE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| v.clamp(0.5, 1.5))
        .unwrap_or(DEFAULT_UI_SCALE)
}

/// The Lua UI host (a `NonSend` resource: an mlua VM is `!Send`) and its per-frame passes.
pub struct UiScriptPlugin;

/// `uiScale`'s change callback: the dial's own `[0.5, 1.5]` clamp.
pub fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut scale: ResMut<UiScaleCvar>) {
    if ev.is("uiScale") {
        scale.0 = ev.num().clamp(0.5, 1.5);
    }
}

/// `/console reloadUI`: the same deferred rebuild `ReloadUI()` and `/reload` queue.
fn console_reload_ui(world: &mut World, _args: &str) -> Vec<String> {
    match world.get_non_send_resource_mut::<UiScript>() {
        Some(mut script) => {
            script.queue_session_request(wrathplus_ui::script::SessionRequest::ReloadUi);
            Vec::new()
        }
        None => vec!["reloadUI: no interface to reload".to_string()],
    }
}

/// `$WOW_UI_SCALE` seeds [`UiScaleCvar`] and owns `uiScale` for the session.
const UI_SCALE_LEVER: crate::cvars::SessionLever = crate::cvars::SessionLever {
    name: "WOW_UI_SCALE",
    taken: || crate::cvars::env_set("WOW_UI_SCALE"),
    read: |world| {
        let v = world.get_resource::<UiScaleCvar>();
        vec![("uiScale", v.map(|s| s.0.to_string()))]
    },
};

impl Plugin for UiScriptPlugin {
    fn build(&self, app: &mut App) {
        // The one route: the stock build-12340 FrameXML and addon host. Its input feed serves
        // the world and the stock glue screens alike.
        info!("ui_script: stock build-12340 FrameXML and addon host");
        wrath_native::build(app);
        wrath_addons::build(app);
        crate::cvars::RegisterCvars::register_session_lever(app, UI_SCALE_LEVER);
    }
}

/// `PointerOverUi = egui dev overlay ∨ player-UI hover`; the dev half is optional.
fn arbitrate_pointer_over_ui(
    egui: Option<Res<EguiPointerOver>>,
    hover: Res<PlayerUiHover>,
    plate_hover: Res<crate::vplates::PlateHover>,
    mut over: ResMut<PointerOverUi>,
    mut panel: ResMut<PointerOverUiPanel>,
) {
    // No cinematic term: the hit test finds the full-screen, mouse-enabled `CinematicFrame`.
    over.0 = egui.is_some_and(|e| e.0) || hover.0.is_some();
    // Chrome excludes the plates; the plate hover is last frame's, as the plate driver runs later.
    panel.0 = over.0 && plate_hover.0.is_none();
}

/// The session lifecycle: the VM's birth, identity, death and reload.
mod lifecycle;
#[cfg(test)]
pub(crate) use crate::app_core::ui_state::ingame_ui_up;
pub(crate) use crate::app_core::ui_state::seam_scale;
pub(crate) use lifecycle::{
    end_ui_session, run_pending_reload, setup_script, AddOnIdentity, LeavingWorldArmed,
    PendingEntryUiLoad, ReloadUiPending,
};
// Test-only: other modules' tests consume these, and a plain re-export would warn unused.
#[cfg(test)]
pub(crate) use lifecycle::{finish_ui_load, load_ingame_ui_on_world_entry};

/// `GetFramerate()`'s host half: the reference writes each frame's time into a zeroed ring of 30
/// (`0xcd76b0`, at `0x7831bc`) and answers 1 / their mean, or 1000 while the mean is under 0.001 s
/// (`0x77f440`), so it reads 1000 until the frames' total passes 0.03 s, and a spike lingers for
/// 30 frames.
fn feed_framerate(
    script: Option<NonSendMut<UiScript>>,
    time: Res<Time<bevy::time::Real>>,
    mut ring: Local<([f32; 30], usize)>,
) {
    let Some(mut script) = script else {
        return;
    };
    let (times, at) = &mut *ring;
    times[*at] = time.delta_secs();
    *at = (*at + 1) % times.len();
    script.set_framerate(f64::from(ring_rate(times)));
}

/// `0x77f440` over the ring: 1 / the mean of all 30 slots, or 1000 under a 0.001 s mean.
fn ring_rate(times: &[f32; 30]) -> f32 {
    let mean = times.iter().sum::<f32>() / 30.0;
    if mean >= 0.001 {
        1.0 / mean
    } else {
        1000.0
    }
}

#[cfg(test)]
#[test]
fn the_framerate_is_the_30_frame_mean_from_a_zeroed_ring() {
    let mut ring = [0.0f32; 30];
    assert_eq!(ring_rate(&ring), 1000.0);
    ring[0] = 1.0 / 60.0;
    assert_eq!(
        ring_rate(&ring),
        1000.0,
        "one frame in, the mean is under 0.001 s"
    );
    ring = [1.0 / 60.0; 30];
    assert!((ring_rate(&ring) - 60.0).abs() < 0.01);
    ring[0] = 1.0;
    assert!(
        (ring_rate(&ring) - 30.0 / (1.0 + 29.0 / 60.0)).abs() < 0.01,
        "a spike lingers"
    );
}

/// A test font engine: every character `.0` wide, 12 tall per line, greedily wrapped.
#[cfg(test)]
pub(crate) struct FixedWidthFont(pub(crate) f32);

#[cfg(test)]
impl wrathplus_ui::script::TextMeasure for FixedWidthFont {
    fn measure(&mut self, req: &wrathplus_ui::script::MeasureRequest) -> (f32, f32, f32) {
        let natural = req.text.chars().count() as f32 * self.0;
        match req.wrap_width {
            Some(w) if w > 0.0 && natural > w => (w, 12.0 * (natural / w).ceil(), natural),
            _ => (natural, 12.0, natural),
        }
    }

    /// Inline, as [`crate::ui_text::AtlasMeasurer`] answers it: `.0` per byte, a multi-line box's
    /// rows starting after each newline (no width wrap).
    fn editbox_advances(
        &mut self,
        req: &wrathplus_ui::script::EditBoxAdvanceRequest,
    ) -> Option<(Vec<f32>, Vec<usize>, f32)> {
        let cum = (0..=req.text.len()).map(|i| i as f32 * self.0).collect();
        if req.wrap_width.is_none() {
            return Some((cum, vec![0], 0.0));
        }
        let breaks = req.text.match_indices('\n').map(|(i, _)| i + 1);
        Some((cum, std::iter::once(0).chain(breaks).collect(), 12.0))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_ui;

#[cfg(test)]
mod chat_login;
#[cfg(test)]
pub(crate) use chat_login::fire_chat_login;

/// The `wrathplus_formats::TokenContext::printf` seam: `SStrPrintf` over the template filler.
pub(crate) fn token_printf(template: &str, args: &[wrathplus_formats::TokenNumber]) -> String {
    let args: Vec<_> = args
        .iter()
        .map(|n| match n {
            wrathplus_formats::TokenNumber::Int(v) => wrathplus_ui::strings::Arg::D(*v),
            wrathplus_formats::TokenNumber::Float(v) => wrathplus_ui::strings::Arg::F(*v),
        })
        .collect();
    wrathplus_ui::strings::fill(template, &args)
}

/// [`test_ui::load_ui`] for a test module outside `ui_script`, such as `ui_action::feed_tests`.
#[cfg(test)]
pub(crate) fn load_ui_for_test(script: &mut wrathplus_ui::script::UiScript, entry: &str) -> usize {
    test_ui::load_ui(script, entry)
}

#[cfg(test)]
mod achievement_alert_tests;
#[cfg(test)]
mod cinematic_tests;

#[cfg(test)]
mod cast_tests;

#[cfg(test)]
mod mirror_timer_tests;

#[cfg(test)]
mod combat_text_tests;

#[cfg(test)]
mod combo_frame_tests;

#[cfg(test)]
mod unit_frame_tests;

#[cfg(test)]
mod unit_popup_tests;

#[cfg(test)]
mod chat_resize_tests;

#[cfg(test)]
mod dropdown_tests;

#[cfg(test)]
mod action_bar_tests;

#[cfg(test)]
mod exp_bar_tests;

#[cfg(test)]
mod multibar_stance_tests;

#[cfg(test)]
mod pet_bar_tests;

#[cfg(test)]
mod pet_frame_tests;

#[cfg(test)]
mod pet_stable_tests;

#[cfg(test)]
mod tot_frame_tests;

#[cfg(test)]
mod micro_menu_tests;

#[cfg(test)]
mod perf_bar_tests;

#[cfg(test)]
mod panel_tests;

#[cfg(test)]
mod fade_tests;

#[cfg(test)]
mod merchant_tests;

#[cfg(test)]
mod money_frame_tests;

#[cfg(test)]
mod faux_scroll_tests;

/// The stock UIPanelTemplates and OptionsFrameTemplates kit, driven the way an addon drives it.
#[cfg(test)]
mod panel_template_tests;

/// The stock BasicControls.xml, which WrathPlus never calls, entered from Lua as an addon does.
#[cfg(test)]
mod basic_controls_tests;

#[cfg(test)]
mod color_picker_tests;

/// `UIParent.xml`'s loose addon-facing helpers, such as `MouseIsOver`.
#[cfg(test)]
mod uiparent_tests;

#[cfg(test)]
pub(crate) mod trainer_tests;

#[cfg(test)]
mod bank_tests;

#[cfg(test)]
mod taxi_tests;

#[cfg(test)]
mod loot_tests;

#[cfg(test)]
mod group_loot_tests;
#[cfg(test)]
mod group_loot_tooltip_tests;

#[cfg(test)]
mod chat_tests;

/// The chat bubble's `UIMenu` kit driven as a menu: the rows' label and shortcut anchoring.
#[cfg(test)]
mod ui_menu_tests;

/// The chat tab's options menu end to end, over the whole dropdown and colour-picker stack.
#[cfg(test)]
mod chat_options_tests;

#[cfg(test)]
mod bag_tests;
#[cfg(test)]
mod resolve_bench;

#[cfg(test)]
mod tooltip_anchor_tests;

#[cfg(test)]
mod tooltip_compare_tests;

/// `GameTooltipTemplate` as an addon sees it, through `inherits=` and `CreateFrame`.
#[cfg(test)]
mod tooltip_template_tests;

#[cfg(test)]
mod escape_tests;

#[cfg(test)]
mod macro_tests;

// `pub(crate)` for `harness`, `on_page` and `label`, which the bindings dispatch tests use.
#[cfg(test)]
pub(crate) mod keybindings_tests;

#[cfg(test)]
mod delete_item_tests;
#[cfg(test)]
mod instance_tests;

#[cfg(test)]
mod static_popup_tests;

#[cfg(test)]
mod binder_tests;

#[cfg(test)]
mod summon_tests;

#[cfg(test)]
mod talent_wipe_tests;

#[cfg(test)]
mod death_tests;

#[cfg(test)]
mod duel_tests;

#[cfg(test)]
mod enchant_confirm_tests;

#[cfg(test)]
mod friends_tests;

/// The contact-list storm: a reply never asks again.
#[cfg(test)]
mod friends_storm_tests;

/// The four guild windows, over a Lua stand-in for the guild engine API.
#[cfg(test)]
mod guild_tests;

/// The GM help window and its ticket toast, over a pushed `GMTicketCategory.dbc` catalog.
#[cfg(test)]
mod help_frame_tests;

/// The guild-charter registrar and petition sheet, over a Lua stand-in for the charter API.
#[cfg(test)]
mod petition_tests;

/// The social window's fourth tab: the raid pane and its grid, over a pushed raid roster.
#[cfg(test)]
mod raid_tests;

#[cfg(test)]
mod quest_share_tests;

#[cfg(test)]
mod quest_fade_tests;
#[cfg(test)]
mod quest_tests;

#[cfg(test)]
mod quest_timer_tests;

#[cfg(test)]
mod battlefield_minimap_options_tests;
/// The battle map: the stock `Blizzard_BattlefieldMinimap` addon, demand-loaded as SHIFT-M does.
#[cfg(test)]
mod battlefield_minimap_tests;

#[cfg(test)]
mod tutorial_tests;

#[cfg(test)]
mod durability_tests;
// `pub(crate)` for `harness`, `push` and `row`, which `perf::hud`'s test uses.
#[cfg(any(test, feature = "test-support"))]
pub mod world_state_tests;

#[cfg(test)]
mod screenshot_tests;

#[cfg(test)]
mod questlog_tests;

#[cfg(test)]
mod questlog_description_tests;

#[cfg(test)]
mod character_tests;

#[cfg(test)]
mod skills_frame_tests;

#[cfg(test)]
mod reputation_frame_tests;

#[cfg(test)]
mod honor_frame_tests;

#[cfg(test)]
mod pet_paperdoll_tests;

#[cfg(test)]
mod inspect_tests;

#[cfg(test)]
mod dressup_tests;

#[cfg(test)]
mod minimap_tests;

#[cfg(test)]
mod spellbook_tests;

#[cfg(test)]
mod buff_tests;

#[cfg(test)]
mod target_aura_tests;

#[cfg(test)]
mod zone_text_tests;

#[cfg(test)]
mod errors_tests;

#[cfg(test)]
mod shipped_xml_tests;

#[cfg(test)]
mod bottom_hud_tests;

/// The character window (C) on the installed interface.
#[cfg(test)]
mod character_window_tests;

/// Teardown at the character screen and a genuine second load at the next login.
#[cfg(test)]
mod world_entry_tests;

/// The guild tabard designer: the stock `TabardFrame.xml` off the chain.
#[cfg(test)]
mod tabard_tests;
/// The stock world map: its POI pool, unit blips, player arrow and full-screen quads.
#[cfg(test)]
mod world_map_tests;

#[cfg(test)]
mod seam_scale_tests {
    use super::seam_scale;

    #[test]
    fn seam_scale_is_the_768_base_times_the_dial() {
        // Identity at the design height, proportional elsewhere.
        assert_eq!(seam_scale(768.0, 1.0), 1.0);
        assert_eq!(seam_scale(1536.0, 1.0), 2.0);
        // The dial multiplies it.
        assert_eq!(seam_scale(768.0, 0.9), 0.9);
        // The reference's pixel-perfect setting: uiScale = 768/screenH → 1 px per UI unit.
        assert!((seam_scale(1080.0, 768.0 / 1080.0) - 1.0).abs() < 1e-6);
        // A degenerate (pre-winit) window is identity, never a division blow-up.
        assert_eq!(seam_scale(0.0, 0.9), 1.0);
    }
}

#[cfg(test)]
mod pointer_arbiter_tests {
    use super::*;

    /// A world to run the arbiter in: its inputs and its two outputs, nothing else.
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<PlayerUiHover>()
            .init_resource::<crate::vplates::PlateHover>()
            .init_resource::<PointerOverUi>()
            .init_resource::<PointerOverUiPanel>()
            .add_systems(Update, arbitrate_pointer_over_ui);
        app
    }

    #[test]
    fn a_hovered_plate_is_ui_but_not_chrome() {
        let mut app = app();
        let plate = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<PlayerUiHover>().0 = Some(7);
        app.world_mut()
            .resource_mut::<crate::vplates::PlateHover>()
            .0 = Some(plate);
        app.update();
        assert!(
            app.world().resource::<PointerOverUi>().0,
            "the plate takes the pointer"
        );
        assert!(
            !app.world().resource::<PointerOverUiPanel>().0,
            "…and the WHEEL looks straight through it"
        );
    }

    /// The camera reads the raw flag (`0x7662c0` gives a mouse-down to exactly one frame, the
    /// plate); the wheel reads chrome, walking past a frame that only takes the mouse.
    #[test]
    fn the_camera_yields_to_a_plate_and_the_wheel_does_not() {
        let mut app = app();
        let plate = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<PlayerUiHover>().0 = Some(7);
        app.world_mut()
            .resource_mut::<crate::vplates::PlateHover>()
            .0 = Some(plate);
        app.update();
        // `latch_world_mouse` reads this one: the press is not the world's, so no look starts.
        assert!(
            app.world().resource::<PointerOverUi>().0,
            "the camera must yield the press to the plate"
        );
        // …and `bindings`' wheel branch reads this one.
        assert!(
            !app.world().resource::<PointerOverUiPanel>().0,
            "the wheel must still reach the world over a plate"
        );
    }

    #[test]
    fn a_hovered_panel_is_both() {
        let mut app = app();
        app.world_mut().resource_mut::<PlayerUiHover>().0 = Some(7);
        app.update();
        assert!(app.world().resource::<PointerOverUi>().0);
        assert!(app.world().resource::<PointerOverUiPanel>().0);
    }

    #[test]
    fn no_hover_is_neither() {
        let mut app = app();
        app.update();
        assert!(!app.world().resource::<PointerOverUi>().0);
        assert!(!app.world().resource::<PointerOverUiPanel>().0);
    }
}
