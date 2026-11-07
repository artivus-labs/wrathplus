//! Character select: the app's lifecycle state ([`ClientState`]), the roster pick policy and the
//! `CharacterSelect.xml` glue screen. The parked IO thread emits the roster ([`CharListMessage`]),
//! answered by the pending pick (reconnect), `WOW_CHAR`, or a choice on the screen, which opens on
//! the character last entered as (`lastCharacterIndex`).

mod addons;
mod dialog;
pub(crate) mod glue_rows;
mod input;
mod refresh;
mod screen;
pub(crate) mod service_reply;

use bevy::prelude::*;
use wrathplus_protocol::CharAction;
#[cfg(any(test, feature = "test-support"))]
use wrathplus_protocol::Character;

use crate::glue_strings::GlueStrings;
use crate::ui_script::glue::native_glue;

use crate::app_core::session::EnteredWorldMessage;
use crate::net::{
    CharListMessage, CharPick, CharRequest, CharacterLoginFailedMessage, LoggedOutMessage,
};

pub(crate) use crate::app_core::schedule::InWorldGated;
pub(crate) use crate::app_core::state::{ClientState, Roster};

/// The character-select subsystem: the state machine + the select screen.
pub struct CharSelectPlugin {
    /// The screen this session opens on: `Login` when connected, `InWorld` for a world capture,
    /// the photographed glue screen for a glue capture.
    pub start: ClientState,
}

/// Mirror the session's screen onto the engine's one-bit "is there a world" fact, in one place so
/// no transition can forget it.
fn publish_world_live(
    state: Res<State<ClientState>>,
    mut live: ResMut<wrathplus_world::schedule::WorldLive>,
) {
    let now = wrathplus_world::schedule::WorldLive(*state.get() == ClientState::InWorld);
    if *live != now {
        *live = now;
    }
}

/// The CVar rows this feature owns ([`crate::cvars::RegisterCvars`]), each with the
/// reference's register site and its default's standing ([`crate::cvars::Registered`]).
pub(crate) const CVARS: &[crate::cvars::Registered] = {
    use crate::cvars::same;
    &[
        // `lastCharacterIndex` (`0x402d93`, "0" `0x82e570`, category 4, handle `[0x882674]`), help
        // "Last character selected": a 0-based row (the selection cell `[0x83856c]` under `"%d"`), so
        // "0" is the first character. It mirrors [`Roster::pending_index`].
        same(CVAR_LAST_CHARACTER, "0"),
    ]
};

impl Plugin for CharSelectPlugin {
    fn build(&self, app: &mut App) {
        service_reply::register(app);
        crate::cvars::RegisterCvars::register_cvars(app, CVARS);
        app.insert_state(self.start)
            .configure_sets(Update, InWorldGated.run_if(in_state(ClientState::InWorld)))
            .init_resource::<Roster>()
            // Ahead of every world stage, so `WorldLive` and its falling edge are this frame's.
            .add_systems(
                Update,
                publish_world_live.before(wrathplus_world::schedule::WorldStage::Net),
            )
            .init_resource::<dialog::DeleteDialog>()
            .add_message::<GlueRaise>()
            .init_resource::<addons::AddonsPanel>()
            .add_systems(
                OnEnter(ClientState::CharSelect),
                (screen::enter_select.run_if(native_glue), stamp_select_entry),
            )
            .add_systems(
                OnExit(ClientState::CharSelect),
                (
                    screen::exit_select,
                    screen::preview_exit::clear_preview,
                    clear_select_entry,
                ),
            )
            .add_systems(Update, (debug_glue_roundtrip, debug_logout_smoke))
            .add_systems(
                Update,
                (
                    // Runs in every state: the reconnect auto-answer and the logout edge arrive
                    // `InWorld`. `back_on_disconnect` runs after the entry: a session that died
                    // during entry queues both edges into one frame, and the dead one must win.
                    (
                        apply_roster_policy,
                        enter_on_connected,
                        back_on_logout,
                        back_on_login_refused,
                        back_on_disconnect,
                        // Last and ungated: it mirrors the pick the frame ended with, and world
                        // entry is reached from the create screen too.
                        persist_last_character.before(crate::spell::auto_ranged::tick),
                    )
                        .chain(),
                    (
                        screen::materialize_screen,
                        input::select_input,
                        input::rotate_model,
                        debug_select_dialog,
                        dialog::drive_delete_dialog,
                        // Ahead of the dialog driver, so a refused delete shows the frame it lands.
                        delete_result,
                        // The one `GlueDialog`: here, the refused login and the refused delete.
                        crate::glue::dialog::drive_glue_dialog,
                        debug_select_addons,
                        debug_select_walk,
                        addons::drive_addons_panel,
                        refresh::refresh_list,
                        refresh::refresh_banner_and_buttons,
                        refresh::feed_glue_preview,
                        debug_select_shot,
                    )
                        .chain()
                        .in_set(crate::glue::GlueScreens::Select)
                        .before(crate::glue::GlueVisuals)
                        // After the UI tick: the addons panel holds the VM, and a glue screen
                        // has no push the tick must see, so the chain takes the drain side.
                        .after(crate::app_core::schedule::UiInput)
                        .run_if(in_state(ClientState::CharSelect))
                        .run_if(native_glue),
                )
                    .chain()
                    .after(wrathplus_world::schedule::WorldStage::Net),
            )
            // After the cover's stage, so the raise it reads is this frame's.
            .add_systems(
                Update,
                screen::hide_under_world_cover
                    .after(wrathplus_world::schedule::WorldStage::Present)
                    .run_if(in_state(ClientState::CharSelect)),
            );
    }
}

// ── The roster and pick policy ──────────────────────────────────────────────────────────────────

// ── The remembered character (`lastCharacterIndex`) ───────────────────────────────

/// The 1.12 CVar the select screen remembers by: registered at `0x402d93` (name `0x82e8f8`,
/// default `"0"`, pointer at `[0x882674]`), written engine-side, named by no GlueXML.
pub(crate) const CVAR_LAST_CHARACTER: &str = "lastCharacterIndex";

/// The stored index is 0-based, `"0"` the first character: the engine's selection cell
/// `[0x83856c]` printed with `"%d"` (`0x473470` decrements the Lua's 1-based row).
/// `CVar::SaveConfig` (`0x63d980`) skips a value equal to the default, so row 0 writes no line.
fn last_character_value(row: usize) -> String {
    row.to_string()
}

/// The stored value as a row; out of range is the caller's, since the reference does not clamp.
fn last_character_row(value: &str) -> Option<usize> {
    value.trim().parse::<usize>().ok()
}

/// The remembered row for a roster of `len`: out of range falls back to the first row, not the
/// nearest (`CGlueMgr::SetSelectedCharacter`, `0x472740`).
fn remembered_row(cvars: &crate::cvars::Cvars, len: usize) -> usize {
    cvars
        .get(CVAR_LAST_CHARACTER)
        .and_then(last_character_row)
        .filter(|&row| row < len)
        .unwrap_or(0)
}

/// Mirror the character entering the world into [`CVAR_LAST_CHARACTER`], at Enter World, not on
/// a click: `CGlueMgr::EnterWorld` (`0x46b500`) writes the CVar at `0x46b5fa`. The reference
/// flushes `Config.wtf` in the same call (`0x46b6f6`); this reaches disk on exit with every CVar.
fn persist_last_character(
    roster: Res<Roster>,
    mut cvars: ResMut<crate::cvars::Cvars>,
    // The registry outlives every VM, so a row written once stays written.
    mut mirrored: Local<Option<usize>>,
) {
    let Some(row) = roster.pending_index() else {
        return;
    };
    if *mirrored == Some(row) {
        return;
    }
    cvars.set(CVAR_LAST_CHARACTER, &last_character_value(row));
    *mirrored = Some(row);
}

