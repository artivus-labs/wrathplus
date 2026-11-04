//! The chat event model and its queues: every line the session, combat log and UI raise
//! (`ChatEvent`, `ChatEventKind`), the client-composed lines, pending broadcasts and
//! the drain's log. `ui_chat` keeps the systems and re-exports these from their old paths.

use bevy::prelude::*;
use wrathplus_protocol::types::{ChatMessage, LevelUpInfo, XpGain};

/// One value slot of a client-composed line's format string, in the order the shipped template
/// consumes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// `%s`: the subject's name, resolved at the drain off [`PendingCombat::subject`].
    Subject,
    /// `%s`: a name, already localized (a faction) or resolved at the drain
    /// ([`PendingCombat::item`]).
    Named,
    /// `%d`: the amount.
    Amount,
}

/// A client-composed line: its GlobalString key and the ordered slots its template consumes.
#[derive(Clone, Copy, Debug)]
pub struct Family {
    pub key: &'static str,
    pub slots: &'static [Slot],
}

/// The value bound to each [`Slot`] for one line.
#[derive(Clone, Debug, Default)]
pub struct Fills {
    /// The [`Slot::Subject`] text.
    pub subject: String,
    /// The [`Slot::Named`] text.
    pub named: String,
    pub amount: i64,
}

/// A client-composed line queued for the drain, which resolves its names and then its template off
/// the VM's `GlobalStrings.lua`.
#[derive(Clone, Debug)]
pub struct PendingCombat {
    pub kind: ChatEventKind,
    pub family: Family,
    pub fills: Fills,
    /// The unit naming [`Slot::Subject`], through the name cache; 0 when the family has none.
    pub subject: u64,
    /// The item naming [`Slot::Named`], through the item cache, as the reference's line waits on
    /// its item callback (`0x67ca30`).
    pub item: Option<u32>,
    /// Drain passes spent waiting on a name.
    pub tries: u16,
}

/// The chat-event kinds: the `ChatTypeInfo` keys WrathPlus produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatEventKind {
    Say,
    Party,
    Raid,
    Guild,
    Officer,
    Yell,
    Whisper,
    WhisperInform,
    Emote,
    TextEmote,
    System,
    MonsterSay,
    MonsterYell,
    MonsterEmote,
    MonsterWhisper,
    Channel,
    ChannelJoin,
    ChannelLeave,
    ChannelNotice,
    ChannelNoticeUser,
    ChannelList,
    Afk,
    Dnd,
    Ignored,
    Skill,
    Loot,
    Money,
    /// The client-composed XP line (`SMSG_LOG_XPGAIN`).
    CombatXpGain,
    /// The client-composed honor line (`SMSG_PVP_CREDIT`), built once the victim's name resolves.
    CombatHonorGain,
    RaidLeader,
    RaidWarning,
    RaidBossEmote,
    /// `CHAT_MSG_FILTERED` (`0x5B`), the server's "your message was filtered" notice; `arg2` is the
    /// addressee the stock frame formats `CHAT_FILTERED` over (`ChatFrame.lua:1406`).
    Filtered,
    Battleground,
    BattlegroundLeader,
    BgSystemNeutral,
    BgSystemAlliance,
    BgSystemHorde,
    // ── the combat log ───────────────────────────────────────────────────────────────
    // Every kind below has a producer.
    /// `0x19`, the reference's catch-all combat line: `DURABILITYDAMAGE_DEATH`, `SPELLDISMISSPET*`,
    /// `SPELLHAPPINESSDRAIN*` and the "string not found" warning.
    CombatMiscInfo,
    /// `0x55`, a reputation delta (`FACTION_STANDING_INCREASED`/`_DECREASED`).
    CombatFactionChange,
    /// `0x1e`, a cast's created item (`TRADESKILL_LOG_*`, `0x50bd90`).
    Tradeskills,
    /// `0x1f`, the pet eating what it was fed (`FEEDPET_LOG_*`, `0x50baf0`).
    PetInfo,
}

impl ChatEventKind {
    /// The combat log's own kinds. The reference tests the `COMBAT_`/`SPELL_` prefix
    /// (`ChatFrame.lua:1397-1399`); `COMBAT_XP_GAIN` and `COMBAT_HONOR_GAIN`, composed apart from
    /// the combat log, are outside this set.
    pub fn is_combat_log(self) -> bool {
        use ChatEventKind as K;
        matches!(self, K::CombatMiscInfo | K::CombatFactionChange)
    }

