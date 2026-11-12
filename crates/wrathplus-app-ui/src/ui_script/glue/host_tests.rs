//! The stock glue host, hermetic.

use super::*;
use crate::net::{LoginAbandon, LoginFailedMessage, LoginStageMessage, LoginSubmit};

/// A glue VM whose event frame records each status event as `event:which:text;`.
fn vm() -> UiScript {
    let s = UiScript::new().unwrap();
    s.install_glue_api().unwrap();
    s.run(
        "local f = CreateFrame('Frame') \
         for _, e in ipairs({'OPEN_STATUS_DIALOG', 'UPDATE_STATUS_DIALOG', 'CLOSE_STATUS_DIALOG'}) do f:RegisterEvent(e) end \
         f:SetScript('OnEvent', function(self, e, a, b) LOG = (LOG or '') .. e .. ':' .. tostring(a) .. ':' .. tostring(b) .. ';' end)",
    )
    .unwrap();
    s
}

pub(super) fn app() -> (App, crossbeam_channel::Receiver<crate::net::LoginRequest>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let (realm_tx, _realm_rx) = crossbeam_channel::unbounded();
    let (pick_tx, _pick_rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
        .insert_state(crate::app_core::state::ClientState::Login)
        .init_resource::<crate::app_core::ui_flags::LoginIntent>()
        .insert_resource(LoginSubmit(tx))
        .insert_resource(LoginAbandon(Default::default()))
        .insert_resource(crate::realmlist::Realmlist::at("127.0.0.1"))
        .init_resource::<crate::app_core::ui_flags::Realms>()
        .insert_resource(crate::net::RealmChoice(realm_tx))
        .init_resource::<crate::app_core::state::Roster>()
        .insert_resource(crate::net::CharPick(pick_tx))
        .init_resource::<crate::portrait::GluePreview>()
        .add_message::<LoginStageMessage>()
        .add_message::<LoginFailedMessage>()
        .add_message::<AppExit>()
        .add_message::<crate::net::CharListMessage>()
        .add_systems(Update, stock_glue);
    app.insert_non_send_resource(vm());
    (app, rx)
}

fn log(app: &mut App) -> String {
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .eval::<String>("local l = LOG or '' LOG = nil return l")
        .unwrap()
}

/// `DefaultServerLogin` submits the attempt to the realmlist and opens the stock connecting
/// dialog; a stage updates its text, a refusal opens the OKAY dialog, Cancel closes it.
#[test]
fn the_stock_login_drives_the_attempt_and_the_stock_status_dialog() {
    let (mut app, rx) = app();
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("DefaultServerLogin('Bob', 'hunter2')")
        .unwrap();
    app.update();
    let sent: Vec<_> = rx.try_iter().collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].user.as_str(), sent[0].host.as_str()),
        ("Bob", "127.0.0.1")
    );
    assert!(
        app.world()
            .resource::<crate::app_core::ui_flags::LoginIntent>()
            .in_flight
    );
    assert_eq!(log(&mut app), "OPEN_STATUS_DIALOG:CANCEL:Connecting;");

    app.world_mut().write_message(LoginStageMessage {
        stage: wrathplus_protocol::LoginStage::Authenticating,
    });
    app.update();
    assert_eq!(log(&mut app), "UPDATE_STATUS_DIALOG:Authenticating:nil;");

    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("StatusDialogClick()")
        .unwrap();
    app.update();
    assert_eq!(log(&mut app), "CLOSE_STATUS_DIALOG:nil:nil;");
    assert!(
        !app.world()
            .resource::<crate::app_core::ui_flags::LoginIntent>()
            .in_flight
    );

    // An empty box asks for it instead of dialing.
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("DefaultServerLogin('Bob', '')")
        .unwrap();
    app.update();
    assert_eq!(rx.try_iter().count(), 0);
    assert!(log(&mut app).starts_with("OPEN_STATUS_DIALOG:OKAY:"));
}

