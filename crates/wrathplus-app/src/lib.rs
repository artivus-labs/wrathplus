//! `wrathplus`: a from-scratch World of Warcraft 1.12.1 client on Bevy. It opens the patch chain from
//! the install (`$WOW_DATA`, the project folder on a dev build, else beside the binary), streams
//! the world around the player through the `wrathplus-assets` `mpq://` pipeline, and runs the network
//! session on a background thread ([`net`]).
//!
//! The world is loaded when a character enters it and released when they leave; the glue screens
//! have no world behind them, so with no server the client sits at the login screen, as the
//! reference does. The capture harness (`$WOW_CAPTURE`) boots straight in-world.

// A player build's dead code and instrument-only re-exports are the seam working; a `cfg` per item
// would spread seam knowledge into gameplay modules. Dev builds warn normally.
#![cfg_attr(not(feature = "dev"), allow(dead_code, unused_imports))]

/// The realtime-audio allocation tripwire: in debug builds an allocation inside `sound::output`'s
/// `no_alloc` scopes (the output IO callback, the render pass) aborts the process; release builds
/// carry no wrapper.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

pub use wrathplus_app_core::app_core;
pub use wrathplus_app_core::area;
pub use wrathplus_app_core::chr_classes;
pub use wrathplus_app_core::crash;
pub use wrathplus_app_core::cvars;
pub use wrathplus_app_net::area_trigger;
pub use wrathplus_app_net::creature_type;
pub use wrathplus_app_tools::addon_harness;
#[cfg(feature = "dev")]
pub use wrathplus_app_tools::capture;
#[cfg(feature = "dev")]
pub use wrathplus_app_tools::debug_panel;
/// The dev/player seam: the instrument groups and the boundary rule. Always compiled; what it holds
/// is not.
pub use wrathplus_app_tools::dev;
pub use wrathplus_app_ui::bindings;
pub use wrathplus_app_ui::char_create;
pub use wrathplus_app_ui::char_select;
pub use wrathplus_app_ui::combat_log;
pub use wrathplus_app_ui::console;
pub use wrathplus_app_ui::experience_ui;
pub use wrathplus_app_world::area_poi;
#[cfg(feature = "dev")]
pub use wrathplus_app_world::asset_churn;
pub use wrathplus_app_world::aura_visual;
pub use wrathplus_app_world::blob_shadow;
pub use wrathplus_app_world::bowstring;
pub use wrathplus_app_world::camera_shake;
pub use wrathplus_app_world::chat_bubble;
pub use wrathplus_app_world::cinematic;
pub use wrathplus_app_world::combat_text;
pub use wrathplus_app_world::creature_anim;
pub use wrathplus_app_world::cursor;
pub use wrathplus_app_world::death;
pub use wrathplus_app_world::doodad_events;
pub use wrathplus_app_world::entities;
pub use wrathplus_app_world::experience;
pub use wrathplus_app_world::experience_ao;
pub use wrathplus_app_world::fishing_line;
pub use wrathplus_app_world::footprints;
mod game_plugins;
pub use wrathplus_app_core::glue_strings;
pub use wrathplus_app_core::launch_plan;
pub use wrathplus_app_core::local_state;
pub use wrathplus_app_core::pending_item_ops;
pub use wrathplus_app_core::query_cache;
pub use wrathplus_app_core::run_mode;
pub use wrathplus_app_net::go_templates;
pub use wrathplus_app_net::items;
pub use wrathplus_app_net::names;
pub use wrathplus_app_net::net;
pub use wrathplus_app_net::realmlist;
#[cfg(feature = "dev")]
pub use wrathplus_app_tools::hover_log;
/// Ships in part (the FPS journal and the clocks it reads); the rest is `dev`.
pub use wrathplus_app_tools::perf;
#[cfg(feature = "dev")]
pub use wrathplus_app_tools::preflight;
#[cfg(feature = "dev")]
pub use wrathplus_app_tools::probe_shield;
pub use wrathplus_app_ui::glue;
pub use wrathplus_app_ui::login;
pub use wrathplus_app_ui::npc_text;
pub use wrathplus_app_ui::realm_select;
pub use wrathplus_app_ui::script_calls;
pub use wrathplus_app_world::go_anim;
pub use wrathplus_app_world::guild;
pub use wrathplus_app_world::inventory;
pub use wrathplus_app_world::loading_screen;
pub use wrathplus_app_world::loot;
pub use wrathplus_app_world::minimap;
pub use wrathplus_app_world::motion;
pub use wrathplus_app_world::nameplates;
pub use wrathplus_app_world::overlay;
pub use wrathplus_app_world::pipe_warm;
pub use wrathplus_app_world::player;
pub use wrathplus_app_world::poi_marker;
pub use wrathplus_app_world::portrait;
pub use wrathplus_app_world::quest_markers;
pub use wrathplus_app_world::raid_marks;
pub use wrathplus_app_world::ranged_flex;
pub use wrathplus_app_world::screen_fade;
pub use wrathplus_app_world::screenshot;
pub use wrathplus_app_world::shaders;

