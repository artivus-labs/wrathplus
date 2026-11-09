//! The stock glue interface: build 12340's `Interface\GlueXML\GlueXML.toc` over the chain, the
//! screens `GlueParent.lua` switches between (`AccountLogin`, `CharacterSelect`, …), on its own VM
//! outside the world. The loader is FrameXML's ([`reference_ui::core_at`]) on the glue toc.

use bevy::prelude::*;
use wrathplus_ui::script::{ScriptValue, UiScript};

use super::reference_ui;

mod host;
pub(crate) use host::stock_glue;
#[cfg(test)]
mod character_select_installed_tests;
#[cfg(test)]
mod click_tests;
#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod installed_tests;
#[cfg(test)]
mod measure_tests;
#[cfg(test)]
mod password_tests;

/// The glue load list, which `CGlueMgr` walks as `UI_Init` walks FrameXML's.
pub(super) const GLUE_TOC: &str = r"Interface\GlueXML\GlueXML.toc";

/// Load the chain's GlueXML onto `script`; returns every failure, as [`super::manifest::load_core`]
/// does for FrameXML.
pub(crate) fn load_glue_ui(script: &UiScript) -> Vec<String> {
    let Some(bytes) = reference_ui::read(GLUE_TOC) else {
        let e = format!("{GLUE_TOC}: not found — the stock glue interface is not built");
        error!("ui_script: {e}");
        return vec![e];
    };
    let core = reference_ui::core_at(GLUE_TOC, &wrathplus_ui::source::decode(&bytes));
    let mut toc = wrathplus_ui::status::Status::default();
    let mut failures = core.load_files_into(script, &core.toc.files, &mut toc);
    // Then AzerothBench's own additions (the Realmlist button), after the whole stock glue.
    let ours = super::addons::Addon::glue_additions();
    failures.extend(ours.load_files(script, &ours.toc.files));
    let mut log = wrathplus_ui::status::Status::default();
    toc.close_into(
        &mut log,
        script.framexml_debug(),
        wrathplus_ui::status::toc_banner(GLUE_TOC),
    );
    script.report_load_status(log);
    failures
}

/// Whether the stock GlueXML screens host the glue screens this run: wherever the install carries
/// `GlueXML.toc`, unless `WOW_NATIVE_GLUE=1` asks for the native Rust screens, the fallback kept
/// through the first live sessions and deleted then. With no install (a hermetic run) the native
/// screens host.
pub(crate) fn stock_glue_active() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var("WOW_NATIVE_GLUE").as_deref() != Ok("1")
            && reference_ui::read(GLUE_TOC).is_some()
    });
    *ON
}

/// The run condition of a native glue screen's systems: the stock screens are off.
pub(crate) fn native_glue() -> bool {
    !stock_glue_active()
}

/// UI input's run condition: the in-game UI in the world, or a screen the stock glue hosts.
pub(crate) fn ui_input_live(
    state: Option<Res<State<crate::app_core::state::ClientState>>>,
) -> bool {
    use crate::app_core::state::ClientState;
    let state = state.map(|s| *s.get());
    state == Some(ClientState::InWorld)
        || (stock_glue_active()
            && matches!(
                state,
                Some(ClientState::Login | ClientState::CharSelect | ClientState::CharCreate)
            ))
}

/// Run condition: the stock glue hosts the screen (login, realm list, character select and, with
/// Native UI's host, character create).
pub(crate) fn stock_glue_up(
    state: Option<Res<State<crate::app_core::state::ClientState>>>,
) -> bool {
    use crate::app_core::state::ClientState;
    stock_glue_active()
        && state.is_some_and(|s| {
            matches!(
                s.get(),
                ClientState::Login | ClientState::CharSelect | ClientState::CharCreate
            )
        })
}

/// The session of the glue VM in the world, if the VM there is one: kept across the glue screens,
/// as the reference's glue Lua state is; the world's own load replaces it at entry and a logout's
/// boot VM ends it, so the next glue screen builds a fresh one.
#[derive(Resource, Default)]
pub(crate) struct GlueVm(Option<u64>);

