//! The mail feed: the app half of [`wrathplus_ui::script`]'s `mail` module.
//!
//! The mailbox window opens locally on a right-click or from `SMSG_SHOW_MAILBOX`. A click opens the
//! session and sends nothing (the MAILBOX use handler `0x5f6820` opens locally, never
//! `CMSG_GAMEOBJ_USE`), and the window's `MAIL_SHOW` handler calls `CheckInbox()`, the first
//! `CMSG_GET_MAIL_LIST` (`MailFrame.lua:42`).
//!
//! [`MailPending`] is the arrival layer behind `HasNewMail()` and the minimap icon, whose only
//! listener, `MiniMapMailFrame` (`Minimap.xml:278-289`), never reads the inbox. Reading mail clears
//! the icon because opening a letter arms a deferred refresh (`[0xb6efcc]`) and the mailbox close
//! re-asks the server.

use bevy::prelude::*;
use wrathplus_protocol::types::mail_error;

use wrathplus_ui::script::{MailInboxRow, ScriptValue, StationeryView, UiScript};

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::EnteredWorldMessage;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::entities::ItemDisplays;
use crate::items::Items;
use crate::names::NameCache;
use crate::net::{ClientCommand, NetCommands, Objects};
use crate::ui_session::{close_npc_session_out_of_range, NpcSession};

mod net;
mod pending;
mod wrath;

pub(crate) use pending::MailPending;

/// `checked` mask bits (vmangos `Mail.h`): READ, RETURNED, COPIED.
const CHECKED_READ: u32 = 0x1;
const CHECKED_RETURNED: u32 = 0x2;
const CHECKED_COPIED: u32 = 0x4;

/// `MAIL_STATIONERY_GM` (`Stationery.dbc` row 61): a GM row shows its stationery icon even when it
/// carries a package (`MailFrame.lua:108`).
const STATIONERY_GM: u32 = 61;

/// `CheckInbox`'s client-side rate limit on `CMSG_GET_MAIL_LIST`, in seconds (`0x4aeab0`).
const CHECK_INBOX_THROTTLE_SECS: f64 = 60.0;

/// The letter-row icon: the stationery's item icon (`Stationery.dbc` 41 → item 9311, display
/// 7798; 61 → item 18154, display 30658).
const STATIONERY_ICON_NORMAL: &str = "Interface\\Icons\\INV_Misc_Note_01";
const STATIONERY_ICON_GM: &str = "Interface\\Icons\\Mail_GMIcon";

pub(crate) use crate::app_core::interaction::{MailOpen, MailSendAck, Packages, Stationery};
/// A package row as the engine answers it: the icon under `StringLookups.dbc` row 3's folder
/// (`0x4ae4ab`, the `"%s%s"` of folder and `\`, then the basename appended).
fn package_view(row: &wrathplus_formats::PackageRow) -> wrathplus_ui::script::PackageView {
    wrathplus_ui::script::PackageView {
        id: row.id,
        name: row.name.clone(),
        icon: format!("Interface\\Icons\\{}", row.icon),
        cost: row.cost,
    }
}

/// The range guard ([`crate::ui_session`]) closes the window, the `CloseMail` clear, past the
/// client's own interaction gate (`0x493230`, 5.5556 yd); the server's `CheckMailBox`
/// (`MailHandler.cpp:69`) checks 5 yd on each mailbox `CMSG`.
impl NpcSession for MailOpen {
    fn npc(&self) -> Option<u64> {
        self.mailbox
    }

    fn close(&mut self) {
        self.clear();
    }
}

pub struct UiMailPlugin;

impl Plugin for UiMailPlugin {
    fn build(&self, app: &mut App) {
        crate::query_cache::register::<MailOpen>(app);
        net::register(app);
        app.init_resource::<MailOpen>()
            .init_resource::<MailPending>()
            .add_systems(
                Update,
                (
                    // Range-close, feed, then drain after input, so each edge lands the same
                    // frame; the feed runs after `UnitFeed` so `SetInboxItem` reads the template.
                    close_npc_session_out_of_range::<MailOpen>.before(feed_mail),
                    feed_mail
                        .after(crate::app_core::schedule::UnitFeed)
                        .in_set(UiFeed),
                    drain_mail.after(UiInput),
                    send_query_next_mail_time_on_enter,
                ),
            );
    }
}