/// Ask the parked IO thread to log in as `guid` and remember it as pending; the probe rig picks
/// through here too, so a reconnect re-answers with its character.
pub fn send_pick(roster: &mut Roster, pick: &CharPick, guid: u64) {
    roster.pending_pick = Some(guid);
    let _ = pick.0.send(CharRequest::Enter(guid));
}

/// Drain each [`CharListMessage`] into the roster, then answer it with the pending pick
/// (reconnect), the launch plan's character and Enter World (first roster only), or nothing, leaving
/// the screen up.
fn apply_roster_policy(
    mut msgs: MessageReader<CharListMessage>,
    mut roster: ResMut<Roster>,
    pick: Res<CharPick>,
    // The remembered row, read off the registry, the store that outlives every VM.
    cvars: Res<crate::cvars::Cvars>,
    // Present only when a rig drives the run and picks its own character.
    rig: Option<Res<crate::run_mode::RigCharacter>>,
    // A test's stand-in plan; the run's own otherwise.
    launch: Option<Res<crate::launch_plan::LaunchPlan>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !roster.env_read {
        roster.env_read = true;
        // `WOW_RIG` outranks the plan's character: the rig may have to create its character
        // first, which this one-shot fast path cannot wait for.
        let plan = launch
            .as_deref()
            .cloned()
            .unwrap_or_else(crate::launch_plan::plan);
        match rig.as_deref() {
            Some(crate::run_mode::RigCharacter(name)) => {
                if let Some(ignored) = &plan.character {
                    warn!("char select: WOW_RIG names {name} — ignoring the character {ignored}");
                }
            }
            None => {
                roster.env_char = plan.character.clone();
                roster.env_enter = plan.enter_world;
                // The login smoke's optional third field picks and enters.
                if roster.env_char.is_none() {
                    if let Some(name) = std::env::var("WOW_LOGIN_SMOKE")
                        .ok()
                        .and_then(|s| crate::login::smoke_character(&s))
                    {
                        roster.env_char = Some(name);
                        roster.env_enter = true;
                    }
                }
            }
        }
    }
    for msg in msgs.read() {
        roster.chars = msg.characters.clone();
        roster.realm = msg.realm.clone();
        // A still-armed create selects by name, else the last row: the reference's
        // `SELECT_LAST_CHARACTER` is `SelectCharacter(numChars)`, and vmangos enumerates
        // `ORDER BY create_time, guid`. It outranks the remembered row: the reference pushes the
        // CVar's row (`0x472563`) before `CHARACTER_LIST_UPDATE`, whose `selectLast` overwrites it.
        let created = roster.just_created.is_some();
        if created {
            roster.select_created_by_name();
            if let Some(name) = roster.just_created.take() {
                warn!(
                    "char select: created {name:?} is not on the fresh roster — selecting the last row"
                );
                let last = roster.chars.len().checked_sub(1);
                roster.select(last);
            }
        }
        if roster.chars.is_empty() {
            roster.select(None);
        } else if !created {
            // Re-applied on every roster: the reference reads the CVar in the list rebuild
            // (`0x4724d0` to `0x472740`), not once at startup.
            let row = remembered_row(&cvars, roster.chars.len());
            let who = roster.chars[row].name.clone();
            info!("char select: {CVAR_LAST_CHARACTER} selects row {row} ({who})");
            roster.select(Some(row));
        }
        // `WOW_CHARSELECT_PICK=<name>` selects that row and stays on the screen.
        if let Ok(name) = std::env::var("WOW_CHARSELECT_PICK") {
            match roster
                .chars
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(&name))
            {
                Some(row) => {
                    info!("char select: WOW_CHARSELECT_PICK={name} — selecting row {row}");
                    roster.select(Some(row));
                }
                None => warn!("char select: WOW_CHARSELECT_PICK={name} not on this account"),
            }
        }
        if let Some(guid) = roster.pending_pick {
            // A reconnect, or a pick that raced a dying socket: re-answer without the screen.
            send_pick(&mut roster, &pick, guid);
        } else if roster.env_char.is_some() || roster.env_enter {
            // The plan's steps, first roster only: `--character` selects, `--enter-world` enters
            // the selected row (the named one, else the remembered one).
            let name = roster.env_char.take();
            let enter = std::mem::take(&mut roster.env_enter);
            let row = match &name {
                Some(name) => roster
                    .chars
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(name)),
                None => roster.selected,
            };
            match (row, name) {
                (Some(row), name) => {
                    if let Some(name) = name {
                        info!("char select: character {name} — selecting row {row}");
                        roster.select(Some(row));
                    }
                    if enter {
                        let guid = roster.chars[row].guid;
                        info!("char select: entering the world on row {row}");
                        send_pick(&mut roster, &pick, guid);
                    }
                }
                (None, Some(name)) => {
                    warn!("char select: character {name} is not on this account — showing roster");
                    // A driverless run cannot pick another row, so it fails rather than idles.
                    if std::env::var_os("WOW_LOGIN_SMOKE").is_none()
                        && crate::run_mode::fatal_when_driverless(&format!(
                            "character {name} is not on this account"
                        ))
                    {
                        exit.write(AppExit::error());
                    }
                }
                (None, None) => warn!("char select: no character to enter the world with"),
            }
        }
    }
}

/// `Connected` ([`EnteredWorldMessage`]): the world owns the session.
fn enter_on_connected(
    mut msgs: MessageReader<EnteredWorldMessage>,
    mut next: ResMut<NextState<ClientState>>,
) {
    if msgs.read().next().is_some() {
        next.set(ClientState::InWorld);
    }
}

/// The session died: back to the login screen, as `GlueParent.lua`'s `DISCONNECTED_FROM_SERVER`
/// does. Chained after [`enter_on_connected`]: a stream dying during entry drains `Connected` and
/// `Disconnected` in one frame, and the dead session must be the last word.
fn back_on_disconnect(
    mut msgs: MessageReader<crate::app_core::session::DisconnectedMessage>,
    mut roster: ResMut<Roster>,
    mut next: ResMut<NextState<ClientState>>,
) {
    if !msgs.read().any(|m| m.session_over) {
        return;
    }
    // With no reconnect coming, a kept pick would auto-answer the next roster.
    roster.pending_pick = None;
    next.set(ClientState::Login);
}

/// `SMSG_CHARACTER_LOGIN_FAILED`: back to select with the refusal in the glue dialog. The IO
/// thread announces the entry with the pick, so this undoes a half-built one. The pick must be
/// cleared, or the relist behind the refusal is auto-answered and the refused login loops.
fn back_on_login_refused(
    mut msgs: MessageReader<CharacterLoginFailedMessage>,
    mut roster: ResMut<Roster>,
    mut next: ResMut<NextState<ClientState>>,
    mut dialog: ResMut<crate::glue::dialog::GlueDialog>,
    strings: Option<Res<GlueStrings>>,
) {
    let Some(msg) = msgs.read().last().copied() else {
        return;
    };
    roster.pending_pick = None;
    next.set(ClientState::CharSelect);
    let empty = GlueStrings::default();
    let strings = strings.as_deref().unwrap_or(&empty);
    dialog.open_error(char_login_refusal_text(strings, msg.result));
}