    /// Every kind, for the exhaustive sweeps; `ui_chat::tests::every_kind_is_in_all` fails when one
    /// is missing.
    #[cfg(any(test, feature = "test-support"))]
    pub const ALL: &'static [ChatEventKind] = {
        use ChatEventKind as K;
        &[
            K::Say,
            K::Party,
            K::Raid,
            K::Guild,
            K::Officer,
            K::Yell,
            K::Whisper,
            K::WhisperInform,
            K::Emote,
            K::TextEmote,
            K::System,
            K::MonsterSay,
            K::MonsterYell,
            K::MonsterEmote,
            K::MonsterWhisper,
            K::Channel,
            K::ChannelJoin,
            K::ChannelLeave,
            K::ChannelNotice,
            K::ChannelNoticeUser,
            K::ChannelList,
            K::Afk,
            K::Dnd,
            K::Ignored,
            K::Skill,
            K::Loot,
            K::Money,
            K::CombatXpGain,
            K::CombatHonorGain,
            K::RaidLeader,
            K::RaidWarning,
            K::RaidBossEmote,
            K::Filtered,
            K::Battleground,
            K::BattlegroundLeader,
            K::BgSystemNeutral,
            K::BgSystemAlliance,
            K::BgSystemHorde,
            K::CombatMiscInfo,
            K::CombatFactionChange,
            K::Tradeskills,
            K::PetInfo,
        ]
    };
}

/// One `CHAT_MSG_*` fire, typed. The fire helper `0x49b0b0` signals it (`0x703f50`) with the
/// format `"%s%s%s%s%s%s%d%d%s%d"` (`.rdata 0x844608`): arg7, arg8 and arg10 are numbers, the
/// rest strings, never nil, as `ChatFrame_OnEvent` compares `arg7 > 0` bare.
///
/// | arg | field | what `ChatFrame_OnEvent` reads |
/// |---|---|---|
/// | 1 | `text`, `notice` | the body, or a channel notice's `CHAT_<token>_NOTICE` token |
/// | 2 | `sender` | the speaker, or a notice's affected player |
/// | 3 | `language` | a language name; empty prints no header |
/// | 4 | `channel` | the display form, "N. Name - Zone" when numbered |
/// | 5 | `target` | a two-name notice's second name, "X kicked by Y" |
/// | 6 | `flag` | "AFK", "DND" or "GM", as `CHAT_FLAG_<flag>` |
/// | 7 | `zone_channel_id` | the ChannelID, 0 for a custom channel (`ChatFrame.lua:1379`) |
/// | 8 | `channel_number` | the local slot, `ChatTypeInfo["CHANNEL"..arg8]` |
/// | 9 | `channel_base` | the channel name without its number |
/// | 10 | none | the split index, 0 from vmangos (`Chat/Channel.cpp:823-827`) |
/// | 11 | none | the 3.3.5 line id, which the player link carries (`"|Hplayer:"..arg2..":"..arg11`) |
/// | 12 | none | the 3.3.5 sender GUID string, `GetColoredName`'s class lookup; `""` for none |
///
/// arg4, arg7, arg8 and arg9 are one record (`ui_chat::edit::ChannelState::stamp_channel`). arg1
/// is the garbled text: the reference fills one buffer (`SStrCopy` at `0x49a9f0`, the garble
/// `0x49b560` at `0x49aa7c`) that the line, arg1 and the bubble share (`ui_chat::language`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatEvent {
    pub kind: Option<ChatEventKind>,
    pub text: String,
    pub sender: String,
    pub language: String,
    pub channel: String,
    pub target: String,
    pub flag: String,
    /// arg7, the ChannelID behind a zone channel (1 General, 2 Trade, 22 LocalDefense), else 0.
    pub zone_channel_id: u32,
    pub channel_number: u32,
    pub channel_base: String,
    pub notice: String,
    /// Our slot's state for this channel as the notice arrived (`None`: no slot), set before
    /// `ui_chat::feed::deliver` changes it: the reference's notice arms read `slot+0x9c` for the
    /// token (`0x49c0c2`) before they write it (`0x49bb20`).
    pub slot_state: Option<SlotState>,
}