/// `MSG_QUERY_NEXT_MAIL_TIME` at every world enter, from the mail module's init in the world-enter
/// cascade (`0x4908c0`). The init stamps "no mail" first, so the icon goes dark across a loading
/// screen until the reply.
fn send_query_next_mail_time_on_enter(
    mut entered: MessageReader<EnteredWorldMessage>,
    commands: Res<NetCommands>,
    mut pending: ResMut<MailPending>,
) {
    if entered.read().next().is_some() {
        query_next_mail_time(&commands, &mut pending);
    }
}

/// Send only commands backed by the selected dialect's writer; never fabricate a reply.
/// A row pick as its mail command, by the native inbox; a 1.12 row has no native command.
fn inbox_action(
    mail: &MailOpen,
    mailbox: u64,
    index: u32,
    action: wrath::Action,
) -> Option<ClientCommand> {
    wrath::native_action(mail.wrath_inbox.as_ref()?, mailbox, index, action)
}

fn send_supported(commands: &NetCommands, command: ClientCommand) -> bool {
    if !commands.supports(&command) {
        return false;
    }
    commands.0.send(command).is_ok()
}

fn query_next_mail_time(commands: &NetCommands, pending: &mut MailPending) {
    if send_supported(commands, ClientCommand::QueryNextMailTime) {
        pending.on_query_sent();
    }
}

/// The message key a `SMSG_SEND_MAIL_RESULT` error shows: the handler `0x4ad050` indexes
/// `[0x4ad1a0 + error]` into the nine-arm jump table at `0x4ad17c`, and codes 6 to 13 and past 15
/// share `ERR_MAIL_DATABASE_ERROR` (`0x4ad147`). `OK` shows [`MAIL_SENT_KEY`] and `EQUIP_ERROR`
/// goes through the equip-error table (`0x4ad12e` → `0x622630`).
pub(crate) fn mail_refusal(error: u32) -> &'static str {
    match error {
        mail_error::CANNOT_SEND_TO_SELF => "ERR_MAIL_TO_SELF", // 0x164
        mail_error::NOT_ENOUGH_MONEY => "ERR_NOT_ENOUGH_MONEY", // 0x25, voiced (speech 0x28)
        mail_error::RECIPIENT_NOT_FOUND => "ERR_MAIL_TARGET_NOT_FOUND", // 0x165
        mail_error::NOT_YOUR_TEAM => "ERR_PLAYER_WRONG_FACTION", // 0xff
        mail_error::TRIAL_ACCOUNT => "ERR_RESTRICTED_ACCOUNT", // 0x1be
        mail_error::TOO_MANY_ATTACHMENTS => "ERR_MAIL_REACHED_CAP", // 0x1c4
        // INTERNAL_ERROR (6) and every other code (`0x4ad147`).
        _ => "ERR_MAIL_DATABASE_ERROR", // 0x166
    }
}

/// The yellow info line (id `0x167`) a successful `SEND` shows before the form reset (`0x4ad0b1`,
/// `0x4acdc0`); a successful take, return or delete says nothing.
pub(crate) const MAIL_SENT_KEY: &str = "ERR_MAIL_SENT";

mod invoice;

use invoice::auction_mail;

/// The notices that carry an invoice: won, sold, and build 12340's sale pending payment.
fn is_invoice_result(result: u32) -> bool {
    matches!(
        result,
        auction_mail::WON | auction_mail::SOLD | auction_mail::SALE_PENDING
    )
}

/// An auction notice's displayed subject (`AUCTION_*_MAIL_SUBJECT` with the item's name) once the
/// item template lands, else the raw subject.
fn auction_subject(
    raw: &str,
    auction: Option<invoice::AuctionSubject>,
    item_name: Option<&String>,
    get_text: &dyn Fn(&str) -> Option<String>,
) -> String {
    match (auction, item_name) {
        (Some(a), Some(name)) => invoice::subject_key(a.result)
            .and_then(get_text)
            .map(|t| t.replace("%s", name))
            .unwrap_or_else(|| raw.to_string()),
        _ => raw.to_string(),
    }
}

/// A won, sold or sale-pending notice's invoice, once its body and counterparty are known.
fn auction_invoice(
    auction: Option<invoice::AuctionSubject>,
    raw_body: Option<&String>,
    item_name: Option<String>,
    names: &NameCache,
    commands: &NetCommands,
) -> Option<wrathplus_ui::script::MailInvoice> {
    let a = auction.filter(|a| is_invoice_result(a.result))?;
    let seller = a.result != auction_mail::WON;
    let n = invoice::parse_body(raw_body?, seller)?;
    // An unresolved counterparty fills in when its name lands, as in the reference.
    let player_name = names.resolve(n.player_guid, commands)?.to_string();
    let (eta_hour, eta_min) = n.eta_hour_min();
    Some(wrathplus_ui::script::MailInvoice {
        seller,
        item_name: item_name?,
        player_name,
        bid: n.bid,
        buyout: n.buyout,
        deposit: n.deposit,
        consignment: n.consignment,
        pending: a.result == auction_mail::SALE_PENDING,
        money_delay: n.money_delay,
        eta_hour,
        eta_min,
    })
}