/// The glue VM in the world, building it if the VM there is not the glue's: a fresh VM with the
/// glue API, GlueXML loaded silenced (as the in-game load is), then GlueParent's start,
/// `FRAMES_LOADED`. Returns whether a glue VM is in place.
fn ensure_glue_vm(world: &mut World) -> bool {
    let current = world
        .get_non_send_resource::<UiScript>()
        .map(UiScript::session);
    if current.is_some() && world.get_resource::<GlueVm>().and_then(|g| g.0) == current {
        return true;
    }
    if !glue_entry_ready(world) {
        return false;
    }
    let Some(mut script) = super::lifecycle::new_vm(world) else {
        return false;
    };
    if let Err(e) = script.install_glue_api() {
        error!("ui_script: glue API install failed: {e}");
        return false;
    }
    super::lifecycle::seat_raster_seam_for_load(world, &mut script);
    script.set_glue_saved_account(&crate::login::load_saved_account());
    let server = world
        .get_resource::<crate::realmlist::Realmlist>()
        .map(|r| r.address().to_string())
        .unwrap_or_default();
    script.set_glue_connection(false, &server);
    script.set_glue_realmlist(&server);
    super::manifest::silenced_ui_load(&mut script, |script| {
        for failure in load_glue_ui(script) {
            warn!("ui_script: GlueXML load: {failure}");
        }
    });
    script.fire_event("FRAMES_LOADED", vec![]);
    world.insert_resource(GlueVm(Some(script.session())));
    world.insert_non_send_resource(script);
    true
}

/// Whether a glue VM may be built now: no startup cover is up, and it can measure text.
///
/// The startup cover holds until every pipeline is compiled (`loading_screen::startup`), and a
/// glue screen drawn under it would show through. The reference measures text synchronously, and
/// GlueXML sizes from it in `OnLoad` (`CharSelectChangeRealmButton`:
/// `SetWidth(GetTextWidth() + 50)`, `CharacterSelect.xml:475`), but the font atlas arrives an
/// `Update` after the first screen's `OnEnter`, once the window reports its scale. A windowed
/// client waits for it; a world with no window (a headless test) has no atlas to wait for.
fn glue_entry_ready(world: &mut World) -> bool {
    if world
        .get_resource::<crate::loading_screen::LoadingScreen>()
        .is_some_and(|s| s.startup_covering())
    {
        return false;
    }
    let windowed = world
        .query_filtered::<(), With<bevy::window::PrimaryWindow>>()
        .iter(world)
        .next()
        .is_some();
    !windowed || world.contains_resource::<crate::ui_text::UiFontAtlas>()
}

/// A glue screen entry deferred until a VM may be built ([`glue_entry_ready`]).
#[derive(Resource, Default)]
pub(crate) struct DeferredGlueEntry(Option<crate::app_core::state::ClientState>);

/// Finish a deferred entry once the measure is ready, for the screen the client is still on.
pub(crate) fn finish_deferred_entry(world: &mut World) {
    use crate::app_core::state::ClientState;
    let Some(wanted) = world.get_resource::<DeferredGlueEntry>().and_then(|d| d.0) else {
        return;
    };
    if !glue_entry_ready(world) {
        return;
    }
    world.resource_mut::<DeferredGlueEntry>().0 = None;
    let now = world.get_resource::<State<ClientState>>().map(|s| *s.get());
    match (wanted, now) {
        (ClientState::Login, Some(ClientState::Login)) => enter_stock_login(world),
        (ClientState::CharSelect, Some(ClientState::CharSelect)) => enter_stock_select(world),
        _ => {}
    }
}

/// Record an entry that could not build its VM yet.
fn defer_entry(world: &mut World, state: crate::app_core::state::ClientState) {
    if !glue_entry_ready(world) {
        world
            .get_resource_or_insert_with(DeferredGlueEntry::default)
            .0 = Some(state);
    }
}

/// `SET_GLUE_SCREEN name` on the glue VM in the world.
fn set_glue_screen(world: &mut World, name: &str) {
    if let Some(mut script) = world.get_non_send_resource_mut::<UiScript>() {
        script.fire_event("SET_GLUE_SCREEN", vec![ScriptValue::Str(name.into())]);
    }
}

