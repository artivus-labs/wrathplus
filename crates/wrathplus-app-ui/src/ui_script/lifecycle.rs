//! The UI session's lifecycle, all edges and nothing per-frame: `Startup` installs a boot VM
//! (strings and emote tokens); world entry builds a fresh VM and loads the in-game UI and every
//! addon onto it; leaving the world runs the reference's shutdown tail and installs a boot
//! VM again; `ReloadUI()` is the two edges back to back without leaving the world.

use bevy::prelude::*;

use wrathplus_assets::LockRecover;
use wrathplus_ui::script::UiScript;

use super::manifest::silenced_ui_load;
use super::{CursorPayloadHeld, PlayerUiHover, UiClock, UiKeyboardCapture};
use crate::ui_script::addons;

pub(crate) fn setup_script(world: &mut World) {
    install_boot_vm(world);
}

/// Build and install a boot VM, the one the world holds outside an in-game UI session: the
/// strings the feeds resolve lines from and the emote tokens `/`-commands are built from
/// (`ui_chat::commands`), no frames and no font objects. Installed at `Startup` and at every
/// [`end_ui_session`].
pub(super) fn install_boot_vm(world: &mut World) {
    let Some(script) = new_vm(world) else {
        return;
    };
    world.insert_non_send_resource(script);
}

/// A fresh VM on the process clock with the addon asset probes wired, or `None` (the world's VM
/// removed) when the VM cannot start.
pub(super) fn new_vm(world: &mut World) -> Option<UiScript> {
    let created = UiScript::new();
    let mut script = match created {
        Ok(s) => s,
        Err(e) => {
            error!("ui_script: VM init failed: {e}");
            world.remove_non_send_resource::<UiScript>();
            return None;
        }
    };
    seed_vm_clock(world, &mut script);
    crate::ui_spellbook::seed_spell_catalog(world, &mut script);
    crate::ui_achievements::seed(world, &mut script);
    seed_class_names(world, &mut script);
    seed_link_catalogs(world, &mut script);
    seed_item_icons(world, &mut script);
    seed_session(world, &mut script);
    install_addon_asset_resolvers(world, &mut script);
    info!("ui_script: new VM, session {}", script.session());
    Some(script)
}

/// Seed a new VM with the classes' localized names before its FrameXML loads: stock
/// `Constants.lua` fills `LOCALIZED_CLASS_NAMES_MALE`/`_FEMALE` through `FillLocalizedClassList`
/// (`0x60a510`) at file scope.
fn seed_class_names(world: &World, script: &mut UiScript) {
    if let Some(table) = world.get_resource::<crate::chr_classes::ChrClassTable>() {
        script.set_localized_class_names(table.0.localized_names());
    }
}

/// Seed a new VM with each `Item.dbc` entry's display icon, which `GetItemIcon` reads (`0x517020`).
fn seed_item_icons(world: &World, script: &mut UiScript) {
    let records = world.get_resource::<crate::app_core::catalogs::ItemRecords>();
    let displays = world.get_resource::<crate::entities::ItemDisplays>();
    let (Some(records), Some(displays)) = (records, displays) else {
        return;
    };
    let mut paths = std::collections::HashMap::<u32, Option<std::sync::Arc<str>>>::new();
    let icons = records.0.iter().map(|(entry, row)| {
        let icon = paths.entry(row.display_id).or_insert_with(|| {
            crate::ui_items::item_icon(Some(displays), row.display_id).map(Into::into)
        });
        (entry, icon.clone())
    });
    script.set_item_icons(std::sync::Arc::new(icons.collect()));
}

/// Seed a new VM with Talent.dbc and GlyphProperties/GlyphSlot.dbc, which `GameTooltip:
/// SetHyperlink`'s `talent:` and `glyph:` links read (`0x62dae0`): a linked talent or glyph can be
/// any class's, not only the player's tree and sockets.
fn seed_link_catalogs(world: &World, script: &mut UiScript) {
    if let Some(talents) = world.get_resource::<crate::ui_talent::Talents>() {
        script.set_talent_link_catalog(
            talents
                .catalog
                .talents()
                .map(|t| {
                    let view = wrathplus_ui::script::TalentLinkView {
                        ranks: t.ranks.to_vec(),
                        exceptional: t.exceptional,
                    };
                    (t.id, view)
                })
                .collect(),
        );
    }
    if let Some(glyphs) = world.get_resource::<crate::spell::targeting::glyph::GlyphTables>() {
        script.set_glyph_link_catalog(wrathplus_ui::script::GlyphLinkCatalog {
            glyph_spells: glyphs.0.properties().map(|p| (p.id, p.spell_id)).collect(),
            slots: glyphs
                .0
                .slots()
                .map(|s| (s.id, (s.type_flags, s.order)))
                .collect(),
        });
    }
}

/// Seed a new VM with the account's expansion (`SMSG_AUTH_RESPONSE`, known before the world VM)
/// and the installed locale, which `GetExpansionLevel` and `GetExistingLocales` read.
fn seed_session(world: &World, script: &mut UiScript) {
    use wrathplus_assets::LockRecover;
    let expansion = world
        .get_resource::<crate::ui_lfg::auth::NativeAccountExpansion>()
        .and_then(|e| e.0);
    let locales = world
        .get_resource::<wrathplus_assets::WorldAssets>()
        .map(|a| match a.chain.lock_recover().profile() {
            wrathplus_formats::DataProfile::Wrath { locale } => vec![locale.clone()],
        })
        .unwrap_or_default();
    script.set_wrath_session(expansion, locales);
}

