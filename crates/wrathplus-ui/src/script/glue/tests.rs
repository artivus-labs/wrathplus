use super::GlueRequest as R;
use crate::script::UiScript;

fn vm() -> UiScript {
    let s = UiScript::new().unwrap();
    s.install_glue_api().unwrap();
    s
}

#[test]
fn the_login_calls_queue_for_app_in_order() {
    let mut s = vm();
    s.run("DefaultServerLogin('Bob', 'hunter2') StatusDialogClick() CancelLogin() SetSavedAccountName('Bob') QuitGame()")
        .unwrap();
    let requests = s.take_glue_requests();
    assert_eq!(
        requests,
        vec![
            R::Login {
                account: "Bob".into(),
                password: "hunter2".into()
            },
            R::StatusDialogClick,
            R::CancelLogin,
            R::SaveAccountName("Bob".into()),
            R::Quit,
        ]
    );
    assert!(
        !format!("{requests:?}").contains("hunter2"),
        "Debug never prints it"
    );
}

#[test]
fn the_reads_answer_what_app_seeds() {
    let mut s = vm();
    s.set_glue_saved_account("Alice");
    s.set_glue_connection(true, "AzerothCore");
    s.set_glue_num_characters(3);
    s.run("local a, b, v, i, d = GetBuildInfo(); assert(a == 'Version' and v == '3.3.5' and i == '12340' and d == 'Jun 24 2010'); assert(GetSavedAccountName() == 'Alice'); assert(IsConnectedToServer() == 1); assert(GetServerName() == 'AzerothCore'); assert(GetNumCharacters() == 3); assert(GetClientExpansionLevel() == 2); assert(not IsStreamingTrial()); assert(not ShowTOSNotice()); assert(TOSAccepted() == 1 and EULAAccepted() == 1); SetCurrentScreen('login')")
        .unwrap();
    assert_eq!(s.glue_current_screen().as_deref(), Some("login"));
}

#[test]
fn the_realm_list_reads_the_tabs_app_seeds_and_queues_the_choice() {
    use super::{GlueRealm, GlueRealmTab};
    let mut s = vm();
    let realm = |name: &str, current: bool| GlueRealm {
        name: name.into(),
        num_characters: 2,
        current,
        pvp: true,
        load: -1.0,
        realm_type: 1,
        ..Default::default()
    };
    s.set_glue_realms(
        vec![
            GlueRealmTab {
                name: "Development".into(),
                realms: vec![realm("AzerothCore", true), realm("Other", false)],
            },
            GlueRealmTab {
                name: "Test".into(),
                realms: vec![realm("Third", false)],
            },
        ],
        1,
        5.0,
    );
    s.run(
        "local a, b = GetRealmCategories() assert(a == 'Development' and b == 'Test') \
         assert(GetNumRealms(1) == 2 and GetNumRealms(2) == 1 and GetNumRealms(9) == 2) \
         local name, chars, invalid, down, current, pvp, rp, load = GetRealmInfo(1, 1) \
         assert(name == 'AzerothCore' and chars == 2 and not invalid and not down and current == 1 and pvp == 1 and not rp and load == -1) \
         assert(GetRealmInfo(1, 3) == nil) assert(GetSelectedCategory() == 1 and RealmListUpdateRate() == 5) \
         CancelRealmListQuery() ChangeRealm(1, 2) RequestRealmList() RealmListDialogCancelled()",
    )
    .unwrap();
    assert_eq!(
        s.take_glue_requests(),
        vec![
            R::ChangeRealm {
                category: 1,
                index: 2
            },
            R::RequestRealmList,
            R::RealmListCancelled,
        ]
    );
}

#[test]
fn character_select_reads_the_rows_and_queues_its_calls() {
    use super::GlueCharacter;
    let mut s = vm();
    s.set_glue_characters(
        vec![GlueCharacter {
            name: "Benchone".into(),
            race: "Human".into(),
            class: "Warrior".into(),
            level: 7,
            zone: "Elwynn Forest".into(),
            ghost: true,
            faction_change: true,
            stage: "Human".into(),
            ..Default::default()
        }],
        "Orc",
    );
    s.run(
        "assert(GetNumCharacters() == 1) \
         local name, race, class, level, zone, sex, ghost, pcc, prc, pfc = GetCharacterInfo(1) \
         assert(name == 'Benchone' and race == 'Human' and class == 'Warrior' and level == 7 and zone == 'Elwynn Forest') \
         assert(ghost == 1 and not pcc and not prc and pfc == 1) assert(GetCharacterInfo(2) == nil) \
         assert(GetSelectBackgroundModel(1) == 'Human' and GetSelectBackgroundModel(5) == 'Orc') \
         SetCharacterSelectFacing(30) assert(GetCharacterSelectFacing() == 30) \
         SelectCharacter(1) SetCharSelectBackground('Interface\\\\Glues\\\\Models\\\\UI_Human\\\\UI_Human.m2') \
         EnterWorld() DeleteCharacter(1) GetCharacterListUpdate() DisconnectFromServer()",
    )
    .unwrap();
    assert_eq!(
        s.take_glue_requests(),
        vec![
            R::SetSelectFacing(30.0),
            R::SelectCharacter(1),
            R::SetBackground(r"Interface\Glues\Models\UI_Human\UI_Human.m2".into()),
            R::EnterWorld,
            R::DeleteCharacter(1),
            R::RefreshCharacters,
            R::Disconnect,
        ]
    );
}

#[test]
fn the_glue_backdrops_answer_the_model_ffx_verbs_only_on_the_glue_vm() {
    let s = vm();
    s.run(
        "local m = CreateFrame('Model') m:SetGlow(0.3) m:ResetLights() m:AddLight(1, 0, 0, 0, 1)",
    )
    .unwrap();
    let plain = UiScript::new().unwrap();
    assert!(plain
        .eval::<bool>("return CreateFrame('Model').SetGlow == nil")
        .unwrap());
}

#[test]
fn the_in_game_vm_has_no_glue_api() {
    let s = UiScript::new().unwrap();
    assert!(s.eval::<bool>("return DefaultServerLogin == nil").unwrap());
}

/// The in-world VM has no glue API: a glue setter scheduled across the world switch is a no-op and
/// a getter answers nothing, never a panic.
#[test]
fn a_vm_without_the_glue_api_answers_nothing() {
    let mut s = UiScript::new().unwrap();
    s.set_glue_saved_account("Bob");
    s.set_glue_connection(true, "x");
    s.set_glue_num_characters(2);
    s.set_glue_realmlist("127.0.0.1");
    assert!(s.take_glue_requests().is_empty());
    assert_eq!(s.glue_current_screen(), None);
}
