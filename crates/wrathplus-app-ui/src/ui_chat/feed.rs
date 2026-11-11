//! Vanilla inbound sources become [`ChatEvent`] here; native Wrath MESSAGECHAT dispatches
//! stock events separately. Names resolve ask-once through
//! [`crate::names::NameCache`] with bounded re-checks, then routed by [`super::frames::route`].

use bevy::prelude::*;

use wrathplus_protocol::types::{channel_notice, ChannelNoticeTail};

use wrathplus_assets::{LockRecover, WorldAssets};
use wrathplus_formats::{EmoteLine, EmoteTextCatalog};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfGuid;
use crate::names::NameCache;
use crate::net::{GuidIndex, NetCommands};
use crate::npc_text::chat_macros;

use super::edit::ChannelState;
use super::event::{flag_of_tag, kind_of_wire, language_name, ChatEvent, ChatEventKind};
use super::frames::{route, ChatWindows};

/// Wrath currently has no guaranteed stock `ChatFrame`. Raw host-composed events must therefore
/// also reach the durable host log; this does not claim a visible native overlay. The event still
/// dispatches normally so addons receive it.
fn wrath_needs_raw_event_host_log(script: &wrathplus_ui::script::UiScript) -> bool {
    !script
        .eval::<bool>("return ChatFrame1 ~= nil")
        .unwrap_or(false)
}

/// Frames a line waits on a name, about 2 s at 60 fps, before it gives up.
pub const NAME_MAX_TRIES: u16 = 120;

/// The text-emote sentence tables, read once off the patch chain.
#[derive(Resource)]
pub struct EmoteTexts(pub EmoteTextCatalog);

/// The DBC string column: 0, enUS; the reference reads its locale slot `[0xc0e080]`.
const LOCALE: usize = 0;

/// Load `EmotesText.dbc` with `EmotesTextData.dbc`; before `AssetSet::Open` it loads nothing.
pub(super) fn load_emote_texts(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_emote_text_catalog(&mut chain)
    };
    match loaded {
        Ok(cat) => {
            info!("chat: {} emote sentence tables", cat.len());
            commands.insert_resource(EmoteTexts(cat));
        }
        Err(e) => warn!("chat: emote text catalog failed to load: {e:#}"),
    }
}

pub use crate::app_core::chat::{ChatLog, Pending};
/// Fire `CHAT_MSG_ADDON(prefix, message, distribution, sender)` (`0x49a95f`), not through
/// [`route`]: it is no `ChatTypeInfo` key and takes four arguments, not ten.
pub(super) fn fire_addon_message(
    script: &mut wrathplus_ui::script::UiScript,
    prefix: String,
    message: String,
    distribution: String,
    sender: String,
) {
    use wrathplus_ui::script::ScriptValue::Str;
    script.fire_event(
        "CHAT_MSG_ADDON",
        vec![Str(prefix), Str(message), Str(distribution), Str(sender)],
    );
}

/// A text emote's chat event, or `None` when the sentence is blank, as SIT, STAND and TRAIN are
/// in every locale (`0x49b4bd`). `sender` is the performer, seen only by addons as arg2:
/// `TEXT_EMOTE` prints verbatim (`ChatFrame.lua:1395`), and the fire (`0x49b495`) pushes the name
/// record resolved at `0x49b289`.
pub(super) fn text_emote_event(
    cat: &EmoteTextCatalog,
    text_id: u32,
    line: &EmoteLine,
) -> Option<ChatEvent> {
    Some(ChatEvent {
        kind: Some(ChatEventKind::TextEmote),
        text: cat.compose(text_id, line, LOCALE)?,
        sender: line.performer.to_string(),
        ..Default::default()
    })
}

/// The VM's globals as a string lookup, borrowed per composition so `route` can take the VM.
fn globals(script: &wrathplus_ui::script::UiScript) -> impl Fn(&str) -> Option<String> + '_ {
    |key: &str| script.lua().globals().get::<String>(key).ok()
}

