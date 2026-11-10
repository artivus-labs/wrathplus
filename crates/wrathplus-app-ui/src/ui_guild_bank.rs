//! The guild bank from native traffic: the vault opens with `CMSG_GUILD_BANKER_ACTIVATE` (the
//! object click, [`GuildBankOpen`]), and `SMSG_GUILD_BANK_LIST` (money, tab names and icons on a
//! full update, slot contents), `MSG_GUILD_PERMISSIONS`, `MSG_GUILD_BANK_MONEY_WITHDRAWN`, the logs
//! and the tab texts fill [`GuildBank`], published as Native UI's `WrathGuildBankView` with the
//! frame's events fired here: `GUILDBANKFRAME_OPENED`/`CLOSED`, `GUILDBANKBAGSLOTS_CHANGED`,
//! `GUILDBANK_UPDATE_MONEY`, `GUILDBANK_UPDATE_TABS`, `GUILDBANK_UPDATE_WITHDRAWMONEY`,
//! `GUILDBANK_UPDATE_TEXT(tab)` and `GUILDBANKLOG_UPDATE`, plus `GUILDBANK_TEXT_CHANGED(tab)` off
//! `SMSG_GUILD_EVENT`. The view is published whether or not the vault is open, as 12340 keeps the
//! bank state in globals the merchant and guild-control panes read. The frame's gestures become the
//! native queries and swaps; a held bank item (Native UI's mode-11 cursor) moves to another bank
//! slot, a bag slot or a worn slot. Labelled: the frame does not close by distance.
use std::collections::BTreeMap;

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::names::NameCache;
use crate::net::{ClientCommand, NetCommands, NetHandlerApp};
use bevy::prelude::*;
use wrathplus_protocol::wrath_guild_bank::{Contents, Item, Packet, TabInfo};
use wrathplus_protocol::wrath_guild_bank_log::{Log, Payload};
use wrathplus_protocol::wrath_guild_bank_swap::{BankSlot, Swap, Transfer};
use wrathplus_protocol::{SessionEvent, SessionEventKind};
use wrathplus_ui::script::{
    UiScript, WrathGuildBankItemLog, WrathGuildBankMoneyLog, WrathGuildBankRankRights,
    WrathGuildBankRequest, WrathGuildBankSlot, WrathGuildBankTab, WrathGuildBankView,
};

/// `GUILD_BANK_MAX_TABS`; a log for tab 6 is the money log (AC `GuildBankLog`).
const TABS: usize = 6;
const MONEY_LOG_TAB: u8 = 6;

pub(crate) use crate::app_core::interaction::GuildBankOpen;

#[derive(Resource, Default)]
pub(crate) struct GuildBank {
    /// A list has answered the activate: the frame is open.
    open: bool,
    money: u64,
    remaining_withdrawals: i32,
    tabs: Vec<TabInfo>,
    /// The purchased tab count (`0xc1dc0c`): written by `MSG_GUILD_PERMISSIONS` (`0x5cba74`) and a
    /// full tab-0 list (`0x5a5720`), cleared only at a session start (`0x5a7820`).
    tab_count: u8,
    slots: [BTreeMap<u8, (u32, Item)>; TABS],
    money_remaining: Option<i32>,
    logs: BTreeMap<u8, Log>,
    texts: BTreeMap<u8, String>,
    /// `GE_BANK_TEXT_CHANGED` tabs, 0-based, each one `GUILDBANK_TEXT_CHANGED` at the next feed.
    text_changed: Vec<u8>,
}

impl GuildBank {
    /// `BuyGuildBankTab`'s reads: the frame is open, the purchased tab count, the vault's money.
    pub(crate) fn tab_purchase(&self) -> (bool, usize, u64) {
        (self.open, usize::from(self.tab_count), self.money)
    }
}

pub struct UiGuildBankPlugin;
impl Plugin for UiGuildBankPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GuildBank>()
            .init_resource::<GuildBankOpen>()
            .net_handler(SessionEventKind::WrathGuildBank, receive)
            .net_handler(SessionEventKind::GuildEvent, text_changed)
            .net_handler(SessionEventKind::Connected, clear)
            .net_handler(SessionEventKind::Disconnected, clear)
            .add_systems(
                Update,
                feed_guild_bank
                    .in_set(crate::app_core::schedule::UiFeed)
                    .run_if(crate::app_core::ui_state::ingame_ui_up),
            );
    }
}

fn receive(
    In(event): In<SessionEvent>,
    mut bank: ResMut<GuildBank>,
    mut native: ResMut<crate::ui_guild::NativeGuild>,
    open: Res<GuildBankOpen>,
) {
    if let SessionEvent::WrathGuildBank { packet } = event {
        if let Packet::Permissions(p) = &packet {
            native.apply_permissions(p);
        }
        apply(&mut bank, packet, open.banker.is_some());
    }
}

