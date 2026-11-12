//! Installed 12340 GlueDialog.lua chooses SimpleHTML only for the OKAY_HTML variant.

use super::*;
use crate::net::LoginFailedMessage;
use wrathplus_ui::script::QuadContent;

#[test]
fn login_dialog_blank_password_and_html_refusal_draw_without_markup() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_configured_chain(&data).unwrap();
    let strings = crate::glue_strings::table_from_chain(&mut chain);
    let authored = strings.get("LOGIN_UNKNOWN_ACCOUNT").unwrap().to_string();
    assert!(crate::login::is_html(&authored));
    assert_eq!(
        crate::login::fail_text(
            &strings,
            Some(wrathplus_protocol::LoginRefusal::Logon(0x05)),
            None,
        ),
        authored,
        "URL trimming must not truncate the HTML document"
    );
    let (mut app, rx) = super::host_tests::app();
    app.insert_resource(strings);
    {
        let mut script = app.world_mut().non_send_resource_mut::<UiScript>();
        script.set_screen_size(1600.0, 1000.0);
        assert!(load_glue_ui(&script).is_empty());
        script.fire_event("FRAMES_LOADED", vec![]);
        script.fire_event("SET_GLUE_SCREEN", vec![ScriptValue::Str("login".into())]);
        script.tick(0.1);
        script.fill_login("Probeacct", "").unwrap();
        script.run("AccountLogin_Login()").unwrap();
    }
    app.update();
    assert_eq!(rx.try_iter().count(), 0, "empty password never dials");
    {
        let script = app.world_mut().non_send_resource_mut::<UiScript>();
        let (shown, which, text): (bool, String, String) = script
            .eval("return GlueDialog:IsShown(), GlueDialog.which, GlueDialogText:GetText()")
            .unwrap();
        assert!(shown);
        assert_eq!(which, "OKAY");
        assert_eq!(text, "Please enter your password.");
        script.run("GlueDialogButton1:Click()").unwrap();
    }
    app.update();
    // The adjacent server-refusal path uses the same stock popup but a different text widget.
    app.world_mut().write_message(LoginFailedMessage {
        refusal: Some(wrathplus_protocol::LoginRefusal::Logon(0x05)),
        reason: String::new(),
        terminal: false,
        dial: None,
    });
    app.update();
    {
        let mut script = app.world_mut().non_send_resource_mut::<UiScript>();
        let (shown, which, html, plain): (bool, String, bool, bool) = script
            .eval("return GlueDialog:IsShown(), GlueDialog.which, GlueDialogHTML:IsShown(), GlueDialogText:IsShown()")
            .unwrap();
        assert!(shown && html && !plain);
        assert_eq!(which, "OKAY_HTML");
        script.resolve();
        let texts: Vec<_> = script
            .extract()
            .into_iter()
            .filter_map(|q| match q.content {
                QuadContent::Text { text: Some(t), .. } => Some(t),
                _ => None,
            })
            .collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("The information you have entered is not valid.")),
            "{texts:?}"
        );
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("<html>") || t.contains("<body>") || t.contains("<p align=")),
            "{texts:?}"
        );
        assert!(script.errors().is_empty(), "{:?}", script.errors());
        script.run("GlueDialogButton1:Click()").unwrap();
    }
    app.update();
    let script = app.world_mut().non_send_resource_mut::<UiScript>();
    assert!(!script.eval::<bool>("return GlueDialog:IsShown()").unwrap());
}