/// Fill a key's template; a missing or empty string prints no line, as in the reference.
fn keyed(
    get: &dyn Fn(&str) -> Option<String>,
    key: &str,
    args: &[wrathplus_ui::strings::Arg<'_>],
) -> Option<String> {
    let text = wrathplus_ui::strings::fill(&get(key)?, args);
    (!text.is_empty()).then_some(text)
}

/// `0x6ceca0`'s line: the coin string (`0x7e7d80`) in `YOU_LOOT_MONEY` for the only looter, else
/// `LOOT_MONEY_SPLIT`, which the drain prints as `CHAT_MSG_MONEY`.
pub(super) fn loot_money_line(
    amount: u32,
    sole: bool,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let coins = crate::ui_quest::lines::coin_text(amount, &|k| get(k).unwrap_or_default());
    let key = if sole {
        "YOU_LOOT_MONEY"
    } else {
        "LOOT_MONEY_SPLIT"
    };
    keyed(get, key, &[wrathplus_ui::strings::Arg::S(&coins)])
}

/// The XP line: `COMBATLOG_XPGAIN_FIRSTPERSON`, the `_UNNAMED` form, or the rested
/// `_EXHAUSTION1`, whose last two arguments are the signed bonus and the state word.
pub(super) fn xp_gain_line(
    victim: Option<&str>,
    total: u32,
    bonus: u32,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    use wrathplus_ui::strings::Arg;
    match victim {
        Some(name) if bonus > 0 => keyed(
            get,
            "COMBATLOG_XPGAIN_EXHAUSTION1",
            &[
                Arg::S(name),
                Arg::D(i64::from(total)),
                Arg::S(&format!("+{bonus}")),
                Arg::S(RESTED_STATE),
            ],
        ),
        Some(name) => keyed(
            get,
            "COMBATLOG_XPGAIN_FIRSTPERSON",
            &[Arg::S(name), Arg::D(i64::from(total))],
        ),
        None => keyed(
            get,
            "COMBATLOG_XPGAIN_FIRSTPERSON_UNNAMED",
            &[Arg::D(i64::from(total))],
        ),
    }
}

/// The `_EXHAUSTION1` template's state word. 1.12 has no GlobalString for it and the reference's
/// own table is untraced; rested is the only bonus state vmangos sets (`Player.cpp:17990-17993`).
const RESTED_STATE: &str = "Rested";

/// The honor line (12340 `0x50b6d0`): `COMBATLOG_HONORAWARD` with no victim, else
/// `COMBATLOG_HONORGAIN` for a ranked victim (a signed rank of at least 0) and
/// `COMBATLOG_HONORGAIN_NO_RANK` below that. No `rank_title` leaves the rank slot empty.
pub(super) fn honor_gain_line(
    victim: Option<&str>,
    rank_title: Option<&str>,
    ranked: bool,
    honor: i32,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    use wrathplus_ui::strings::Arg;
    let points = Arg::D(i64::from(honor));
    match victim {
        None => keyed(get, "COMBATLOG_HONORAWARD", &[points]),
        Some(name) if !ranked => keyed(get, "COMBATLOG_HONORGAIN_NO_RANK", &[Arg::S(name), points]),
        Some(name) => keyed(
            get,
            "COMBATLOG_HONORGAIN",
            &[Arg::S(name), Arg::S(rank_title.unwrap_or_default()), points],
        ),
    }
}

/// The `ERR_ZONE_EXPLORED` toast (error route 1, `AddErrorMessage` `0x4945b0`), every packet.
pub(super) fn exploration_toast(
    area_name: &str,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    keyed(
        get,
        "ERR_ZONE_EXPLORED",
        &[wrathplus_ui::strings::Arg::S(area_name)],
    )
}

/// The `ERR_ZONE_EXPLORED_XP` system line, only when the packet carries XP (`jle` at `0x5e422f`).
pub(super) fn exploration_line(
    area_name: &str,
    xp: u32,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    use wrathplus_ui::strings::Arg;
    keyed(
        get,
        "ERR_ZONE_EXPLORED_XP",
        &[Arg::S(area_name), Arg::D(i64::from(xp))],
    )
}

/// The event a channel notice becomes; [`super::frames::compose_notice`] reads its `notice`.
pub(super) fn notice_event(
    notice_byte: u8,
    channel: String,
    a: Option<String>,
    b: Option<String>,
) -> Option<ChatEvent> {
    // Each arm's `mov edi` in the `0x49c60c` jump table: MODE_CHANGE fires nothing, a byte past
    // THROTTLED fires SYSTEM, the rest CHANNEL_NOTICE (0x12) or CHANNEL_NOTICE_USER (0x13).
    use channel_notice as n;
    let kind = match notice_byte {
        n::JOINED => ChatEventKind::ChannelJoin,
        n::LEFT => ChatEventKind::ChannelLeave,
        n::MODE_CHANGE => return None,
        n::YOU_JOINED
        | n::YOU_LEFT
        | n::WRONG_PASSWORD
        | n::NOT_MEMBER
        | n::NOT_MODERATOR
        | n::NOT_OWNER
        | n::MUTED
        | n::BANNED
        | n::THROTTLED => ChatEventKind::ChannelNotice,
        n::PASSWORD_CHANGED
        | n::OWNER_CHANGED
        | n::PLAYER_NOT_FOUND
        | n::CHANNEL_OWNER
        | n::ANNOUNCEMENTS_ON
        | n::ANNOUNCEMENTS_OFF
        | n::MODERATION_ON
        | n::MODERATION_OFF
        | n::PLAYER_KICKED
        | n::PLAYER_BANNED
        | n::PLAYER_UNBANNED
        | n::PLAYER_NOT_BANNED
        | n::PLAYER_ALREADY_MEMBER
        | n::INVITE
        | n::INVITE_WRONG_FACTION
        | n::WRONG_FACTION
        | n::INVALID_NAME
        | n::NOT_MODERATED
        | n::PLAYER_INVITED
        | n::PLAYER_INVITE_BANNED => ChatEventKind::ChannelNoticeUser,
        _ => ChatEventKind::System,
    };
    Some(ChatEvent {
        kind: Some(kind),
        sender: a.unwrap_or_default(),
        target: b.unwrap_or_default(),
        channel,
        notice: notice_byte.to_string(),
        ..Default::default()
    })
}

/// Deliver one built event: a `YOU_JOINED` claims its slot before the render, a `YOU_LEFT` frees
/// it after, so the leave line keeps its number and colour ("Left Channel: [2. General - Elwynn
/// Forest]") and a handler's `GetChannelName` still finds it. The reference flags the teardown
/// (`0x49c115`), fires the event (`0x49c5b0` calling `0x49a870`), then drops the record
/// (`0x49c5b5`, `0x49c5c2` calling `0x49bbd0`); its suspended leg (`0x49c0e9`) skips the flag.
/// Both arms mirror the list into the VM for `GetChannelName`.
pub(super) fn deliver(
    script: &mut wrathplus_ui::script::UiScript,
    windows: &mut ChatWindows,
    channels: &mut ChannelState,
    event: &mut ChatEvent,
) {
    let notice = event
        .notice_byte()
        .filter(|_| event.kind == Some(ChatEventKind::ChannelNotice));
    // Read first: the reference picks the token from `slot+0x9c` (`0x49c0c2`) before writing it
    // (`0x49bb20`), and the renamed and suspended tokens keep the window's registration.
    event.slot_state = channels.slot_state(&event.channel);
    if let Some(byte) = notice {
        // The server's half of every join and leave, for a log to tell a request from its answer.
        debug!(
            "chat: channel notice {byte:#04x} for {:?} (slot {:?}, {:?})",
            event.channel,
            channels.number_of(&event.channel),
            event.slot_state
        );
    }
    if notice == Some(channel_notice::YOU_JOINED) {
        // A confirmed join sets the `ZONECHANNELS` bit (`0x49bbaf`), its only runtime growth.
        channels.note_zone_channel_joined(&event.channel);
        // A custom channel's confirmed join enters the chat cache's re-join list.
        channels.note_custom_channel_joined(&event.channel);
        // A slot the walk renamed is numbered already, but its state must return to `Joined`.
        channels.confirm_slot(&event.channel);
    }
    if notice == Some(channel_notice::YOU_JOINED) && channels.number_of(&event.channel).is_none() {
        match channels.claim_slot(&event.channel) {
            Some(slot) => {
                debug!(
                    "chat: server confirms channel {:?} joined (slot {slot})",
                    event.channel
                );
                script.set_joined_channels(channels.names());
            }
            // All ten slots taken: no number, as in the reference, whose slot allocator also
            // prints `ERR_TOO_MANY_CHAT_CHANNELS` (`0x199` at `0x49b9c5`); here it is only logged.
            None => warn!(
                "chat: server confirms channel {:?} joined but all {} slots are taken — it has no \
                 number, so /N cannot reach it",
                event.channel,
                super::edit::MAX_CHANNELS
            ),
        }
    }
    // The wire name, kept before `stamp_channel` decorates arg4 with the slot number.
    let leaving = (notice == Some(channel_notice::YOU_LEFT)).then(|| event.channel.clone());
    // The channel renders numbered when its slot is known.
    channels.stamp_channel(event);
    route(script, windows, event);
    if let Some(name) = leaving {
        // A suspended slot (state 3, `0x49c0e0`) survives its leave: the reference skips the
        // teardown (`0x49c115`), so leaving a capital keeps Trade's number for the re-join.
        if event.slot_state == Some(super::edit::SlotState::Suspended) {
            debug!("chat: channel {name:?} suspended — the record and its number stay");
        } else {
            // Cleared in place, never compacted: slot 3 stays 3 when slot 2 empties.
            let freed = channels.free_slot(&name);
            debug!("chat: server confirms channel {name:?} left (slot {freed:?} now free)");
            script.set_joined_channels(channels.names());
        }
    }
}

/// Pair self-notice metadata with its shared delivery, not a frame-end snapshot: a
/// single receive batch may contain join/leave/join for the same slot.
fn deliver_native(
    script: &mut wrathplus_ui::script::UiScript,
    windows: &mut ChatWindows,
    channels: &mut ChannelState,
    event: &mut ChatEvent,
    membership: Option<&mut super::native_membership::NativeMembership>,
    notices: Option<&mut super::native_roster::NativeChannelNotices>,
) {
    use wrathplus_protocol::wrath_channel::NotifyKind;
    let native = match (membership, notices) {
        (Some(ledger), Some(notices)) => {
            let expected = event.notice_byte();
            let matches = notices.0.front().is_some_and(|notice| {
                notice.channel.eq_ignore_ascii_case(&event.channel)
                    && matches!(
                        (&notice.kind, expected),
                        (
                            NotifyKind::YouJoined { .. },
                            Some(channel_notice::YOU_JOINED)
                        ) | (NotifyKind::YouLeft { .. }, Some(channel_notice::YOU_LEFT))
                    )
            });
            matches.then(|| (ledger, notices.0.pop_front().unwrap()))
        }
        _ => None,
    };
    if let Some((ledger, notice)) = native {
        let name = notice.channel;
        match notice.kind {
            NotifyKind::YouJoined { channel_id, .. } => {
                // AC UpdateLocalChannels sends the new constant-channel join without an old-name
                // leave. Reuse its confirmed ChannelID's slot before the stock YOU_CHANGED event.
                if channels.number_of(&name).is_none() {
                    if let Some(old) = ledger.replacement_name(channel_id).map(str::to_owned) {
                        channels.rename_slot(&old, &name);
                        script.set_joined_channels(channels.names());
                    }
                }
                deliver(script, windows, channels, event);
                script.set_joined_channels(channels.names());
                ledger.joined(&name, channel_id, channels.number_of(&name));
            }
            NotifyKind::YouLeft { is_constant, .. } => {
                ledger.left(&name, is_constant);
                // Raw native suspension preserves the actual record, even when the
                // Vanilla approximation had not marked this slot suspended locally.
                if ledger.retains(&name) {
                    channels.suspend_slot(&name);
                } else {
                    channels.confirm_slot(&name);
                }
                deliver(script, windows, channels, event);
            }
        }
    } else {
        deliver(script, windows, channels, event);
    }
}

/// The speaker's bubble and gesture, bundled for Bevy's sixteen-parameter limit on
/// [`feed_chat`]; the bubble has a 20 yd range test and two CVars, the gesture neither.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SpeakerEffects<'w> {
    native_state: Option<ResMut<'w, super::native_message::NativeChatFeed>>,
    social: Option<Res<'w, crate::app_core::interaction::SocialState>>,
    native_chat: Option<ResMut<'w, crate::ui_native_chat::NativeChatPackets>>,
    native_membership: Option<ResMut<'w, super::native_membership::NativeMembership>>,
    native_notices: Option<ResMut<'w, super::native_roster::NativeChannelNotices>>,
    /// The achievement line's `$a` link: the row names and the game date.
    achievements: Option<Res<'w, crate::ui_achievements::AchievementMetadata>>,
    server_time: Option<Res<'w, crate::net::ServerTime>>,
    bubbles: ResMut<'w, crate::chat_bubble::BubbleQueue>,
    bubble_cfg: Res<'w, crate::chat_bubble::BubbleConfig>,
    gestures: ResMut<'w, crate::creature_anim::GestureQueue>,
}