/// A realm list to show reaches the stock frames as the snapshot and `OPEN_REALM_LIST`, and the
/// stock `ChangeRealm` answers the park with that row's realm.
#[test]
fn the_stock_realm_list_is_fed_and_answers_the_park() {
    let (realm_tx, realm_rx) = crossbeam_channel::unbounded();
    let (mut app, _rx) = app();
    app.insert_resource(crate::net::RealmChoice(realm_tx));
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(
            "local f = CreateFrame('Frame') f:RegisterEvent('OPEN_REALM_LIST') \
             f:SetScript('OnEvent', function() OPENED = (OPENED or 0) + 1 end)",
        )
        .unwrap();
    {
        let mut realms = app
            .world_mut()
            .resource_mut::<crate::app_core::ui_flags::Realms>();
        realms.realms = ["AzerothCore", "Second"]
            .into_iter()
            .map(|name| wrathplus_protocol::RealmInfo {
                name: name.into(),
                address: "127.0.0.1:8085".into(),
                population: 1.0,
                characters: 1,
                realm_type: 0,
                flags: 0,
                category: 1,
                id: 1,
            })
            .collect();
        realms.shown = true;
    }
    app.update();
    let (opened, second) = {
        let s = app.world_mut().non_send_resource_mut::<UiScript>();
        (
            s.eval::<i64>("return OPENED or 0").unwrap(),
            s.eval::<String>("return (GetRealmInfo(1, 2))").unwrap(),
        )
    };
    assert_eq!(opened, 1);
    assert_eq!(second, "Second");
    app.update();
    assert_eq!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<i64>("return OPENED")
            .unwrap(),
        1,
        "an unchanged list does not reopen"
    );
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("ChangeRealm(1, 2)")
        .unwrap();
    app.update();
    let sent: Vec<_> = realm_rx.try_iter().collect();
    assert!(
        matches!(sent.last(), Some(crate::net::RealmRequest::Enter(name)) if name == "Second"),
        "{sent:?}"
    );
    assert!(
        !app.world()
            .resource::<crate::app_core::ui_flags::Realms>()
            .shown
    );
}

/// The Realmlist button's address becomes the session's; one that is not an address opens the
/// OKAY dialog and changes nothing.
#[test]
fn the_realmlist_button_sets_the_session_address() {
    let (mut app, _rx) = app();
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("WrathPlusSetRealmlist('logon.example.org:3725')")
        .unwrap();
    app.update();
    assert_eq!(
        app.world()
            .resource::<crate::realmlist::Realmlist>()
            .address(),
        "logon.example.org:3725"
    );
    assert_eq!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<String>("return (WrathPlusGetRealmlist())")
            .unwrap(),
        "logon.example.org:3725"
    );
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("WrathPlusSetRealmlist('two words')")
        .unwrap();
    app.update();
    assert!(log(&mut app).starts_with("OPEN_STATUS_DIALOG:OKAY:"));
    assert_eq!(
        app.world()
            .resource::<crate::realmlist::Realmlist>()
            .address(),
        "logon.example.org:3725"
    );
}