fn apply(bank: &mut GuildBank, packet: Packet, opened: bool) {
    match packet {
        Packet::Contents(Contents {
            money,
            tab,
            remaining_withdrawals,
            full_update,
            tab_info,
            slots,
        }) => {
            bank.open |= opened;
            bank.money = money;
            bank.remaining_withdrawals = remaining_withdrawals;
            if let Some(tabs) = tab_info {
                bank.tab_count = tabs.len().min(TABS) as u8;
                bank.tabs = tabs;
            }
            if let Some(contents) = bank.slots.get_mut(usize::from(tab)) {
                // A full update lists every occupied slot; otherwise only the changed ones.
                if full_update != 0 {
                    contents.clear();
                }
                for slot in slots {
                    match slot.item {
                        Some(item) => {
                            contents.insert(slot.slot, (slot.item_id, item));
                        }
                        None => {
                            contents.remove(&slot.slot);
                        }
                    }
                }
            }
        }
        Packet::Permissions(p) => bank.tab_count = p.purchased_tabs,
        Packet::RemainingMoney { remaining } => bank.money_remaining = Some(remaining),
        Packet::Log(log) => {
            bank.logs.insert(log.tab, log);
        }
        Packet::Text(text) => {
            bank.texts
                .insert(text.tab, String::from_utf8_lossy(&text.text).into_owned());
        }
    }
}

/// `SMSG_GUILD_EVENT` case `0x13` (`0x6d92d0`): another member edited a tab's text; the frame
/// re-queries it when that tab is showing (`Blizzard_GuildBankUI.lua:151-155`). Fired whether or
/// not the vault is open, as the reference does. Labelled: the cached text stays until the new
/// one lands; the reference's `0x5a4310(tab, 1)` beside the fire is untraced.
fn text_changed(In(event): In<SessionEvent>, mut bank: ResMut<GuildBank>) {
    use wrathplus_protocol::types::guild_event::BANK_TEXT_CHANGED;
    if let SessionEvent::GuildEvent(notice) = event {
        // `atol` of the first string, as the reference reads it.
        let tab = notice
            .params
            .first()
            .and_then(|p| p.trim().parse::<u8>().ok());
        if let (BANK_TEXT_CHANGED, Some(tab)) = (notice.event, tab) {
            bank.text_changed.push(tab);
        }
    }
}

fn clear(
    In(event): In<SessionEvent>,
    mut bank: ResMut<GuildBank>,
    mut open: ResMut<GuildBankOpen>,
) {
    if matches!(
        event,
        SessionEvent::Connected { .. } | SessionEvent::Disconnected { .. }
    ) {
        *bank = GuildBank::default();
        *open = GuildBankOpen::default();
    }
}

/// A 1-based Lua tab or slot, 0-based.
fn zero_based(n: u8) -> Option<u8> {
    n.checked_sub(1)
}