/// `UNIT_FIELD_FLAGS` bit `0x20000000` (`PREVENT_ANIM`: Polymorph, Kidney Shot, Gouge, Sleep).
const UNIT_FLAG_PREVENT_ANIM: u32 = 0x2000_0000;

/// Whether the speaker's flags suppress the chat talk gesture. Only the handler's own copy of
/// the selector tests the bit (`0x49d7f7`, the one `0xa0` test of `0x20000000` in the image),
/// which runs when the sender's name is already cached (`tries == 0`); a line held for the name
/// query gestures from the deferred copies (`0x49cf2b`, `0x49d3bb`), which do not.
pub(super) fn gesture_prevented(tries: u16, speaker_flags: Option<u32>) -> bool {
    tries == 0 && speaker_flags.is_some_and(|f| f & UNIT_FLAG_PREVENT_ANIM != 0)
}

/// Drain [`ChatLog`]: resolve names (ask-once, bounded), build the events and [`route`] them.
pub fn feed_chat(
    script: Option<NonSendMut<wrathplus_ui::script::UiScript>>,
    mut log: ResMut<ChatLog>,
    mut windows: ResMut<ChatWindows>,
    mut channels: ResMut<ChannelState>,
    names: Res<NameCache>,
    mut speaker: SpeakerEffects,
    commands: Res<NetCommands>,
    // The text-emote tables, and our guid for the "are you the performer?" test.
    emote_texts: Option<Res<EmoteTexts>>,
    self_guid: Res<SelfGuid>,
    // The `$`-macro subject: `0x579590` expands against the receiver's descriptors.
    guids: Res<GuidIndex>,
    stores: Query<&ObjectStore>,
    states: Res<crate::app_core::state::WorldStates>,
    // The language gate: the word pool, this character's fluency and the GM bit.
    langs: Res<super::language::ChatLanguages>,
    // Item names for `TRADESKILL_LOG_*` and `FEEDPET_LOG_*`.
    items: Res<crate::items::Items>,
    // The spam and profanity filters, which run in the reference's display function `0x49a870`.
    mut text_filter: crate::text_filter::ChatTextFilter,
) {
    let Some(mut script) = script else {
        return;
    };
    if let Some(packets) = speaker.native_chat.as_deref_mut() {
        log.pending.extend(
            super::native_message::take_visible(&mut packets.0)
                .into_iter()
                .map(|msg| Pending::Native { msg, tries: 0 }),
        );
    }
    if log.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut log.pending);
    let mut still = Vec::new();
    for item in pending {
        match item {
            Pending::Native { msg, tries } => {
                let Some(state) = speaker.native_state.as_deref_mut() else {
                    continue;
                };
                if !super::native_message::deliver(
                    &mut script,
                    &mut windows,
                    &mut channels,
                    &names,
                    &commands,
                    speaker.social.as_deref(),
                    state,
                    &msg,
                    tries >= NAME_MAX_TRIES,
                    &|text, guid| {
                        chat_macros(text, guid, (&guids, &stores), &names, &commands, &states)
                    },
                    &|id| {
                        let name = speaker.achievements.as_deref()?.name(id)?.to_string();
                        let date = speaker
                            .server_time
                            .as_deref()
                            .and_then(|t| t.0.as_ref())
                            .map_or((0, 1, 1), |t| t.date());
                        Some((name, date))
                    },
                    &mut |guid, kind, text| {
                        speaker.bubbles.push(&speaker.bubble_cfg, guid, kind, text)
                    },
                ) {
                    still.push(Pending::Native {
                        msg,
                        tries: tries.saturating_add(1),
                    });
                }
            }
            Pending::Event(mut event) => {
                if wrath_needs_raw_event_host_log(&script) {
                    warn!(
                        kind = ?event.kind,
                        text = %event.text,
                        "Wrath raw pending chat event has no ChatFrame; preserving it in the host log"
                    );
                }
                deliver_native(
                    &mut script,
                    &mut windows,
                    &mut channels,
                    &mut event,
                    speaker.native_membership.as_deref_mut(),
                    speaker.native_notices.as_deref_mut(),
                );
            }
            Pending::Wire { msg, tries } => {
                let kind = kind_of_wire(msg.chat_type);
                if kind.is_none() {
                    warn!(
                        "chat: unmodeled wire type {:#04x} dropped: {:?}",
                        msg.chat_type, msg.text
                    );
                    continue;
                }
                // A monster line carries its name inline; a player line resolves its guid.
                let name = match &msg.sender_name {
                    Some(n) => Some(n.clone()),
                    None if needs_name(msg.chat_type) && msg.sender_guid != 0 => names
                        .resolve(msg.sender_guid, &commands)
                        .map(str::to_string),
                    _ => None,
                };
                if name.is_none()
                    && needs_name(msg.chat_type)
                    && msg.sender_guid != 0
                    && tries < NAME_MAX_TRIES
                {
                    still.push(Pending::Wire {
                        msg,
                        tries: tries + 1,
                    });
                    continue;
                }
                // The base name; `stamp_channel` adds arg4's number and arg7-arg9 below.
                let channel_base = msg.channel.clone().unwrap_or_default();
                let plain = msg.text.clone();
                // The language gate: the garble writes the one buffer the line, `arg1` and the
                // bubble read, as `0x49a870` fills `[ebp-0xd0c]` once (`0x49a9f0` copy, `0x49aa7c`
                // garble) and never reads the wire text again.
                let language = langs.effective_language(msg.chat_type, msg.language);
                let mut text = langs.garble(language, &plain);

                // The text filters, on the garbled buffer as in `0x49a870`: spam first, then the
                // mask, so a dropped line consumes no mask indices (the phase is global).
                if text_filter.should_drop(msg.chat_type, msg.chat_tag, langs.is_gm(), &text) {
                    // Nothing shows in its place: the not-a-whisper leg (`0x49ab33`) jumps to the
                    // epilogue `0x49afd7`. A whisper with a non-zero report guid also sends
                    // `CMSG_CHAT_FILTERED` (`0x331`, the `u64` guid, `0x49ab50`) in the reference;
                    // not sent here, as vmangos leaves the opcode unhandled (`Opcodes.cpp:932`).
                    debug!(
                        "chat: spam filter dropped [{:#04x}] {:?}",
                        msg.chat_type, text
                    );
                    continue;
                }
                text_filter.mask_chat(msg.chat_type, &mut text);
                let text = text;
                let mut event = ChatEvent {
                    kind,
                    text: text.clone(),
                    sender: name.unwrap_or_else(|| {
                        if needs_name(msg.chat_type) && msg.sender_guid != 0 {
                            "Unknown".to_string()
                        } else {
                            String::new()
                        }
                    }),
                    // The effective language, not the wire's: narration types and GM mode force
                    // 0, which also drops the `[Language]` header.
                    language: language_name(language).to_string(),
                    channel: channel_base,
                    flag: flag_of_tag(msg.chat_tag).to_string(),
                    ..Default::default()
                };
                channels.stamp_channel(&mut event);
                route(&mut script, &mut windows, &event);
                // The bubble gets the same text: the reference spawns it in this path (`0x49acd9`).
                if let Some(kind) = event.kind {
                    speaker
                        .bubbles
                        .push(&speaker.bubble_cfg, msg.sender_guid, kind, &text);
                }
                // The gesture reads the raw type and `plain`, not the garbled text: the selector is
                // in the parser (`0x49d560`, `0x49d820`-`0x49d8ae`) on the buffer `0x49dbc2` hands
                // `0x49a870`, so a Horde `lol` laughs for every observer.
                // The immediate path alone refuses a speaker that cannot animate (`0x49d7f7`).
                let speaker_flags = guids
                    .0
                    .get(&msg.sender_guid)
                    .and_then(|e| stores.get(*e).ok())
                    .map(|s| s.0.unit_flags());
                if let Some(gesture) =
                    crate::creature_anim::select_gesture(msg.chat_type, &plain, |n| {
                        script
                            .lua()
                            .globals()
                            .get::<String>(format!("LAUGH_WORD{n}"))
                            .ok()
                    })
                    .filter(|_| !gesture_prevented(tries, speaker_flags))
                {
                    speaker.gestures.push(msg.sender_guid, gesture);
                }
            }
            Pending::Notice {
                notice,
                channel,
                a_guid,
                b_guid,
                tries,
            } => {
                let a = names.resolve(a_guid, &commands).map(str::to_string);
                let b = if b_guid != 0 {
                    names.resolve(b_guid, &commands).map(str::to_string)
                } else {
                    Some(String::new())
                };
                if (a.is_none() || b.is_none()) && tries < NAME_MAX_TRIES {
                    still.push(Pending::Notice {
                        notice,
                        channel,
                        a_guid,
                        b_guid,
                        tries: tries + 1,
                    });
                    continue;
                }
                // Stamped against the channel list as it stands at delivery: the reference hands
                // the composer `0x49a870` the notice's channel name, directly when the names are
                // cached (`0x49c59f`-`0x49c5b0`), else from the queued line's node (`+0x2c`) in
                // the name-query callback (`0x49cfb4`-`0x49cfc1`), and the composer looks it up
                // among the joined channels (`0x49aa1e`-`0x49aa31`). Unstamped, arg7-arg9 are
                // empty and the stock filter (`ChatFrame.lua:1374-1392`) drops the line.
                if let Some(mut event) = notice_event(
                    notice,
                    channel,
                    Some(a.unwrap_or_else(|| "Unknown".into())),
                    b,
                ) {
                    deliver_native(
                        &mut script,
                        &mut windows,
                        &mut channels,
                        &mut event,
                        speaker.native_membership.as_deref_mut(),
                        speaker.native_notices.as_deref_mut(),
                    );
                }
            }
            Pending::Addon {
                prefix,
                message,
                distribution,
                guid,
                tries,
            } => {
                let name = names.resolve(guid, &commands).map(str::to_string);
                if name.is_none() && tries < NAME_MAX_TRIES {
                    still.push(Pending::Addon {
                        prefix,
                        message,
                        distribution,
                        guid,
                        tries: tries + 1,
                    });
                    continue;
                }
                fire_addon_message(
                    &mut script,
                    prefix,
                    message,
                    distribution,
                    name.unwrap_or_else(|| "Unknown".into()),
                );
            }
            Pending::Roll {
                min,
                max,
                roll,
                guid,
                tries,
            } => {
                let name = names.resolve(guid, &commands).map(str::to_string);
                if name.is_none() && tries < NAME_MAX_TRIES {
                    still.push(Pending::Roll {
                        min,
                        max,
                        roll,
                        guid,
                        tries: tries + 1,
                    });
                    continue;
                }
                // `RANDOM_ROLL_RESULT`, "%s rolls %d (%d-%d)" (`GlobalStrings.lua:3290`).
                let name = name.unwrap_or_else(|| "Unknown".into());
                let line = {
                    use wrathplus_ui::strings::Arg;
                    keyed(
                        &globals(&script),
                        "RANDOM_ROLL_RESULT",
                        &[
                            Arg::S(&name),
                            Arg::D(i64::from(roll)),
                            Arg::D(i64::from(min)),
                            Arg::D(i64::from(max)),
                        ],
                    )
                };
                if let Some(text) = line {
                    route(
                        &mut script,
                        &mut windows,
                        &ChatEvent::text_only(ChatEventKind::System, text),
                    );
                }
            }
            Pending::XpGain {
                victim,
                total,
                bonus,
                tries,
            } => {
                // `victim == 0` is the unnamed form, with no name to wait for.
                let name = if victim == 0 {
                    None
                } else {
                    let resolved = names.resolve(victim, &commands).map(str::to_string);
                    if resolved.is_none() && tries < NAME_MAX_TRIES {
                        still.push(Pending::XpGain {
                            victim,
                            total,
                            bonus,
                            tries: tries + 1,
                        });
                        continue;
                    }
                    Some(resolved.unwrap_or_else(|| "Unknown".into()))
                };
                let line = xp_gain_line(name.as_deref(), total, bonus, &globals(&script));
                if let Some(text) = line {
                    route(
                        &mut script,
                        &mut windows,
                        &ChatEvent::text_only(ChatEventKind::CombatXpGain, text),
                    );
                }
            }
            Pending::HonorGain {
                victim,
                honor,
                rank,
                tries,
            } => {
                // `victim == 0` is the award form: no name, no rank title.
                let name = if victim == 0 {
                    None
                } else {
                    let resolved = names.resolve(victim, &commands).map(str::to_string);
                    if resolved.is_none() && tries < NAME_MAX_TRIES {
                        still.push(Pending::HonorGain {
                            victim,
                            honor,
                            rank,
                            tries: tries + 1,
                        });
                        continue;
                    }
                    resolved
                };
                // The side was read off the victim at receipt; the gender is ours (unconfirmed for
                // 12340: `0x7225e0` is called with gender -1).
                let female = self_guid
                    .0
                    .and_then(|g| names.player_traits(g))
                    .is_some_and(|(_, _, gender)| gender == 1);
                let title = match rank {
                    Some((rank, Some(side))) => script.pvp_rank_title(rank, side, female),
                    _ => None,
                };
                let name = (victim != 0).then(|| name.unwrap_or_else(|| "Unknown".into()));
                let line = honor_gain_line(
                    name.as_deref(),
                    title.as_deref(),
                    rank.is_some(),
                    honor,
                    &globals(&script),
                );
                if let Some(text) = line {
                    route(
                        &mut script,
                        &mut windows,
                        &ChatEvent::text_only(ChatEventKind::CombatHonorGain, text),
                    );
                }
            }
            Pending::TextEmote {
                performer,
                text_id,
                target,
                tries,
            } => {
                // Both names ask-once: the performer's, and ours for the "is the target me?" test.
                let performer_name = names.resolve(performer, &commands).map(str::to_string);
                let your_name = self_guid
                    .0
                    .and_then(|g| names.resolve(g, &commands).map(str::to_string));
                if (performer_name.is_none() || your_name.is_none()) && tries < NAME_MAX_TRIES {
                    still.push(Pending::TextEmote {
                        performer,
                        text_id,
                        target,
                        tries: tries + 1,
                    });
                    continue;
                }
                // No performer name, no line (not "Unknown"): the reference bails when its
                // player-only name cache misses (`0x49b28c`); the animation rode `EmoteMessage`.
                let (Some(performer_name), Some(cat)) = (performer_name, emote_texts.as_deref())
                else {
                    debug!("chat: text emote {text_id} from {performer:#x} has no sentence source");
                    continue;
                };
                let event = text_emote_event(
                    &cat.0,
                    text_id,
                    &EmoteLine {
                        performer: &performer_name,
                        performer_is_you: self_guid.0 == Some(performer),
                        // Sex 1 is female, where the reference reads it (`record+0x144`).
                        performer_female: names
                            .player_traits(performer)
                            .is_some_and(|(_, _, sex)| sex == 1),
                        target: &target,
                        your_name: your_name.as_deref().unwrap_or_default(),
                    },
                );
                // No sentence is a real outcome: `/sit` prints nothing.
                let Some(event) = event else { continue };
                route(&mut script, &mut windows, &event);
            }
            Pending::Combat(mut line) => {
                // Names, ask-once within `tries`: the subject's through the name cache, an item's
                // through the item cache; an item the server answered unknown composes as it is.
                let mut wait = false;
                if line.subject != 0 && line.fills.subject.is_empty() {
                    let unit = guids.0.get(&line.subject).and_then(|e| stores.get(*e).ok());
                    match names.resolve_unit(line.subject, unit, &commands) {
                        Some(name) => line.fills.subject = name.to_owned(),
                        None => wait = true,
                    }
                }
                if let Some(entry) = line.item.filter(|_| line.fills.named.is_empty()) {
                    match items.template(entry, 0, &commands).map(|t| t.name.clone()) {
                        Some(name) => line.fills.named = name,
                        None if !items.template_answered_unknown(entry) => wait = true,
                        None => {}
                    }
                }
                if wait {
                    if line.tries < NAME_MAX_TRIES {
                        line.tries += 1;
                        still.push(Pending::Combat(line));
                    }
                    continue;
                }
                let composed = super::combat::compose_line(&script, line.family, &line.fills);
                if let Some(text) = composed {
                    route(
                        &mut script,
                        &mut windows,
                        &ChatEvent::text_only(line.kind, text),
                    );
                }
            }
            Pending::LootMoney { amount, sole } => {
                let text = loot_money_line(amount, sole, &globals(&script));
                if let Some(text) = text {
                    let event = ChatEvent::text_only(ChatEventKind::Money, text);
                    route(&mut script, &mut windows, &event);
                }
            }
            Pending::Discovery { area, xp } => {
                // The toast fires every time; the chat line only rides XP.
                let (toast, line) = {
                    let get = globals(&script);
                    (
                        exploration_toast(&area, &get),
                        (xp > 0)
                            .then(|| exploration_line(&area, xp, &get))
                            .flatten(),
                    )
                };
                if let Some(toast) = toast {
                    script.fire_event(
                        "UI_INFO_MESSAGE",
                        vec![wrathplus_ui::script::ScriptValue::Str(toast)],
                    );
                }
                if let Some(text) = line {
                    route(
                        &mut script,
                        &mut windows,
                        &ChatEvent::text_only(ChatEventKind::System, text),
                    );
                }
            }
        }
    }
    log.pending = still;
}

