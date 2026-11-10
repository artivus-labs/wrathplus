//! The open windows and NPC interactions the wire opens and the UI reads: a trainer, mailbox,
//! bank, reader, gossip, binder, meeting stone, spirit healer, taxi, trade skill, merchant,
//! social lists and tabard designer; their owners keep the systems and re-export the types.

use std::collections::HashMap;
use std::time::Instant;

use bevy::prelude::*;
use wrathplus_formats::GuildEmblem;
use wrathplus_protocol::types::{
    friend_result, friend_status, FriendEntry, FriendStatusUpdate, GossipOption, MailListEntry,
    NpcTextBlock, TrainerSpell, VendorItem,
};

use crate::query_cache::QueryCache;

/// The open trainer as `SMSG_TRAINER_LIST` delivered it.
#[derive(Resource, Default)]
pub struct TrainerOpen {
    pub trainer: Option<u64>,
    /// In wire order, which is the 1-based display order.
    pub services: Vec<TrainerSpell>,
    pub trainer_type: u32,
    pub greeting: String,
    /// A list landed and the engine's filter, collapse set and selection are not yet reset, as
    /// the reference's builder resets them per list (`0x4d7b42` for the selection); an open window
    /// repaints through [`reeval`], never a second list.
    pub fresh_list: bool,
    /// One of the reference's twelve re-evaluation triggers fired with the window open; the feed
    /// re-derives and fires `TRAINER_UPDATE` even if no state moved, as `0x4d7d40`'s tail does
    /// (`0x4d83f4`), and the stock window re-reads `GetMoney()` on it.
    pub re_derive: bool,
}

impl TrainerOpen {
    /// Open (or replace) the window with a trainer's freshly-listed services.
    pub fn open(
        &mut self,
        trainer: u64,
        trainer_type: u32,
        services: Vec<TrainerSpell>,
        greeting: String,
    ) {
        self.trainer = Some(trainer);
        self.trainer_type = trainer_type;
        self.services = services;
        self.greeting = greeting;
        // A fresh list carries the server's own states, so nothing is pending against it.
        self.fresh_list = true;
        self.re_derive = false;
    }

    /// One of the twelve triggers fired: re-derive on the next feed, if a window is open.
    pub fn trigger_re_derive(&mut self) {
        if self.trainer.is_some() {
            self.re_derive = true;
        }
    }

    /// A client-side close; a re-open re-lists.
    pub fn clear(&mut self) {
        self.trainer = None;
        self.services.clear();
        self.trainer_type = 0;
        self.greeting.clear();
        self.fresh_list = false;
        self.re_derive = false;
    }

    /// Disconnect: drop the open window.
    pub fn clear_session(&mut self) {
        self.clear();
    }
}

/// The `Stationery.dbc` catalog; without it every mail gets the default backdrop.
#[derive(Resource)]
pub struct Stationery(pub wrathplus_formats::StationeryCatalog);

/// `Package.dbc`'s rows, in file order; without them `GetNumPackages()` answers 0.
#[derive(Resource)]
pub struct Packages(pub Vec<wrathplus_formats::PackageRow>);

/// A `SEND` result queued for [`feed_mail`]. `MAIL_FAILED` fires on every one, success included:
/// the reference uses it as "the send resolved" (`0x4ad15f`), and the stock handler only
/// re-enables the button.
pub struct MailSendAck {
    pub ok: bool,
    /// The message key of a failure other than an equip error.
    pub refusal: Option<&'static str>,
}