/// One frame gesture as the packet it sends; `held` is the bag item on the cursor, as its wire
/// container and slot.
fn command(
    bank: &mut GuildBank,
    open: &mut GuildBankOpen,
    request: WrathGuildBankRequest,
    held: Option<(u8, u8, i32)>,
) -> Option<ClientCommand> {
    use wrathplus_protocol::wrath_guild_bank_query::Query;
    use wrathplus_protocol::wrath_guild_bank_text::{Command as TextCommand, Text};
    use WrathGuildBankRequest as R;
    let banker = open.banker?;
    let slot_of = |tab: u8, slot: u8| -> Option<BankSlot> {
        let (tab, slot) = (zero_based(tab)?, zero_based(slot)?);
        let item = bank
            .slots
            .get(usize::from(tab))?
            .get(&slot)
            .map_or(0, |(id, _)| *id);
        Some(BankSlot { tab, slot, item })
    };
    Some(match request {
        R::QueryTab { tab } => ClientCommand::GuildBankQuery(Query::Tab {
            banker,
            tab: zero_based(tab)?,
            full_update: 1,
        }),
        R::QueryText { tab } => ClientCommand::NativeGuildBankText(TextCommand::QueryText {
            tab: zero_based(tab)?,
        }),
        // Tab 7 asks for the money log, the server's tab 6.
        R::QueryLog { tab } => ClientCommand::NativeGuildBankText(TextCommand::QueryLog {
            tab: zero_based(tab)?,
        }),
        R::SetText { tab, text } => {
            ClientCommand::NativeGuildBankText(TextCommand::SetText(Text {
                tab: zero_based(tab)?,
                text: text.into_bytes(),
            }))
        }
        R::SetTabInfo { tab, name, icon } => {
            ClientCommand::NativeGuildBankTab(wrathplus_protocol::wrath_guild_bank_tab::UpdateTab {
                banker,
                tab: zero_based(tab)?,
                name: name.into_bytes(),
                icon: icon.into_bytes(),
            })
        }
        R::AutoStoreItem { tab, slot } => {
            let source = slot_of(tab, slot)?;
            let count = bank.slots[usize::from(source.tab)]
                .get(&source.slot)
                .map_or(0, |(_, item)| item.count as i32);
            ClientCommand::NativeGuildBankSwap(Swap {
                banker,
                transfer: Transfer::AutoStore { source, count },
            })
        }
        // A bag item on the cursor goes into the slot; picking a bank item up awaits the cursor.
        R::PickupItem { tab, slot } => {
            let (container, container_slot, stack_count) = held?;
            ClientCommand::NativeGuildBankSwap(Swap {
                banker,
                transfer: Transfer::InventoryToBank {
                    destination: slot_of(tab, slot)?,
                    container,
                    container_slot,
                    stack_count,
                },
            })
        }
        // A held bank item: another bank slot (`0x5a5410`), a bag slot, or a worn slot. The
        // cursor already gave it up; count `None` moves the stack (AC treats 0 as no split).
        R::MoveItem {
            from_tab,
            from_slot,
            to_tab,
            to_slot,
            count,
        } => ClientCommand::NativeGuildBankSwap(Swap {
            banker,
            transfer: Transfer::BankToBank {
                destination: slot_of(to_tab, to_slot)?,
                source: slot_of(from_tab, from_slot)?,
                count: count.unwrap_or(0) as i32,
            },
        }),
        R::DropOnBag {
            tab,
            slot,
            bag,
            bag_slot,
            count,
        } => {
            let (container, container_slot) = crate::ui_items::wire_pos_for(bag, bag_slot)?;
            ClientCommand::NativeGuildBankSwap(Swap {
                banker,
                transfer: Transfer::BankToInventory {
                    source: slot_of(tab, slot)?,
                    container,
                    container_slot,
                    stack_count: count.unwrap_or(0) as i32,
                },
            })
        }
        R::DropOnEquip {
            tab,
            slot,
            equip_slot,
            count,
        } => ClientCommand::NativeGuildBankSwap(Swap {
            banker,
            transfer: Transfer::BankToInventory {
                source: slot_of(tab, slot)?,
                container: 255,
                container_slot: u8::try_from(equip_slot.checked_sub(1)?).ok()?,
                stack_count: count.unwrap_or(0) as i32,
            },
        }),
        // `CloseGuildBankFrame` (`0x5a4500` -> `0x5a4270`) clears the bank guid and nothing else.
        R::Close => {
            bank.open = false;
            *open = GuildBankOpen::default();
            return None;
        }
        R::PickupMoney { .. } | R::SummonFriend { .. } => return None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn feed_guild_bank(
    script: Option<NonSendMut<UiScript>>,
    mut bank: ResMut<GuildBank>,
    mut open: ResMut<GuildBankOpen>,
    native_guild: Res<crate::ui_guild::NativeGuild>,
    names: Res<NameCache>,
    items: Res<crate::items::Items>,
    icons: Option<Res<crate::entities::ItemDisplays>>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    commands: Res<NetCommands>,
    mut last: Local<crate::app_core::vm_memo::VmMemo<Option<WrathGuildBankView>>>,
) {
    let Some(mut script) = script else {
        return;
    };
    events::fire_text_changed(&mut bank, &mut script);
    if open.pending {
        open.pending = false;
        if let Some(banker) = open.banker {
            let _ = commands.0.send(ClientCommand::NativeGuildBank(
                wrathplus_protocol::wrath_guild_bank_commands::Command::Activate {
                    banker,
                    full_update: 1,
                },
            ));
        }
    }
    for request in script.take_wrath_guild_bank_requests() {
        let held = match request {
            WrathGuildBankRequest::PickupItem { .. } => script.cursor_item().and_then(|item| {
                let (container, slot) = crate::ui_items::wire_pos_for(item.bag, item.slot)?;
                Some((container, slot, item.count.map_or(0, |n| n as i32)))
            }),
            _ => None,
        };
        let had_item = held.is_some();
        if let Some(command) = command(&mut bank, &mut open, request, held) {
            if had_item {
                script.clear_cursor_payload();
            }
            let _ = commands.0.send(command);
        }
    }
    for (tab, bag, slot) in script.take_guild_bank_deposits() {
        if let Some(command) = crate::ui_items::guild_bank_deposit(open.banker, tab, bag, slot) {
            let _ = commands.0.send(command);
        }
    }
    let level = self_q
        .iter()
        .next()
        .and_then(|s| s.0.unit_level())
        .unwrap_or(0);
    let link = |entry: u32, item: Option<&Item>| -> (Option<String>, Option<String>) {
        let Some(t) = items.template(entry, 0, &commands) else {
            return (None, None);
        };
        let gems = |i: usize| {
            item.and_then(|item| item.sockets.get(i))
                .map_or(0, |&(_, enchant)| enchant)
        };
        let link = crate::items::wrath_item_link(
            entry,
            &t.name,
            t.quality,
            [item.map_or(0, |i| i.enchantment), gems(0), gems(1), gems(2)],
            item.map_or(0, |i| i.random_property as i32),
            item.and_then(|i| i.random_property_seed).unwrap_or(0),
            level,
        );
        (
            crate::ui_items::item_icon(icons.as_deref(), t.display_info_id),
            Some(link),
        )
    };
    let name = |guid: u64| names.resolve(guid, &commands).map(str::to_string);
    let view = view(&bank, &native_guild, &link, &name);
    let (last, reset) = last.get_reset(&script);
    if !reset && last.as_ref() == Some(&view) {
        return;
    }
    let before = if reset { None } else { last.clone() };
    *last = Some(view.clone());
    script.set_wrath_guild_bank(view.clone());
    for (event, args) in events(before.as_ref(), &view) {
        script.queue_event(event, args);
    }
}

/// The published view, vault open or not; `link` resolves an item's icon and link, `name` a guid.
fn view(
    bank: &GuildBank,
    native_guild: &crate::ui_guild::NativeGuild,
    link: &dyn Fn(u32, Option<&Item>) -> (Option<String>, Option<String>),
    name: &dyn Fn(u64) -> Option<String>,
) -> WrathGuildBankView {
    let purchased = usize::from(bank.tab_count).min(TABS);
    WrathGuildBankView {
        open: bank.open,
        money: bank.money,
        // A tab the count names before any list has an empty record (`0x5a68a0`).
        tabs: (0..purchased)
            .map(|i| {
                let text = |b: Option<&Vec<u8>>| {
                    b.map_or_else(String::new, |b| String::from_utf8_lossy(b).into_owned())
                };
                let t = bank.tabs.get(i);
                WrathGuildBankTab {
                    name: text(t.map(|t| &t.name)),
                    icon: text(t.map(|t| &t.icon)),
                }
            })
            .collect(),
        remaining_withdrawals: bank.remaining_withdrawals,
        rank_rights: native_guild
            .ranks()
            .iter()
            .map(|r| WrathGuildBankRankRights {
                money_per_day: r.money_limit,
                tabs: r.tabs,
            })
            .collect(),
        slots: bank.slots[..purchased]
            .iter()
            .map(|tab| {
                tab.iter()
                    .map(|(&slot, (item_id, item))| {
                        let (texture, link) = link(*item_id, Some(item));
                        WrathGuildBankSlot {
                            slot,
                            item_id: *item_id,
                            count: item.count as i32,
                            random_property_id: item.random_property as i32,
                            random_property_seed: item.random_property_seed.unwrap_or(0) as i32,
                            enchant_id: item.enchantment as i32,
                            charges: i32::from(item.charges),
                            flags: item.flags as i32,
                            gems: item.sockets.iter().map(|&(i, e)| (i, e as i32)).collect(),
                            texture,
                            link,
                            locked: false,
                        }
                    })
                    .collect()
            })
            .collect(),
        money_log: bank
            .logs
            .get(&MONEY_LOG_TAB)
            .into_iter()
            .flat_map(|log| &log.entries)
            .filter_map(|e| match e.payload {
                Payload::Money { amount } => Some(WrathGuildBankMoneyLog {
                    kind: e.kind,
                    player: name(e.actor),
                    copper: amount as i32,
                    seconds_ago: e.elapsed,
                }),
                Payload::Item { .. } => None,
            })
            .collect(),
        tab_logs: (0..=MONEY_LOG_TAB)
            .map(|tab| {
                bank.logs
                    .get(&tab)
                    .into_iter()
                    .flat_map(|log| &log.entries)
                    .filter_map(|e| match e.payload {
                        Payload::Item {
                            item,
                            count,
                            destination_tab,
                        } => Some(WrathGuildBankItemLog {
                            kind: e.kind,
                            player: name(e.actor),
                            link: link(item, None).1,
                            count: count as i32,
                            other_tab: destination_tab.map_or(0, |t| t + 1),
                            seconds_ago: e.elapsed,
                        }),
                        Payload::Money { .. } => None,
                    })
                    .collect()
            })
            .collect(),
        tab_text: (0..purchased as u8)
            .map(|tab| bank.texts.get(&tab).cloned().unwrap_or_default())
            .collect(),
        withdraw_money_remaining: bank.money_remaining.map_or(0, i64::from),
    }
}

#[path = "ui_guild_bank_events.rs"]
mod events;
use events::events;

#[cfg(test)]
#[path = "ui_guild_bank_tests.rs"]
mod tests;