/// Start the new VM's `GetTime()` where the process already is and re-anchor [`UiClock`] to it,
/// both off the `Time<Real>` it anchors on, and read it live between frames. 12340's `GetTime`
/// (`0x6081f0`) is the OS clock × 0.001, never restarted; ours is rebuilt at every login and
/// `ReloadUI`, and a clock back at zero hides every cooldown (`CooldownFrame_SetTimer`).
fn seed_vm_clock(world: &mut World, script: &mut UiScript) {
    let (anchor, elapsed) = world
        .get_resource::<Time<bevy::time::Real>>()
        .map_or_else(Default::default, |t| {
            (t.last_update(), t.elapsed_secs_f64())
        });
    script.set_now(elapsed);
    script.set_live_clock(true);
    if let Some(mut clock) = world.get_resource_mut::<UiClock>() {
        *clock = UiClock {
            anchor: anchor.unwrap_or_else(std::time::Instant::now),
            ui_now: elapsed,
        };
    }
}

/// Wire `Interface\AddOns\` art and fonts: the decoder's loose-file root and the VM's texture,
/// size and font probes, so the path forms of `SetTexture` and `SetFont` answer the reference's
/// 1|nil load verdict, and a region with no authored size on an axis takes it from its art as the
/// client's virtual size getters do. Each probe resolves [`addons::root`] through the store its
/// renderer reads, so the answer is what draws; they test existence, not decode.
fn install_addon_asset_resolvers(world: &mut World, script: &mut UiScript) {
    let root = addons::root();
    let Some(mut assets) = world.get_resource_mut::<wrathplus_assets::WorldAssets>() else {
        return;
    };
    assets.set_loose_addon_root(root.clone());
    let chain = assets.chain.clone();
    let size_chain = chain.clone();
    let size_root = root.clone();
    let font_chain = chain.clone();
    let font_root = root.clone();
    // Memoised: layout asks per textured region per pass, and the chain's lock is shared with
    // the asset loaders, which hold it through whole MPQ reads during the loading screen.
    let present: std::cell::RefCell<std::collections::HashMap<String, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    script.set_texture_probe(Box::new(move |path| {
        if let Some(&cached) = present.borrow().get(path) {
            return cached;
        }
        let found = wrathplus_assets::sprite_candidates(path).iter().any(|c| {
            chain.lock_recover().contains(c)
                || root
                    .as_deref()
                    .is_some_and(|r| wrathplus_assets::loose_addon_file(r, c).is_some())
        });
        present.borrow_mut().insert(path.to_string(), found);
        found
    }));
    // Keyed by the path exactly as the region carries it, so a hit allocates nothing (this is
    // asked per zero-size textured region per layout pass); misses are cached too.
    let sizes: std::cell::RefCell<std::collections::HashMap<String, Option<(u32, u32)>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    script.set_texture_size_probe(Box::new(move |path| {
        if let Some(cached) = sizes.borrow().get(path) {
            return *cached;
        }
        let measured = wrathplus_assets::sprite_dimensions(&size_chain, size_root.as_deref(), path);
        sizes.borrow_mut().insert(path.to_string(), measured);
        measured
    }));
    // Existence over the same two stores, in the same order, as `ui_text::engine`'s face loader,
    // memoised because a path's answer cannot change and an addon may call `SetFont` every frame.
    let seen: std::cell::RefCell<std::collections::HashMap<String, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    script.set_font_probe(Box::new(move |path| {
        if let Some(&cached) = seen.borrow().get(path) {
            return cached;
        }
        let found = font_chain.lock_recover().contains(path)
            || font_root.as_deref().is_some_and(|r| {
                wrathplus_assets::loose_addon_file(r, &wrathplus_assets::normalize_path(path))
                    .is_some()
            });
        seen.borrow_mut().insert(path.to_string(), found);
        found
    }));
}

/// Whether this run wants the player UI: always outside a capture, and in one only when opted in
/// ([`crate::run_mode::capture_ui_opted_in`]), since capture baselines test the world render.
fn ui_wanted(world: &World) -> bool {
    !world.contains_resource::<crate::run_mode::CaptureMode>()
        || crate::run_mode::capture_ui_opted_in()
}

pub(crate) use crate::app_core::ui_flags::PendingEntryUiLoad;
/// The boot VM, parked between the world-entry edge and the deferred load, out of every feed's
/// reach: each feed takes the VM as an `Option` and returns on `None`.
pub(crate) struct ParkedBootVm(UiScript);

/// `OnEnter(InWorld)`: arm the deferred entry load and park the boot VM.
pub(crate) fn arm_entry_ui_load(world: &mut World) {
    world.insert_resource(PendingEntryUiLoad);
    if let Some(vm) = world.remove_non_send_resource::<UiScript>() {
        world.insert_non_send_resource(ParkedBootVm(vm));
    }
}