impl ChatEvent {
    /// A bare kind+text event (SYSTEM lines, loot receive lines, TEXT_EMOTE sentences).
    pub fn text_only(kind: ChatEventKind, text: String) -> Self {
        ChatEvent {
            kind: Some(kind),
            text,
            ..Default::default()
        }
    }

    /// The `SMSG_CHANNEL_NOTIFY` byte behind a channel notice, which `notice` holds in decimal.
    pub fn notice_byte(&self) -> Option<u8> {
        self.notice.parse().ok()
    }
}

/// One parked world broadcast, as the wire sent it, awaiting the catalogs and the channel walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Broadcast {
    /// `SMSG_ZONE_UNDER_ATTACK`: the attacked area's `AreaTable.dbc` id.
    ZoneUnderAttack { area_id: u32 },
    /// `SMSG_DEFENSE_MESSAGE`: the zone it is about, and the server's text.
    Defense { zone_id: u32, text: String },
    /// `SMSG_SERVER_MESSAGE`: a `ServerMessages.dbc` row id and the text for its `%s`.
    Server { message_type: u32, text: String },
    /// `SMSG_CHAT_RESTRICTED`, which carries nothing.
    ChatRestricted,
}

/// One queued item awaiting its turn through [`feed_chat`].
pub enum Pending {
    Native {
        msg: wrathplus_protocol::wrath_chat::Message,
        tries: u16,
    },
    /// A decoded `SMSG_MESSAGECHAT`; the player types wait on the sender's name.
    Wire { msg: ChatMessage, tries: u16 },
    /// A channel notice with guids to name (`b_guid` is a kick's or ban's actor, 0 when absent).
    Notice {
        notice: u8,
        channel: String,
        a_guid: u64,
        b_guid: u64,
        tries: u16,
    },
    /// An inbound addon line waiting on its sender's name: the reference fires `CHAT_MSG_ADDON`
    /// (event 227, `0x49a95f`) from the name-query callback (`0x49ccc0`), so `sender` is a name.
    Addon {
        prefix: String,
        message: String,
        distribution: String,
        guid: u64,
        tries: u16,
    },
    /// A `/random` broadcast awaiting the roller's name.
    Roll {
        min: u32,
        max: u32,
        roll: u32,
        guid: u64,
        tries: u16,
    },
    /// An XP award waiting on the victim's name (0: unnamed); `bonus` is total minus base.
    XpGain {
        victim: u64,
        total: u32,
        bonus: u32,
        tries: u16,
    },
    /// An honor line waiting on the victim's name; `rank` is `None` for the unranked form, else the
    /// internal rank and the victim's title side, which becomes a title at the drain.
    HonorGain {
        victim: u64,
        honor: i32,
        rank: Option<(u8, Option<u8>)>,
        tries: u16,
    },
    /// A text emote waiting on its performer's name (`PENDINGTEXTEMOTE`, `0x49cc00`, drained by
    /// `0x49d0d0`); `target` is the wire's name, empty when untargeted, used as sent.
    TextEmote {
        performer: u64,
        text_id: u32,
        target: String,
        tries: u16,
    },
    /// An area discovery (`SMSG_EXPLORATION_EXPERIENCE`), fired at the drain, which holds the VM.
    Discovery { area: String, xp: u32 },
    /// `SMSG_LOOT_MONEY_NOTIFY`'s line (`0x6ceca0`), resolved at the drain, which holds the VM.
    LootMoney { amount: u32, sole: bool },
    /// One combat-log line, classified at the packet, waiting on its two endpoint names; the
    /// reference's deferred queue (`0xc4e208`) waits only on an item name.
    Combat(Box<PendingCombat>),
    /// A ready event (client-composed lines; name-carrying notices).
    Event(ChatEvent),
}

/// The chat items the feeds queue and [`feed_chat`] drains. World broadcasts wait apart, as their
/// resolve needs more than [`feed_chat`] can hold at Bevy's `SystemParam` limit;
/// `feed_broadcasts` resolves them onto `pending` earlier in the same frame.
#[derive(Resource, Default)]
pub struct ChatLog {
    pub pending: Vec<Pending>,
    pub broadcasts: Vec<Broadcast>,
    /// A level-up's gains for `ui_unit`'s `PLAYER_LEVEL_UP` fire, matched by level: the fire
    /// follows a descriptor change and the gains a packet.
    pub(crate) level_up_gains: Vec<(LevelUpInfo, u32)>,
}

