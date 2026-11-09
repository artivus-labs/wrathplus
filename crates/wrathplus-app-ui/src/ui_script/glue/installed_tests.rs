//! The stock glue on the installed data.

use super::*;
use wrathplus_ui::script::glue::GlueRequest;
use wrathplus_ui::script::ScriptValue;

/// The stock glue on the installed data: GlueXML loads clean, the engine's first screen shows
/// AccountLogin with its boxes, and the stock Login button's handler queues the login with the
/// typed account and password. Skips without an install.
#[test]
fn the_stock_account_login_opens_and_submits_on_the_installed_data() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut script = UiScript::new().unwrap();
    script.install_glue_api().unwrap();
    script.set_screen_size(1600.0, 1000.0);
    let failures = load_glue_ui(&script);
    assert!(failures.is_empty(), "{failures:#?}");
    // GlueParent's own start: FRAMES_LOADED, then the engine's first screen.
    script.fire_event("FRAMES_LOADED", vec![]);
    script.fire_event("SET_GLUE_SCREEN", vec![ScriptValue::Str("login".into())]);
    script.tick(0.1);
    let shown = script
        .eval::<String>(
            "local function v(f) return f and (f:IsVisible() and 'v' or (f:IsShown() and 's' or 'h')) or 'nil' end \
             return table.concat({v(GlueParent), v(AccountLogin), v(AccountLoginUI), \
             v(AccountLoginAccountEdit), v(AccountLoginPasswordEdit), \
             tostring(GetCurrentGlueScreenName())}, ' ')",
        )
        .unwrap();
    assert_eq!(
        shown, "v v v v v login",
        "GlueParent AccountLogin AccountLoginUI account password screen"
    );
    // AzerothBench's Server field (`assets/ui/glue/ServerField.xml`): it shows the address,
    // and leaving it after an edit asks the host to keep the new one.
    script.set_glue_realmlist("127.0.0.1");
    script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);
    let field = |script: &UiScript| {
        script
            .eval::<String>(
                "return tostring(AccountLoginServerEdit:IsVisible())..' '..\
                 AccountLoginServerEdit:GetText()..' '..AccountLoginServerEditLabel:GetText()",
            )
            .unwrap()
    };
    assert_eq!(field(&script), "1 127.0.0.1 Server");
    script
        .run(
            "AccountLoginServerEdit:SetFocus() AccountLoginServerEdit:SetText('10.0.0.5:3724') \
             AccountLoginServerEdit:ClearFocus()",
        )
        .unwrap();
    assert_eq!(
        script.take_glue_requests(),
        vec![GlueRequest::SetRealmlist("10.0.0.5:3724".into())]
    );
    // The host keeps it and says so.
    script.set_glue_realmlist("10.0.0.5:3724");
    script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);
    assert_eq!(field(&script), "1 10.0.0.5:3724 Server");
    // A launch's address is only where the field starts: it stays editable, and an edit is sent.
    script.set_glue_realmlist("127.0.0.1:13724");
    script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);
    assert_eq!(field(&script), "1 127.0.0.1:13724 Server");
    script
        .run(
            "AccountLoginServerEdit:SetFocus() AccountLoginServerEdit:SetText('wrathplus.artivus.com.au') \
             AccountLoginServerEdit:ClearFocus()",
        )
        .unwrap();
    assert_eq!(
        script.take_glue_requests(),
        vec![GlueRequest::SetRealmlist("wrathplus.artivus.com.au".into())]
    );
    assert!(
        script.errors().is_empty(),
        "script errors: {:?}",
        script.errors()
    );
    script.set_glue_realmlist("127.0.0.1");
    script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);
    // `--fill-only`: the launch's credentials wait in the boxes; nothing submits until Login.
    script.fill_login("Bob", "hunter2").unwrap();
    assert_eq!(
        script
            .eval::<(String, String)>(
                "return AccountLoginAccountEdit:GetText(), AccountLoginPasswordEdit:GetText()"
            )
            .unwrap(),
        ("Bob".to_string(), "hunter2".to_string())
    );
    assert!(
        script.take_glue_requests().is_empty(),
        "a fill does not log in"
    );
    script.run("AccountLogin_Login()").unwrap();
    assert_eq!(
        script.take_glue_requests(),
        vec![
            GlueRequest::Login {
                account: "Bob".into(),
                password: "hunter2".into()
            },
            // Save Account Name unticked: the saved name clears.
            GlueRequest::SaveAccountName(String::new()),
        ]
    );
    assert_eq!(
        script
            .eval::<String>("return AccountLoginPasswordEdit:GetText()")
            .unwrap(),
        "",
        "the stock handler clears the password box"
    );
    // The realm list the engine opens after the logon: the stock RealmList over a seeded
    // list, and a double-click on a row is `ChangeRealm(tab, row)`.
    let realm = |name: &str| wrathplus_ui::script::glue::GlueRealm {
        name: name.into(),
        num_characters: 1,
        ..Default::default()
    };
    script.set_glue_realms(
        vec![wrathplus_ui::script::glue::GlueRealmTab {
            name: String::new(),
            realms: vec![realm("AzerothCore"), realm("Second")],
        }],
        1,
        5.0,
    );
    script.fire_event("OPEN_REALM_LIST", vec![]);
    script.tick(0.1);
    assert_eq!(
        script
            .eval::<String>(
                "return tostring(RealmList:IsVisible())..' '..RealmListRealmButton1:GetText()..' '..\
                 tostring(RealmListRealmButton3:IsShown())"
            )
            .unwrap(),
        // 12340's visibility queries answer 1 or nil.
        "1 AzerothCore nil"
    );
    script
        .run("RealmSelectButton_OnDoubleClick(RealmListRealmButton2, 2)")
        .unwrap();
    assert_eq!(
        script.take_glue_requests(),
        vec![GlueRequest::ChangeRealm {
            category: 1,
            index: 2
        }]
    );
    // Character select over seeded rows: the stock list shows them, a click selects the row
    // and asks the stage for it, a double-click enters the world.
    script.set_glue_characters(
        vec![
            wrathplus_ui::script::glue::GlueCharacter {
                name: "Benchone".into(),
                race: "Human".into(),
                class: "Warrior".into(),
                level: 7,
                zone: "Elwynn Forest".into(),
                stage: "Human".into(),
                ..Default::default()
            },
            wrathplus_ui::script::glue::GlueCharacter {
                name: "Grunt".into(),
                race: "Orc".into(),
                class: "Death Knight".into(),
                level: 55,
                zone: "Acherus".into(),
                stage: "DeathKnight".into(),
                ..Default::default()
            },
        ],
        "Orc",
    );
    // Leaving login fades AccountLoginUI out first (`GlueScreenExit` → `GlueFrameFadeOut(...,
    // GoToPendingGlueScreen)`), so the switch lands once the fade has run.
    script.fire_event(
        "SET_GLUE_SCREEN",
        vec![ScriptValue::Str("charselect".into())],
    );
    for _ in 0..30 {
        script.tick(0.1);
    }
    script.fire_event("CHARACTER_LIST_UPDATE", vec![]);
    script.tick(0.1);
    assert_eq!(
        script
            .eval::<String>(
                "return tostring(CharacterSelect:IsVisible())..' '..GetCurrentGlueScreenName()..' '..\
                 CharSelectCharacterButton1ButtonTextName:GetText()..' '..\
                 tostring(CharSelectCharacterButton2:IsShown())..' '..tostring(CharSelectCharacterButton3:IsShown())"
            )
            .unwrap(),
        "1 charselect Benchone 1 nil"
    );
    script.take_glue_requests();
    script
        .run("CharacterSelectButton_OnClick(CharSelectCharacterButton2)")
        .unwrap();
    let requests = script.take_glue_requests();
    assert!(
        requests.contains(&GlueRequest::SelectCharacter(2)),
        "{requests:?}"
    );
    assert!(
        requests.contains(&GlueRequest::SetBackground(
            r"Interface\Glues\Models\UI_DeathKnight\UI_DeathKnight.m2".into()
        )),
        "{requests:?}"
    );
    script
        .run("CharacterSelectButton_OnDoubleClick(CharSelectCharacterButton2)")
        .unwrap();
    assert!(script
        .take_glue_requests()
        .contains(&GlueRequest::EnterWorld));
    let errors = script.take_errors();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// AzerothBench's Server field on the stock login (Shannon's 10-24 layout pass): the row under