/// Whether a wire chat type carries a player guid to name (not a monster's inline name).
fn needs_name(chat_type: u8) -> bool {
    use wrathplus_protocol::types as m;
    matches!(
        chat_type,
        m::CHAT_MSG_SAY
            | m::CHAT_MSG_PARTY
            | m::CHAT_MSG_RAID
            | m::CHAT_MSG_GUILD
            | m::CHAT_MSG_OFFICER
            | m::CHAT_MSG_YELL
            | m::CHAT_MSG_WHISPER
            | m::CHAT_MSG_WHISPER_INFORM
            | m::CHAT_MSG_EMOTE
            | m::CHAT_MSG_CHANNEL
            | m::CHAT_MSG_AFK
            | m::CHAT_MSG_DND
            | m::CHAT_MSG_IGNORED
            | m::CHAT_MSG_RAID_LEADER
            | m::CHAT_MSG_RAID_WARNING
            | m::CHAT_MSG_BATTLEGROUND
            | m::CHAT_MSG_BATTLEGROUND_LEADER
    )
}

/// `/played` for scripts: each `RequestTimePlayed()` sends one empty `CMSG_PLAYED_TIME`, and the
/// answer fires `TIME_PLAYED_MSG`. The stock `ChatFrame_OnEvent` prints that event as the
/// `TIME_PLAYED_TOTAL` and `TIME_PLAYED_LEVEL` lines (`ChatFrame.lua:1279`), so the English lines
/// `ui_chat::net::played_time` also prints are a duplicate.
pub fn played_time_bridge(
    script: Option<NonSendMut<wrathplus_ui::script::UiScript>>,
    commands: Res<NetCommands>,
    answer: Option<ResMut<crate::net::PlayedTimeAnswer>>,
) {
    let Some(mut script) = script else {
        return;
    };
    for _ in 0..script.take_played_time_asks() {
        let _ = commands.0.send(crate::net::ClientCommand::PlayedTime);
    }
    let Some(mut answer) = answer else {
        return;
    };
    if let Some((total, level)) = answer.0.take() {
        script.fire_event(
            "TIME_PLAYED_MSG",
            vec![
                wrathplus_ui::script::ScriptValue::Int(i64::from(total)),
                wrathplus_ui::script::ScriptValue::Int(i64::from(level)),
            ],
        );
    }
}