/// The usable stationery list (`0x4ad970`): rows always available (`Flags & 1`) or whose item the
/// player carries in bags (`0x622270`), with a cached template, sorted by buy price (`0x4ada90`).
/// A carried paper has no `cost`.
fn stationeries(
    catalog: &Stationery,
    self_q: &Query<(&ObjectStore, &crate::net::Guid), With<SelfPlayer>>,
    objects: &Objects,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
) -> Vec<StationeryView> {
    let store = self_q.iter().next().map(|(s, _)| s);
    let mut out: Vec<(u32, StationeryView)> = Vec::new();
    for row in catalog.0.rows() {
        let Some(t) = items.template(row.item, 0, commands).cloned() else {
            continue;
        };
        let carried = store.is_some_and(|s| {
            crate::inventory::count_of(
                &s.0,
                objects,
                row.item,
                crate::inventory::InventoryScope::CARRIED,
            ) > 0
        });
        if row.flags & 1 == 0 && !carried {
            continue;
        }
        out.push((
            t.buy_price,
            StationeryView {
                id: row.id,
                name: t.name.clone(),
                icon: crate::ui_items::item_icon(icons, t.display_info_id).unwrap_or_default(),
                cost: (!carried).then_some(t.buy_price),
                texture: row.texture.clone(),
            },
        ));
    }
    out.sort_by_key(|(price, v)| (*price, v.id));
    out.into_iter().map(|(_, v)| v).collect()
}

/// The feed's remaining parameters, bundled under Bevy's 16-parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
struct MailFeedExtras<'w, 's> {
    wrath_snapshot:
        Local<'s, crate::app_core::vm_memo::VmMemo<Option<wrathplus_ui::script::WrathMailInbox>>>,
    props: Option<Res<'w, crate::items::RandomProperties>>,
    enchants: Option<Res<'w, crate::items::Enchants>>,
    sink: crate::ui_action::MessageSink<'w>,
    stationeries: Local<'s, crate::app_core::vm_memo::VmMemo<Vec<StationeryView>>>,
    packages: Option<Res<'w, Packages>>,
    /// The package rows went to this VM.
    packages_pushed: Local<'s, crate::app_core::vm_memo::VmMemo<bool>>,
    /// `GetLatestThreeSenders`' auction-house names and the list this VM last heard.
    auction_houses: Option<Res<'w, crate::ui_auction::AuctionHouses>>,
    senders_pushed: Local<'s, crate::app_core::vm_memo::VmMemo<Option<Vec<String>>>>,
}

/// `GetLatestThreeSenders`' names (`0x56f3a0` over `0x56d960`): the delivered previews
/// (`delay <= 0`) in wire order, each named by kind. Normal (0) and item (5) mail: the GM
/// stationery (61) is `GM_EMAIL_NAME`, else the sender's player name; auction (2): the
/// `AuctionHouse.dbc` row's name; creature (3): its template name. A name still being asked is
/// skipped until it lands. Labelled, not built: game-object senders (4), whose template cache is
/// keyed by spawn here, not by entry.
fn latest_senders(
    previews: &[wrathplus_protocol::wrath_mail::MailPreview],
    names: &NameCache,
    commands: &NetCommands,
    houses: Option<&crate::ui_auction::AuctionHouses>,
    gm_name: Option<&str>,
) -> Vec<String> {
    previews
        .iter()
        .filter(|p| p.delay <= 0.0)
        .filter_map(|p| match p.kind {
            0 | 5 if p.stationery == 61 => gm_name.map(str::to_string),
            0 | 5 => names.resolve(p.sender, commands).map(str::to_string),
            2 => houses?.0.get(p.entry).map(|h| h.name.clone()),
            3 => names
                .resolve_creature(p.entry, 0, commands)
                .map(str::to_string),
            _ => None,
        })
        .collect()
}