/// The open mailbox session: the guid, the inbox rows as the wire sent them, the letter-body cache
/// and the result queues. Cleared on close and on disconnect.
#[derive(Resource, Default)]
pub struct MailOpen {
    pub mailbox: Option<u64>,
    /// Wire order, which is the 1-based display order.
    pub mails: Vec<MailListEntry>,
    /// Native inbox retained without projecting twelve attachments into one Vanilla slot.
    pub wrath_inbox: Option<wrathplus_protocol::wrath_mail::Inbox>,
    /// Letter bodies by `item_text_id`, asked once.
    pub bodies: QueryCache<u32, String>,
    /// `MAIL_SHOW` owed: every mailbox use re-shows the window (`0x4acd10`).
    pub show_requested: bool,
    /// When the last `CMSG_GET_MAIL_LIST` went out, in `Time::elapsed_secs_f64`.
    pub last_list_query: Option<f64>,
    pub send_acks: Vec<MailSendAck>,
    /// 1-based rows a take emptied and purged, each owed a `CLOSE_INBOX_ITEM`.
    pub close_inbox: Vec<u32>,
    /// Take, return and delete refusals, as message keys.
    pub errors: Vec<&'static str>,
}

impl MailOpen {
    /// A mailbox right-click. A different mailbox resets the rows and the throttle; the same one
    /// only re-shows, as the reference fires `MAIL_SHOW` on every use.
    pub fn click(&mut self, mailbox: u64) {
        if self.mailbox != Some(mailbox) {
            self.mailbox = Some(mailbox);
            self.mails.clear();
            self.wrath_inbox = None;
            self.last_list_query = None;
        }
        self.show_requested = true;
    }

    /// A client-side close, sending nothing as 1.12 does; a re-open re-lists.
    pub fn clear(&mut self) {
        self.mailbox = None;
        self.mails.clear();
        self.wrath_inbox = None;
        self.bodies.clear();
        self.show_requested = false;
        self.last_list_query = None;
        self.send_acks.clear();
        self.errors.clear();
    }

    pub fn clear_session(&mut self) {
        self.clear();
    }
}

/// The vault the player opened, set by the object click and cleared by the frame's close.
#[derive(Resource, Default)]
pub struct GuildBankOpen {
    pub banker: Option<u64>,
    /// The click is waiting for this feed to send the activate.
    pub pending: bool,
}

impl GuildBankOpen {
    /// The object click on a guild vault (`GAMEOBJECT_TYPE_GUILD_BANK`).
    pub fn open(&mut self, banker: u64) {
        self.banker = Some(banker);
        self.pending = true;
    }

    /// The vault in use, for the bank writers that name it.
    pub fn banker(&self) -> Option<u64> {
        self.banker
    }
}

/// Build 12340's letter texts, by item GUID: `CMSG_ITEM_TEXT_QUERY` names the item and
/// `SMSG_ITEM_TEXT_QUERY_RESPONSE` answers with it (AC `HandleItemTextQuery`); a `None` answer is
/// an item the server has no text for.
#[derive(Resource, Default)]
pub struct LetterTexts {
    pub texts: QueryCache<u64, String>,
}

impl LetterTexts {
    /// Record an answer (`wrath_inventory_capture`).
    pub fn insert(&mut self, item_guid: u64, text: Option<String>) {
        self.texts.insert(item_guid, text);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn text(&mut self, item_guid: u64) -> Option<String> {
        self.texts.get_or_ask(item_guid, || {}).cloned()
    }
}

/// Which of the reference's two text sources a session shows; the object decides, not the click.
pub enum ReadSource {
    /// A mail-made letter: one body, asked by the letter's GUID, and a creator line.
    Letter,
    /// A page chain. `visited` runs from the first page to the one shown, as pages link only
    /// forwards (the reference's array at `0xbc3fd0`); it is empty until the object's template
    /// answers, since the reference asks for the page id at paint time (`0x5f59d0`).
    Pages { visited: Vec<u32> },
}

pub struct ReadSession {
    /// An item's or a GameObject's guid; reading the same object again closes the reader.
    pub object_guid: u64,
    pub source: ReadSource,
    /// Keyed on the VM, so a `/reload` re-fires `ITEM_TEXT_BEGIN` for a reader still open.
    pub told: crate::app_core::vm_memo::VmMemo<ItemTextTold>,
}

#[derive(Default)]
pub struct ItemTextTold {
    /// `ITEM_TEXT_BEGIN` fired: once per open, before the text lands.
    pub begun: bool,
    pub ready: bool,
}

#[derive(Resource, Default)]
pub struct ItemTextOpen {
    pub pending: Option<ReadSession>,
    /// A toggle closed the reader and the frame is not told yet; the feed holds the script.
    pub(crate) closing: bool,
}

impl ItemTextOpen {
    /// Open a bag letter; re-opening restarts from `ITEM_TEXT_BEGIN`.
    pub fn open_letter(&mut self, item_guid: u64) {
        self.open(item_guid, ReadSource::Letter);
    }