/// Retire the glue VM and build the one the entry load runs on: the reference re-makes its Lua
/// state inside `UI_Init` (`0x48fe97`, in `0x48fbf0`), so no login shares a session id (and so a
/// [`crate::app_core::vm_memo::VmMemo`]) with the character screen before it. The glue VM's CVars fold first, since
/// the character screen can write one (the AddOns panel's out-of-date toggle) and this is the last
/// moment its table exists. Everything else is re-seeded onto the new VM or is a
/// [`crate::app_core::vm_memo::VmMemo`], which is meant to reset here. It starts bare: `FrameXML.toc` runs
/// `GlobalStrings.lua` and `Fonts.xml` as its first rows, and `ChatFrame.lua` the emote tokens.
fn mint_entry_vm(world: &mut World) {
    if let Some(script) = new_vm(world) {
        world.insert_non_send_resource(script);
    }
}

/// Put the parked boot VM back into the world, if one is parked.
fn unpark_boot_vm(world: &mut World) {
    if let Some(ParkedBootVm(vm)) = world.remove_non_send_resource::<ParkedBootVm>() {
        world.insert_non_send_resource(vm);
    }
}

/// `PreUpdate`, after [`run_pending_reload`] so a reload never interleaves an armed entry load:
/// run the armed entry load once the cover has presented, at once when there is no cover (a
/// capture booting straight `InWorld`). Leaving the world first drops the latch unrun, and
/// [`end_ui_session`] then skips the shutdown writes, so a UI-less VM never overwrites the saved
/// files.
pub(crate) fn run_pending_entry_load(world: &mut World) {
    if world.get_resource::<PendingEntryUiLoad>().is_none() {
        return;
    }
    let in_world = *world
        .resource::<State<crate::app_core::state::ClientState>>()
        .get()
        == crate::app_core::state::ClientState::InWorld;
    if !in_world {
        // Left the world before the load ran: nothing to build a UI for.
        world.remove_resource::<PendingEntryUiLoad>();
        unpark_boot_vm(world);
        return;
    }
    let covering = world
        .get_resource::<crate::loading_screen::LoadingScreen>()
        .is_some_and(|s| s.covering());
    // `presented()` is also true with no cover up: nothing to protect, so load now.
    if !world
        .get_resource::<crate::loading_screen::EntryCover>()
        .is_none_or(|c| c.presented())
    {
        return;
    }
    world.remove_resource::<PendingEntryUiLoad>();
    let start = std::time::Instant::now();
    load_ingame_ui_on_world_entry(world);
    // Every entry logs the load's cost, whether the cover hid it, and the session every
    // `VmMemo` keys on.
    info!(
        "ui_script: in-game UI up in {:.0} ms (behind the cover: {}, vm session: {})",
        start.elapsed().as_secs_f32() * 1000.0,
        covering,
        world
            .get_non_send_resource::<UiScript>()
            .map_or(0, UiScript::session),
    );
}

/// Seat the VM's screen size and text measurer for the load, under the current window and
/// `uiScale`: the load-edge half of [`super::extract::tick_script`]'s per-frame pair. A fresh VM
/// answers 1024×768 until then, and a value computed once at load keeps it: stock
/// `WorldMapFrame_OnLoad` sizes the full-screen `BlackoutWorld` from `GetScreenWidth()` and
/// `GetScreenHeight()` and never again (`WorldMapFrame.lua:19-28`). With no atlas yet (it bakes on
/// the first `Update`) the per-frame pass seats the measurer.
pub(super) fn seat_raster_seam_for_load(world: &mut World, script: &mut UiScript) {
    let ui_scale = world
        .get_resource::<super::UiScaleCvar>()
        .map_or(1.0, |c| c.0);
    let (w, h) = {
        let mut q = world.query_filtered::<&Window, With<bevy::window::PrimaryWindow>>();
        q.single(world)
            .map_or((0.0, 0.0), |win| (win.width(), win.height()))
    };
    let s = super::seam_scale(h, ui_scale);
    if w > 0.0 && h > 0.0 {
        // The VM lives in 768-tall virtual space, so logical size over the seam scale is its
        // screen. No `UIParent_ManageFramePositions` here, unlike `tick_script`: nothing is laid
        // out yet.
        script.set_screen_size(w / s, h / s);
    }
    let Some(atlas) = world.get_resource::<crate::ui_text::UiFontAtlas>() else {
        return;
    };
    super::extract::seat_text_measurer(script, atlas, s);
}