fn feed_mail(
    script: Option<NonSendMut<UiScript>>,
    mut mail: ResMut<MailOpen>,
    objects: Objects,
    items: Res<Items>,
    icons: Option<Res<ItemDisplays>>,
    stationery: Option<Res<Stationery>>,
    names: Res<NameCache>,
    commands: Res<NetCommands>,
    mut pending: ResMut<MailPending>,
    time: Res<Time>,
    mut last_mailbox: Local<crate::app_core::vm_memo::VmMemo<Option<u64>>>,
    mut last_has_new_mail: Local<crate::app_core::vm_memo::VmMemo<bool>>,
    // The local player, the letter body's `$`-macro subject.
    self_q: Query<
        (&crate::app_core::session::ObjectStore, &crate::net::Guid),
        With<crate::app_core::session::SelfPlayer>,
    >,
    extras: MailFeedExtras,
) {
    let Some(mut script) = script else {
        return;
    };
    let MailFeedExtras {
        wrath_snapshot: mut last_wrath,
        props,
        enchants,
        mut sink,
        stationeries: mut last_stationeries,
        packages,
        mut packages_pushed,
        auction_houses,
        mut senders_pushed,
    } = extras;
    if let Some(packages) = packages.as_deref() {
        let pushed = packages_pushed.get(&script);
        if !*pushed {
            *pushed = true;
            script.set_mail_packages(packages.0.iter().map(package_view).collect());
        }
    }
    let rolls = crate::items::RollCatalogs {
        props: props.as_deref(),
        enchants: enchants.as_deref(),
    };
    let last_wrath = last_wrath.get(&script);
    let last_mailbox = last_mailbox.get(&script);
    let last_has_new_mail = last_has_new_mail.get(&script);

    // Take, return and delete refusals, on the surface and voice their message row names.
    let refusals: Vec<_> = std::mem::take(&mut mail.errors)
        .into_iter()
        .filter_map(|key| crate::ui_action::keyed_line(&script, key))
        .collect();
    crate::ui_action::show_messages(&mut script, &mut sink, "ui_mail", refusals);
    // A purged row fires `CLOSE_INBOX_ITEM(index)` before the list's `MAIL_INBOX_UPDATE`, as both
    // take legs do (money `0x4ad772` before `0x4ad7a3`, item `0x4ad87d` before `0x4ad8ae`).
    for index in std::mem::take(&mut mail.close_inbox) {
        script.fire_event("CLOSE_INBOX_ITEM", vec![ScriptValue::Int(i64::from(index))]);
    }
    // `0x4ad050`'s order: the message line, then on a successful send the compose reset
    // (`0x4acdc0`) with its three events, then `MAIL_FAILED` on every path (`0x4ad15f`).
    for ack in std::mem::take(&mut mail.send_acks) {
        let key = if ack.ok {
            Some(MAIL_SENT_KEY)
        } else {
            ack.refusal
        };
        let line = key.and_then(|k| crate::ui_action::keyed_line(&script, k));
        crate::ui_action::show_messages(&mut script, &mut sink, "ui_mail", line);
        if ack.ok {
            script.reset_compose_tab();
        }
        script.fire_event("MAIL_FAILED", vec![]);
    }

    let opened = last_mailbox.is_none() && mail.mailbox.is_some();
    let closed = last_mailbox.is_some() && mail.mailbox.is_none();
    let show_requested = std::mem::take(&mut mail.show_requested);
    let changed = {
        let native = {
            // Scoped, as above: the resolver borrows the VM.
            let get_text = |key: &str| script.lua().globals().get::<String>(key).ok();
            wrath::snapshot(
                &mail,
                &items,
                icons.as_deref(),
                &names,
                &commands,
                rolls,
                self_q
                    .single()
                    .ok()
                    .and_then(|(store, _)| store.0.unit_level()),
                &get_text,
                auction_houses.as_deref(),
            )
        };
        let changed = native != *last_wrath;
        if changed {
            script.set_wrath_mail(native.clone());
        }
        // `CanComplainInboxItem`'s not-from-self test (`0x56fa70`) reads the player's GUID.
        script.set_wrath_mail_self_guid(self_q.single().map_or(0, |(_, guid)| guid.0));
        let gm_name = script.lua().globals().get::<String>("GM_EMAIL_NAME").ok();
        let senders = latest_senders(
            &pending.previews,
            &names,
            &commands,
            auction_houses.as_deref(),
            gm_name.as_deref(),
        );
        let last = senders_pushed.get(&script);
        if last.as_ref() != Some(&senders) {
            script.set_wrath_mail_latest_senders(senders.clone());
            *last = Some(senders);
        }
        *last_wrath = native;
        changed
    };
    // The usable stationery is asked ahead of any mailbox so the list is whole when
    // `SendMailFrame_Reset` selects row 1 on show; an empty list leaves every send unsent. Only
    // with a player: an ask sent before the socket exists is dropped and never re-asked.
    let usable = stationery
        .as_deref()
        .filter(|_| !self_q.is_empty())
        .map(|catalog| {
            stationeries(
                catalog,
                &self_q,
                &objects,
                &items,
                icons.as_deref(),
                &commands,
            )
        })
        .unwrap_or_default();
    let memo = last_stationeries.get(&script);
    if *memo != usable {
        *memo = usable.clone();
        script.set_mail_stationeries(usable);
    }
    if opened {
        // The open core (`0x4acd10`) resets the compose tab before `MAIL_SHOW`, the stationery
        // selection with it (`0x4ace07`), which `MAIL_SHOW`'s reset re-picks as row 1; the reset's
        // `MAIL_SEND_SUCCESS`, page-turn sound and all, is the reference's too.
        script.reset_compose_tab();
        script.fire_event("MAIL_SHOW", vec![]);
    } else if closed {
        // The close core's silent reset (`0x4acd50` → `0x4acdc0(0)`).
        script.reset_compose_tab_silently();
        script.fire_event("MAIL_CLOSED", vec![]);
        // The close core's tail (`0x4acdad`, `0x4acdb1`): after a read, or an arrival while open,
        // re-ask the server, stamping "no mail". `CloseMail` and the range close both land here.
        if pending.take_refresh() {
            query_next_mail_time(&commands, &mut pending);
        }
    } else if mail.mailbox.is_some() {
        // A re-click re-shows the open window; `MAIL_SHOW`'s `CheckInbox` refreshes the list.
        if show_requested {
            script.fire_event("MAIL_SHOW", vec![]);
        }
        if changed {
            script.fire_event("MAIL_INBOX_UPDATE", vec![]);
        }
    }
    *last_mailbox = mail.mailbox;

    // `HasNewMail()` is pushed on change, but `UPDATE_PENDING_MAIL` fires only at the reference's
    // three sites, so a query's "no mail" stamp does not move the icon until the reply lands.
    pending.step(time.delta_secs());
    let has_new_mail = pending.has_new_mail();
    if has_new_mail != *last_has_new_mail {
        script.set_has_new_mail(has_new_mail);
        *last_has_new_mail = has_new_mail;
    }
    if pending.take_notify() {
        script.fire_event("UPDATE_PENDING_MAIL", vec![]);
    }
}