    /// Open a page chain; the feed asks the object's template for the first page.
    pub fn open_pages(&mut self, object_guid: u64) {
        self.open(
            object_guid,
            ReadSource::Pages {
                visited: Vec::new(),
            },
        );
    }

    /// The reference's toggle, `0x4e32e0` with flag 0 from both click routes (`0x5d8e5e`,
    /// `0x5f58e7`): reading the open object again closes it. Returns whether it closed.
    pub fn toggle_closed(&mut self, object_guid: u64) -> bool {
        let open = self
            .pending
            .as_ref()
            .is_some_and(|s| s.object_guid == object_guid);
        if open {
            self.pending = None;
            self.closing = true;
        }
        open
    }

    pub fn take_closing(&mut self) -> bool {
        std::mem::take(&mut self.closing)
    }

    pub(crate) fn open(&mut self, object_guid: u64, source: ReadSource) {
        self.pending = Some(ReadSession {
            object_guid,
            source,
            told: Default::default(),
        });
    }
}

/// `PageTextMaterial.dbc`; absent without client data, leaving every reader on Parchment.
#[derive(Resource)]
pub struct PageMaterials(pub wrathplus_formats::PageTextMaterialCatalog);

/// The open gossip menu, plus the NPC-text records by id, which outlive it. The cache holds whole
/// records: the reference draws the greeting afresh at each open, from the NPC's gender and a roll.
#[derive(Resource, Default)]
pub struct GossipState {
    pub npc: Option<u64>,
    pub text_id: u32,
    /// This open's drawn greeting; `None` while its text query is out.
    pub greeting: Option<String>,
    pub options: Vec<GossipOption>,
    /// The packet's quest rows, `(quest_id, dialog-status icon, level, title)`. The level is kept
    /// because `GetGossipAvailableQuests` and `GetGossipActiveQuests` return `(title, level)`
    /// pairs, walked in twos by stock `GossipFrame.lua` (lines 66 and 90).
    pub quests: Vec<(u32, u32, i32, String)>,
    pub quest_info: Vec<wrathplus_protocol::types::WrathQuestInfo>,
    pub records: HashMap<u32, Vec<NpcTextBlock>>,
}

impl GossipState {
    /// Close the menu; the records stay.
    pub fn clear(&mut self) {
        self.npc = None;
        self.text_id = 0;
        self.greeting = None;
        self.options.clear();
        self.quests.clear();
        self.quest_info.clear();
    }

    /// The disconnect clear; NPC text is static per id, so the records stay.
    pub fn clear_session(&mut self) {
        self.clear();
    }
}

impl GossipState {
    pub fn cached_record(&self, text_id: u32) -> Option<&[NpcTextBlock]> {
        self.records.get(&text_id).map(Vec::as_slice)
    }

    pub fn remember_record(&mut self, text_id: u32, blocks: Vec<NpcTextBlock>) {
        self.records.insert(text_id, blocks);
    }

