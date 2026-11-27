//! One installed check per line family [`super::frames::route`] fires, against the text the stock
//! 3.3.5 `ChatFrame_MessageEventHandler` hands `ChatFrame1:AddMessage`: TEXT_EMOTE, PARTY, SYSTEM,
//! and a channel's join and leave notices. The handler concatenates arg11 into every player
//! link, so a line fired with the 1.12 ten arguments raised and printed nothing.

use super::tests::chat_vm;
use super::{ChatEvent, ChatEventKind as K};
use wrathplus_ui::script::UiScript;

/// Every text `ChatFrame1:AddMessage` takes, in order, in the global `Printed`, and its colour in
/// `PrintedColor`.
pub(super) const PRINTED: &str = r#"
    Printed, PrintedColor = {}, {}
    local add = ChatFrame1.AddMessage
    ChatFrame1.AddMessage = function(self, text, r, g, b, ...)
        table.insert(Printed, text)
        table.insert(PrintedColor, { r, g, b })
        return add(self, text, r, g, b, ...)
    end
"#;

/// The stock chat stack with the [`PRINTED`] hook on its window.
fn printing_vm() -> UiScript {
    let s = chat_vm();
    s.run(PRINTED).unwrap();
    s
}

pub(super) fn printed(s: &UiScript) -> Vec<String> {
    s.eval::<Vec<String>>("return Printed").unwrap()
}

fn line(kind: K, text: &str, sender: &str) -> ChatEvent {
    ChatEvent {
        kind: Some(kind),
        text: text.into(),
        sender: sender.into(),
        ..Default::default()
    }
}

/// Fire `event` through [`super::frames::route`]; what the window printed, and no handler error.
fn route(event: &ChatEvent) -> Vec<String> {
    let mut s = printing_vm();
    let mut windows = super::frames::ChatWindows::default();
    super::frames::route(&mut s, &mut windows, event);
    assert!(s.errors().is_empty(), "handler errors: {:?}", s.errors());
    printed(&s)
}

/// `body = gsub(arg1, arg2, pflag..playerLink..coloredName.."|h", 1)`: the sender's name in the
/// sentence becomes the link, whose payload is `arg2:arg11:chatGroup`.
#[test]
fn a_text_emote_links_the_performer_in_its_sentence() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    assert_eq!(
        route(&line(K::TextEmote, "Bob waves at you.", "Bob")),
        ["|Hplayer:Bob:0:TEXT_EMOTE|hBob|h waves at you."]
    );
}

/// `format(CHAT_PARTY_GET..arg1, pflag..playerLink.."["..coloredName.."]".."|h")`.
#[test]
fn a_party_line_wears_the_party_link_and_the_linked_name() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    assert_eq!(
        route(&line(K::Party, "inc 3", "Ann")),
        ["|Hchannel:PARTY|h[Party]|h |Hplayer:Ann:0:PARTY|h[Ann]|h: inc 3"]
    );
}

/// SYSTEM prints arg1 as it is.
#[test]
fn a_system_line_prints_verbatim() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    assert_eq!(
        route(&ChatEvent::text_only(
            K::System,
            "You are now Away: AFK".into()
        )),
        ["You are now Away: AFK"]
    );
    // A binding's unkeyed system line (`0x509dd0(text, 0)`) and a chat-kind refusal (`0x50`) reach
    // the frame through the engine drain, where a bare `CHAT_MSG_SYSTEM` raised in the handler.
    let mut s = printing_vm();
    s.run("SendSystemMessage('An invented line') SetLootMethod('group')")
        .unwrap();
    s.tick(0.0);
    let mut windows = super::frames::ChatWindows::default();
    for line in crate::ui_action::engine_lines(&mut s) {
        assert_eq!(line.kind(), wrathplus_ui::messages::MsgKind::Chat);
        let event = ChatEvent::text_only(K::System, line.text().into());
        super::frames::route(&mut s, &mut windows, &event);
    }
    assert!(s.errors().is_empty(), "handler errors: {:?}", s.errors());
    let not_in_group = s.eval::<String>("return ERR_NOT_IN_GROUP").unwrap();
    assert_eq!(printed(&s), [not_in_group.as_str(), "An invented line"]);
}

/// `format(CHAT_<token>_NOTICE, arg8, arg4)`, once the window carries the channel; the leave
/// drops the window's record after the line.
#[test]
fn a_channels_join_and_leave_notices_print_their_numbered_link() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = printing_vm();
    let mut windows = super::frames::ChatWindows::default();
    let mut channels = super::edit::ChannelState::default();
    channels.claim_slot("General - Elwynn Forest");
    s.run("ChatFrame_AddChannel(ChatFrame1, 'General - Elwynn Forest')")
        .unwrap();

    for byte in ["2", "3"] {
        // YOU_JOINED, then YOU_LEFT.
        let mut e = ChatEvent::text_only(K::ChannelNotice, String::new());
        e.channel = "General - Elwynn Forest".into();
        e.notice = byte.into();
        super::feed::deliver(&mut s, &mut windows, &mut channels, &mut e);
    }
    assert!(s.errors().is_empty(), "handler errors: {:?}", s.errors());
    assert_eq!(
        printed(&s),
        [
            "Joined Channel: |Hchannel:CHANNEL:1|h[1. General - Elwynn Forest]|h",
            "Left Channel: |Hchannel:CHANNEL:1|h[1. General - Elwynn Forest]|h",
        ]
    );
}

/// 12340's coin string (`0x7e7d80`) with the shipped `*_AMOUNT` templates, as the quest reward
/// line uses it, and the loot-money line (`0x6ceca0`) through the stock frame as
/// `CHAT_MSG_MONEY`. Skips without client data.
#[test]
fn coin_lines_name_each_coin_in_the_shipped_words() {
    let _data = wrathplus_formats::wow_data_or_skip!();
    let mut s = printing_vm();
    let get = |k: &str| wrathplus_ui::strings::global(s.lua(), k);
    let coins = |copper| crate::ui_quest::lines::coin_text(copper, &|k| get(k).unwrap_or_default());
    assert_eq!(coins(150), "1 Silver, 50 Copper");
    assert_eq!(coins(10_005), "1 Gold, 5 Copper");
    let reward = wrathplus_ui::strings::fill(
        &get("ERR_QUEST_REWARD_MONEY_S").unwrap(),
        &[wrathplus_ui::strings::Arg::S(&coins(150))],
    );
    assert_eq!(reward, "Received 1 Silver, 50 Copper.");
    let line = super::feed::loot_money_line(5, true, &get).expect("YOU_LOOT_MONEY");
    let mut windows = super::frames::ChatWindows::default();
    super::frames::route(&mut s, &mut windows, &line_of(K::Money, &line));
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert_eq!(printed(&s), ["You loot 5 Copper"]);
}

fn line_of(kind: K, text: &str) -> ChatEvent {
    ChatEvent::text_only(kind, text.into())
}