/// The refusal byte as the reference's words, its jump table `0x5aae08`: the byte is a 1-based
/// reason index (`0x5aad70` does `dec eax; cmp eax,5; ja <default>`), mapping `1..=6` onto status
/// codes `0x3e 0x3f 0x40 0x42 0x43 0x44`, skipping `CHAR_LOGIN_FAILED` (`0x41`), the default arm.
/// vmangos sends `1` for all its refusal guards, so it always reads "World server is down".
pub(crate) fn char_login_refusal_text(strings: &GlueStrings, result: u8) -> &str {
    let (key, fallback): (&str, &str) = match result {
        1 => ("CHAR_LOGIN_NO_WORLD", "World server is down"),
        2 => (
            "CHAR_LOGIN_DUPLICATE_CHARACTER",
            "A character with that name already exists",
        ),
        3 => (
            "CHAR_LOGIN_NO_INSTANCES",
            "No instance servers are available",
        ),
        4 => (
            "CHAR_LOGIN_DISABLED",
            "Login for that race, class, or character is currently disabled.",
        ),
        5 => ("CHAR_LOGIN_NO_CHARACTER", "Character not found"),
        6 => (
            "CHAR_LOGIN_LOCKED_FOR_TRANSFER",
            "Your character is currently locked as part of the paid character transfer process.",
        ),
        // `0` and `7` up: the default arm, the only way to `CHAR_LOGIN_FAILED`.
        _ => ("CHAR_LOGIN_FAILED", "Login failed"),
    };
    strings.text(key, fallback)
}

/// A confirmed `/logout`: back to select, the pick cleared so the next roster is shown.
fn back_on_logout(
    mut msgs: MessageReader<LoggedOutMessage>,
    mut roster: ResMut<Roster>,
    mut next: ResMut<NextState<ClientState>>,
) {
    if msgs.read().next().is_some() {
        roster.pending_pick = None;
        next.set(ClientState::CharSelect);
    }
}

/// Surface a refused delete in the glue dialog (vmangos refuses a guild master). The reference's
/// `SMSG_CHAR_DELETE` handler (`0x5b45c0`) passes the byte through as the status (`0x5ab0e0`), and
/// on failure `CGlueMgr::Update` (`0x46c14e`) opens the one-button `OKAY` dialog (`0x46c177`
/// pushes `"OKAY"`, `0x837084`) with the key table `0x85cae8`'s text. A success needs nothing:
/// `net::io` re-enumerates first.
///
/// Native refusals ride the same system (the transport has already published the refreshed roster
/// on success, `9bab9156`): the 12340 key's text; an unlisted code reads `CHAR_DELETE_FAILED`.
///
/// The native character services (rename, customize, faction and race change) answer here too, by
/// the same table: the transport publishes the refreshed roster before an applied change
/// (`2163ca6b`), so a success only takes down the `CHAR_RENAME_IN_PROGRESS` status and a refusal
/// replaces it with the key's dialog. The status's Cancel closes it (labelled: native's
/// `StatusDialogClick` for the rename state is not traced).
/// Labelled: a declined-names refusal (a `u32`, outside the table) reads `RESPONSE_FAILURE`; the
/// realm-split reply has no character-screen surface here.
/// A glue dialog an earlier link of the select chain raises, by its key: the one select system
/// that writes the glue dialog is [`delete_result`], since another writer would run unordered
/// against the login screen's copies of it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GlueRaise {
    /// The one-button `OKAY` dialog (Enter World's lock refusals).
    Error(&'static str),
    /// The `CANCEL` status dialog (`CHAR_RENAME_IN_PROGRESS`).
    Status(&'static str),
}

fn delete_result(
    native: Option<MessageReader<crate::net::WrathCharActionResultMessage>>,
    services: Option<MessageReader<service_reply::CharServiceReply>>,
    mut dialog: ResMut<crate::glue::dialog::GlueDialog>,
    strings: Option<Res<GlueStrings>>,
    (answers, raises): (
        Option<MessageReader<crate::glue::dialog::GlueDialogAnswer>>,
        Option<MessageReader<GlueRaise>>,
    ),
) {
    use crate::glue::dialog::DialogKind;
    use wrathplus_protocol::wrath_char_services::Reply;
    let empty = GlueStrings::default();
    let strings = strings.as_deref().unwrap_or(&empty);
    for raise in raises
        .into_iter()
        .flat_map(|mut r| r.read().copied().collect::<Vec<_>>())
    {
        match raise {
            GlueRaise::Error(key) => dialog.open_error(strings.text(key, key)),
            GlueRaise::Status(key) => dialog.open_status(strings.text(key, key)),
        }
    }
    for answer in answers
        .into_iter()
        .flat_map(|mut r| r.read().copied().collect::<Vec<_>>())
    {
        if answer.kind == DialogKind::Status && answer.button1 {
            dialog.close();
        }
    }
    for reply in service_reply::replies_for(services, ClientState::CharSelect) {
        let code = match &reply {
            _ if reply.changed_roster() => {
                if dialog.kind == Some(DialogKind::Status) {
                    dialog.close();
                }
                continue;
            }
            Reply::Rename { result, .. }
            | Reply::Customize { result, .. }
            | Reply::ChangeRace { result, .. } => *result,
            Reply::DeclinedNames { result: 0, .. } | Reply::RealmSplit { .. } => continue,
            Reply::DeclinedNames { .. } => 1,
        };
        let key = wrath_char_result_key(code).unwrap_or("RESPONSE_FAILURE");
        let text = strings.text(key, "Character change failed");
        info!("char select: character service refused (code {code:#04x}) — {text}");
        dialog.open_error(text);
    }
    for msg in native
        .into_iter()
        .flat_map(|mut r| r.read().copied().collect::<Vec<_>>())
    {
        if msg.action != CharAction::Delete
            || msg.code == wrathplus_protocol::wrath_world::CHAR_DELETE_SUCCESS
        {
            continue;
        }
        let key = wrath_char_result_key(msg.code).unwrap_or("CHAR_DELETE_FAILED");
        let text = strings.text(key, "Character deletion failed");
        info!(
            "char select: native delete refused (code {:#04x}) — {text}",
            msg.code
        );
        dialog.open_error(text);
    }
}

/// Build 12340's glue status keys, the `RESPONSE_*` table at `0xad91f0` indexed by result code
/// (AzerothCore `ResponseCodes`), `0x67` long (`0x6b1100`'s bound, `0x6b0f40`).
const WRATH_RESPONSE_KEYS: [&str; 104] = [
    "RESPONSE_SUCCESS",
    "RESPONSE_FAILURE",
    "RESPONSE_CANCELLED",
    "RESPONSE_DISCONNECTED",
    "RESPONSE_FAILED_TO_CONNECT",
    "RESPONSE_CONNECTED",
    "RESPONSE_VERSION_MISMATCH",
    "CSTATUS_CONNECTING",
    "CSTATUS_NEGOTIATING_SECURITY",
    "CSTATUS_NEGOTIATION_COMPLETE",
    "CSTATUS_NEGOTIATION_FAILED",
    "CSTATUS_AUTHENTICATING",
    "AUTH_OK",
    "AUTH_FAILED",
    "AUTH_REJECT",
    "AUTH_BAD_SERVER_PROOF",
    "AUTH_UNAVAILABLE",
    "AUTH_SYSTEM_ERROR",
    "AUTH_BILLING_ERROR",
    "AUTH_BILLING_EXPIRED",
    "AUTH_VERSION_MISMATCH",
    "AUTH_UNKNOWN_ACCOUNT",
    "AUTH_INCORRECT_PASSWORD",
    "AUTH_SESSION_EXPIRED",
    "AUTH_SERVER_SHUTTING_DOWN",
    "AUTH_ALREADY_LOGGING_IN",
    "AUTH_LOGIN_SERVER_NOT_FOUND",
    "AUTH_WAIT_QUEUE",
    "AUTH_BANNED",
    "AUTH_ALREADY_ONLINE",
    "AUTH_NO_TIME",
    "AUTH_DB_BUSY",
    "AUTH_SUSPENDED",
    "AUTH_PARENTAL_CONTROL",
    "AUTH_LOCKED_ENFORCED",
    "REALM_LIST_IN_PROGRESS",
    "REALM_LIST_SUCCESS",
    "REALM_LIST_FAILED",
    "REALM_LIST_INVALID",
    "REALM_LIST_REALM_NOT_FOUND",
    "ACCOUNT_CREATE_IN_PROGRESS",
    "ACCOUNT_CREATE_SUCCESS",
    "ACCOUNT_CREATE_FAILED",
    "CHAR_LIST_RETRIEVING",
    "CHAR_LIST_RETRIEVED",
    "CHAR_LIST_FAILED",
    "CHAR_CREATE_IN_PROGRESS",
    "CHAR_CREATE_SUCCESS",
    "CHAR_CREATE_ERROR",
    "CHAR_CREATE_FAILED",
    "CHAR_CREATE_NAME_IN_USE",
    "CHAR_CREATE_DISABLED",
    "CHAR_CREATE_PVP_TEAMS_VIOLATION",
    "CHAR_CREATE_SERVER_LIMIT",
    "CHAR_CREATE_ACCOUNT_LIMIT",
    "CHAR_CREATE_SERVER_QUEUE",
    "CHAR_CREATE_ONLY_EXISTING",
    "CHAR_CREATE_EXPANSION",
    "CHAR_CREATE_EXPANSION_CLASS",
    "CHAR_CREATE_LEVEL_REQUIREMENT",
    "CHAR_CREATE_UNIQUE_CLASS_LIMIT",
    "CHAR_CREATE_CHARACTER_IN_GUILD",
    "CHAR_CREATE_RESTRICTED_RACECLASS",
    "CHAR_CREATE_CHARACTER_CHOOSE_RACE",
    "CHAR_CREATE_CHARACTER_ARENA_LEADER",
    "CHAR_CREATE_CHARACTER_DELETE_MAIL",
    "CHAR_CREATE_CHARACTER_SWAP_FACTION",
    "CHAR_CREATE_CHARACTER_RACE_ONLY",
    "CHAR_CREATE_CHARACTER_GOLD_LIMIT",
    "CHAR_CREATE_FORCE_LOGIN",
    "CHAR_DELETE_IN_PROGRESS",
    "CHAR_DELETE_SUCCESS",
    "CHAR_DELETE_FAILED",
    "CHAR_DELETE_FAILED_LOCKED_FOR_TRANSFER",
    "CHAR_DELETE_FAILED_GUILD_LEADER",
    "CHAR_DELETE_FAILED_ARENA_CAPTAIN",
    "CHAR_LOGIN_IN_PROGRESS",
    "CHAR_LOGIN_SUCCESS",
    "CHAR_LOGIN_NO_WORLD",
    "CHAR_LOGIN_DUPLICATE_CHARACTER",
    "CHAR_LOGIN_NO_INSTANCES",
    "CHAR_LOGIN_FAILED",
    "CHAR_LOGIN_DISABLED",
    "CHAR_LOGIN_NO_CHARACTER",
    "CHAR_LOGIN_LOCKED_FOR_TRANSFER",
    "CHAR_LOGIN_LOCKED_BY_BILLING",
    "CHAR_LOGIN_LOCKED_BY_MOBILE_AH",
    "CHAR_NAME_SUCCESS",
    "CHAR_NAME_FAILURE",
    "CHAR_NAME_NO_NAME",
    "CHAR_NAME_TOO_SHORT",
    "CHAR_NAME_TOO_LONG",
    "CHAR_NAME_INVALID_CHARACTER",
    "CHAR_NAME_MIXED_LANGUAGES",
    "CHAR_NAME_PROFANE",
    "CHAR_NAME_RESERVED",
    "CHAR_NAME_INVALID_APOSTROPHE",
    "CHAR_NAME_MULTIPLE_APOSTROPHES",
    "CHAR_NAME_THREE_CONSECUTIVE",
    "CHAR_NAME_INVALID_SPACE",
    "CHAR_NAME_CONSECUTIVE_SPACES",
    "CHAR_NAME_RUSSIAN_CONSECUTIVE_SILENT_CHARACTERS",
    "CHAR_NAME_RUSSIAN_SILENT_CHARACTER_AT_BEGINNING_OR_END",
    "CHAR_NAME_DECLENSION_DOESNT_MATCH_BASE_NAME",
];

/// A native character-screen result code's status key (`0x6b1100`): create, delete, login and the
/// rename/customize/faction-change `CHAR_NAME_*` and `RESPONSE_*` codes alike; `None` past the
/// table.
pub fn wrath_char_result_key(code: u8) -> Option<&'static str> {
    WRATH_RESPONSE_KEYS.get(usize::from(code)).copied()
}

