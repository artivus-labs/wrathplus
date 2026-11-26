//! The contact-list storm (session 115138: 35,469 `CMSG_CONTACT_LIST` with MultiBot Chatless
//! loaded, none without it): a reply must never ask again. Each request is answered with
//! AzerothCore's own packet through the real decode, then the social handlers, feed and drain,
//! frame by frame over a simulated minute.

use bevy::prelude::*;
use wrathplus_protocol::wrath_events::{WrathDispatch, WrathEventAdapter};
use wrathplus_ui::script::UiScript;

use crate::net::{ClientCommand, NetCommands};

/// AzerothCore's `SendSocialList` answer to `ShowFriends`' `CMSG_CONTACT_LIST{flags 1}`: the
/// flags, then an empty list.
const EMPTY_CONTACT_LIST: [u8; 8] = [1, 0, 0, 0, 0, 0, 0, 0];

/// MultiBot Chatless's roster listener (`UI/MultiBotUnitsRootUI.lua:2121-2166`), with no
/// `MultiBot.TimerAfter`, so it takes the addon's own synchronous branch: a rebuild from the
/// caches on `FRIENDLIST_UPDATE` that never asks the server (`requestRemoteRefresh` false), and
/// its explicit `requestFriendRosterRefresh` (`:1218-1226`), the one `ShowFriends()` it sends.
const MULTIBOT: &str = r#"
    MBStorm = { rebuilds = 0 }
    local pending = false
    local f = CreateFrame("Frame")
    local function rebuild()
        local maxFriends = GetNumFriends() or 0
        if maxFriends <= 0 then maxFriends = 50 end
        for index = 1, maxFriends do
            local name, level, className = GetFriendInfo(index)
            if name == nil or level == nil or className == nil then break end
        end
        MBStorm.rebuilds = MBStorm.rebuilds + 1
    end
    f:RegisterEvent("GUILD_ROSTER_UPDATE")
    f:RegisterEvent("FRIENDLIST_UPDATE")
    f:SetScript("OnEvent", function()
        if pending then return end
        pending = true
        rebuild()
        pending = false
    end)
    function MBStorm.refresh() ShowFriends() end
"#;

/// Stock `FriendsFrame_OnEvent`'s two list arms (`FriendsFrame.lua:1189-1192`), for the gate run
/// without client data: `FRIENDLIST_SHOW` re-runs `FriendsFrame_Update`, whose friends tab calls
/// `ShowFriends()` (`:254-258`) whether or not the frame is shown.
const STOCK_ARMS: &str = r#"
    local f = CreateFrame("Frame")
    f:RegisterEvent("FRIENDLIST_SHOW")
    f:RegisterEvent("FRIENDLIST_UPDATE")
    f:SetScript("OnEvent", function(self, event)
        if event == "FRIENDLIST_SHOW" then ShowFriends() end
    end)
"#;

fn storm_app(script: UiScript) -> (App, crossbeam_channel::Receiver<ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut app = App::new();
    app.insert_resource(NetCommands(tx))
        .init_resource::<crate::app_core::interaction::SocialState>()
        .init_resource::<crate::names::NameCache>()
        .init_resource::<crate::app_core::chat::ChatLog>()
        .init_resource::<crate::sound::MessageSounds>()
        .insert_non_send_resource(script)
        .add_systems(
            Update,
            (
                crate::ui_social::feed::feed_social,
                crate::ui_social::feed::drain_social,
            )
                .chain(),
        );
    crate::ui_social::net::register(&mut app);
    (app, rx)
}

/// One frame, then AzerothCore's answer to every request it sent; the requests it sent.
fn frame(app: &mut App, rx: &crossbeam_channel::Receiver<ClientCommand>) -> usize {
    app.update();
    let mut asked = 0;
    for command in rx.try_iter() {
        if matches!(command, ClientCommand::FriendListRequest) {
            asked += 1;
            let WrathDispatch::Handled { session_events, .. } = WrathEventAdapter::new()
                .dispatch(0x0067, &EMPTY_CONTACT_LIST, 0)
                .unwrap()
            else {
                panic!("SMSG_CONTACT_LIST decodes");
            };
            crate::net::handlers::dispatch(app.world_mut(), session_events);
        }
    }
    asked
}

/// The requests across `frames` frames of 16.7 ms.
fn run(app: &mut App, rx: &crossbeam_channel::Receiver<ClientCommand>, frames: usize) -> usize {
    (0..frames).map(|_| frame(app, rx)).sum()
}

const MINUTE: usize = 3600;

/// Without client data: stock's list arms and MultiBot's listener, kicked by one explicit
/// `ShowFriends()`. The reply fires `FRIENDLIST_UPDATE`, never `FRIENDLIST_SHOW`, so the one
/// request is all there is.
#[test]
fn a_contact_list_reply_never_asks_again() {
    let script = UiScript::new().unwrap();
    script.run(STOCK_ARMS).unwrap();
    script.run(MULTIBOT).unwrap();
    let (mut app, rx) = storm_app(script);
    assert_eq!(run(&mut app, &rx, 10), 0, "nothing asks unprompted");
    app.world()
        .non_send_resource::<UiScript>()
        .run("MBStorm.refresh()")
        .unwrap();
    assert_eq!(
        run(&mut app, &rx, MINUTE),
        1,
        "the explicit ShowFriends, once"
    );
    let rebuilds = app
        .world()
        .non_send_resource::<UiScript>()
        .eval::<i64>("return MBStorm.rebuilds")
        .unwrap();
    assert!(
        (1..=2).contains(&rebuilds),
        "the addon rebuilt on the seed and the reply, not per frame: {rebuilds}"
    );
}

/// Stock `FriendsFrame.xml` with MultiBot's listener: the explicit refresh asks once, showing the
/// friends tab asks once more (`FriendsFrame_OnShow` → `FriendsFrame_Update`), and neither the
/// replies nor the hidden or shown frame ask again over a minute each.
#[test]
fn the_stock_friends_frame_and_multibot_ask_only_when_told() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut script = UiScript::new().unwrap();
    script.set_screen_size(1024.0, 768.0);
    super::test_ui::load_social_ui(&mut script);
    script.run(MULTIBOT).unwrap();
    let (mut app, rx) = storm_app(script);
    let lua = |app: &mut App, code: &str| {
        app.world()
            .non_send_resource::<UiScript>()
            .run(code)
            .unwrap()
    };
    assert_eq!(
        run(&mut app, &rx, MINUTE),
        0,
        "hidden: nothing asks unprompted"
    );
    lua(&mut app, "MBStorm.refresh()");
    assert_eq!(run(&mut app, &rx, MINUTE), 1, "the addon's refresh, once");
    lua(&mut app, "ToggleFriendsFrame(1)");
    assert_eq!(
        run(&mut app, &rx, MINUTE),
        1,
        "the friends tab's show, once"
    );
    lua(&mut app, "ToggleFriendsFrame(1)");
    assert_eq!(run(&mut app, &rx, MINUTE), 0, "hidden again: silent");
}