/// Password in the stock boxes' style, the account-to-password gap below it, and clear of the
/// Login button, the Remember Account Name checkbox, the ESRB notice and the Blizzard logo.
#[test]
fn the_server_field_sits_under_password_in_the_stock_style() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut script = UiScript::new().unwrap();
    script.install_glue_api().unwrap();
    script.set_screen_size(1600.0, 1000.0);
    script.set_text_measurer(Box::new(crate::ui_script::FixedWidthFont(8.0)));
    assert!(load_glue_ui(&script).is_empty());
    script.fire_event("FRAMES_LOADED", vec![]);
    script.fire_event("SET_GLUE_SCREEN", vec![ScriptValue::Str("login".into())]);
    script.tick(0.1);
    script.resolve();
    let rect = |script: &UiScript, region: &str| -> [f32; 4] {
        let (l, r, b, t): (f32, f32, f32, f32) = script
            .eval(&format!(
                "local f = {region} return f:GetLeft(), f:GetRight(), f:GetBottom(), f:GetTop()"
            ))
            .unwrap_or_else(|e| panic!("{region}: {e}"));
        [l, r, b, t]
    };
    let overlaps =
        |a: [f32; 4], b: [f32; 4]| a[0] < b[1] && b[0] < a[1] && a[2] < b[3] && b[2] < a[3];
    let account = rect(&script, "AccountLoginAccountEdit");
    let password = rect(&script, "AccountLoginPasswordEdit");
    let field = rect(&script, "AccountLoginServerEdit");

    assert_eq!(
        field[1] - field[0],
        password[1] - password[0],
        "the stock boxes' width"
    );
    assert_eq!(field[3] - field[2], password[3] - password[2], "and height");
    assert_eq!(
        field[0] + field[1],
        password[0] + password[1],
        "centred under Password"
    );
    assert_eq!(
        password[2] - field[3],
        account[2] - password[3],
        "the account-to-password gap"
    );
    assert!(
        script
            .eval::<bool>(
                "local a, s = AccountLoginAccountEditLabel, AccountLoginServerEditLabel \
                 return a:GetFontObject() == s:GetFontObject() and a:GetWidth() == s:GetWidth() \
                 and a:GetHeight() == s:GetHeight()"
            )
            .unwrap(),
        "the label is the account label's font object and size"
    );

    let logo = rect(
        &script,
        "(function() for _, r in ipairs({AccountLoginUI:GetRegions()}) do \
         if r.GetTexture and tostring(r:GetTexture()):find('BlizzardLogo') then return r end end end)()",
    );
    let login = rect(&script, "AccountLoginLoginButton");
    let check = rect(&script, "AccountLoginSaveAccountName");
    let check_text = rect(&script, "AccountLoginSaveAccountNameText");
    let esrb = rect(&script, "WorldOfWarcraftRating");
    for (name, other) in [
        ("the Login button", login),
        ("the Remember checkbox", check),
        ("its label", check_text),
        ("the ESRB notice", esrb),
        ("the Blizzard logo", logo),
    ] {
        assert!(
            !overlaps(field, other),
            "the field overlaps {name}: {field:?} {other:?}"
        );
    }
    // The backdrop, textures and colours alike: stock colours its boxes in `AccountLogin_OnLoad`
    // (`DEFAULT_TOOLTIP_COLOR`), so the field's XML backdrop alone drew a lighter grey.
    let backdrop = |name: &str| {
        script
            .eval::<String>(&format!(
                "local f = {name} local b = f:GetBackdrop() local i = b.insets \
                 return string.format('%s %s %s %s %s %s %s %s %s ' .. string.rep('%.3f ', 4), \
                 b.bgFile, b.edgeFile, tostring(b.tile), b.tileSize, b.edgeSize, \
                 i.left, i.right, i.top, i.bottom, f:GetBackdropColor())
                 .. string.format(string.rep('%.3f ', 4), f:GetBackdropBorderColor())"
            ))
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    let stock = backdrop("AccountLoginAccountEdit");
    assert_eq!(backdrop("AccountLoginPasswordEdit"), stock);
    assert_eq!(
        backdrop("AccountLoginServerEdit"),
        stock,
        "the stock boxes' backdrop"
    );
    assert!(login[3] <= field[2], "Login sits below the field");
    for (name, r) in [("the Remember checkbox", check), ("its label", check_text)] {
        assert!(
            !overlaps(r, logo),
            "{name} overlaps the Blizzard logo: {r:?} {logo:?}"
        );
    }
    assert!(
        script.errors().is_empty(),
        "script errors: {:?}",
        script.errors()
    );
}