/// Where "the client is going down" is observed, in `Last`; every system that persists state on the
/// way out registers through it.
pub use wrathplus_app_core::shutdown;
pub use wrathplus_app_net::name_persist;
#[cfg(feature = "dev")]
pub use wrathplus_app_tools::showcase_capture;
pub use wrathplus_app_ui::shipped_cvars;
pub use wrathplus_app_world::game_tip;
pub use wrathplus_app_world::opaque2d;
pub use wrathplus_app_world::smart_rect;
pub use wrathplus_app_world::sound;
/// The two talent spell-modifier tables (`SMSG_SET_FLAT_/PCT_SPELL_MODIFIER`) and the read that
/// puts them on a number.
pub use wrathplus_app_world::spell;
/// The melee swing refusal's latch + 4 s repeat (`SMSG_ATTACKSWING_*`).
pub use wrathplus_app_world::swing_refusal;
pub use wrathplus_app_world::target;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support;
pub use wrathplus_app_core::text_filter;
pub use wrathplus_app_core::text_reshape;
pub use wrathplus_app_ui::textinput;
pub use wrathplus_app_ui::tutorial;
pub use wrathplus_app_ui::ui_achievements;
pub use wrathplus_app_ui::ui_action;
pub use wrathplus_app_ui::ui_arena_team;
pub use wrathplus_app_ui::ui_auction;
pub use wrathplus_app_ui::ui_aura;
pub use wrathplus_app_ui::ui_bank;
pub use wrathplus_app_ui::ui_battlefield;
pub use wrathplus_app_ui::ui_battlefield_arena_lines;
pub use wrathplus_app_ui::ui_battlefield_positions;
pub use wrathplus_app_ui::ui_battlefield_score;
pub use wrathplus_app_ui::ui_bind_confirm;
pub use wrathplus_app_ui::ui_binder;
pub use wrathplus_app_ui::ui_calendar;
pub use wrathplus_app_ui::ui_cast;
pub use wrathplus_app_ui::ui_char;
pub use wrathplus_app_ui::ui_chat;
pub use wrathplus_app_ui::ui_chat_verbs;
pub use wrathplus_app_ui::ui_guild_bank;
pub use wrathplus_app_ui::ui_lfg;
pub use wrathplus_app_ui::ui_native;
pub use wrathplus_app_ui::ui_native_chat;
pub use wrathplus_app_world::transport;
// The native pet-name and WHO capture FIFOs live in `net` (split step 0 residue); the old paths stay.
use net::{wrath_pet_name_capture as ui_native_pet_name, wrath_who_capture as ui_native_who};
pub use wrathplus_app_core::quest_reputation;
pub use wrathplus_app_core::wrath_vehicle_seats;
pub use wrathplus_app_ui::quest_rewards;
pub use wrathplus_app_ui::ui_companions;
pub use wrathplus_app_ui::ui_death;
pub use wrathplus_app_ui::ui_dialog_verbs;
pub use wrathplus_app_ui::ui_dressup;
pub use wrathplus_app_ui::ui_duel;
pub use wrathplus_app_ui::ui_follow;
pub use wrathplus_app_ui::ui_gamma;
pub use wrathplus_app_ui::ui_glyphs;
pub use wrathplus_app_ui::ui_gm_ticket;
pub use wrathplus_app_ui::ui_gossip;
pub use wrathplus_app_ui::ui_guild;
pub use wrathplus_app_ui::ui_honor;
pub use wrathplus_app_ui::ui_inspect;
pub use wrathplus_app_ui::ui_inspect_wrath;
pub use wrathplus_app_ui::ui_instance;
pub use wrathplus_app_ui::ui_item_text;
pub use wrathplus_app_ui::ui_items;
pub use wrathplus_app_ui::ui_layout;
pub use wrathplus_app_ui::ui_logout;
pub use wrathplus_app_ui::ui_loot;
pub use wrathplus_app_ui::ui_loot_roll;
pub use wrathplus_app_ui::ui_macro;
pub use wrathplus_app_ui::ui_mail;
pub use wrathplus_app_ui::ui_merchant;
pub use wrathplus_app_ui::ui_mirror;
pub use wrathplus_app_ui::ui_models;
pub use wrathplus_app_ui::ui_net;
pub use wrathplus_app_ui::ui_paperdoll;
pub use wrathplus_app_ui::ui_party;
pub use wrathplus_app_ui::ui_pass;
pub use wrathplus_app_ui::ui_pet;
pub use wrathplus_app_ui::ui_pet_book;
pub use wrathplus_app_ui::ui_pet_doll;
pub use wrathplus_app_ui::ui_pet_stats;
pub use wrathplus_app_ui::ui_pet_talent_tree;
pub use wrathplus_app_ui::ui_petition;
pub use wrathplus_app_ui::ui_quest;
pub use wrathplus_app_ui::ui_quest_log;
pub use wrathplus_app_ui::ui_quest_share;
pub use wrathplus_app_ui::ui_raid_vehicles;
pub use wrathplus_app_ui::ui_reputation;
pub use wrathplus_app_ui::ui_runes;
pub use wrathplus_app_ui::ui_saved;
pub use wrathplus_app_ui::ui_script;
pub use wrathplus_app_ui::ui_session;
pub use wrathplus_app_ui::ui_shapeshift;
pub use wrathplus_app_ui::ui_social;
pub use wrathplus_app_ui::ui_spellbook;
pub use wrathplus_app_ui::ui_stable;
pub use wrathplus_app_ui::ui_static_popup;
pub use wrathplus_app_ui::ui_summon;
pub use wrathplus_app_ui::ui_tabard;
pub use wrathplus_app_ui::ui_talent;
pub use wrathplus_app_ui::ui_talent_wipe;
pub use wrathplus_app_ui::ui_taxi;
pub use wrathplus_app_ui::ui_tooltip;
pub use wrathplus_app_ui::ui_totems;
pub use wrathplus_app_ui::ui_trade;
pub use wrathplus_app_ui::ui_tradeskill;
pub use wrathplus_app_ui::ui_trainer;
pub use wrathplus_app_ui::ui_unit;
pub use wrathplus_app_ui::ui_unit_popup;
pub use wrathplus_app_ui::ui_vehicle;
pub use wrathplus_app_ui::ui_world_map;
pub use wrathplus_app_ui::world_state_ui;
pub use wrathplus_app_ui::wrathplus_pack;
pub use wrathplus_app_world::ui_text;
pub use wrathplus_app_world::video;
pub use wrathplus_app_world::vplates;
pub use wrathplus_app_world::weapon_trail;
pub use wrathplus_app_world::world_backdrop;
pub use wrathplus_app_world::world_state;