/// Glue-flow smoke (`WOW_GLUE_ROUNDTRIP=1`): once a roster is up, go to CharCreate, Back to
/// CharSelect, and exit. Ungated, since it crosses states.
fn debug_glue_roundtrip(
    roster: Res<Roster>,
    state: Res<State<ClientState>>,
    mut next: ResMut<NextState<ClientState>>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    mut phase: Local<u8>,
    mut mark: Local<f32>,
) {
    if std::env::var("WOW_GLUE_ROUNDTRIP").is_err() {
        return;
    }
    let now = time.elapsed_secs();
    match *phase {
        0 if !roster.chars.is_empty() && *state.get() == ClientState::CharSelect => {
            info!(
                "glue-roundtrip: initial roster = {} char(s) → entering CharCreate",
                roster.chars.len()
            );
            next.set(ClientState::CharCreate);
            (*phase, *mark) = (1, now);
        }
        1 if *state.get() == ClientState::CharCreate && now - *mark > 1.5 => {
            info!("glue-roundtrip: in CharCreate → Back to CharSelect");
            next.set(ClientState::CharSelect);
            (*phase, *mark) = (2, now);
        }
        2 if *state.get() == ClientState::CharSelect && now - *mark > 1.5 => {
            info!(
                "glue-roundtrip: back at CharSelect, roster = {} char(s) — done",
                roster.chars.len()
            );
            exit.write(AppExit::Success);
            *phase = 3;
        }
        _ => {}
    }
}

/// The logout smoke (`WOW_LOGOUT_SMOKE=1`, with `WOW_CHAR`): once in the world, `/logout`, return
/// to CharSelect, enter the world again and close the window. The second entry streams a map
/// `terrain_stream::release_world` tore down. It ends by a window close, not an `AppExit`, since
/// only the close reaches the shutdown tail that saves variables and addon files.
fn debug_logout_smoke(
    state: Res<State<ClientState>>,
    player: Res<crate::app_core::player::Player>,
    commands: Res<crate::net::NetCommands>,
    mut roster: ResMut<Roster>,
    pick: Res<CharPick>,
    streamer: Res<wrathplus_world::terrain_stream::TerrainStreamer>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    mut close: MessageWriter<bevy::window::WindowCloseRequested>,
    windows: Query<Entity, With<bevy::window::PrimaryWindow>>,
    mut phase: Local<u8>,
    mut mark: Local<f32>,
) {
    if std::env::var("WOW_LOGOUT_SMOKE").is_err() {
        return;
    }
    let now = time.elapsed_secs();
    match *phase {
        0 if *state.get() == ClientState::InWorld && player.active => {
            info!("logout-smoke: seated in world — lingering");
            (*phase, *mark) = (1, now);
        }
        1 if now - *mark > 3.0 => {
            info!("logout-smoke: requesting logout");
            let _ = commands.0.send(crate::net::ClientCommand::Logout);
            *phase = 2;
        }
        2 if *state.get() == ClientState::CharSelect => {
            info!("logout-smoke: back at character select — lingering");
            (*phase, *mark) = (3, now);
        }
        // Residency is read here, not on arrival: `release_world` runs about 150 ms after the
        // state edge, on the world-live falling edge.
        3 if now - *mark > 4.0 => match roster.chars.first().map(|c| (c.guid, c.name.clone())) {
            Some((guid, name)) => {
                info!(
                    "logout-smoke: {} tiles resident after the release (must be 0)",
                    streamer.residency().1
                );
                info!("logout-smoke: re-entering the world as {name}");
                send_pick(&mut roster, &pick, guid);
                (*phase, *mark) = (4, now);
            }
            None => {
                warn!("logout-smoke: empty roster — cannot test re-entry");
                *phase = 5;
            }
        },
        4 if *state.get() == ClientState::InWorld && player.active && now - *mark > 3.0 => {
            // The suppressor reading catches a character that re-entered unable to move;
            // `scripts/smoke.sh` fails on anything but `none`. A GM account's logout is instant,
            // not rooted, so it does not exercise the rooted path.
            info!(
                "logout-smoke: re-entered — {} tiles resident, suppressors: {}, done",
                streamer.residency().1,
                player.movement_suppressors()
            );
            *phase = 5;
        }
        5 => {
            info!("logout-smoke: done");
            // Close the window as a player does; a headless run has none and exits directly.
            match windows.single() {
                Ok(window) => {
                    close.write(bevy::window::WindowCloseRequested { window });
                }
                Err(_) => {
                    warn!("logout-smoke: no primary window — exiting by AppExit instead");
                    exit.write(AppExit::Success);
                }
            }
            *phase = 6;
        }
        _ => {}
    }
}