/// A VPS's non-default logon port through the Server field (Shannon's 10-24 pass): typed, kept in
/// `realmlist.wtf` with its port, and dialled on it; the field's tooltip says a port may follow.
#[test]
fn a_server_field_address_keeps_its_port_from_field_to_file_to_dial() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut script = UiScript::new().unwrap();
    script.install_glue_api().unwrap();
    script.set_screen_size(1600.0, 1000.0);
    assert!(load_glue_ui(&script).is_empty());
    script.fire_event("FRAMES_LOADED", vec![]);
    script.fire_event("SET_GLUE_SCREEN", vec![ScriptValue::Str("login".into())]);
    script.tick(0.1);
    script.set_glue_realmlist("127.0.0.1");
    script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);

    // Hovering the field explains the syntax.
    let hint = script
        .eval::<(bool, String)>(
            "local f = AccountLoginServerEdit f:GetScript('OnEnter')(f) \
             local shown, text = GlueTooltip:IsShown(), GlueTooltipTextLeft1:GetText() \
             f:GetScript('OnLeave')(f) return shown and not GlueTooltip:IsShown(), text",
        )
        .unwrap();
    assert!(hint.0, "the hint shows on hover and hides on leave");
    assert!(hint.1.contains("host or host:port"), "{}", hint.1);

    script
        .run(
            "AccountLoginServerEdit:SetFocus() AccountLoginServerEdit:SetText('192.168.1.1:1337') \
             AccountLoginServerEdit:ClearFocus()",
        )
        .unwrap();
    let typed = match script.take_glue_requests().as_slice() {
        [GlueRequest::SetRealmlist(typed)] => typed.clone(),
        other => panic!("one SetRealmlist, got {other:?}"),
    };
    assert_eq!(typed, "192.168.1.1:1337");

    // The host's arm for that request (`host.rs`), into a scratch `WRATHPLUS_HOME`.
    let _l = crate::local_state::test_env::ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("azb-server-port-{}", std::process::id()));
    let _home =
        crate::local_state::test_env::EnvGuard::set("WRATHPLUS_HOME", dir.to_str().unwrap());
    let mut realmlist = crate::realmlist::Realmlist::at("127.0.0.1");
    assert!(crate::login::accept_realmlist(
        &typed,
        &mut realmlist,
        Some(&mut script)
    ));
    let saved = std::fs::read_to_string(dir.join("realmlist.wtf")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        saved.contains("192.168.1.1:1337"),
        "realmlist.wtf: {saved:?}"
    );
    // The next attempt's `LoginRequest::host`, split as `wrath_auth::logon` dials it.
    assert_eq!(
        wrathplus_protocol::host_port(realmlist.address(), wrathplus_protocol::AUTH_PORT),
        ("192.168.1.1", 1337)
    );
    script.set_glue_realmlist(realmlist.address());
    script.fire_event("WRATHPLUS_REALMLIST_CHANGED", vec![]);
    assert_eq!(
        script
            .eval::<String>("return AccountLoginServerEdit:GetText()")
            .unwrap(),
        "192.168.1.1:1337"
    );
    assert!(
        script.errors().is_empty(),
        "script errors: {:?}",
        script.errors()
    );
}