use bevy::prelude::*;

// A launcher stamps the build id at compile time and hands it to [`run`] or [`run_with`];
// re-exported so the `wrathplus` shim needs no bevy dependency of its own.
pub use bevy::app::AppExit;
pub use wrathplus_world::build_id::BuildId;
/// The world viewer's entry point, the engine with no game attached, called by the
/// `wrathplus-worldview` shim.
pub use wrathplus_world::worldview::run as run_worldview;
// A crate on top of WrathPlus reads objects and interacts through these, with no protocol dependency.
pub use net::Objects;
pub use target::Interact;
/// The session interface load, for the integration tests' kit (`tests/common`), which cannot
/// reach this crate's `#[cfg(test)]` loaders. Not an API.
#[doc(hidden)]
pub use ui_script::load_default_ui;
pub use wrathplus_protocol::{types::ObjectType, ObjectFields};

/// Builds and runs the client app. `build` is the launcher's compile-time git stamp, passed in as
/// data so the sha lives in the shim's fingerprint and a commit does not recompile this crate.
pub fn run(build: BuildId) -> AppExit {
    launch(build, None)
}

/// [`run`], plus `extend` adding a crate's own plugins: how a feature 1.12.1 lacks is built on top
/// of WrathPlus (`examples/extended_launcher.rs`), the build marked `extended` wherever it is named.
/// `extend` gets the client fully built and runs before the probe fleet, which sees what it adds.
pub fn run_with(build: BuildId, extend: impl FnOnce(&mut App)) -> AppExit {
    launch(build, Some(Box::new(extend)))
}