    /// The reference's text-pending latch (`[0xbbb670]`): a menu whose text query is out, or was
    /// never sent (`text_id` 0). While it holds, nothing fires and every select is refused.
    pub fn text_pending(&self) -> bool {
        self.npc.is_some() && self.greeting.is_none()
    }
}

/// The innkeeper's pending bind question; everything the dialog reads rides the event's argument.
#[derive(Resource, Default)]
pub struct BinderState {
    /// The innkeeper who asked, sent back on the wire; vmangos binds only a live one in range.
    pub npc: Option<u64>,
    /// A dialog the feed still owes, set per packet: asking again after a decline is a second
    /// dialog, which a state diff would swallow.
    pub ask: bool,
}

impl BinderState {
    /// `SMSG_BINDER_CONFIRM`: parks the innkeeper's guid and owes the UI a dialog.
    pub fn ask(&mut self, npc: u64) {
        self.npc = Some(npc);
        self.ask = true;
    }

    /// The guid to answer with, if a question is live.
    pub fn pending(&self) -> Option<u64> {
        self.npc
    }

    /// Retracts the question: the range guard's close, and `SMSG_PLAYERBOUND`'s.
    pub fn clear(&mut self) {
        self.npc = None;
        self.ask = false;
    }
}

/// The current-area spirit healer (`[0xb4e330/334]`) and its wave clock (`[0xb4e338]`).
#[derive(Resource, Default)]
pub struct AreaSpiritHealer {
    /// Written only by [`Self::set_healer`], the reference's `0x4921c0`.
    pub healer: Option<u64>,
    pub(crate) deadline: Option<Instant>,
    pub in_range: bool,
}

impl AreaSpiritHealer {
    /// `SMSG_AREA_SPIRIT_HEALER_TIME`: arms the clock only for the cached healer (`0x4922a0`) and
    /// a positive time, and owes `AREA_SPIRIT_HEALER_IN_RANGE`.
    pub fn on_time(&mut self, healer: u64, ms: u32, now: Instant) {
        if self.healer == Some(healer) && ms > 0 {
            self.deadline = Some(now + std::time::Duration::from_millis(u64::from(ms)));
            self.in_range = true;
        }
    }

    /// `0x4921c0`: an unchanged guid returns before anything else (`0x4921cf`/`0x4921dd`), so the
    /// poll calls it every frame; a change cancels a pending wave (`0x4921fc`), zeroes the
    /// deadline (`0x492211`) and queries a non-zero healer (`0x492217`).
    pub fn set_healer(&mut self, new: Option<u64>) -> SetHealerOutcome {
        // The reference's zero guid is our `None`.
        let new = new.filter(|&g| g != 0);
        if self.healer == new {
            return SetHealerOutcome::default();
        }
        self.healer = new;
        let cancel_aura = self.deadline.take().is_some();
        if cancel_aura {
            // The cancel closes the dialog, so an `_IN_RANGE` still owed is stale.
            self.in_range = false;
        }
        SetHealerOutcome {
            cancel_aura,
            query: new,
        }
    }

    /// The spirit-guide click (`0x5df950`): `0x4921c0(0)` then `0x4921c0(guid)`, a cache-bust that
    /// makes the second call query even for the healer already cached.
    pub fn click_guide(&mut self, guid: u64) -> SetHealerOutcome {
        let bust = self.set_healer(None);
        let set = self.set_healer(Some(guid));
        SetHealerOutcome {
            cancel_aura: bust.cancel_aura || set.cancel_aura,
            query: set.query,
        }
    }

    /// The cached healer: `AcceptAreaSpiritHeal`'s guid and the poll's retain subject.
    pub fn healer(&self) -> Option<u64> {
        self.healer
    }

    pub fn secs(&self, now: Instant) -> u32 {
        self.deadline
            .map(|d| d.saturating_duration_since(now).as_secs())
            .map_or(0, |s| u32::try_from(s).unwrap_or(u32::MAX))
    }
}

/// A right-click on a `GAMEOBJECT_TYPE_MEETINGSTONE` (23) that passed the shared click gates.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeetingStoneUse {
    pub go_guid: u64,
}