/// `ChatLog`'s methods that need this module (the type lives in `app_core` for the crate split).
pub trait ChatLogFeed {
    /// Queue one client-composed line ([`super::combat`]); the drain fills its template.
    fn push_combat(&mut self, line: super::combat::PendingCombat);
    /// Queue a decoded `SMSG_CHANNEL_NOTIFY`; a guid tail waits for its names.
    fn push_channel_notice(&mut self, notice_byte: u8, channel: String, tail: &ChannelNoticeTail);
    /// Queue one world broadcast for [`super::broadcast::feed_broadcasts`]'s resolve pass.
    fn push_broadcast(&mut self, b: super::broadcast::Broadcast);
    /// Take the parked broadcasts, leaving the queue empty.
    fn take_broadcasts(&mut self) -> Vec<super::broadcast::Broadcast>;
}

impl ChatLogFeed for ChatLog {
    fn push_combat(&mut self, line: super::combat::PendingCombat) {
        self.pending.push(Pending::Combat(Box::new(line)));
    }

    fn push_channel_notice(&mut self, notice_byte: u8, channel: String, tail: &ChannelNoticeTail) {
        let (a_guid, b_guid, name) = match tail {
            ChannelNoticeTail::Guid(g) | ChannelNoticeTail::Actor(g) => (*g, 0, None),
            ChannelNoticeTail::Actors { target, source } => (*target, *source, None),
            ChannelNoticeTail::Name(n) => (0, 0, Some(n.clone())),
            ChannelNoticeTail::YouJoined { .. } | ChannelNoticeTail::Empty => (0, 0, None),
            ChannelNoticeTail::ModeChange { .. } => return, // silent in the 1.12 UI (no string)
        };
        if a_guid != 0 {
            self.pending.push(Pending::Notice {
                notice: notice_byte,
                channel,
                a_guid,
                b_guid,
                tries: 0,
            });
        } else {
            if let Some(event) = notice_event(notice_byte, channel, name, None) {
                self.pending.push(Pending::Event(event));
            }
        }
    }