/// The select walk (`WOW_CHARSELECT_WALK=<period_s>[:<enter_after_s>]`): select each row in turn,
/// one per `period_s`, logging each step with its time; with `enter_after_s`, press Enter World
/// that long after the last step.
fn debug_select_walk(
    mut roster: ResMut<Roster>,
    pick: Res<CharPick>,
    time: Res<Time>,
    mut plan: Local<Option<(f32, Option<f32>)>>,
    mut next_at: Local<Option<f32>>,
    mut row: Local<usize>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (period, enter_after) = *plan.get_or_insert_with(|| {
        let Ok(spec) = std::env::var("WOW_CHARSELECT_WALK") else {
            return (f32::INFINITY, None);
        };
        let (p, e) = spec.split_once(':').unwrap_or((spec.as_str(), ""));
        (
            p.trim().parse::<f32>().unwrap_or(3.0),
            e.trim().parse::<f32>().ok(),
        )
    });
    if !period.is_finite() {
        *done = true;
        return;
    }
    if roster.chars.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let at = *next_at.get_or_insert(now + period);
    if now < at {
        return;
    }
    if *row < roster.chars.len() {
        let i = *row;
        *row += 1;
        let c = &roster.chars[i];
        info!(
            "char select: WALK t={now:.3} → row {i} ({}, race {}, class {})",
            c.name, c.race, c.class
        );
        roster.select(Some(i));
        // The last row waits `enter_after`, so the entry starts from a settled screen.
        *next_at = Some(
            now + if *row == roster.chars.len() {
                enter_after.unwrap_or(period)
            } else {
                period
            },
        );
        return;
    }
    *done = true;
    if enter_after.is_none() {
        return;
    }
    if let Some((guid, name)) = roster.selected_char().map(|c| (c.guid, c.name.clone())) {
        info!("char select: WALK t={now:.3} → ENTER WORLD as {name}");
        send_pick(&mut roster, &pick, guid);
    }
}

/// When the select screen came up, present only while it is; the screen's timed instruments
/// measure from it.
#[derive(Resource, Clone, Copy)]
struct CharSelectEnteredAt(f32);

impl CharSelectEnteredAt {
    fn elapsed(&self, time: &Time) -> f32 {
        time.elapsed_secs() - self.0
    }
}

fn stamp_select_entry(mut commands: Commands, time: Res<Time>) {
    commands.insert_resource(CharSelectEnteredAt(time.elapsed_secs()));
}

fn clear_select_entry(mut commands: Commands) {
    commands.remove_resource::<CharSelectEnteredAt>();
}

/// `WOW_CHARSELECT_DIALOG=<typed>`: open the delete dialog for the selection 4 s after the screen
/// is up, with `<typed>` pre-typed, for a shot.
fn debug_select_dialog(
    roster: Res<Roster>,
    mut dialog: ResMut<dialog::DeleteDialog>,
    time: Res<Time>,
    entered_at: Res<CharSelectEnteredAt>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(typed) = std::env::var("WOW_CHARSELECT_DIALOG") else {
        *done = true;
        return;
    };
    if entered_at.elapsed(&time) < 4.0 {
        return;
    }
    let Some(c) = roster.selected_char() else {
        return;
    };
    let (guid, name, level, class) = (c.guid, c.name.clone(), c.level, class_name(c.class));
    dialog.open_for(guid, name, level, class);
    dialog.typed.set_text(&typed);
    info!("char select: dialog instrument opened the delete confirm");
    *done = true;
}

/// `WOW_CHARSELECT_ADDONS=1`: open the AddOns panel 4 s after the screen is up, as its button
/// does, for a shot.
fn debug_select_addons(
    roster: Res<Roster>,
    mut panel: ResMut<addons::AddonsPanel>,
    time: Res<Time>,
    entered_at: Res<CharSelectEnteredAt>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    if std::env::var("WOW_CHARSELECT_ADDONS").is_err() {
        *done = true;
        return;
    }
    if entered_at.elapsed(&time) < 4.0 {
        return;
    }
    if roster.chars.is_empty() {
        return;
    }
    let realm = roster
        .realm
        .as_ref()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Realm".into());
    let chars = roster.chars.iter().map(|c| c.name.clone()).collect();
    panel.open_for(realm, chars);
    info!("char select: addons instrument opened the AddOn List");
    *done = true;
}

/// Default delay of the select shot, seconds from the screen coming up.
const SELECT_SHOT_AT: f32 = 8.0;

/// The select shot (`WOW_CHARSELECT_SHOT_OUT=<path>`): one PNG of the window by framebuffer
/// readback, [`SELECT_SHOT_AT`] seconds after the screen is up, or `WOW_CHARSELECT_SHOT_AT`.
fn debug_select_shot(
    mut commands: Commands,
    time: Res<Time>,
    entered_at: Res<CharSelectEnteredAt>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(out) = std::env::var("WOW_CHARSELECT_SHOT_OUT") else {
        *done = true;
        return;
    };
    let at = std::env::var("WOW_CHARSELECT_SHOT_AT")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(SELECT_SHOT_AT);
    if entered_at.elapsed(&time) < at {
        return;
    }
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(out.clone()));
    info!("char select: shot instrument writing {out}");
    *done = true;
}

pub(crate) use crate::ui_text::wow_font;

// ── Display names ──────────────────────────────────────────────────────────────────────────

pub(crate) fn race_name(race: u8) -> &'static str {
    match race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        10 => "Blood Elf",
        11 => "Draenei",
        _ => "?",
    }
}

pub(crate) fn class_name(class: u8) -> &'static str {
    match class {
        1 => "Warrior",
        2 => "Paladin",
        3 => "Hunter",
        4 => "Rogue",
        5 => "Priest",
        6 => "Death Knight",
        7 => "Shaman",
        8 => "Mage",
        9 => "Warlock",
        11 => "Druid",
        _ => "?",
    }
}