/// The Lua intents out as the mail `CMSG`s; `CloseMail` is a local clear that sends nothing.
fn drain_mail(
    script: Option<NonSendMut<UiScript>>,
    mut mail: ResMut<MailOpen>,
    commands: Res<NetCommands>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    objects: Objects,
    time: Res<Time>,
    mut pending: ResMut<MailPending>,
) {
    let Some(mut script) = script else {
        return;
    };
    let Some(mailbox) = mail.mailbox else {
        // No session: still honor a stray close (idempotent) and drop everything else.
        if script.take_mail_close() {
            mail.clear();
        }
        let _ = script.take_mail_check_inbox();
        let _ = script.take_mail_opens();
        let _ = script.take_mail_send();
        return;
    };

    // `CheckInbox`: the list query, throttled to one per 60 s (`0x4aeab0`).
    if script.take_mail_check_inbox() {
        let now = time.elapsed_secs_f64();
        if mail
            .last_list_query
            .is_none_or(|t| now - t >= CHECK_INBOX_THROTTLE_SECS)
            && send_supported(&commands, ClientCommand::GetMailList { mailbox })
        {
            mail.last_list_query = Some(now);
        }
    }

    // Build-12340 rows carry their text inline: an open only marks the native row read.
    if let Some(inbox) = mail.wrath_inbox.as_mut() {
        for index in script.take_mail_opens() {
            if let Some(command) = wrath::mark_read(inbox, mailbox, index) {
                pending.arm_refresh();
                send_supported(&commands, command);
            }
        }
    }
    // An open with no native inbox has no row to mark: drained, so it never replays on one.
    let _ = script.take_mail_opens();

    for (index, slot) in script.take_mail_take_items() {
        if let Some(command) = inbox_action(&mail, mailbox, index, wrath::Action::TakeItem(slot)) {
            send_supported(&commands, command);
        }
    }
    // The 12340 mail family's intents. Auto-loot sends the money take and the first item take;
    // native continues with the next item on each take result (`0x570270`), which is not wired
    // yet. Complaints are capture-only until a CMSG_COMPLAIN writer exists.
    for request in script.take_wrath_mail_requests() {
        match request {
            wrathplus_ui::script::WrathMailRequest::AutoLoot { row } => {
                // `0x570410` sends the money take only when the row holds money.
                let has_money = mail
                    .wrath_inbox
                    .as_ref()
                    .and_then(|inbox| inbox.mails.get(row.checked_sub(1)? as usize))
                    .is_some_and(|m| m.money > 0);
                let actions = [
                    has_money.then_some(wrath::Action::TakeMoney),
                    Some(wrath::Action::TakeItem(None)),
                ];
                for action in actions.into_iter().flatten() {
                    if let Some(command) = inbox_action(&mail, mailbox, row, action) {
                        send_supported(&commands, command);
                    }
                }
            }
            wrathplus_ui::script::WrathMailRequest::Complain { row, sender } => {
                info!(
                    "mail: complaint about row {row} (sender {sender:#x}) not sent: capture-only"
                );
            }
        }
    }
    for index in script.take_mail_take_money() {
        if let Some(command) = inbox_action(&mail, mailbox, index, wrath::Action::TakeMoney) {
            send_supported(&commands, command);
        }
    }
    for index in script.take_mail_deletes() {
        if let Some(command) = inbox_action(&mail, mailbox, index, wrath::Action::Delete) {
            send_supported(&commands, command);
        }
    }
    for index in script.take_mail_returns() {
        if let Some(command) = inbox_action(&mail, mailbox, index, wrath::Action::Return) {
            send_supported(&commands, command);
        }
    }
    for index in script.take_mail_take_texts() {
        if let Some(command) = inbox_action(&mail, mailbox, index, wrath::Action::CreateTextItem) {
            send_supported(&commands, command);
        }
    }

    // `SendMail`: each attachment's bag slot is read at send time, as in the reference. The first
    // that no longer holds an item aborts the send: an error, no packet, that send slot cleared
    // and `MAIL_SEND_INFO_UPDATE` fired (12340 `0x570910`).
    if let Some(req) = script.take_mail_send() {
        let guid_of = |bag: i64, slot: u32| {
            self_q.iter().next().and_then(|s| {
                crate::inventory::slot_guid(&s.0, bag, (slot.max(1) - 1) as u8, &objects)
            })
        };
        let mut items = Vec::with_capacity(req.items.len());
        let mut missing = None;
        for &(send_slot, bag, slot) in &req.items {
            match guid_of(bag, slot) {
                Some(guid) => items.push((send_slot, guid)),
                None => {
                    missing = Some(send_slot);
                    break;
                }
            }
        }
        if let Some(send_slot) = missing {
            mail.errors.push("ERR_ITEM_NOT_FOUND");
            script.drop_send_mail_item(send_slot);
        } else {
            // The package rides only with an item.
            let package = if items.is_empty() { 0 } else { req.package };
            let command = wrath::send(
                mailbox,
                req.target,
                req.subject,
                req.body,
                req.stationery,
                package,
                items,
                req.money,
                req.cod,
            );
            send_supported(&commands, command);
        }
    }

    if script.take_mail_close() {
        mail.clear();
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod latest_senders_tests {
    use super::*;
    use wrathplus_protocol::wrath_mail::MailPreview;

    /// `0x56d960` by kind over the delivered previews, in wire order.
    #[test]
    fn senders_are_named_by_kind_and_only_once_delivered() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let commands = NetCommands(tx);
        let mut names = NameCache::default();
        names.insert_player(0x42, "Arthas".into(), None);
        let preview = |sender, entry, kind, stationery, delay| MailPreview {
            sender,
            entry,
            kind,
            stationery,
            delay,
        };
        let previews = [
            preview(0x42, 0, 0, 41, -5.0),
            preview(0x43, 0, 0, 61, 0.0),
            // Not yet deliverable.
            preview(0x42, 0, 0, 41, 30.0),
            // A player whose name is still being asked: skipped until it lands.
            preview(0x99, 0, 0, 41, -1.0),
        ];
        assert_eq!(
            latest_senders(&previews, &names, &commands, None, Some("Game Master")),
            ["Arthas", "Game Master"]
        );
        assert!(
            latest_senders(&[preview(0, 7, 4, 41, -1.0)], &names, &commands, None, None).is_empty()
        );
    }
}

#[cfg(test)]
mod open_tests;