/// Load the in-game UI for this session onto a VM built for it, once per world entry. The
/// reference builds the UI in `UI_Init` (`0x48fbf0`, re-making the Lua state at `0x48fe97`) and
/// destroys it at `0x490bd0`, so every login runs every addon's file scope afresh. Safe on the
/// state edge because the initial transition runs after `PostStartup`: a capture booting straight
/// into `InWorld` still loads after [`wrathplus_assets::AssetSet::Open`].
pub(crate) fn load_ingame_ui_on_world_entry(world: &mut World) {
    // The parked VM goes back into the world, where the CVar fold reads it; a reload has none
    // parked and uses the live one.
    unpark_boot_vm(world);
    if !ui_wanted(world) {
        return;
    }
    mint_entry_vm(world);
    let Some(mut script) = world.remove_non_send_resource::<UiScript>() else {
        warn!("ui_script: entering the world with no VM — the in-game UI will not load");
        return;
    };
    // The character whose AddOn enable state gates which addons run.
    let identity = world
        .get_resource::<crate::app_core::state::Roster>()
        .and_then(crate::app_core::state::identity);
    // The realm's character list is the enable store's node set: the reference keeps one
    // `ADDONSTATELIST` node per character, and an addon this one never set resolves from the rest.
    let roster: Vec<String> = world
        .get_resource::<crate::app_core::state::Roster>()
        .map(|r| r.chars.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default();
    // The addon array before any addon file runs: `SMSG_ADDON_INFO` lands during the handshake, so
    // the reference has it before `UI_Init`'s addon walk and `GetNumAddOns()` at file scope sees
    // it. A server that never answered leaves it empty for the session, as in the reference.
    if let Some(reply) = world
        .get_resource::<crate::net::AddonInfoReply>()
        .and_then(|r| r.0.clone())
    {
        script.note_addon_info_reply(&reply);
    }
    // The CVar table goes in before the UI loads: stock `UIOptionsFrame.xml`'s camera dropdowns
    // read `cameraSmoothStyle` and `cameraSmoothTrackingStyle` in their `OnLoad`, where a nil is a
    // concat error (`UIOptionsFrame.lua:379-384`, `:501-506`), and this VM was born inside this
    // call, before `cvars::sync_cvars`'s per-VM seed. Its order: the file's unclaimed entries
    // first, so an addon's `RegisterCVar` starts at the player's value, then the live table.
    match world.get_resource::<crate::cvars::Cvars>() {
        Some(cvars) => {
            script.set_cvar_saved_base(cvars.orphans());
            script.seed_cvars(cvars.vm_seed());
        }
        // No registry: a stripped test world, which has only the core table.
        None => script.register_cvars(crate::cvars::REGISTERED.iter().map(|r| (r.name, r.default))),
    }
    // The realm before the load, since addons read `GetRealmName()` at file scope; the roster
    // carries the realm-list entry this session connected to.
    let realm = world
        .get_resource::<crate::app_core::state::Roster>()
        .and_then(|r| r.realm.as_ref().map(|r| r.name.clone()))
        .unwrap_or_default();
    script.set_realm_name(&realm);
    // `CGGameUI::InitializeGame` flags these read-only before `UI_Init` loads anything
    // (`0x48f566`-`0x48f584`), so an in-game `SetCVar` of one raises, logout handlers included;
    // `ShutdownGame` clears them (`0x491240`), and here the flags die with this VM. The host's
    // own realm writes are engine writes, which the flag does not stop.
    for name in wrathplus_ui::script::IN_WORLD_READ_ONLY_CVARS {
        script.set_cvar_read_only(name, true);
    }
    // The player too, before the addon walk. The record first: the reference's
    // `UnitName`/`UnitRace`/`UnitClass`/`UnitSex`/`UnitFactionGroup` read only a copy of the
    // char-enum row made at the Enter World commit (1.12 `0x5abd9e`; 12340 not located) and never
    // cleared, and each rebuilt VM is told once. The snapshot stands in for the descriptor until
    // that streams in.
    if let Some(record) = record_from_roster(world) {
        script.set_player_record(record);
    }
    if let Some(seat) = world
        .get_resource::<crate::app_core::state::Roster>()
        .and_then(seat_from_roster)
    {
        script.set_unit("player", Some(seat));
    }
    // The XP pair from the descriptor the login burst already delivered: stock 3.3.5 loads with
    // it in place, and `CharacterFrame_OnLoad`'s `TextStatusBar_UpdateTextString(MainMenuExpBar)`
    // hides a bar whose max reads 0, which its `OnValueChanged` (`MainMenuBar.xml`, returns while
    // hidden) never undoes. The feed's own first sight still fires `PLAYER_XP_UPDATE` after.
    let xp = world
        .query_filtered::<&crate::app_core::session::ObjectStore, With<crate::app_core::session::SelfPlayer>>()
        .iter(world)
        .next()
        .map(|store| (store.0.player_xp(), store.0.player_next_level_xp()));
    if let Some((Some(xp), Some(next))) = xp {
        script.set_player_xp(xp, next);
    }
    // The addon version gate, read from the persisted base the fold above just made current, so a
    // character-screen "Load out of date AddOns" click counts. A bare test world is check on.
    let version_check = world
        .get_resource::<crate::cvars::Cvars>()
        .is_none_or(crate::cvars::Cvars::addon_version_check);
    // Per-VM `Update` claims the load itself reads, seeded now: the zone-channel catalog (empty,
    // `General` would be filed and joined as a custom channel), the binding table (stock
    // `ActionButton_OnLoad` paints the hotkeys), the default language (`ChatFrame_OnEvent`'s
    // `PLAYER_ENTERING_WORLD` arm stores it, `ChatFrame.lua:1275-1276`) and the language table a
    // file-scope `SendChatMessage` names its tongue from.
    crate::ui_chat::seed_zone_channel_catalog(world, &mut script);
    crate::bindings::seed_bindings_for_vm(world, &mut script);
    crate::ui_unit::seed_default_language(world, &mut script);
    crate::ui_unit::seed_language_table(world, &mut script);
    // The world map's continent and zone lists: static DBC data in the reference, read at file
    // scope (Astrolabe builds its whole zone table from `GetMapContinents`/`GetMapZones` there).
    crate::ui_world_map::seed_world_map_catalog(world, &mut script);
    // Screen size and measurer before the first `<OnLoad>`: this VM is loaded before `tick_script`
    // seats either, and the load measures text (the stock tab template's `OnLoad` sizes from its
    // text width, `UIPanelTemplates.xml:370-373`), which answers 0 with no measurer.
    seat_raster_seam_for_load(world, &mut script);
    crate::video::seed_display_modes(world, &mut script);
    // Read out of the world first: the bracket below borrows `world` for the chat-cache restore.
    let zoom = {
        let z = world.resource::<crate::minimap::MinimapZoom>();
        (z.outdoor, z.inside)
    };
    // No sound during the load edge, as in the reference's `UI_Init` (`0x48fbfa` → `0x49016d`).
    silenced_ui_load(&mut script, |script| {
        for failure in
            super::manifest::load_wrath_ui(script, identity.as_ref(), &roster, version_check)
        {
            warn!("ui_script: Wrath UI load: {failure}");
        }
        crate::ui_chat::commands::register_dev_commands(script, crate::run_mode::dev_affordances());
        // The new Minimap's zoom indices from the persisted CVars, as the reference's minimap reset
        // copies each CVar into its live index; `Minimap:SetZoom` owns them from here.
        script.set_minimap_zoom(zoom.0, zoom.1);
        // The saved-variables chunk runs after the files set their defaults and before
        // `VARIABLES_LOADED`, the reference's order (`AddOn_Load` `0x51f240`); reversed, the
        // defaults always win. The chat cache restores after `VARIABLES_LOADED` and before any chat
        // is routed: its `UPDATE_CHAT_WINDOWS` registers each chat frame for its `CHAT_MSG_*`
        // events (`ChatFrame.lua:1261-1273`).
        finish_ui_load_with(
            script,
            |_| {},
            |script| {
                crate::ui_chat::restore_chat_looks(world, script);
            },
        );
    });
    // The load's record goes to `Logs\FrameXML.log` (`0x490187`-`0x4901b6`), after any
    // `LoadAddOn` block the load ran; nothing reaches chat.
    script.finish_ui_load_log();
    super::load_log::write(script.take_load_log_writes());
    // `DAMAGE_TEXT_FONT` binds at the end of the load, after every addon's `ADDON_LOADED` (where
    // MikScrollingBattleText and pfUI assign it). The reference reads it eagerly, once per world
    // entry (`0x6c8470`, called at `0x401620` in `0x401570`, after the UI load at `0x401602`),
    // and never again: `[0xce8820]` has one writer, and a `/reloadui` does not re-run it, where
    // this edge re-reads it on every reload.
    world.insert_resource(crate::combat_text::read_damage_text_font(&script));
    // The load is over: disarm its instruction bound, so no handler is killed for being slow.
    script.clear_instruction_budget();
    world.insert_non_send_resource(script);
    world.insert_resource(AddOnIdentity(identity));
    // Arm the world latch, the reference's `0x490168 call 0x4908c0`: a reload creates no entity
    // to re-arm it, so without this a second `/reload` would not fire `PLAYER_LEAVING_WORLD`. The
    // reference skips this leg on a fresh login (`0x490166 je`, the GUID still 0) and arms from
    // the player's create instead; both reach the same armed byte.
    if let Some(mut armed) = world.get_resource_mut::<LeavingWorldArmed>() {
        armed.arm();
    }
}

/// The wire's gender byte (0 male, 1 female) on `UnitSex`'s 2/3 scale, as `ui_unit::snapshot`
/// maps the descriptor's; the record and the seat share it.
fn roster_sex(gender: u8) -> u8 {
    match gender {
        0 => 2,
        1 => 3,
        _ => 0,
    }
}

/// The local player record the UI loads under: our copy of the char-enum row the reference copies
/// at the Enter World commit (layout on [`wrathplus_ui::script::PlayerRecord`]). Race, class and
/// side are frozen tables of the reference's `ChrRaces`, `ChrClasses` and race-to-`FactionGroup`
/// walks (`0x60d0f4`-`0x60d22b`), the side's `Name0` the catalog's when loaded. No level: the
/// record's accessor (`0x5abe00`, `+0x108`) has no caller, so `UnitLevel` reads the descriptor.
pub(crate) fn record_from_roster(world: &World) -> Option<wrathplus_ui::script::PlayerRecord> {
    let roster = world.get_resource::<crate::app_core::state::Roster>()?;
    let row = roster.pending_row()?;
    let factions = world.get_resource::<crate::app_core::state::Factions>();
    let race = crate::app_core::catalogs::race_names(row.race);
    let class = crate::app_core::catalogs::class_names(row.class);
    Some(wrathplus_ui::script::PlayerRecord {
        name: row.name.clone(),
        race: race.map(|(n, f)| (n.to_string(), f.to_string())),
        class: class.map(|(n, f)| (n.to_string(), f.to_string())),
        sex: roster_sex(row.gender),
        faction_group: crate::ui_unit::race_faction_group(row.race).map(|g| {
            let bit = if g == "Alliance" { 2 } else { 4 };
            let name = factions.and_then(|f| f.catalog().faction_group_name(bit));
            (g.into(), name.unwrap_or(g).into())
        }),
    })
}

/// The `"player"` snapshot the UI loads under, from the roster row of the pick in flight, until
/// the self descriptor streams in; `None` with no pick (a capture, a scenario, a test world).
/// [`record_from_roster`] answers name, race, class, sex and faction group; this carries what
/// else addon file scope reads. Health and power stay zero, the descriptor's to say.
pub(crate) fn seat_from_roster(
    roster: &crate::app_core::state::Roster,
) -> Option<wrathplus_ui::script::UnitState> {
    let row = roster.pending_row()?;
    let race = crate::app_core::catalogs::race_names(row.race);
    let class = crate::app_core::catalogs::class_names(row.class);
    let sex = roster_sex(row.gender);
    Some(wrathplus_ui::script::UnitState {
        // Before the player object exists the reference answers `UnitExists("player")`
        // (`0x515fb0`) nil: the resolver reads the GUID from the object (`0x515994`), and the
        // fallback `0x491900` bails on the zero GUID (`0x4e80aa`) to `0x516001 lua_pushnil`.
        // `UnitLevel` (`0x517fc0`) answers the number 0 (`0x51813e push 0`); the two misses differ.
        exists: false,
        name: Some(row.name.clone()),
        race: race.map(|(n, _)| n.to_string()),
        race_file: race.map(|(_, f)| f.to_string()),
        class: class.map(|(n, _)| n.to_string()),
        class_file: class.map(|(_, f)| f.to_string()),
        sex,
        is_player: true,
        player_controlled: true,
        ..Default::default()
    })
}

/// The character the loaded AddOn state belongs to, kept so the shutdown writes go back to its
/// files after the roster's pick is gone.
#[derive(Resource, Default)]
pub(crate) struct AddOnIdentity(pub(crate) Option<(String, String)>);

/// The world latch, the reference's `[0xb4b424]`: set at `0x4908ce` in the world-enter cascade
/// `0x4908c0` and cleared at `0x490a8d` as `PLAYER_LEAVING_WORLD` fires (`0x490b4d`, its one
/// site), so the first departure in a world fires and the rest are no-ops until the next entry.
/// Ours has two producers, `wrath_addons::shutdown`'s tail and `ui_unit`'s worldport fire. A
/// cross-map worldport spends it without leaving `InWorld`, so a quit on its loading screen
/// fires nothing; a `/reload` fires, and its rebuild re-arms.
#[derive(Resource, Default)]
pub(crate) struct LeavingWorldArmed(bool);

impl LeavingWorldArmed {
    /// A world began. Idempotent, like the reference's `mov byte [0xb4b424],1`.
    pub(crate) fn arm(&mut self) {
        self.0 = true;
    }

    /// Take the one-shot: `true` at most once per world, `0x490a8d`'s clear folded in.
    pub(crate) fn spend(&mut self) -> bool {
        std::mem::take(&mut self.0)
    }

    /// Read without taking, for tests.
    #[cfg(test)]
    pub(crate) fn is_armed(&self) -> bool {
        self.0
    }
}

/// Arm the latch when the local player's entity appears: the reference's `0x5deb60` entry into
/// the world-enter cascade, which a fresh login and a worldport's new-world create both take
/// (`0x5deb49 call 0x468570` / `0x5deb50 jne 0x5deb6a`).
pub(crate) fn arm_leaving_world_on_self_create(
    created: Query<(), Added<crate::app_core::session::SelfPlayer>>,
    mut armed: ResMut<LeavingWorldArmed>,
) {
    if !created.is_empty() {
        armed.arm();
    }
}

/// `OnExit(InWorld)`, a `/logout` or a disconnect (two of the reference's five roots): run the
/// shutdown tail, then end this session's Lua state, so no frame, global or addon upvalue reaches
/// the next login. The reference's state outlives `0x490bd0` and is replaced twice through
/// `0x703b80`, in `ShutdownGame` (at `0x491231`) and then in the glue build (`0x46a7b0`), so the
/// character screen runs on a state of its own. Ours is a fresh boot VM.
pub(crate) fn end_ui_session(world: &mut World) {
    // A VM still parked (the load never ran) goes back into the world first.
    unpark_boot_vm(world);
    // An entry load still armed means no in-game UI was built, and the tail would overwrite the
    // saved files with the boot VM's emptiness.
    let ui_never_loaded = world.remove_resource::<PendingEntryUiLoad>().is_some();
    let identity = world
        .get_resource::<AddOnIdentity>()
        .and_then(|id| id.0.clone());
    // Spent at the root: of this edge and the worldport's, the first to a departure owns it.
    let leaving_world = world
        .get_resource_mut::<LeavingWorldArmed>()
        .is_some_and(|mut l| l.spend());
    if !ui_never_loaded {
        if let Some(mut script) = world.get_non_send_resource_mut::<UiScript>() {
            super::wrath_addons::shutdown(&mut script, identity.as_ref(), leaving_world);
        }
    }
    // Only tutorial history is persisted; generic CVar folding stays off.
    let history = world
        .get_non_send_resource_mut::<UiScript>()
        .and_then(|mut script| script.persist_wrath_tutorial_history());
    if let (Some(history), Some(mut cvars)) =
        (history, world.get_resource_mut::<crate::cvars::Cvars>())
    {
        cvars.set("flaggedTutorials", &history);
    }
    world.insert_resource(AddOnIdentity(None));

    // Host values taken from the dying frame tree go with it (a `VmMemo` resets on its own): the
    // hovered frame, the keyboard capture (`feed_ui_input` stops outside `InWorld`), the cursor
    // payload and the minimap's hole. Not [`UiClock`]: `GetTime` is the reference's OS clock, and
    // [`seed_vm_clock`] re-anchors it for the next VM.
    if let Some(mut hover) = world.get_resource_mut::<PlayerUiHover>() {
        hover.0 = None;
    }
    if let Some(mut keys) = world.get_resource_mut::<UiKeyboardCapture>() {
        keys.typing = false;
        keys.arrows_fall_through = false;
        keys.consumed.clear();
    }
    if let Some(mut held) = world.get_resource_mut::<CursorPayloadHeld>() {
        *held = CursorPayloadHeld::default();
    }
    if let Some(mut minimap) = world.get_resource_mut::<crate::minimap::MinimapWidget>() {
        minimap.0 = None;
    }

    install_boot_vm(world);
}

/// A `ReloadUI()` waiting to run: set when [`crate::ui_logout`] drains
/// [`wrathplus_ui::script::SessionRequest::ReloadUi`], run by [`run_pending_reload`] in the next
/// frame's `PreUpdate`. A flag, as in the reference (`[0xb4b3f4]`, written only by `0x491380` and
/// read only by the per-frame `0x495590`), so the VM that asked is never mid-call when destroyed.
#[derive(Resource, Default)]
pub(crate) struct ReloadUiPending(pub(crate) bool);

/// Run a pending `ReloadUI()`: the reference's teardown and rebuild (`0x495664 call 0x490bd0`,
/// `0x495669 call 0x48fbf0`), for us [`end_ui_session`] then [`load_ingame_ui_on_world_entry`]
/// without leaving the world, so a `DisableAddOn` staged in the dying VM reaches `AddOns.txt`
/// before the rebuild reads it. `PLAYER_ENTERING_WORLD` then refires from [`crate::ui_unit`]'s
/// feed on the new VM. In-world only, as the reference's gate (`0x494a50(0xa)`) is; elsewhere the
/// request is dropped.
pub(crate) fn run_pending_reload(world: &mut World) {
    if !std::mem::take(&mut world.resource_mut::<ReloadUiPending>().0) {
        return;
    }
    let in_world = *world
        .resource::<State<crate::app_core::state::ClientState>>()
        .get()
        == crate::app_core::state::ClientState::InWorld;
    if !in_world {
        info!("ui_script: ReloadUI outside the world — dropped");
        return;
    }
    info!("ui_script: ReloadUI — ending the UI session and building a new one");
    end_ui_session(world);
    load_ingame_ui_on_world_entry(world);
}

/// The load's ordered tail without the host seams, for tests; production runs
/// [`finish_ui_load_with`]. The reference's `UI_Init` (`0x48fbf0`) loads every non-LoadOnDemand
/// addon, each firing `ADDON_LOADED` at `0x51f5ad` (`0x4900a3` → `0x51f600`), reads the flat
/// saved file and fires `VARIABLES_LOADED` (`0x4900b2` → `0x4913b0`), and arms `PLAYER_LOGIN`
/// (`[0xb4e260]`, set at `0x49011d`). The world-enter cascade `0x4908c0` fires `PLAYER_LOGIN` if
/// armed (read at `0x49094b`, fired at `0x490959`, disarmed at `0x49095e`), then
/// `PLAYER_ENTERING_WORLD` (`0x49096a`).
///
/// On a `/reload`, `UI_Init` enters the cascade itself (`0x490168`). On a fresh login that call is
/// skipped (`0x490166 je`, the active-player GUID still 0) and the cascade runs from the local
/// player's own create (`0x5deb60 call 0x4908c0`). This fires `PLAYER_LOGIN` at the end of the
/// load on both, the `/reload` shape; `PLAYER_ENTERING_WORLD` fires from [`crate::ui_unit`] once
/// the self descriptor lands.
#[cfg(test)]
pub(crate) fn finish_ui_load(script: &mut UiScript) {
    finish_ui_load_with(script, |_| {}, |_| {});
}

/// [`finish_ui_load`] with its two host seams. `host_settings` runs between the saved-variables
/// chunk and `VARIABLES_LOADED`, carrying over a setting `config.toml` held before it moved into
/// the saved variables. `between`
/// is the chat-cache restore's `UPDATE_CHAT_WINDOWS` + `UPDATE_CHAT_COLOR` burst, after
/// `VARIABLES_LOADED` and before `PLAYER_LOGIN`, where the reference registers its reader
/// (`0x4900d6` → `0x498a20`). On a fresh login the reference's cache (`0x5afe50`) defers that
/// reader until the server's account data reconciles (`0x5afa42`/`0x5afd01`), after `UI_Init` has
/// returned. Either way a `VARIABLES_LOADED` handler sees the boot chat colours, not the file's.
pub(crate) fn finish_ui_load_with(
    script: &mut UiScript,
    host_settings: impl FnOnce(&mut UiScript),
    between: impl FnOnce(&mut UiScript),
) {
    // Still the load edge, so still bounded: re-armed so the saved variables and every
    // `PLAYER_LOGIN` handler get the full allowance; the entry edge disarms it after.
    script.set_instruction_budget(addons::LOAD_INSTRUCTION_BUDGET);
    crate::ui_saved::load_saved_variables(script, host_settings);
    between(script);
    script.fire_event("PLAYER_LOGIN", vec![]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_edge_parks_the_boot_vm_and_the_session_end_returns_it() {
        let mut world = World::new();
        let boot = UiScript::new().unwrap();
        let session = boot.session();
        world.insert_non_send_resource(boot);
        arm_entry_ui_load(&mut world);
        assert!(
            world.get_non_send_resource::<UiScript>().is_none(),
            "no VM in the deferral window"
        );
        assert!(world.get_resource::<PendingEntryUiLoad>().is_some());
        assert!(world.get_non_send_resource::<ParkedBootVm>().is_some());
        // Left the world before the load ran: the VM comes back, untouched.
        unpark_boot_vm(&mut world);
        let vm = world
            .get_non_send_resource::<UiScript>()
            .expect("the parked VM is back");
        assert_eq!(vm.session(), session, "the same VM, no session moved");
        assert!(world.get_non_send_resource::<ParkedBootVm>().is_none());
    }

    /// The reference re-makes its Lua state inside `UI_Init` (`0x48fe97`).
    #[test]
    fn the_world_entry_builds_its_own_vm_rather_than_adopting_the_glue_phases() {
        let mut world = World::new();
        let glue = UiScript::new().unwrap();
        let glue_session = glue.session();
        world.insert_non_send_resource(glue);

        mint_entry_vm(&mut world);

        let entry = world
            .get_non_send_resource::<UiScript>()
            .expect("the entry load has a VM to run on");
        assert_ne!(
            entry.session(),
            glue_session,
            "the entry VM is a new session — every VmMemo resets here, so a one-shot spent \
             against the frameless glue VM is spent again against the one that has an interface"
        );
    }

    /// A one-shot fired at the frameless glue VM reaches nobody (the reference's `0x703f50` drops
    /// an event with no listener), so the entry VM must be told again.
    #[test]
    fn a_one_shot_spent_against_the_character_screens_vm_is_spent_again_after_the_entry() {
        let mut world = World::new();
        world.insert_non_send_resource(UiScript::new().expect("a glue VM"));
        let mut told: crate::app_core::vm_memo::VmMemo<bool> = Default::default();

        assert!(
            told.claim(world.non_send_resource::<UiScript>()),
            "the window: a feed fires its login one-shot at the frameless character-screen VM"
        );
        assert!(
            !told.claim(world.non_send_resource::<UiScript>()),
            "and the memo has it as told — which is correct for THAT VM"
        );

        mint_entry_vm(&mut world);

        assert!(
            told.claim(world.non_send_resource::<UiScript>()),
            "the VM the in-game UI loads onto has never been told, so the feed tells it again — \
             no run condition, no ordering edge, and nothing the feed had to know about"
        );
    }

    /// The reference's `GetTime` is `GetTickCount` × 0.001 (`0x515ea0`) and never restarts; stock
    /// `CooldownFrame_SetTimer` hides a cooldown whose start is not above 0.
    #[test]
    fn a_rebuilt_vm_inherits_the_running_gettime_clock() {
        use std::time::{Duration, Instant};

        let mut world = World::new();
        world.init_resource::<UiClock>();
        // The process has been up 90 s, on `Time<Real>` and on the dying VM alike.
        let startup = Instant::now() - Duration::from_secs(90);
        let mut time = Time::<bevy::time::Real>::new(startup);
        // `Time<Real>` measures `elapsed` from its first update, not from `startup`.
        time.update_with_instant(startup);
        time.update_with_instant(startup + Duration::from_secs(90));
        world.insert_resource(time);
        let mut dying = UiScript::new().unwrap();
        dying.set_now(90.0);
        world.insert_non_send_resource(dying);
        // No in-game UI was loaded, so the shutdown tail and its file writes are skipped.
        world.init_resource::<PendingEntryUiLoad>();

        end_ui_session(&mut world);

        let vm = world
            .get_non_send_resource::<UiScript>()
            .expect("the session end installs a fresh boot VM");
        assert_eq!(
            vm.now(),
            90.0,
            "the new VM's GetTime clock restarted at {} instead of the process's 90 s",
            vm.now()
        );
        // The conversion pair moved too: a 10-minute cooldown armed 30 s before the rebuild keeps
        // its real start.
        let clock = world.resource::<UiClock>();
        let armed = crate::app_core::cooldowns::CooldownInfo {
            start: startup + Duration::from_secs(60),
            remaining_ms: 570_000,
            duration_ms: 600_000,
            enabled: true,
        };
        let triple = armed
            .ui_triple(clock.anchor, clock.ui_now)
            .expect("a running cooldown pushes a triple");
        assert_eq!(
            triple,
            (60_000, 600_000, true),
            "a cooldown armed before the rebuild derived start {} ms; stock Cooldown.lua hides \
             anything but `start > 0`",
            triple.0
        );
    }
}