/// A `Character` with only its identity filled in.
#[cfg(any(test, feature = "test-support"))]
pub fn test_character(guid: u64, name: &str) -> Character {
    Character {
        guid,
        name: name.to_string(),
        race: 1,
        class: 1,
        gender: 0,
        skin: 0,
        face: 0,
        hair_style: 0,
        hair_color: 0,
        facial_hair: 0,
        level: 1,
        zone: 0,
        map: 0,
        position: wrathplus_protocol::wire::Vector3d {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        flags: 0,
        customize_flags: 0,
        equipment: [wrathplus_protocol::CharEnumItem::default(); 19],
        pet_display_id: 0,
        pet_level: 0,
        pet_family: 0,
    }
}

/// The roster policy and its CVar mirror without the screen, for tests over a real CVar host.
#[cfg(any(test, feature = "test-support"))]
pub fn add_test_systems(app: &mut App, pick: crossbeam_channel::Sender<CharRequest>) {
    app.insert_state(ClientState::CharSelect)
        .init_resource::<Roster>()
        .insert_resource(CharPick(pick))
        .add_message::<CharListMessage>()
        .add_systems(
            Update,
            (apply_roster_policy, persist_last_character).chain(),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::test_character as character;

    /// A socket dying mid-entry (a displacement kick) drains `Connected` and `Disconnected` in one
    /// frame; the ordering alone makes the disconnect win.
    #[test]
    fn a_session_lost_during_entry_beats_the_entry() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(ClientState::CharSelect)
            .init_resource::<Roster>()
            .init_resource::<crate::app_core::ui_flags::PlayerUiHover>()
            .init_resource::<crate::app_core::ui_flags::UiKeyboardCapture>()
            .add_message::<EnteredWorldMessage>()
            .add_message::<crate::app_core::session::DisconnectedMessage>()
            .add_systems(Update, (enter_on_connected, back_on_disconnect).chain());
        app.world_mut().resource_mut::<Roster>().pending_pick = Some(7);

        app.world_mut().write_message(EnteredWorldMessage {
            billing_time_rested: 0,
            tutorial_flags: None,
        });
        app.world_mut()
            .write_message(crate::app_core::session::DisconnectedMessage {
                reason: "disconnected: world stream closed: failed to fill whole buffer".into(),
                end: wrathplus_protocol::SessionEnd::Lost,
                session_over: true,
            });
        app.update();
        app.update(); // `StateTransition` applies the pending state at the next frame

        assert_eq!(
            *app.world().resource::<State<ClientState>>().get(),
            ClientState::Login,
            "the dead session must be the last word — the reference's DISCONNECTED_FROM_SERVER \
             puts the client back on the account screen, not into a world with no session",
        );
        assert_eq!(
            app.world().resource::<Roster>().pending_pick,
            None,
            "and the reconnect's pending pick goes with it, or the next roster walks straight \
             back into the world the player was just thrown out of",
        );
    }

    /// A teardown whose session is not over (a logout, or a reconnect) still enters the world.
    #[test]
    fn a_teardown_that_is_not_the_end_leaves_the_entry_alone() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(ClientState::CharSelect)
            .init_resource::<Roster>()
            .init_resource::<crate::app_core::ui_flags::PlayerUiHover>()
            .init_resource::<crate::app_core::ui_flags::UiKeyboardCapture>()
            .add_message::<EnteredWorldMessage>()
            .add_message::<crate::app_core::session::DisconnectedMessage>()
            .add_systems(Update, (enter_on_connected, back_on_disconnect).chain());

        app.world_mut().write_message(EnteredWorldMessage {
            billing_time_rested: 0,
            tutorial_flags: None,
        });
        app.world_mut()
            .write_message(crate::app_core::session::DisconnectedMessage {
                reason: "logged out".into(),
                end: wrathplus_protocol::SessionEnd::LoggedOut,
                session_over: false,
            });
        app.update();
        app.update();

        assert_eq!(
            *app.world().resource::<State<ClientState>>().get(),
            ClientState::InWorld,
        );
    }

    /// `Connected` is emitted with `CMSG_PLAYER_LOGIN`, and vmangos refuses before touching the
    /// database, so the refusal lands in the same drain and must be the last word.
    #[test]
    fn a_refused_login_beats_the_entry_it_revokes() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(ClientState::CharSelect)
            .init_resource::<Roster>()
            .init_resource::<crate::app_core::ui_flags::PlayerUiHover>()
            .init_resource::<crate::app_core::ui_flags::UiKeyboardCapture>()
            .add_message::<EnteredWorldMessage>()
            .add_message::<CharacterLoginFailedMessage>()
            .init_resource::<crate::glue::dialog::GlueDialog>()
            .add_systems(Update, (enter_on_connected, back_on_login_refused).chain());
        app.world_mut().resource_mut::<Roster>().pending_pick = Some(7);

        app.world_mut().write_message(EnteredWorldMessage {
            billing_time_rested: 0,
            tutorial_flags: None,
        });
        app.world_mut()
            .write_message(CharacterLoginFailedMessage { result: 0x01 });
        app.update();
        app.update(); // `StateTransition` applies the pending state at the next frame

        assert_eq!(
            *app.world().resource::<State<ClientState>>().get(),
            ClientState::CharSelect,
            "the refusal must win — the reference leaves the player on the select screen it \
             never actually took them off",
        );
        assert_eq!(
            app.world().resource::<Roster>().pending_pick,
            None,
            "and the pick goes with it: the relist behind the refusal is auto-answered with \
             `pending_pick`, so keeping it would re-enter the character just refused, forever",
        );
        // No GlueStrings here, so the fallback literal; the shipped one is checked below.
        assert_eq!(
            app.world()
                .resource::<crate::glue::dialog::GlueDialog>()
                .text,
            "World server is down",
            "a refusal the player cannot see is the bug this whole path exists to end",
        );
    }

    /// Every refusal byte resolves to the sentence 1.12 ships, off the player's chain. `4` is
    /// `CHAR_LOGIN_DISABLED`: the reference's table skips `CHAR_LOGIN_FAILED` (0x41).
    #[test]
    fn every_refusal_byte_resolves_in_the_real_glue_strings() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let strings = crate::glue_strings::table_from_chain(&mut chain);

        assert_eq!(char_login_refusal_text(&strings, 1), "World server is down");
        assert_eq!(
            char_login_refusal_text(&strings, 4),
            "Login for that race, class, or character is currently disabled.",
        );
        assert_eq!(char_login_refusal_text(&strings, 5), "Character not found");

        // Both ends of the reference's `dec eax; cmp eax,5; ja` guard reach the one default.
        let default = char_login_refusal_text(&strings, 0);
        assert_eq!(default, "Login failed");
        for byte in [7u8, 8, 0x3f, 0x43, 0xff] {
            assert_eq!(char_login_refusal_text(&strings, byte), default);
        }

        // Every named byte must resolve to a real key, or a matching fallback hides a missing one.
        let map = crate::glue_strings::table_from_chain(&mut chain).into_map();
        for key in [
            "CHAR_LOGIN_NO_WORLD",
            "CHAR_LOGIN_DUPLICATE_CHARACTER",
            "CHAR_LOGIN_NO_INSTANCES",
            "CHAR_LOGIN_DISABLED",
            "CHAR_LOGIN_NO_CHARACTER",
            "CHAR_LOGIN_LOCKED_FOR_TRANSFER",
            "CHAR_LOGIN_FAILED",
        ] {
            assert!(map.contains_key(key), "{key} is not in the shipped table");
        }
    }

    /// Build 12340's status table (`0xad91f0`): create 0x2e-0x45, delete 0x46-0x4b, by AC code.
    #[test]
    fn the_native_result_keys_follow_the_12340_table() {
        assert_eq!(wrath_char_result_key(0x2f), Some("CHAR_CREATE_SUCCESS"));
        assert_eq!(wrath_char_result_key(0x32), Some("CHAR_CREATE_NAME_IN_USE"));
        assert_eq!(wrath_char_result_key(0x45), Some("CHAR_CREATE_FORCE_LOGIN"));
        assert_eq!(wrath_char_result_key(0x47), Some("CHAR_DELETE_SUCCESS"));
        assert_eq!(
            wrath_char_result_key(0x4a),
            Some("CHAR_DELETE_FAILED_GUILD_LEADER")
        );
        assert_eq!(wrath_char_result_key(0x2d), Some("CHAR_LIST_FAILED"));
        assert_eq!(wrath_char_result_key(0x57), Some("CHAR_NAME_SUCCESS"));
        assert_eq!(wrath_char_result_key(0), Some("RESPONSE_SUCCESS"));
        assert_eq!(wrath_char_result_key(0x68), None);
    }

    /// A native refused delete opens the dialog with its 12340 key; a native success does not.
    #[test]
    fn a_native_refused_delete_raises_the_glue_dialog() {
        use crate::glue::dialog::{DialogKind, GlueDialog};
        use crate::net::WrathCharActionResultMessage as Native;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<Native>()
            .init_resource::<GlueDialog>()
            .add_systems(Update, delete_result);
        app.world_mut().write_message(Native {
            action: CharAction::Delete,
            code: wrathplus_protocol::wrath_world::CHAR_DELETE_SUCCESS,
        });
        app.update();
        assert!(!app.world().resource::<GlueDialog>().is_open());
        app.world_mut().write_message(Native {
            action: CharAction::Delete,
            code: 0x4a,
        });
        app.update();
        let dialog = app.world().resource::<GlueDialog>();
        assert_eq!(dialog.kind, Some(DialogKind::Error));
        assert_eq!(
            dialog.text, "Character deletion failed",
            "fallback without GlueStrings"
        );
    }

    /// `0x4d9bd0`'s order: the two locks, then the rename flag, else enter.
    #[test]
    fn enter_world_checks_the_locks_then_the_rename_flag() {
        use super::input::{enter_world_gate, EnterGate};
        assert_eq!(enter_world_gate(0), EnterGate::Enter);
        assert_eq!(enter_world_gate(0x2000), EnterGate::Enter, "a ghost enters");
        assert_eq!(
            enter_world_gate(0x4 | 0x4000),
            EnterGate::Locked("CHAR_LOGIN_LOCKED_FOR_TRANSFER")
        );
        assert_eq!(
            enter_world_gate(0x100_0000),
            EnterGate::Locked("CHAR_LOGIN_LOCKED_BY_BILLING")
        );
        assert_eq!(enter_world_gate(0x4000), EnterGate::Rename);
    }

    /// `RenameCharacter`'s refusals before a send: empty, the current name in any case, too short
    /// or long by characters.
    #[test]
    fn a_rename_refuses_its_own_name_and_bad_lengths() {
        use super::dialog::{rename_check, RenameCheck};
        assert_eq!(rename_check("Arthas", ""), RenameCheck::Nothing);
        assert_eq!(
            rename_check("Arthas", "arTHas"),
            RenameCheck::Refuse("CHAR_CREATE_NAME_IN_USE")
        );
        assert_eq!(
            rename_check("Arthas", "A"),
            RenameCheck::Refuse("CHAR_NAME_TOO_SHORT")
        );
        assert_eq!(
            rename_check("Arthas", "Abcdefghijklm"),
            RenameCheck::Refuse("CHAR_NAME_TOO_LONG")
        );
        assert_eq!(
            rename_check("Arthas", "Ärthäsmenethi"),
            RenameCheck::Refuse("CHAR_NAME_TOO_LONG")
        );
        assert_eq!(rename_check("Arthas", "Ärthäs"), RenameCheck::Send);
    }

    /// The 3.3.5 delete results (AzerothCore `ResponseCodes` `0x46..=0x4b`) resolve through the
    /// 12340 status-key table to the sentences the installed GlueStrings ship.
    #[test]
    fn every_delete_result_resolves_in_the_real_glue_strings() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let strings = crate::glue_strings::table_from_chain(&mut chain);
        let text = |code: u8| {
            let key = wrath_char_result_key(code).expect("a CHAR_DELETE_* row");
            assert!(key.starts_with("CHAR_DELETE_"), "{code:#04x}: {key}");
            strings
                .get(key)
                .unwrap_or_else(|| panic!("{key} is not in the shipped table"))
        };
        assert_eq!(text(0x46), "Deleting character");
        assert_eq!(text(0x47), "Character deleted");
        assert_eq!(text(0x48), "Character deletion failed");
        assert!(text(0x49).starts_with("You cannot log in until the character update process"));
        assert!(text(0x4A).contains("Guild Master"));
        assert!(text(0x4B).contains("Arena Captain"));
    }

    /// A created character is selected against the roster already in hand: `net::io` emits the
    /// fresh roster before the create result.
    #[test]
    fn a_created_character_is_selected_from_the_roster_in_hand() {
        let mut roster = Roster {
            chars: vec![
                character(1, "Kerwind"),
                character(2, "Xero"),
                character(3, "Zzbullone"), // appended by the re-enum
            ],
            selected: Some(0),
            ..default()
        };
        roster.note_created("Zzbullone".to_string());
        assert_eq!(roster.selected, Some(2), "the new row must be selected");
        assert!(roster.just_created.is_none(), "and the arm consumed");
    }

    /// The name key is case-insensitive: vmangos's `normalizePlayerName` recases a created name.
    #[test]
    fn the_created_name_matches_the_servers_normalized_spelling() {
        let mut roster = Roster {
            chars: vec![character(1, "Kerwind"), character(2, "Zzbullone")],
            selected: Some(0),
            ..default()
        };
        roster.note_created("ZZBULLONE".to_string()); // as typed
        assert_eq!(roster.selected, Some(1));
    }

    // ── The remembered character (`lastCharacterIndex`) ───────────────────────────

    /// Run [`apply_roster_policy`] over one roster with `stored` as `lastCharacterIndex` (`None`
    /// for never set); returns the row the screen opens on.
    fn roster_policy_over(
        stored: Option<&str>,
        names: &[&str],
        preselected: Option<usize>,
    ) -> Option<usize> {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Roster>()
            .insert_resource(CharPick(tx))
            .insert_resource(match stored {
                Some(v) => {
                    crate::cvars::Cvars::with_rows_and_value(&[CVARS], CVAR_LAST_CHARACTER, v)
                }
                None => crate::cvars::Cvars::with_rows(&[CVARS]),
            })
            .insert_resource(crate::launch_plan::LaunchPlan::default())
            .add_message::<CharListMessage>()
            .add_message::<AppExit>()
            .add_systems(Update, apply_roster_policy);
        if let Some(row) = preselected {
            app.world_mut().resource_mut::<Roster>().select(Some(row));
        }
        app.world_mut().write_message(CharListMessage {
            characters: names
                .iter()
                .enumerate()
                .map(|(i, n)| character(i as u64 + 1, n))
                .collect(),
            realm: None,
        });
        app.update();
        app.world().resource::<Roster>().selected()
    }

    /// The launch plan's character select steps on the first roster: `--character` selects and
    /// stays, `--enter-world` enters the selected (remembered) row, both enter the named one, and
    /// with neither the screen stays up. (A later roster re-applies the remembered row, as the
    /// stock list rebuild does, and re-answers a pending pick, the reconnect path.)
    #[test]
    fn the_launch_plan_answers_only_its_own_select_steps() {
        let run = |plan: crate::launch_plan::LaunchPlan| {
            let (tx, rx) = crossbeam_channel::unbounded();
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .init_resource::<Roster>()
                .insert_resource(CharPick(tx))
                .insert_resource(crate::cvars::Cvars::with_rows_and_value(
                    &[CVARS],
                    CVAR_LAST_CHARACTER,
                    "1",
                ))
                .insert_resource(plan)
                .add_message::<CharListMessage>()
                .add_message::<AppExit>()
                .add_systems(Update, apply_roster_policy);
            let roster = || CharListMessage {
                characters: vec![
                    character(1, "Kerwind"),
                    character(2, "Xero"),
                    character(3, "Zed"),
                ],
                realm: None,
            };
            app.world_mut().write_message(roster());
            app.update();
            let selected = app.world().resource::<Roster>().selected();
            let sent: Vec<u64> = rx
                .try_iter()
                .filter_map(|r| match r {
                    CharRequest::Enter(guid) => Some(guid),
                    _ => None,
                })
                .collect();
            (selected, sent)
        };
        use crate::launch_plan::LaunchPlan as P;
        assert_eq!(
            run(P::default()),
            (Some(1), vec![]),
            "the screen, on the remembered row"
        );
        assert_eq!(
            run(P::for_steps(Some("zed"), false)),
            (Some(2), vec![]),
            "selected, not entered"
        );
        assert_eq!(
            run(P::for_steps(None, true)),
            (Some(1), vec![2]),
            "the remembered row entered"
        );
        assert_eq!(
            run(P::for_steps(Some("Zed"), true)),
            (Some(2), vec![3]),
            "the named row entered"
        );
        assert_eq!(run(P::for_steps(Some("Nobody"), false)), (Some(1), vec![]));
    }

    /// The screen opens on the character last logged in as; the stored value is 0-based.
    #[test]
    fn the_roster_opens_on_the_remembered_character() {
        assert_eq!(
            roster_policy_over(
                Some("2"),
                &["Kerwind", "Xero", "Zzbullone", "Wartwof"],
                None
            ),
            Some(2),
            "a config.toml remembering the third character must select it, not row one",
        );
    }

    /// Nothing stored and `"0"`, the registered default and the first character, both land on
    /// row 0.
    #[test]
    fn nothing_remembered_and_a_stored_zero_are_both_the_first_row() {
        assert_eq!(
            roster_policy_over(None, &["Kerwind", "Xero"], None),
            Some(0)
        );
        assert_eq!(
            roster_policy_over(Some("0"), &["Kerwind", "Xero"], None),
            Some(0),
        );
    }

    /// An out-of-range index goes to the first row, not the nearest (`0x472740`).
    #[test]
    fn a_remembered_row_past_the_end_falls_back_to_the_first() {
        assert_eq!(
            roster_policy_over(Some("9"), &["Kerwind", "Xero"], None),
            Some(0),
            "the reference clamps to 0, not to the last row — `(idx >= count) ? 0 : idx`",
        );
    }

    /// A corrupt value selects the first row.
    #[test]
    fn an_unparseable_remembered_value_is_the_first_row() {
        assert_eq!(
            roster_policy_over(Some("Kerwind"), &["Kerwind", "Xero"], None),
            Some(0),
        );
    }

    /// Every roster re-applies the stored row (the reference reads it in the list rebuild,
    /// `0x4724d0`), so a selection never entered as does not survive a re-enum.
    #[test]
    fn a_re_enumerated_roster_returns_to_the_remembered_character() {
        assert_eq!(
            roster_policy_over(Some("0"), &["Kerwind", "Xero", "Zzbullone"], Some(2)),
            Some(0),
            "row 2 was only ever clicked; the roster rebuild goes back to the remembered row 0",
        );
    }

    /// Clicking the current row keeps the dragged facing (`CharacterSelectButton_OnClick` gates on
    /// `id ~= selectedIndex`); the facing reset rides `select_seq`.
    #[test]
    fn re_clicking_the_selected_row_keeps_the_facing() {
        let mut roster = Roster {
            chars: vec![character(1, "Kerwind"), character(2, "Xero")],
            ..Roster::default()
        };
        roster.click_row(0);
        let squared = roster.select_seq;
        roster.click_row(0);
        assert_eq!(
            roster.select_seq, squared,
            "a click on the row already selected must not re-select — the dragged angle survives",
        );
        roster.click_row(1);
        assert_eq!(
            roster.select_seq,
            squared + 1,
            "…while a click on a DIFFERENT row selects, and squares the character it brings up",
        );
    }

    /// A just-created character outranks the remembered row: `UpdateCharacterList`'s deferred
    /// `selectLast` overwrites the restored index.
    #[test]
    fn a_created_character_outranks_the_remembered_one() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Roster>()
            .insert_resource(CharPick(tx))
            .insert_resource(crate::cvars::Cvars::with_value(CVAR_LAST_CHARACTER, "0"))
            .add_message::<CharListMessage>()
            .add_message::<AppExit>()
            .add_systems(Update, apply_roster_policy);
        // The create result lands before the fresh roster, the order `apply_roster_policy` answers.
        app.world_mut()
            .resource_mut::<Roster>()
            .note_created("Zzbullone".into());
        app.world_mut().write_message(CharListMessage {
            characters: vec![
                character(1, "Kerwind"),
                character(2, "Xero"),
                character(3, "Zzbullone"),
            ],
            realm: None,
        });
        app.update();
        assert_eq!(
            app.world().resource::<Roster>().selected(),
            Some(2),
            "SELECT_LAST_CHARACTER outranks the remembered row 0",
        );
    }

    /// The stored value is 0-based.
    #[test]
    fn the_stored_index_is_zero_based_and_round_trips() {
        for row in [0usize, 1, 4, 9] {
            assert_eq!(last_character_row(&last_character_value(row)), Some(row));
        }
        assert_eq!(
            last_character_value(0),
            "0",
            "row 0 IS the registrar default"
        );
        assert_eq!(
            last_character_row("0"),
            Some(0),
            "0 is the first row, not 'none'"
        );
        assert_eq!(last_character_row(""), None);
    }

    /// The write is at Enter World, not at selection (`0x46b500`).
    #[test]
    fn only_entering_the_world_writes_the_cvar() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Roster>()
            .insert_resource(crate::cvars::Cvars::with_rows(&[CVARS]))
            .insert_resource(CharPick(tx))
            .add_systems(Update, persist_last_character);
        {
            let mut roster = app.world_mut().resource_mut::<Roster>();
            roster.chars = vec![
                character(1, "Kerwind"),
                character(2, "Xero"),
                character(3, "Zz"),
            ];
            roster.select(Some(2));
            roster.select(Some(1));
        }
        app.update();
        assert!(
            !app.world().resource::<crate::cvars::Cvars>().has_events(),
            "a selection alone must NOT be remembered — the reference writes nothing here",
        );

        app.world_mut().resource_mut::<Roster>().pending_pick = Some(2);
        app.update();
        let moved: Vec<(String, String)> = app
            .world_mut()
            .resource_mut::<crate::cvars::Cvars>()
            .take_events()
            .into_iter()
            .map(|e| (e.name, e.new))
            .collect();
        assert_eq!(
            moved,
            vec![(CVAR_LAST_CHARACTER.to_string(), "1".to_string())],
            "guid 2 sits at row 1, and the row is what the registry took",
        );
        // Steady frames stay quiet; the next VM is seeded from the registry.
        app.update();
        let cvars = app.world().resource::<crate::cvars::Cvars>();
        assert!(!cvars.has_events());
        assert!(cvars
            .vm_seed()
            .iter()
            .any(|r| r.name == CVAR_LAST_CHARACTER && r.value == "1"));
    }

    /// Result first, roster after: the arm is answered by the roster policy, and a name not found
    /// falls back to the last row (`SELECT_LAST_CHARACTER`).
    #[test]
    fn an_unanswerable_create_stays_armed() {
        let mut roster = Roster {
            chars: vec![character(1, "Kerwind")],
            selected: Some(0),
            ..default()
        };
        roster.note_created("Zzbulltwo".to_string());
        assert_eq!(
            roster.just_created.as_deref(),
            Some("Zzbulltwo"),
            "no row to select yet — the arm must survive for the roster policy"
        );
        assert_eq!(roster.selected, Some(0), "and nothing is mis-selected");
    }
}