/// What a crate on top of WrathPlus adds to the built app ([`run_with`]).
type Extension<'a> = Box<dyn FnOnce(&mut App) + 'a>;

/// The one body behind [`run`] and [`run_with`], not generic, so a launcher compiles none of it.
fn launch(build: BuildId, extend: Option<Extension<'_>>) -> AppExit {
    // First: `run_mode`'s pre-plugin answers below read the capture harness through it.
    dev::install_run_mode_seam();
    // The command line, before anything reads the plan; a refused one prints why and exits.
    if let Err(why) = launch_plan::install_from_process() {
        eprintln!("{why}");
        return AppExit::error();
    }
    // `WOW_HOVER_LOG_REPORT=<csv>` and `WOW_CAPTURE=list` print and exit before any setup.
    if let Ok(path) = std::env::var("WOW_HOVER_LOG_REPORT") {
        dev::report_recorded_hover_log(&path);
        return AppExit::Success;
    }
    if std::env::var("WOW_CAPTURE").as_deref() == Ok("list") {
        dev::print_scenario_names();
        return AppExit::Success;
    }
    // `WOW_PROBE=list` likewise prints the probe environment registry.
    if std::env::var("WOW_PROBE").as_deref() == Ok("list") {
        dev::print_probe_vars();
        return AppExit::Success;
    }

    // Every line that names the build says whether a crate on top extended it.
    let build = BuildId {
        extended: extend.is_some(),
        ..build
    };

    // A dev build's install, state folder and probe identity resolve from the launcher's folder,
    // so a crate on top of WrathPlus keeps them in its own and not in cargo's checkout of wrathplus.
    wrathplus_formats::set_project_folder(build.project_dir);

    // From here a panic leaves `wrathplus-config/Diagnostics/crash-<unix>.txt`; armed before the
    // `App` exists, so a panic while plugins build is reported too.
    crash::install(build);

    let mut app = App::new();
    // The panel footer and the build banner read the stamp back.
    app.insert_resource(build);
    let experience = experience::boot();
    let packs = experience.asset_overrides();
    let overrides = wrathplus_pack::mount(packs.expect("valid enabled asset packs"));
    ui_script::initialize_asset_overrides(overrides.clone());
    app.insert_resource(wrathplus_assets::AssetOverridePolicy(overrides.clone()));
    app.insert_resource(experience_ao::AoBootPolicy::from_experience(&experience));
    app.insert_resource(experience);
    app.add_systems(PostUpdate, experience_ao::sync_camera);
    app.init_resource::<wrathplus_world::modern_lighting::ModernLighting>();
    app.add_systems(
        PostUpdate,
        experience::sync_lighting
            .before(wrathplus_world::modern_shadows::ShadowSyncSet)
            .before(bevy::transform::TransformSystems::Propagate),
    );
    app.add_systems(
        Update,
        experience_ui::sync_text
            .after(ui_script::UiFeed)
            .before(ui_script::UiInput),
    );
    // Static-scene transform tracking pinned on: the default threshold re-decides every frame with
    // two full scans of the rows the tracking exists to skip, and this scene is static-heavy. It
    // costs more only when most rows move in one frame (a load burst into a near-empty world).
    app.insert_resource(bevy::transform::systems::StaticTransformOptimizations::enabled());
    // Update runs single-threaded: most per-frame Update systems are non-Send (mlua's Lua VM,
    // kira's audio handles) and serialize through the multi-threaded executor anyway, so its
    // dispatch is pure overhead (measured faster, tails flat). `WOW_MT_UPDATE=1` restores it.
    if std::env::var_os("WOW_MT_UPDATE").is_none() {
        app.edit_schedule(Update, |s| {
            s.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
        });
    }
    // PostUpdate too: 208 systems paid about 10 µs each of multi-threaded dispatch, and
    // single-threaded measured faster parked and in motion. `WOW_MT_POSTUPDATE=1` restores it; the
    // log line names the config a run measured.
    if std::env::var_os("WOW_MT_POSTUPDATE").is_none() {
        app.edit_schedule(PostUpdate, |s| {
            s.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
        });
        println!(
            "executor: PostUpdate -> single-threaded (the default; WOW_MT_POSTUPDATE=1 for MT)"
        );
    }
    // The build banner ships in every build: which build produced a log is the first thing a report
    // from another machine must establish.
    app.add_systems(Startup, wrathplus_world::build_id::banner);

    // With `$WOW_CAPTURE` set the app runs a deterministic, server-less capture (net off, so no
    // NPCs stream in) and exits.
    #[cfg(feature = "dev")]
    if let Err(error) = showcase_capture::driver::prepare(&mut app) {
        eprintln!("showcase: {error}");
        return AppExit::error();
    }
    let capturing = run_mode::scenario_active();
    // Every instrumented run, captures and live probes, opens in the background so it never takes
    // over a person's screen; `WOW_BG` overrides.
    let background = wrathplus_world::bgwin::background_run();
    if capturing {
        // Ground clutter scatters with per-run randomness, so captures turn it off for byte-stable
        // baselines; set before plugins build so `ClutterConfig::from_env` reads it. The frame
        // clock, frozen in `capture`, is the other source of drift.
        std::env::set_var("WOW_CLUTTER_DENSITY", "0");
        // Anim-LOD park/wake (`creature_anim::lod::gate_rig_animation`) hangs on asset-load timing,
        // which the frozen clock does not control, so captures turn it off unless overridden. A rig
        // in frame should never be parked, so the shot keeps its subject.
        if std::env::var("WOW_NO_ANIM_LOD").is_err() {
            std::env::set_var("WOW_NO_ANIM_LOD", "1");
        }
    }

    // The `mpq://` source registers before `AssetPlugin` builds, on `wrathplus_formats::wow_data`'s
    // install (none: terrain loads fail gracefully); `[data] nonstock_patches` may widen its chain.
    wrathplus_app_core::data_config::apply_nonstock_patches();
    let data_profile = wrathplus_formats::DataProfile::from_env()
        .expect("valid explicit WOW_CLIENT_PROFILE/WOW_LOCALE configuration");
    eprintln!("wrathplus: client data profile={data_profile:?}");
    // Profile routing must not depend on a readable install: in particular, an unavailable Wrath
    // chain must still never fall through to the Vanilla FrameXML/Lua host.
    app.insert_resource(wrathplus_assets::ClientDataProfile(data_profile.clone()));
    match wrathplus_formats::wow_data() {
        Some(data_dir) => {
            if let Err(e) = wrathplus_assets::register_mpq_source_overrides(
                &mut app,
                &data_dir,
                &data_profile,
                overrides,
            ) {
                eprintln!("wrathplus-assets: mpq:// source unavailable ({e:#})");
            }
        }
        None => eprintln!(
            "wrathplus: no WoW install found — looked in {:?}",
            wrathplus_formats::candidates()
        ),
    }

    // No `game://` source: the five UI shaders are compiled in by `crate::shaders`
    // (`embedded://<its crate>/shaders/…`), so no build-machine path reaches the binary.

    app.add_plugins(wrathplus_world::boot::tuned_default_plugins(Window {
        title: "wrathplus".into(),
        // Born in the player's display mode (`gxWindow` read straight off `config.toml`) rather
        // than flipped into it at `Startup`, which would flash on every launch and, under
        // gamescope, spend the first second in the input state fullscreen is meant to end.
        // Instrumented runs stay windowed (`video::windowed_env`).
        mode: video::boot_window_mode(),
        // UI-fixture captures size the window to what they photograph: the action bar's 1024 px
        // plus 128 px end caps get a wide, short window, and vplates pins 1024×768, where one gx
        // unit is 1280 px and the plate lands at the border texture's native 128×32, diffable
        // against the BLP.
        resolution: if run_mode::showcase_active() {
            bevy::window::WindowResolution::new(1920, 1080).with_scale_factor_override(1.0)
        } else {
            video::at_requested_dpi(
                // Same opt-in as the UI load (`ui_script::lifecycle::ui_wanted`); the two must agree,
                // or the right content is captured at the wrong size.
                if capturing && crate::run_mode::capture_ui_opted_in() {
                    // `$WOW_WIN` overrides here too, for scale-dependent UI bugs.
                    if let Some(win) = video::requested_window_size() {
                        win
                    } else {
                        match std::env::var("WOW_CAPTURE").as_deref() {
                            Ok("ui-actionbar") => UVec2::new(1300, 260),
                            Ok("vplates") => UVec2::new(1024, 768),
                            // Short enough for the action bar to overlap the chat edit box.
                            Ok("ui-chatedit") => UVec2::new(566, 377),
                            // The map's chrome is a centered 1024×768 block, with margin.
                            Ok("ui-worldmap") => UVec2::new(1100, 800),
                            _ => UVec2::new(640, 700),
                        }
                    }
                    .into()
                } else {
                    // `$WOW_WIN=WxH` (logical px) overrides the size. FFXGlow's blur is pinned in
                    // texels, so thin bright features self-amplify at high resolution; matching the
                    // reference's pixel density (`WOW_WIN=512x288` on a 2× display is 1024×576)
                    // isolates that term.
                    video::requested_window_size()
                        // A run that reads no pixels gets a small window: it is held `AlwaysOnTop`
                        // against the occlusion throttle (`capture::ProbeFocusPlugin`), and small and
                        // cornered keeps it out of the way. Pixel runs keep the full size.
                        .unwrap_or(if wrathplus_world::bgwin::no_pixel_run() {
                            UVec2::new(640, 360)
                        } else {
                            // The player's `gxResolution`; `bevy_winit` ignores it while fullscreen.
                            video::boot_windowed_size()
                        })
                        .into()
                },
            )
        },
        // `$WOW_NOVSYNC=1` uncaps presentation from boot so a headless FPS-journal run measures
        // frame cost, not the vsync ceiling; otherwise the player's `gxVSync` takes over from
        // `Startup` ([`crate::video`]).
        present_mode: video::present_mode(!video::novsync_env()),
        // A background run opens unfocused, so it cannot take keystrokes meant for another app, and
        // is born `AlwaysOnBottom` (`kCGNormalWindowLevel - 1`), so winit's two raises before the
        // first frame cannot flash it on top. `BgWinPlugin` promotes it to Normal once the launch
        // settles, so it can still be raised.
        focused: !background,
        window_level: if background {
            bevy::window::WindowLevel::AlwaysOnBottom
        } else {
            bevy::window::WindowLevel::Normal
        },
        ..default()
    }))
    .add_plugins(wrathplus_world::thread_qos::ThreadQosPlugin)
    // Undoes winit's forced macOS app activation for background runs; the window-side half is the
    // `Window` above.
    .add_plugins(wrathplus_world::bgwin::BgWinPlugin)
    // macOS `Cmd+Q` goes straight to `terminate:`, which never runs another frame, so the session
    // was never written; re-pointed at the window close, which [`shutdown`] sees.
    .add_plugins(wrathplus_world::mac_quit::MacQuitPlugin)
    // The engine as one group; `world_plugins.rs` has its load-bearing ordering edges and what it
    // leaves out (`pipe_warm`).
    .add_plugins(wrathplus_world::world_plugins::WorldPlugins)
    // The instruments, which the engine group does not carry; `--no-default-features` compiles them
    // all out (see `dev.rs`).
    .add_plugins(dev::DevToolsPlugin)
    // The FPS journal ships: `/console fpsJournal 1` appends a per-second row of position, frame
    // cost and the GPU's per-pass split to `wrathplus-config/Diagnostics/fps-journal.csv`;
    // `WOW_FPS_JOURNAL=<csv>` is the harness lever.
    .add_plugins(perf::FpsJournalPlugin)
    // The game as one group on top of the engine; `game_plugins.rs` has its members, its
    // load-bearing ordering edges and the test that builds it headless.
    .add_plugins(game_plugins::GamePlugins {
        connect: !run_mode::offline_active(),
        start: run_mode::start_state(),
    });

    // wrathplus-assets' loaders go into the live `AssetServer`, so they register after `AssetPlugin`.
    wrathplus_assets::register_asset_loaders(&mut app);

    // `ExtractSchedule` runs single-threaded too: bevy_render leaves it multi-threaded, with no
    // non-Send member, and single-threaded measured faster. `WOW_MT_EXTRACT=1` restores it.
    //
    // `Render` stays multi-threaded: under pipelined rendering that executor is also bevy's route
    // for non-Send systems to the main thread, and `create_surfaces` needs it because macOS makes a
    // Metal layer only on the UI thread (the single-threaded executor panics at startup).
    //
    // Set here because the render sub-app exists only once the plugin chain has built.
    if std::env::var_os("WOW_MT_EXTRACT").is_none() {
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.edit_schedule(bevy::render::ExtractSchedule, |s| {
                s.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
            });
            println!(
                "executor: ExtractSchedule -> single-threaded (the default; WOW_MT_EXTRACT=1 for MT)"
            );
        } else {
            // A missing sub-app would silently flip nothing.
            eprintln!("executor: no render app — ExtractSchedule flip NOT applied");
        }
    }

    // A crate on top of WrathPlus adds its plugins here ([`run_with`]).
    if let Some(extend) = extend {
        extend(&mut app);
    }

    // The probe fleet, last so it observes the fully-built app; compiled out by
    // `--no-default-features`.
    app.add_plugins(dev::DevProbesPlugin);

    // Returns the app's exit status: a failed capture sets `AppExit::error()`
    // (`capture::drive_capture`), which must not exit 0 with no PNG on disk.
    app.run()
}
