//! The registry a built client holds is the shipped table ([`crate::shipped_cvars`]): the core
//! rows and every feature's own, each once and at its default. The features register their rows
//! and their `on_cvar` observers in their own plugins, so both orders follow the plugins'; these
//! pin what that order may change and what it may not.

use bevy::ecs::observer::Observer;

use crate::cvars::Cvars;

/// The latched rows in the order [`Cvars::commit_latched`] fires them: the sound-init row, then
/// the `gx*` block `RestartGx` commits, as the core table listed them before the features owned
/// their rows.
const LATCHED_ORDER: [&str; 13] = [
    "SoundBufferSize",
    "gxApi",
    "gxVSync",
    "gxWindow",
    "gxResolution",
    "gxMultisample",
    "gxColorBits",
    "gxDepthBits",
    "gxMaximize",
    "gxTripleBuffer",
    "gxCursor",
    "gxFixLag",
    "gxRefresh",
];

#[test]
fn a_built_client_registers_exactly_the_shipped_cvar_rows() {
    let app = crate::game_plugins::schedule_tests::headless_client();
    let cvars = app.world().resource::<Cvars>();
    let row =
        |name: &str, default: &str, latched: bool| (name.to_string(), default.to_string(), latched);
    let mut built: Vec<_> = cvars
        .rows()
        .filter(|r| !r.addon)
        .map(|r| row(&r.name, &r.default, r.latched))
        .collect();
    let mut shipped: Vec<_> = crate::shipped_cvars::rows()
        .map(|r| row(r.name, r.default, r.latched))
        .collect();
    built.sort();
    shipped.sort();
    assert_eq!(
        built, shipped,
        "the client registers the shipped table, no more, no less"
    );
    assert_eq!(shipped.len(), 156, "the shipped row count moved");

    let latched: Vec<&str> = cvars
        .rows()
        .filter(|r| r.latched)
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(
        latched, LATCHED_ORDER,
        "the latched rows commit in their old order"
    );
}

/// The `on_cvar` observers, in the order their plugins registered them: each feature attaches
/// its own, and a registration-order change would reorder the hooks a change fires. Compared by
/// module path: the crate prefix names whichever layer crate the hook lives in.
///
/// Dev builds only: Bevy keeps system and observer names only with its `debug` feature, which only
/// the `dev` build turns on (through `wrathplus-world/dev` and `bevy_egui`); a player build reads
/// empty names, so there is nothing to compare and the test returns at once.
#[test]
fn the_cvar_hooks_register_in_plugin_order() {
    if !crate::run_mode::dev_affordances() {
        return;
    }
    let mut app = crate::game_plugins::schedule_tests::headless_client();
    let world = app.world_mut();
    let mut hooks: Vec<(u32, String)> = world
        .query::<(bevy::prelude::Entity, &Observer)>()
        .iter(world)
        .map(|(e, o)| (e.index_u32(), o.system_name().to_string()))
        .filter(|(_, name)| name.ends_with("::on_cvar"))
        .collect();
    hooks.sort();
    let names: Vec<&str> = hooks
        .iter()
        .map(|(_, n)| n.split_once("::").map_or(n.as_str(), |(_, path)| path))
        .collect();
    assert_eq!(
        names, HOOK_ORDER,
        "the CVar hooks' registration order moved"
    );
}

/// The order before the rows moved: no `add_observer` call moved with them. `perf`'s hook is
/// absent because the headless client builds no `dev` tools plugin.
const HOOK_ORDER: &[&str] = &[
    "player::approach::on_cvar",
    "player::camera::on_cvar",
    "player::camera_dynamics::on_cvar",
    "sound::cvars::on_cvar",
    "target::on_cvar",
    "ui_gamma::on_cvar",
    "world_backdrop::on_cvar",
    "minimap::on_cvar",
    "portrait::on_cvar",
    "ui_script::on_cvar",
    "video::on_cvar",
    "realmlist::on_cvar",
    "game_tip::on_cvar",
    "text_filter::on_cvar",
    "ui_guild::on_cvar",
    "spell::auto_ranged::on_cvar",
    "spell::cast_target::on_cvar",
    "spell::secure_toggle::on_cvar",
    "combat_text::on_cvar",
    "nameplates::on_cvar",
    "vplates::on_cvar",
    "chat_bubble::on_cvar",
    "ui_trade::on_cvar",
    "ui_loot::on_cvar",
    "ui_chat::combat::on_cvar",
];

/// The `$WOW_*` levers a built client marks session-owned, in the order [`load_config`] reads
/// them: the registry's own (`CORE_LEVERS`), then each feature's in plugin order. Every row is
/// owned on its own, so the order fixes only the log's; the rows are what `session_values` read
/// before the features registered their levers.
///
/// [`load_config`]: crate::cvars
const LEVERS: [(&str, &[&str]); 10] = [
    ("WOW_FARCLIP", &["farclip"]),
    ("WOW_CLUTTER_DENSITY", &["WorldDetail", "frillDensity"]),
    ("WOW_TRILINEAR", &["trilinear"]),
    ("WOW_ANISO", &["anisotropic"]),
    ("WOW_MSAA", &["gxMultisample"]),
    ("WOW_RENDER_SCALE", &["renderScale"]),
    ("WOW_UI_SCALE", &["uiScale"]),
    ("novsync_env", &["gxVSync"]),
    ("windowed_env", &["gxWindow", "gxMaximize", "gxResolution"]),
    ("WOW_HOST", &["realmList"]),
];

#[test]
fn a_built_client_marks_the_same_session_levers() {
    let app = crate::game_plugins::schedule_tests::headless_client();
    // A lever's rows, read off a world without its knob: names only.
    let bare = bevy::ecs::world::World::new();
    let registered = &app.world().resource::<crate::cvars::SessionLevers>().0;
    let built: Vec<(&str, Vec<&str>)> = crate::cvars::CORE_LEVERS
        .iter()
        .chain(registered)
        .map(|l| {
            (
                l.name,
                (l.read)(&bare)
                    .into_iter()
                    .map(|(n, v)| {
                        assert!(v.is_none(), "{n}: no knob, no value");
                        n
                    })
                    .collect(),
            )
        })
        .collect();
    let expected: Vec<(&str, Vec<&str>)> =
        LEVERS.iter().map(|(n, rows)| (*n, rows.to_vec())).collect();
    assert_eq!(built, expected, "the levers and their order");
    // Every lever row is a shipped row.
    let cvars = app.world().resource::<Cvars>();
    for (_, rows) in &built {
        for row in rows {
            assert!(cvars.get(row).is_some(), "{row} is registered");
        }
    }
}