impl ChatLog {
    /// Queue a decoded wire line (`SMSG_MESSAGECHAT`).
    pub fn push_wire(&mut self, msg: ChatMessage) {
        self.pending.push(Pending::Wire { msg, tries: 0 });
    }

    /// Queue a client-composed event: loot, quest and system lines, the `/played` answer.
    pub fn push_event(&mut self, event: ChatEvent) {
        self.pending.push(Pending::Event(event));
    }

    /// Queue an inbound addon line for its sender's name. The text splits on its first tab, and
    /// with none it is all prefix (`0x49a8d0`); `distribution` names PARTY, RAID, GUILD and
    /// BATTLEGROUND, any other type `"UNKNOWN"` (`0x49aff4`).
    pub fn push_addon(&mut self, text: &str, chat_type: u8, guid: u64) {
        let (prefix, message) = match text.find('\t') {
            Some(i) => (text[..i].to_string(), text[i + 1..].to_string()),
            None => (text.to_string(), String::new()),
        };
        // The protocol's `CHAT_TYPE_*` constants.
        let distribution = {
            use wrathplus_protocol::types as m;
            match u32::from(chat_type) {
                m::CHAT_TYPE_PARTY => "PARTY",
                m::CHAT_TYPE_RAID => "RAID",
                m::CHAT_TYPE_GUILD => "GUILD",
                m::CHAT_TYPE_BATTLEGROUND => "BATTLEGROUND",
                _ => "UNKNOWN",
            }
        }
        .to_string();
        self.pending.push(Pending::Addon {
            prefix,
            message,
            distribution,
            guid,
            tries: 0,
        });
    }

    /// Queue a `/random` broadcast (`MSG_RANDOM_ROLL`) for the roller-name resolve.
    pub fn push_roll(&mut self, min: u32, max: u32, roll: u32, guid: u64) {
        self.pending.push(Pending::Roll {
            min,
            max,
            roll,
            guid,
            tries: 0,
        });
    }

    /// Queue an XP award's line (`SMSG_LOG_XPGAIN`); a kill waits on the victim's name.
    pub fn push_xp_gain(&mut self, x: &XpGain) {
        // Both forks park: the template resolves at the drain; `victim: 0` is the unnamed form.
        let named = x.kill && x.victim != 0;
        self.pending.push(Pending::XpGain {
            victim: if named { x.victim } else { 0 },
            total: x.total,
            bonus: if named {
                x.total.saturating_sub(x.base)
            } else {
                0
            },
            tries: 0,
        });
    }

    /// Queue an honor line (`SMSG_PVP_CREDIT`); victim 0 is a bonus or objective award.
    pub fn push_pvp_credit(&mut self, honor: i32, victim: u64, rank: Option<(u8, Option<u8>)>) {
        self.pending.push(Pending::HonorGain {
            victim,
            honor,
            rank,
            tries: 0,
        });
    }

    /// Queue a text emote (`SMSG_TEXT_EMOTE`) for its performer's name, then its sentence.
    pub fn push_text_emote(&mut self, performer: u64, text_id: u32, target: String) {
        self.pending.push(Pending::TextEmote {
            performer,
            text_id,
            target,
            tries: 0,
        });
    }

    /// Queue an area discovery: the `ERR_ZONE_EXPLORED` toast on every packet, never chat, and the
    /// `ERR_ZONE_EXPLORED_XP` system line when `xp > 0`.
    pub fn push_exploration(&mut self, area_name: &str, xp: u32) {
        self.pending.push(Pending::Discovery {
            area: area_name.to_string(),
            xp,
        });
    }

    /// Drop every pending item on disconnect.
    pub fn clear_session(&mut self) {
        self.pending.clear();
    }

    /// The parked addon lines as `(prefix, message, distribution)`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_addons(&self) -> Vec<(String, String, String)> {
        self.pending
            .iter()
            .filter_map(|p| match p {
                Pending::Addon {
                    prefix,
                    message,
                    distribution,
                    ..
                } => Some((prefix.clone(), message.clone(), distribution.clone())),
                _ => None,
            })
            .collect()
    }

    /// How many parked items will render in a chat window; addon lines never do.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_len(&self) -> usize {
        self.pending
            .iter()
            .filter(|p| !matches!(p, Pending::Addon { .. }))
            .count()
    }

    /// The text of every composed line waiting to render, in order.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_lines(&self) -> Vec<String> {
        self.pending
            .iter()
            .filter_map(|p| match p {
                Pending::Event(e) => Some(e.text.clone()),
                _ => None,
            })
            .collect()
    }
}