/// Character select on the stock frames: the roster reaches the rows and the engine's list
/// events, and the stock calls drive the native roster, pick and booth.
#[test]
fn the_stock_character_select_drives_the_roster_pick_and_booth() {
    use crate::app_core::state::ClientState;
    use crate::app_core::state::Roster;
    use crate::char_select::test_character;
    let (mut app, _rx) = app();
    let (pick_tx, pick_rx) = crossbeam_channel::unbounded();
    app.insert_resource(crate::net::CharPick(pick_tx));
    app.insert_state(ClientState::CharSelect);
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(
            "local f = CreateFrame('Frame') \
             for _, e in ipairs({'CHARACTER_LIST_UPDATE', 'UPDATE_SELECTED_CHARACTER'}) do f:RegisterEvent(e) end \
             f:SetScript('OnEvent', function(self, e, a) SEL = (SEL or '') .. e .. ':' .. tostring(a) .. ';' end)",
        )
        .unwrap();
    {
        let mut roster = app.world_mut().resource_mut::<Roster>();
        roster.chars = vec![test_character(11, "Benchone"), test_character(12, "Second")];
        roster.select(Some(0));
    }
    // The list as the server sent it, which the roster above stands for.
    app.world_mut().write_message(crate::net::CharListMessage {
        characters: vec![],
        realm: None,
    });
    app.update();
    let (sel, second) = {
        let s = app.world_mut().non_send_resource_mut::<UiScript>();
        (
            s.eval::<String>("local l = SEL or '' SEL = nil return l")
                .unwrap(),
            s.eval::<String>("return (GetCharacterInfo(2))").unwrap(),
        )
    };
    // The engine's order: the selection, then the list (`announce_roster`).
    assert_eq!(
        sel,
        "UPDATE_SELECTED_CHARACTER:1;CHARACTER_LIST_UPDATE:nil;"
    );
    assert_eq!(second, "Second");
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(
            "SelectCharacter(2) \
             SetCharSelectBackground('Interface\\\\Glues\\\\Models\\\\UI_Human\\\\UI_Human.m2') \
             SetCharacterSelectFacing(90) EnterWorld()",
        )
        .unwrap();
    app.update();
    assert_eq!(app.world().resource::<Roster>().selected(), Some(1));
    let preview = app.world().resource::<crate::portrait::GluePreview>();
    assert!(preview.scene == Some(crate::portrait::GlueScene::Stage("Human")));
    assert!((preview.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    assert!(
        preview.look.is_some(),
        "the selected character stands on the stage"
    );
    // A background naming no known stage leaves the booth's scene as it was.
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("SetCharSelectBackground('Interface\\\\Custom\\\\UI_Nowhere.m2')")
        .unwrap();
    app.update();
    let preview = app.world().resource::<crate::portrait::GluePreview>();
    assert!(preview.scene == Some(crate::portrait::GlueScene::Stage("Human")));
    let sent: Vec<_> = pick_rx.try_iter().collect();
    assert!(
        matches!(sent[..], [crate::net::CharRequest::Enter(12)]),
        "{} requests sent",
        sent.len()
    );
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run("DisconnectFromServer()")
        .unwrap();
    app.update();
    app.update(); // the state transition applies
    assert!(matches!(
        pick_rx.try_iter().collect::<Vec<_>>()[..],
        [crate::net::CharRequest::Abandon]
    ));
    assert_eq!(
        *app.world().resource::<State<ClientState>>().get(),
        ClientState::Login
    );
}

/// Two realms up, the list shown, and a frame counting `OPEN_REALM_LIST` on the VM in place.
fn show_realm_list(app: &mut App) {
    count_realm_list_opens(app);
    let mut realms = app
        .world_mut()
        .resource_mut::<crate::app_core::ui_flags::Realms>();
    realms.realms = ["AzerothCore", "Second"]
        .into_iter()
        .map(|name| wrathplus_protocol::RealmInfo {
            name: name.into(),
            address: "127.0.0.1:8085".into(),
            population: 1.0,
            characters: 1,
            realm_type: 0,
            flags: 0,
            category: 1,
            id: 1,
        })
        .collect();
    realms.shown = true;
}

fn count_realm_list_opens(app: &mut App) {
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .run(
            "local f = CreateFrame('Frame') f:RegisterEvent('OPEN_REALM_LIST') \
             f:SetScript('OnEvent', function() OPENED = (OPENED or 0) + 1 end)",
        )
        .unwrap();
}

fn realm_list_opens(app: &mut App) -> i64 {
    app.world_mut()
        .non_send_resource_mut::<UiScript>()
        .eval::<i64>("return OPENED or 0")
        .unwrap()
}

/// A realm list raised before the glue VM is built is shown on the glue VM when it comes: an
/// auto-submitted login reaches the list while the startup cover still holds the glue VM back,
/// and the list had gone to the VM before it, leaving the login screen up with no list on it.
#[test]
fn a_realm_list_raised_before_the_glue_vm_is_shown_on_it() {
    let (mut app, _rx) = app();
    show_realm_list(&mut app);
    app.update();
    assert_eq!(realm_list_opens(&mut app), 1);
    // The glue VM replaces the one the list went to; nothing about the list changes.
    app.insert_non_send_resource(vm());
    count_realm_list_opens(&mut app);
    app.update();
    assert_eq!(realm_list_opens(&mut app), 1, "shown on the new VM");
    assert_eq!(
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .eval::<String>("return (GetRealmInfo(1, 2))")
            .unwrap(),
        "Second",
        "with its rows"
    );
    app.update();
    assert_eq!(realm_list_opens(&mut app), 1, "and once");
}

/// #70: `ChangeRealm` from a script leaves the stock frame up (only the stock Okay hides it
/// first), and the frame's `OnUpdate` asks for the list again, which opens it over character
/// select. World entry takes it down with the rest of the glue: it was left set, and in the world
/// the native dialog drew over the game.
#[test]
fn world_entry_takes_down_a_realm_list_a_script_left_open() {
    use crate::app_core::state::ClientState;
    let shown = |app: &App| {
        app.world()
            .resource::<crate::app_core::ui_flags::Realms>()
            .shown
    };
    let lua = |app: &mut App, code: &str| {
        app.world_mut()
            .non_send_resource_mut::<UiScript>()
            .run(code)
            .unwrap();
        app.update();
    };
    let (mut app, _rx) = app();
    crate::realm_select::hide_at_world_entry(&mut app);
    show_realm_list(&mut app);
    app.update();
    lua(&mut app, "ChangeRealm(1, 1)");
    assert!(!shown(&app));
    // The roster arrives: character select, the stock frame still up and polling.
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::CharSelect);
    app.update();
    lua(&mut app, "RequestRealmList(1)");
    assert!(shown(&app), "the poll opens the list over character select");
    app.world_mut()
        .resource_mut::<NextState<ClientState>>()
        .set(ClientState::InWorld);
    app.update();
    assert!(!shown(&app), "nothing of the glue survives world entry");
}