    fn push_broadcast(&mut self, b: super::broadcast::Broadcast) {
        self.broadcasts.push(b);
    }

    fn take_broadcasts(&mut self) -> Vec<super::broadcast::Broadcast> {
        std::mem::take(&mut self.broadcasts)
    }
}

#[cfg(test)]
mod native_delivery_tests {
    use super::*;
    use wrathplus_protocol::wrath_channel::{Notify, NotifyKind};

    #[test]
    fn ordered_native_self_notices_keep_suspension_and_explicit_leave_distinct() {
        let mut script = wrathplus_ui::script::UiScript::new().unwrap();
        let mut windows = ChatWindows::default();
        let mut channels = ChannelState::default();
        let mut ledger = super::super::native_membership::NativeMembership::default();
        let mut notices = super::super::native_roster::NativeChannelNotices::default();
        let mut apply =
            |kind: NotifyKind, ledger: &mut super::super::native_membership::NativeMembership| {
                let byte = match kind {
                    NotifyKind::YouJoined { .. } => channel_notice::YOU_JOINED,
                    _ => channel_notice::YOU_LEFT,
                };
                notices.0.push_back(Notify {
                    channel: "custom".into(),
                    kind,
                });
                let mut event = notice_event(byte, "custom".into(), None, None).unwrap();
                deliver_native(
                    &mut script,
                    &mut windows,
                    &mut channels,
                    &mut event,
                    Some(ledger),
                    Some(&mut notices),
                );
                channels.number_of("custom")
            };
        let joined = || NotifyKind::YouJoined {
            flags: 0x83,
            channel_id: 0,
            reserved: u32::MAX,
        };
        let left = || NotifyKind::YouLeft {
            channel_id: 0,
            is_constant: 0x83,
        };
        let slot = apply(joined(), &mut ledger).unwrap();
        assert_eq!(apply(left(), &mut ledger), Some(slot));
        assert!(!ledger.snapshot(|_| Some(slot))[0].active);
        assert_eq!(apply(joined(), &mut ledger), Some(slot));
        ledger.explicit_leave("custom");
        apply(joined(), &mut ledger);
        assert_eq!(ledger.snapshot(|_| Some(slot))[0].custom_state, 1);
        assert_eq!(apply(left(), &mut ledger), None);
        assert!(!ledger.retains("custom"));
        assert!(notices.0.is_empty());
    }
}