/// A flight master's `SMSG_TAXINODE_STATUS`: `known = false` shows the green `TalkToMeGreen`
/// overhead icon (`0x5ecdd0` to `0x607480`, resource table `0xc4d9d8` index 4). The query and the
/// teardown belong to [`crate::quest_markers::query`]: the reference's only sender, `0x5eb170`, is
/// called only from `0x607380` (at `0x6073e8`), the per-unit function that also sends the
/// questgiver query, and both icons share one marker slot (`unit+0xb2c`).
#[derive(Component, Clone, Copy)]
pub struct FlightMasterStatus {
    pub known: bool,
}

/// Opener spell ids (effect 47) that `send_spell_cast` intercepted, opened by
/// [`open_trade_skill`] the same frame.
#[derive(Resource, Default)]
pub struct TradeSkillOpens(pub Vec<u32>);

/// The open vendor and its `SMSG_LIST_INVENTORY` rows, until a close or a disconnect.
#[derive(Resource, Default)]
pub struct MerchantOpen {
    pub vendor: Option<u64>,
    /// In wire order, which is the 1-based display order.
    pub items: Vec<VendorItem>,
}

impl MerchantOpen {
    pub fn open(&mut self, vendor: u64, items: Vec<VendorItem>) {
        self.vendor = Some(vendor);
        self.items = items;
    }

    pub fn is_open(&self) -> bool {
        self.vendor.is_some()
    }

    /// The item entry at a 1-based row: `CMSG_BUY_ITEM` buys by entry, not by vendor slot.
    pub fn entry_at(&self, index_1based: u32) -> Option<u32> {
        index_1based
            .checked_sub(1)
            .and_then(|i| self.items.get(i as usize))
            .map(|it| it.entry)
    }

    /// `SMSG_BUY_ITEM`'s new count for the row at 1-based vendor `slot`; the bought item itself
    /// arrives as an object create.
    pub fn update_stock(&mut self, slot: u32, new_count: u32) {
        if let Some(it) = self.items.iter_mut().find(|it| it.slot == slot) {
            it.current_count = new_count;
        }
    }

    /// Zero every row of `entry`, as the reference's `ITEM_ALREADY_SOLD` arm writes 0 (`0x5dcdbf`)
    /// into each matching row of its 128-row cache (`0x5dcdcd`).
    pub fn sold_out(&mut self, entry: u32) {
        for it in self.items.iter_mut().filter(|it| it.entry == entry) {
            it.current_count = 0;
        }
    }

    /// A client-side close; a re-open re-lists.
    pub fn clear(&mut self) {
        self.vendor = None;
        self.items.clear();
    }

    /// The disconnect clear.
    pub fn clear_session(&mut self) {
        self.clear();
    }
}

/// The social session mirror: filled by the social handlers, read by the feed, cleared on
/// disconnect.
#[derive(Resource, Default)]
pub struct SocialState {
    /// The friend list in wire order (vmangos walks a guid-keyed map); the feed sorts for display.
    pub friends: Vec<FriendEntry>,
    /// Build 12340's contact notes by friend guid (`GetFriendInfo`'s seventh return).
    pub notes: std::collections::HashMap<u64, String>,
    pub ignores: Vec<u64>,
    /// The selected friend as a guid, like the reference's `+0x648`; 0 is none.
    pub selected_friend: u64,
    /// The selected ignore as a guid (the reference's `+0x720`).
    pub selected_ignore: u64,
    /// The friend guids in the order the feed last showed them, for Lua's row indices.
    pub display_order: Vec<u64>,
    pub ignore_display_order: Vec<u64>,
    /// The last `/who` answer; `who_total` is the server's full match count.
    pub who: Vec<wrathplus_protocol::types::WhoEntry>,
    pub who_total: u32,
    /// Native routing compares the uncapped first wire count (006b8720).
    pub native_who_count: Option<usize>,
    pub pending_native_who:
        std::collections::VecDeque<(u32, Vec<wrathplus_protocol::wrath_who::Entry>)>,
    /// Native rows retained losslessly; shared display rows intentionally omit gender.
    pub native_who_rows: Vec<wrathplus_protocol::wrath_who::Entry>,
    /// The `/who` sort chain (`SortWho`'s seven `{key, dir}` slots at `0xc2817c`), pushed with the
    /// rows so the binding can re-sort synchronously.
    pub who_sort: wrathplus_ui::script::WhoSortChain,
    /// `SetWhoToUI`, which the Who frame sets on show and clears on hide.
    pub who_to_ui: bool,
    /// Results whose line waits on a name query (the reference formats from the name cache).
    pub pending_lines: Vec<FriendStatusUpdate>,
    /// Set when the friend list changed, so the feed fires its list event.
    pub friends_dirty: bool,
    pub ignores_dirty: bool,
    pub who_dirty: bool,
}