/// Entering the login screen with the stock glue: the glue VM, then the engine's
/// `SET_GLUE_SCREEN "login"` and the login's backdrop, AccountLogin's ModelFFX pane sampling the
/// glue booth's `UI_MainMenu_Northrend` (`portrait::glue_booth`).
pub(crate) fn enter_stock_login(world: &mut World) {
    if !stock_glue_active() {
        return;
    }
    if !ensure_glue_vm(world) {
        return defer_entry(world, crate::app_core::state::ClientState::Login);
    }
    // Back at the login, the realm connection is gone.
    let address = world
        .get_resource::<crate::realmlist::Realmlist>()
        .map(|r| r.address().to_string())
        .unwrap_or_default();
    if let Some(mut script) = world.get_non_send_resource_mut::<UiScript>() {
        script.set_glue_connection(false, &address);
        script.set_glue_server_type(false, false);
    }
    set_glue_screen(world, "login");
    // `--fill-only`: the launch's credentials wait in the boxes for the player's Login.
    if let Some((account, password)) = crate::launch_plan::plan().fill() {
        if let Some(script) = world.get_non_send_resource::<UiScript>() {
            if let Err(e) = script.fill_login(account, password) {
                warn!("ui_script: --fill-only could not fill the login boxes: {e}");
            }
        }
    }
    if let Some(mut preview) = world.get_resource_mut::<crate::portrait::GluePreview>() {
        preview.scene = Some(crate::portrait::GlueScene::MainMenu);
        preview.look = None;
        preview.yaw = 0.0;
    }
    info!("ui_script: stock glue — AccountLogin up");
}

/// Entering character select with the stock glue: the glue VM (rebuilt after a logout), the rows
/// it reads, then `SET_GLUE_SCREEN "charselect"`; the stage comes from the stock Lua's own
/// `SetCharSelectBackground`.
pub(crate) fn enter_stock_select(world: &mut World) {
    if !stock_glue_active() {
        return;
    }
    if !ensure_glue_vm(world) {
        return defer_entry(world, crate::app_core::state::ClientState::CharSelect);
    }
    push_characters(world);
    // The character list arrived over the realm connection: `IsConnectedToServer` answers it
    // (stock shows Create New Character only connected, `UpdateCharacterList`) and
    // `GetServerName` the realm's name and type (`CharSelectRealmName`), not the logon address.
    let realm = world
        .get_resource::<crate::app_core::state::Roster>()
        .and_then(|r| r.realm.clone());
    let fallback = world
        .get_resource::<crate::realmlist::Realmlist>()
        .map(|r| r.address().to_string())
        .unwrap_or_default();
    if let Some(mut script) = world.get_non_send_resource_mut::<UiScript>() {
        match realm {
            Some(realm) => {
                let (pvp, rp) = crate::realm_select::load::pvp_rp(realm.realm_type);
                script.set_glue_connection(true, &realm.name);
                script.set_glue_server_type(pvp, rp);
            }
            None => script.set_glue_connection(true, &fallback),
        }
        show_stock_select(&mut script);
    }
    info!("ui_script: stock glue — CharacterSelect up");
}

/// Character select comes up: the login's connect status dialog ("Connecting", "Handshaking", …)
/// goes first, then `SET_GLUE_SCREEN "charselect"`. Arriving here means the login is past
/// connecting, as the realm list's arrival does (`host::stock_glue`); with a remembered realm
/// that list never opens, and the dialog stayed up over the characters. Inferred: 12340 fires
/// `CLOSE_STATUS_DIALOG` (glue event 5, table `0xb6aff8`) by index through `0x4da5f0`'s callers,
/// and its fire site on this path is not traced.
pub(crate) fn show_stock_select(script: &mut UiScript) {
    script.fire_event("CLOSE_STATUS_DIALOG", vec![]);
    script.fire_event(
        "SET_GLUE_SCREEN",
        vec![ScriptValue::Str("charselect".into())],
    );
}

/// The roster's rows onto the glue VM.
fn push_characters(world: &mut World) {
    let rows = {
        let roster = world.resource::<crate::app_core::state::Roster>();
        crate::char_select::glue_rows::glue_characters(
            roster,
            world.get_resource::<crate::area::AreaTableRes>(),
        )
    };
    if let Some(mut script) = world.get_non_send_resource_mut::<UiScript>() {
        script.set_glue_characters(rows, crate::char_select::glue_rows::EMPTY_SELECT_STAGE);
    }
}