impl ChatLog {
    /// How many broadcasts are waiting, the drain's early-out.
    pub fn broadcasts_pending(&self) -> usize {
        self.broadcasts.len()
    }

    /// Park a level-up's gains for the `PLAYER_LEVEL_UP` fire.
    pub fn push_level_up_gains(&mut self, info: &LevelUpInfo, talent_points: u32) {
        self.level_up_gains.push((*info, talent_points));
    }

    /// Take the parked gains for `level`; with none (a GM's level write), the caller fires zeros.
    pub fn take_level_up_gains(&mut self, level: u32) -> Option<(LevelUpInfo, u32)> {
        let i = self
            .level_up_gains
            .iter()
            .position(|(l, _)| l.level == level)?;
        Some(self.level_up_gains.remove(i))
    }
}

/// The reference's per-slot state (`+0x9c`), in the values whose notice token differs: the stock
/// `ChatFrame_OnEvent`'s `YOU_LEFT` branch deletes the window's channel registration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SlotState {
    /// `0` (confirmed) and `1` (join sent) in one: the reference's walk sends a LEAVE only from 0,
    /// so an unconfirmed slot here sends one the server answers "Not on channel".
    #[default]
    Joined,
    /// `2`, renamed with its re-join pending (`0x49bcd3`, in the rename `0x49bc50`): the confirming
    /// `YOU_JOINED` carries the `YOU_CHANGED` token, "Changed Channel: [%s]".
    Renamed,
    /// `3`, suspended (`0x49bcf0`): the row stopped applying, in 1.12 data a city-only channel
    /// outside a capital. The record and number survive, and `YOU_LEFT` carries the `SUSPENDED`
    /// token (`0x49c0e0`), which keeps the window's registration.
    Suspended,
}

/// The kind's row in build 12340's default colour table (`0x9fa558`, wrathplus-ui's
/// `chat_types::DEFAULTS`), which the chat bubble
/// and the edit box header tint with; the window's line colour is `ChatTypeInfo`'s.
pub fn default_color(kind: ChatEventKind) -> [u8; 3] {
    use ChatEventKind as K;
    match kind {
        K::Say => [255, 255, 255],
        K::Party => [170, 170, 255],
        K::Raid => [255, 127, 0],
        K::Guild => [64, 255, 64],
        K::Officer => [64, 192, 64],
        K::Yell => [255, 64, 64],
        K::Whisper | K::WhisperInform | K::Afk | K::Dnd => [255, 128, 255],
        K::Emote | K::TextEmote => [255, 128, 64],
        K::System => [255, 255, 0],
        K::MonsterSay => [255, 255, 159],
        K::MonsterYell => [255, 64, 64],
        K::MonsterEmote => [255, 128, 64],
        K::MonsterWhisper => [255, 181, 235],
        K::Channel => [255, 192, 192],
        K::ChannelJoin | K::ChannelLeave | K::ChannelList => [192, 128, 128],
        K::ChannelNotice | K::ChannelNoticeUser => [192, 192, 192],
        K::Ignored => [255, 0, 0],
        K::Filtered => [255, 0, 0],
        K::Skill => [85, 85, 255],
        K::Loot => [0, 170, 0],
        K::Money => [255, 255, 0],
        K::CombatXpGain => [111, 111, 255],
        K::CombatHonorGain => [224, 202, 10],
        K::RaidLeader => [255, 72, 9],
        K::RaidWarning => [255, 72, 0],
        K::RaidBossEmote => [255, 221, 0],
        K::BattlegroundLeader => [255, 219, 183],
        K::Battleground => [255, 127, 0],
        K::BgSystemNeutral => [255, 120, 10],
        K::BgSystemAlliance => [0, 174, 239],
        K::BgSystemHorde => [255, 0, 0],
        K::CombatMiscInfo | K::CombatFactionChange | K::PetInfo => [128, 128, 255],
        K::Tradeskills => [255, 255, 255],
    }
}