impl SocialState {
    /// Drop the session's state: the server re-sends both lists at login, and a stale ignore list
    /// would silence the wrong guids. The `/who` sort chain survives, since the reference
    /// initialises it once per process (`0x5adc50`, from `0x401666`), never at login.
    pub fn clear_session(&mut self) {
        *self = Self {
            who_sort: std::mem::take(&mut self.who_sort),
            ..Self::default()
        };
    }

    /// Is `guid` on the ignore list? The reference's `IsIgnored` (`0x5ae5a0`): an ignored player's
    /// chat and text emotes drop silently (`0x49dbe0`), and their duel is declined (`0x4d4a33`).
    pub fn is_ignored(&self, guid: u64) -> bool {
        guid != 0 && self.ignores.contains(&guid)
    }

    /// `SetFriendNotes(index | name, notes)`' friend: the 1-based row as last shown, else the
    /// friend whose known name (`name_of`, the name cache's peek) matches, case-insensitively.
    pub fn friend_guid<'n>(
        &self,
        index: Option<u32>,
        name: Option<&str>,
        name_of: impl Fn(u64) -> Option<&'n str>,
    ) -> Option<u64> {
        if let Some(i) = index {
            return (i as usize)
                .checked_sub(1)
                .and_then(|i| self.display_order.get(i).copied());
        }
        let name = name?;
        self.friends
            .iter()
            .map(|f| f.guid)
            .find(|&g| name_of(g).is_some_and(|n| n.eq_ignore_ascii_case(name)))
    }

    /// Is `guid` on the friend list? The reference's `FindFriendSlot` (`0x5ae810`), which keeps the
    /// guild sign-on line from announcing a friend twice.
    pub fn is_friend(&self, guid: u64) -> bool {
        guid != 0 && self.friends.iter().any(|f| f.guid == guid)
    }

    /// `SMSG_FRIEND_LIST`: the whole list, never a delta.
    pub fn apply_friend_list(&mut self, friends: Vec<FriendEntry>) {
        self.friends = friends;
        self.friends_dirty = true;
    }

    /// Seat the ignore list directly, for tests of the code that consults it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_ignores_for_test(&mut self, guids: Vec<u64>) {
        self.apply_ignore_list(guids);
    }

    /// `SMSG_IGNORE_LIST`: the whole list.
    pub fn apply_ignore_list(&mut self, guids: Vec<u64>) {
        self.ignores = guids;
        self.ignores_dirty = true;
    }

    /// `SMSG_FRIEND_STATUS`: apply the result and queue its line. The server sends no fresh list
    /// after an add or a remove, so the result codes maintain it, and a presence broadcast patches
    /// the row in place.
    pub fn apply_friend_status(&mut self, update: FriendStatusUpdate) {
        match update.result {
            friend_result::ADDED_ONLINE | friend_result::ADDED_OFFLINE => {
                if !self.friends.iter().any(|f| f.guid == update.guid) {
                    let online = update.online;
                    self.friends.push(FriendEntry {
                        guid: update.guid,
                        status: online.map_or(friend_status::OFFLINE, |o| o.status),
                        area: online.map_or(0, |o| o.area),
                        level: online.map_or(0, |o| o.level),
                        class: online.map_or(0, |o| o.class),
                    });
                }
                self.friends_dirty = true;
            }
            friend_result::REMOVED => {
                self.friends.retain(|f| f.guid != update.guid);
                self.notes.remove(&update.guid);
                if self.selected_friend == update.guid {
                    self.selected_friend = 0;
                }
                self.friends_dirty = true;
            }
            friend_result::ONLINE => {
                if let (Some(entry), Some(online)) = (self.friend_mut(update.guid), update.online) {
                    entry.status = online.status;
                    entry.area = online.area;
                    entry.level = online.level;
                    entry.class = online.class;
                }
                self.friends_dirty = true;
            }
            friend_result::OFFLINE => {
                if let Some(entry) = self.friend_mut(update.guid) {
                    entry.status = friend_status::OFFLINE;
                    // An offline friend carries no area, level or class on the wire.
                    entry.area = 0;
                    entry.level = 0;
                    entry.class = 0;
                }
                self.friends_dirty = true;
            }
            friend_result::IGNORE_ADDED => {
                if !self.ignores.contains(&update.guid) {
                    self.ignores.push(update.guid);
                }
                self.ignores_dirty = true;
            }
            friend_result::IGNORE_REMOVED => {
                self.ignores.retain(|g| *g != update.guid);
                if self.selected_ignore == update.guid {
                    self.selected_ignore = 0;
                }
                self.ignores_dirty = true;
            }
            // Every other code is a refusal: a line, no change.
            _ => {}
        }
        self.pending_lines.push(update);
    }

    pub(crate) fn friend_mut(&mut self, guid: u64) -> Option<&mut FriendEntry> {
        self.friends.iter_mut().find(|f| f.guid == guid)
    }

    /// `SMSG_WHO`: the answer to the last query.
    pub fn apply_who(&mut self, results: wrathplus_protocol::types::WhoResults) {
        self.native_who_rows.clear();
        self.native_who_count = None;
        self.who = results.entries;
        self.who_total = results.total;
        self.who_dirty = true;
    }
}

