//! The stock glue screens' calls and the session's progress into them ([`stock_glue`]).

use bevy::prelude::*;
use wrathplus_ui::script::glue::GlueRequest;
use wrathplus_ui::script::{ScriptValue, UiScript};

use crate::glue_strings::GlueStrings;
use crate::net::{LoginFailedMessage, LoginStageMessage};

/// `AccountLogin`'s calls drive the same attempt the native screen does ([`crate::login::Attempt`]),
/// and the attempt's stages and failures reach the stock `GlueDialog` as the engine's status events
/// (`OPEN_STATUS_DIALOG`, `UPDATE_STATUS_DIALOG`, `CLOSE_STATUS_DIALOG`, `GlueDialog.lua`). The
/// login module's `drive_policy` keeps the attempt's bookkeeping in every state.
fn s(text: &str) -> ScriptValue {
    ScriptValue::Str(text.into())
}

/// Open the stock status dialog of `which` (`GlueDialogTypes`) with `text`.
fn open_status(script: &mut UiScript, which: &str, text: &str) {
    script.fire_event("OPEN_STATUS_DIALOG", vec![s(which), s(text)]);
}

/// The stock glue screens' calls and the session's progress into them, while the stock glue hosts
/// the screen: login and its status dialog, the realm list, and character select.
///
/// A one-shot VM consumer by design: its VM is the glue VM `enter_stock_login`/`enter_stock_select`
/// install at OnEnter(Login)/OnEnter(CharSelect), before any Update, and it runs only while that
/// VM hosts the screen (`stock_glue_up`); the in-game interface is never up here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stock_glue(
    script: Option<NonSendMut<UiScript>>,
    mut attempt: crate::login::Attempt,
    strings: Option<Res<GlueStrings>>,
    (mut realms, choice): (
        ResMut<crate::app_core::ui_flags::Realms>,
        Res<crate::net::RealmChoice>,
    ),
    (mut roster, pick, areas): (
        ResMut<crate::app_core::state::Roster>,
        Res<crate::net::CharPick>,
        Option<Res<crate::area::AreaTableRes>>,
    ),
    (mut preview, state, mut next): (
        ResMut<crate::portrait::GluePreview>,
        Res<State<crate::app_core::state::ClientState>>,
        ResMut<NextState<crate::app_core::state::ClientState>>,
    ),
    (mut stages, mut failures): (
        MessageReader<LoginStageMessage>,
        MessageReader<LoginFailedMessage>,
    ),
    mut exit: MessageWriter<AppExit>,
    // A character list from the server not yet announced, and the selection last announced, so
    // a click's selection never replays the list ([`announce_roster`]).
    (mut lists, mut memo): (
        MessageReader<crate::net::CharListMessage>,
        Local<crate::app_core::vm_memo::VmMemo<(bool, u64)>>,
    ),
) {
    use crate::app_core::state::ClientState;
    let Some(mut script) = script else {
        return;
    };
    let empty = GlueStrings::default();
    let strings = strings.as_deref().unwrap_or(&empty);
    for request in script.take_glue_requests() {
        match request {
            GlueRequest::Login { account, password } => {
                if account.is_empty() {
                    let text = strings.text("LOGIN_ENTER_NAME", "Please enter your account name.");
                    open_status(&mut script, "OKAY", text);
                } else if password.is_empty() {
                    let text = strings.text("LOGIN_ENTER_PASSWORD", "Please enter your password.");
                    open_status(&mut script, "OKAY", text);
                } else {
                    info!("login: stock glue submit as {account}");
                    attempt.submit_typed(&account, &password);
                    let text = strings.text("LOGIN_STATE_CONNECTING", "Connecting");
                    open_status(&mut script, "CANCEL", text);
                }
            }
            GlueRequest::CancelLogin | GlueRequest::StatusDialogClick => {
                attempt.abandon_in_flight();
                script.fire_event("CLOSE_STATUS_DIALOG", vec![]);
            }
            GlueRequest::Quit => {
                exit.write(AppExit::Success);
            }
            GlueRequest::SaveAccountName(name) => crate::login::save_account(&name),
            GlueRequest::ChangeRealm { category, index } => {
                crate::realm_select::glue_change_realm(&mut realms, &choice, category, index)
            }
            GlueRequest::RequestRealmList => {
                // From character select (`CharacterSelect_ChangeRealm`), the list opens over it.
                if *state.get() == ClientState::CharSelect && !realms.shown {
                    roster.pending_pick = None;
                    crate::realm_select::open_over_char_select(&mut realms, &choice);
                } else {
                    crate::realm_select::glue_refresh(&choice);
                }
            }
            GlueRequest::RealmListCancelled => {
                crate::realm_select::glue_cancel(&mut realms, &choice)
            }
            // AzerothBench's Realmlist button: the native editor's Okay (normalize, point the
            // session at it, keep it in realmlist.wtf and the CVar).
            GlueRequest::SetRealmlist(text) => {
                let realmlist = attempt.realmlist_mut();
                if crate::login::accept_realmlist(&text, realmlist, Some(&mut *script)) {
                    let address = realmlist.address().to_string();
                    script.set_glue_realmlist(&address);
                    script.set_glue_connection(false, &address);
                } else {
                    open_status(
                        &mut script,
                        "OKAY",
                        &format!("\"{text}\" is not a realmlist address (host or host:port)."),
                    );
                }
                // The Server field re-reads the address, kept or refused (not a stock event).
                script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);
            }
            // ── Character select, onto the native roster and pick ──
            GlueRequest::SelectCharacter(index) => {
                // 1-based, as every stock call site passes it (`CharacterSelect.lua`).
                let n = roster.chars.len();
                let row = index.checked_sub(1).filter(|&i| i < n);
                if row != roster.selected() {
                    let who = row
                        .and_then(|i| roster.chars.get(i))
                        .map(|c| c.name.as_str());
                    info!("char select: stock glue selects row {row:?} ({who:?})");
                }
                roster.select(row);
            }
            GlueRequest::EnterWorld => {
                if let Some(guid) = roster.selected_char().map(|c| c.guid) {
                    crate::char_select::send_pick(&mut roster, &pick, guid);
                }
            }
            GlueRequest::DeleteCharacter(index) => {
                if let Some(c) = index.checked_sub(1).and_then(|i| roster.chars.get(i)) {
                    let _ = pick.0.send(crate::net::CharRequest::Delete(c.guid));
                }
            }
            // Select's Back, as the native one: the parked session dropped, no auto-relogin.
            GlueRequest::Disconnect => {
                roster.pending_pick = None;
                attempt.forget();
                let _ = pick.0.send(crate::net::CharRequest::Abandon);
                next.set(ClientState::Login);
            }
            // The roster arrives with the park; answering with the one in hand is the refresh.
            GlueRequest::RefreshCharacters => roster.set_changed(),
            GlueRequest::SetSelectFacing(degrees) => preview.yaw = degrees.to_radians(),
            GlueRequest::SetBackground(path) => match crate::portrait::stage_from_path(&path) {
                Some(stage) => {
                    preview.scene = Some(crate::portrait::GlueScene::Stage(stage));
                }
                None => warn!("glue: SetBackground({path}) names no stage"),
            },
        }
    }
    for msg in stages.read() {
        let text = crate::login::stage_text(strings, msg.stage).to_string();
        script.fire_event("UPDATE_STATUS_DIALOG", vec![s(&text)]);
    }
    for msg in failures.read() {
        // As the native screen's: the server's words for a terminal failure, else the refusal's
        // or the dial's text.
        let text = if msg.terminal {
            msg.reason.clone()
        } else {
            crate::login::fail_text(strings, msg.refusal, msg.dial.as_ref())
        };
        open_status(&mut script, "OKAY", &text);
    }
    // Reaching the realm list ends the connecting dialog, as `CGlueMgr::UpdateGlueDialog`
    // (`0x46b140`) does once the login state passes `LOGIN_STATE_CONNECTING`. The realm list to
    // show, or a refresh of the one shown: the snapshot, then the engine's `OPEN_REALM_LIST`
    // (shows the frame, or redraws it).
    if realms.shown && realms.is_changed() {
        use crate::realm_select::RealmsCategory;
        script.fire_event("CLOSE_STATUS_DIALOG", vec![]);
        let tabs = crate::realm_select::glue_tabs(realms.bypass_change_detection());
        let selected = realms.selected_category();
        script.set_glue_realms(tabs, selected, crate::realm_select::GLUE_UPDATE_RATE);
        script.fire_event("OPEN_REALM_LIST", vec![]);
    }
    // A roster or a selection change: the rows, the stage's character, and the engine's events.
    // The engine's `CHARACTER_LIST_UPDATE` follows each list the server sends (`SMSG_CHAR_ENUM`),
    // which `char_select` applies to the roster; it is announced once character select is up.
    let shown = memo.get(&script);
    shown.0 |= lists.read().count() > 0;
    if roster.is_changed() {
        let rows = crate::char_select::glue_rows::glue_characters(&roster, areas.as_deref());
        script.set_glue_characters(rows, crate::char_select::glue_rows::EMPTY_SELECT_STAGE);
        let look = roster
            .selected_char()
            .map(|c| crate::portrait::GlueLook::Select(crate::portrait::SelectLook::from(c)));
        if preview.look != look {
            preview.look = look;
        }
        if *state.get() == ClientState::CharSelect {
            let list = std::mem::take(&mut shown.0);
            let selection = shown.1 != roster.select_seq;
            shown.1 = roster.select_seq;
            announce_roster(&mut script, roster.selected(), list, selection);
        }
    }
}

/// Character select's events for a roster change, in the engine's order. The selection goes
/// first, `UPDATE_SELECTED_CHARACTER` being the only writer of stock's `selectedIndex`; then, only
/// when the rows themselves changed, `CHARACTER_LIST_UPDATE`, whose `UpdateCharacterList` re-selects
/// `selectedIndex` through `SelectCharacter`, so it must already be the new row (the reference
/// pushes the remembered row before the list, `0x472563`). A click's selection alone never replays
/// the list: replaying it re-selected the old row a frame later, which Enter World then sent, and
/// the two rows took turns, a flicker.
pub(crate) fn announce_roster(
    script: &mut UiScript,
    selected: Option<usize>,
    list_changed: bool,
    selection_changed: bool,
) {
    if let Some(i) = selected.filter(|_| list_changed || selection_changed) {
        script.fire_event(
            "UPDATE_SELECTED_CHARACTER",
            vec![ScriptValue::Number((i + 1) as f64)],
        );
    }
    if list_changed {
        script.fire_event("CHARACTER_LIST_UPDATE", vec![]);
    }
}