/// The design our own body wears while the frame is open.
#[derive(Resource, Default)]
pub struct TabardDesign(pub Option<[i32; 5]>);

impl TabardDesign {
    pub fn preview(&self) -> Option<GuildEmblem> {
        self.0.map(|d| GuildEmblem {
            emblem_style: d[0],
            emblem_color: d[1],
            border_style: d[2],
            border_color: d[3],
            background_color: d[4],
        })
    }
}

/// `UNIT_NPC_FLAGS` bit 6, `SPIRITGUIDE`, which the acquire callback `0x4924c0` tests (`0x4924fc`).
/// The area spirit healer's aura, cancelled by `0x4921c0` and `CancelAreaSpiritHeal`; `0x6e7040`
/// fires `AREA_SPIRIT_HEALER_OUT_OF_RANGE` for this spell id alone (`0x6e70b6`).
pub const AREA_SPIRIT_HEALER_AURA: u32 = 2584;

/// What [`AreaSpiritHealer::set_healer`] owes the frame; the reference does both in `0x4921c0`.
#[derive(Default, PartialEq, Eq, Debug)]
pub struct SetHealerOutcome {
    /// A wave was pending and the healer changed (`0x4921fc`): `AREA_SPIRIT_HEALER_OUT_OF_RANGE`
    /// fires and `CMSG_CANCEL_AURA` goes out for 2584; this, not Lua, closes the dialog.
    pub cancel_aura: bool,
    /// A newly adopted non-zero healer: `CMSG_AREA_SPIRIT_HEALER_QUERY` with it.
    pub query: Option<u64>,
}
