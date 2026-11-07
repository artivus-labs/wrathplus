//! The mail bindings. The app pushes the open mailbox's inbox, already resolved from the wire
//! (`UiScript::set_mail`), and the Lua verbs queue intents the app drains and sends. Indices are
//! 1-based, as `MailFrame.lua` uses them.

pub mod wrath;

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::flag;
use super::cursor::{self, CursorPayload};
use super::Model;

/// One inbox row, resolved by the app from a wire `MailListEntry`; its 1-based index is its
/// position in [`MailState::inbox`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MailInboxRow {
    /// `packageIcon`, the enclosed item's icon; `None` with no item or while its template loads.
    pub package_icon: Option<String>,
    pub stationery_icon: Option<String>,
    /// `sender`, from the name cache; `None` while in flight or for a non-player sender.
    pub sender: Option<String>,
    pub subject: String,
    pub money: u32,
    pub cod: u32,
    pub days_left: f32,
    /// The enclosed stack's count, answered as `hasItem`; 0 (nil) is no item.
    pub item_count: u32,
    /// `checked & READ(0x1)`.
    pub was_read: bool,
    /// `checked & RETURNED(0x2)`.
    pub was_returned: bool,
    /// `checked & COPIED(0x4)`: the letter was copied to an item (`textCreated`).
    pub text_created: bool,
    /// `sender_guid.is_some() && !was_returned`, the Reply button's gate (`MailFrame.lua:281`).
    pub can_reply: bool,
    /// `stationery == 61`: a GM mail (vmangos `MAIL_STATIONERY_GM`, `Mail.h:85`).
    pub is_gm: bool,
    /// The letter body; `None` until fetched, while `GetInboxText` answers `""`.
    pub body: Option<String>,
    pub stationery_texture: String,
    /// `isInvoice`: only an auction won (result 1) or sold (2), not an outbid or expiry notice
    /// (`0x4af2eb`).
    pub is_invoice: bool,
    /// `GetInboxInvoiceInfo`'s answer; `None`, the reference's miss, also while the body or the
    /// counterparty's name is in flight.
    pub invoice: Option<MailInvoice>,
    /// `item_text_id != 0`: the mail has a letter body to fetch.
    pub has_body: bool,
    /// The enclosed item's template entry, 0 for none; the key `SetInboxItem`'s tooltip reads.
    pub item_id: u32,
    /// The enclosed item's random-suffix id (the wire `randomPropId`), 0 for none, which the
    /// reference's `SetInboxItem` hands the tooltip (`+0x424`); `item_name` carries its suffix.
    pub item_random_property_id: u32,
    /// The enclosed item's name; `None` while its template is in flight.
    pub item_name: Option<String>,
    pub item_texture: Option<String>,
    pub item_quality: Option<u32>,
    /// `InboxItemCanDelete`: `!can_reply`, as a player's unreturned mail is returned instead
    /// (`MailFrame.lua:417`, `:450`).
    pub can_delete: bool,
}

/// One auction invoice, `GetInboxInvoiceInfo`'s answer, parsed by the app from the mail's subject
/// and body text as the reference `sscanf`s them (`0x4af360`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailInvoice {
    /// `"seller"` (your auction sold) or `"buyer"` (you won), literal tokens, not global strings.
    pub seller: bool,
    pub item_name: String,
    /// The counterparty: who bought it (seller invoice) or who sold it (buyer invoice).
    pub player_name: String,
    /// The winning bid; the window reads `bid == buyout` as a buyout (`MailFrame.lua:307`).
    pub bid: u32,
    pub buyout: u32,
    /// Seller invoices only; 0 on a buyer's, whose body carries three fields, not five.
    pub deposit: u32,
    /// The auction house's cut, seller invoices only.
    pub consignment: u32,
    /// Build 12340's sale-pending notice (result 6): `"seller_temp_invoice"`, money not yet sent.
    pub pending: bool,
    /// The body's payment delay, in seconds as the server wrote it (`moneyDelay`).
    pub money_delay: u32,
    /// The payment's estimated arrival, the body's packed game time unpacked (`etaHour`, `etaMin`).
    pub eta_hour: u32,
    pub eta_min: u32,
}

/// The open mailbox's inbox, pushed whole by the app; `set_mail(None)` means no mailbox is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MailState {
    pub inbox: Vec<MailInboxRow>,
}

/// One usable stationery in the send tab's picker; the app builds the list from `Stationery.dbc`
/// and the bags, sorted by price.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationeryView {
    /// The `Stationery.dbc` row id, which `SelectStationery` stores and `CMSG_SEND_MAIL` carries.
    pub id: u32,
    pub name: String,
    /// The item's icon as a full `Interface\Icons\` path, which `MailFrame.lua:671` sets as is.
    pub icon: String,
    /// The item's BuyPrice in copper; `None` (nil) when the player carries one.
    pub cost: Option<u32>,
    /// The bare texture basename that `GetSelectedStationeryTexture` answers.
    pub texture: String,
}

/// One `Package.dbc` row, as `GetPackageInfo` answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageView {
    /// The row id, which `SelectPackage` stores and `CMSG_SEND_MAIL` carries.
    pub id: u32,
    /// The localized name (field 3 on).
    pub name: String,
    /// The icon as a full path: `StringLookups.dbc` row 3's folder, `\` and field 1
    /// (`0x4ae4ab`-`0x4ae4eb`).
    pub icon: String,
    /// Field 2, in copper.
    pub cost: i32,
}

/// A drained `SendMail` intent, which the app turns into `CMSG_SEND_MAIL`.
#[derive(Clone, Debug, PartialEq)]
pub struct MailSendRequest {
    pub target: String,
    pub subject: String,
    pub body: String,
    /// The selected `Stationery.dbc` id; never 0, as a send with none aborts (`0x4ae8dd`).
    pub stationery: u32,
    pub money: u32,
    pub cod: u32,
    /// The attachments as `(0-based send slot, bag, 1-based bag slot)`, ascending by send slot;
    /// the app resolves each guid when the send fires, as the reference re-reads them then
    /// (12340 `0x570910`).
    pub items: Vec<(u8, i64, u32)>,
    /// The selected `Package.dbc` id, sent only with an attached item (`0x4ae8f3`).
    pub package: u32,
}

/// Build 12340's send attachment slots (`ATTACHMENTS_MAX_SEND`, MailFrame.lua:6).
pub const SEND_SLOTS: usize = 12;

mod attachments;
use attachments::send_slot_arg;
pub(super) use attachments::{can_attach, source_reserved};

/// `0x4acdc0`'s clears: every attachment, whose lock it lifts (`0x495420`, firing
/// `ITEM_LOCK_CHANGED`), then money, COD, stationery and package. Returns the lock events to fire.
fn clear_compose(model: &mut Model) -> Vec<(String, Vec<super::ScriptValue>)> {
    let lock: Vec<_> = model
        .mail_send_items
        .iter_mut()
        .filter_map(Option::take)
        .map(|item| {
            (
                "ITEM_LOCK_CHANGED".to_string(),
                vec![
                    super::ScriptValue::Int(item.bag),
                    super::ScriptValue::Int(i64::from(item.slot)),
                ],
            )
        })
        .collect();
    model.wrath_mail_family.send_locked = false;
    model.mail_send_money = 0;
    model.mail_send_cod = 0;
    model.mail_stationery = 0;
    model.mail_package = 0;
    lock
}

impl super::UiScript {
    /// Push (or clear, with `None`) the open mailbox's inbox snapshot.
    pub fn set_mail(&mut self, state: Option<MailState>) {
        self.model_mut().mail = state;
        self.model_mut().wrath_mail = None;
    }

    /// Push the lossless multi-attachment Wrath inbox; Vanilla state is not projected.
    pub fn set_wrath_mail(&mut self, state: Option<wrath::Inbox>) {
        self.model_mut().wrath_mail = state;
        self.model_mut().mail = None;
    }

    /// Take the `CheckInbox` flag; the app answers it with `CMSG_GET_MAIL_LIST`.
    pub fn take_mail_check_inbox(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().mail_check_inbox)
    }

    /// Drain the 1-based indices `GetInboxText` opened; the app marks each read, fetches its body.
    pub fn take_mail_opens(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().mail_opens)
    }

    /// Drain the `TakeInboxItem` picks: the 1-based row and, when given, the 1-based attachment
    /// slot (build 12340 `0x5711c0`; `None` takes the row's first attachment, `0x56d420`).
    pub fn take_mail_take_items(&mut self) -> Vec<(u32, Option<u32>)> {
        std::mem::take(&mut self.model_mut().mail_take_items)
    }

    /// Drain the 1-based `TakeInboxMoney` row picks.
    pub fn take_mail_take_money(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().mail_take_money)
    }

    /// Drain the 1-based `DeleteInboxItem` row picks.
    pub fn take_mail_deletes(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().mail_deletes)
    }

    /// Drain the 1-based `ReturnInboxItem` row picks.
    pub fn take_mail_returns(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().mail_returns)
    }

    /// Drain the 1-based `TakeInboxTextItem` row picks, each a `CMSG_MAIL_CREATE_TEXT_ITEM`.
    pub fn take_mail_take_texts(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.model_mut().mail_take_texts)
    }

    /// Take the `CloseMail` flag; 1.12 has no close opcode, so the app only ends its mail session.
    pub fn take_mail_close(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().mail_close)
    }

    /// Drain the `SendMail` intent with the current money, COD and attachment. The attachment
    /// stays until [`Self::reset_compose_tab`], so a failed send keeps it.
    pub fn take_mail_send(&mut self) -> Option<MailSendRequest> {
        let mut model = self.model_mut();
        let (target, subject, body) = model.mail_send.take()?;
        let items = model
            .mail_send_items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| it.as_ref().map(|it| (i as u8, it.bag, it.slot)))
            .collect();
        Some(MailSendRequest {
            target,
            subject,
            body,
            stationery: model.mail_stationery,
            money: model.mail_send_money,
            cod: model.mail_send_cod,
            items,
            package: model.mail_package,
        })
    }

    /// The reference's compose-tab reset (`0x4acdc0(1)`), run on mailbox open and a sent mail:
    /// clear the attachment (its lock lifted), money, COD, stationery and package, then fire
    /// `SEND_MAIL_MONEY_CHANGED`, `SEND_MAIL_COD_CHANGED` and `MAIL_SEND_SUCCESS` in that order.
    /// The events must follow the clear, as `SendMailFrame_Reset` re-reads `GetSendMailItem` and
    /// selects stationery row 1. `MAIL_SEND_SUCCESS` means the form is clean: opening a mailbox
    /// fires it.
    pub fn reset_compose_tab(&mut self) {
        let lock = clear_compose(&mut self.model_mut());
        // Fired now, not queued: the order against the caller's `MAIL_SHOW`/`MAIL_FAILED` matters.
        for (event, args) in lock {
            self.fire_event(&event, args);
        }
        // One literal call each, as the event census (`ui_script::reference_ui`) reads them.
        self.fire_event("SEND_MAIL_MONEY_CHANGED", Vec::new()); // 0x4ace14
        self.fire_event("SEND_MAIL_COD_CHANGED", Vec::new()); // 0x4ace1e
        self.fire_event("MAIL_SEND_SUCCESS", Vec::new()); // 0x4ace28
    }

    /// The close core's silent reset (`0x4acdc0(0)`, from `0x4acd50`): the same clears, and only
    /// the attachment's `ITEM_LOCK_CHANGED`, which `0x495420` fires on either leg.
    pub fn reset_compose_tab_silently(&mut self) {
        let lock = clear_compose(&mut self.model_mut());
        for (event, args) in lock {
            self.fire_event(&event, args);
        }
    }

    /// `SendMail`'s abort when an attached item left its bag slot (an error, no packet, that send
    /// slot cleared and `MAIL_SEND_INFO_UPDATE`, 12340 `0x570910`): drop that attachment, keep the
    /// rest, the money and COD. `slot` is the 0-based send slot.
    pub fn drop_send_mail_item(&mut self, slot: u8) {
        let mut model = self.model_mut();
        let taken = model
            .mail_send_items
            .get_mut(usize::from(slot))
            .and_then(Option::take);
        if let Some(item) = taken {
            cursor::queue_lock_changed(&mut model, item.bag, item.slot);
            model
                .pending_events
                .push(("MAIL_SEND_INFO_UPDATE".to_string(), Vec::new()));
        }
    }

    /// Push the usable stationery list, in the picker's order.
    pub fn set_mail_stationeries(&mut self, list: Vec<StationeryView>) {
        self.model_mut().mail_stationeries = list;
    }

    /// Push `Package.dbc`'s rows, in file order.
    pub fn set_mail_packages(&mut self, packages: Vec<PackageView>) {
        self.model_mut().mail_packages = packages;
    }

    /// Push `HasNewMail()`'s answer, independent of any open mailbox. The app sets it before each
    /// `UPDATE_PENDING_MAIL`, on which the minimap icon re-reads it (`Minimap.xml:278-289`).
    pub fn set_has_new_mail(&mut self, has: bool) {
        self.model_mut().has_new_mail = has;
    }
}

/// A clone of the inbox row at a 1-based index.
fn row_at(model: &Model, index: usize) -> Option<MailInboxRow> {
    model
        .wrath_mail
        .as_ref()
        .and_then(|m| index.checked_sub(1).and_then(|i| m.rows.get(i)))
        .map(|row| row.header.clone())
}

/// Register the mail globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetInboxNumItems",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let (shown, total) = model
                .wrath_mail
                .as_ref()
                .map_or((0, 0), |m| (m.rows.len() as i64, i64::from(m.total_count)));
            Ok(MultiValue::from_vec(vec![
                Value::Integer(shown),
                Value::Integer(total),
            ]))
        })?,
    )?;

    // GetInboxHeaderInfo(index): the 13 values `MailFrame.lua:105` reads; nil out of range.
    g.set(
        "GetInboxHeaderInfo",
        lua.create_function(|lua, index: usize| {
            let row = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                row_at(&model, index)
            };
            let Some(r) = row else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            let opt_str = |s: &Option<String>| -> mlua::Result<Value> {
                Ok(match s {
                    Some(v) => Value::String(lua.create_string(v)?),
                    None => Value::Nil,
                })
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let first_quantity = model
                .wrath_mail
                .as_ref()
                .and_then(|m| index.checked_sub(1).and_then(|i| m.rows.get(i)))
                .and_then(|r| r.attachments.first())
                .map_or(0, |a| a.count);
            // Stock Wrath MailFrame.lua:154 consumes attachment count and first stack separately.
            let package_icon = if r.item_count != 0 && !r.is_gm {
                opt_str(&r.package_icon)?
            } else {
                Value::Nil
            };
            let values = vec![
                package_icon,
                opt_str(&r.stationery_icon)?,
                opt_str(&r.sender)?,
                Value::String(lua.create_string(&r.subject)?),
                Value::Integer(i64::from(r.money)),
                Value::Integer(i64::from(r.cod)),
                Value::Number(f64::from(r.days_left)),
                if r.item_count > 0 {
                    Value::Integer(i64::from(r.item_count))
                } else {
                    Value::Nil
                },
                flag(r.was_read),
                flag(r.was_returned),
                flag(r.text_created),
                flag(r.can_reply),
                flag(r.is_gm),
                Value::Integer(i64::from(first_quantity)),
            ];
            Ok(MultiValue::from_vec(values))
        })?,
    )?;

    // GetInboxText(index): body, stationeryTexture, isTakeable, isInvoice (`MailFrame.lua:292`);
    // reading a mail queues its open. isTakeable with `not textCreated` gates the copy button
    // (`MailFrame.lua:366`), the pair vmangos requires (`MailHandler.cpp:867`).
    g.set(
        "GetInboxText",
        lua.create_function(|lua, index: usize| {
            let row = {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let row = row_at(&model, index);
                if row.is_some() {
                    // Queued once per drain.
                    let i = index as u32;
                    if !model.mail_opens.contains(&i) {
                        model.mail_opens.push(i);
                    }
                }
                row
            };
            let Some(r) = row else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            Ok(MultiValue::from_vec(vec![
                // An invoice's body is nil (`0x4af1cf`): its text is the auction house's
                // bookkeeping, which only `GetInboxInvoiceInfo` parses.
                if r.is_invoice {
                    Value::Nil
                } else {
                    Value::String(lua.create_string(r.body.as_deref().unwrap_or(""))?)
                },
                Value::String(lua.create_string(&r.stationery_texture)?),
                flag(r.has_body),
                flag(r.is_invoice),
            ]))
        })?,
    )?;

    // GetInboxInvoiceInfo(index) (`MailFrame.lua:302`): seven values in 1.12 (`0x4af5a1`), a miss
    // being three nils and four zeros (`0x4af559`). Build 12340 adds `moneyDelay, etaHour,
    // etaMin` (`MailFrame.lua:348`) and the sale-pending type `"seller_temp_invoice"`; a miss
    // there pads with zeros too (labelled: the 12340 miss path is untraced).
    g.set(
        "GetInboxInvoiceInfo",
        lua.create_function(|lua, index: usize| {
            let row = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                row_at(&model, index)
            };
            let width = 10;
            let Some(inv) = row.and_then(|r| r.invoice) else {
                let mut miss = vec![Value::Nil, Value::Nil, Value::Nil];
                miss.resize(width, Value::Integer(0));
                return Ok(MultiValue::from_vec(miss));
            };
            let kind = match (inv.seller, inv.pending) {
                (true, true) => "seller_temp_invoice",
                (true, false) => "seller",
                (false, _) => "buyer",
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(kind)?),
                Value::String(lua.create_string(&inv.item_name)?),
                Value::String(lua.create_string(&inv.player_name)?),
                Value::Integer(i64::from(inv.bid)),
                Value::Integer(i64::from(inv.buyout)),
                Value::Integer(i64::from(inv.deposit)),
                Value::Integer(i64::from(inv.consignment)),
                Value::Integer(i64::from(inv.money_delay)),
                Value::Integer(i64::from(inv.eta_hour)),
                Value::Integer(i64::from(inv.eta_min)),
            ]))
        })?,
    )?;

    // GetInboxItem(index): name, texture, count, quality, canUse (`MailFrame.lua:379`); canUse is
    // the shared item-usable gate, true while the template is in flight.
    g.set(
        "GetInboxItem",
        lua.create_function(|lua, (index, attachment): (usize, Option<usize>)| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            wrath::item(lua, &model, index, attachment.unwrap_or(1))
        })?,
    )?;

    g.set(
        "InboxItemCanDelete",
        lua.create_function(|lua, index: usize| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(row_at(&model, index).is_some_and(|r| r.can_delete)))
        })?,
    )?;

    g.set(
        "HasNewMail",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(model.has_new_mail))
        })?,
    )?;

    g.set(
        "CheckInbox",
        lua.create_function(|lua, ()| {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .mail_check_inbox = true;
            Ok(())
        })?,
    )?;

    // TakeInboxItem(messageIndex [, attachIndex]) (12340 `0x5711c0`): the row must be a number
    // (the usage line otherwise); the attachment slot is optional, rounded, and names the slot
    // whose item the take sends (`0x56d920`), absent meaning the first occupied one.
    g.set(
        "TakeInboxItem",
        lua.create_function(|lua, args: mlua::MultiValue| {
            let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
            let Some(index) = lua.coerce_number(arg(0))? else {
                return Err(mlua::Error::runtime(
                    "Usage: TakeInboxItem(messageIndex, attachIndex)",
                ));
            };
            let attach = lua
                .coerce_number(arg(1))?
                .map(|n| n.round_ties_even() as i64 as u32);
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .mail_take_items
                .push((index.round_ties_even() as i64 as u32, attach));
            Ok(())
        })?,
    )?;

    for (name, field) in [
        ("TakeInboxMoney", 1u8),
        ("DeleteInboxItem", 2),
        ("ReturnInboxItem", 3),
        ("TakeInboxTextItem", 4),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, index: u32| {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                match field {
                    1 => model.mail_take_money.push(index),
                    2 => model.mail_deletes.push(index),
                    3 => model.mail_returns.push(index),
                    _ => model.mail_take_texts.push(index),
                }
                Ok(())
            })?,
        )?;
    }

    g.set(
        "CloseMail",
        lua.create_function(|lua, ()| {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .mail_close = true;
            Ok(())
        })?,
    )?;

    g.set(
        "SendMail",
        lua.create_function(|lua, (target, subject, body): (String, String, String)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // `SendMail` (`0x4ae800`) aborts silently, no packet and no return value, on money
            // with COD or on COD with no attached item.
            if model.mail_send_money != 0 && model.mail_send_cod != 0 {
                return Ok(());
            }
            if model.mail_send_cod != 0 && model.mail_send_items.iter().all(Option::is_none) {
                return Ok(());
            }
            // The same silent abort with no stationery selected (`0x4ae8dd`).
            if model.mail_stationery == 0 {
                return Ok(());
            }
            model.mail_send = Some((target, subject, body));
            Ok(())
        })?,
    )?;

    // SetSendMailMoney(copper) (`0x4ae0f0`): a non-number raises; more than the purse shows
    // ERR_NOT_ENOUGH_MONEY and answers nil; else it stores, fires SEND_MAIL_MONEY_CHANGED and
    // answers 1, which `StaticPopup.lua:257` branches on.
    g.set(
        "SetSendMailMoney",
        lua.create_function(|lua, copper: Value| {
            let n = crate::script::binding_abi::number_arg(
                lua,
                copper,
                "Usage: SetSendMailMoney(amount)",
            )? as u32;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if u64::from(n) > model.money {
                model.ui_errors.push("ERR_NOT_ENOUGH_MONEY");
                return Ok(Value::Nil);
            }
            model.mail_send_money = n;
            model
                .pending_events
                .push(("SEND_MAIL_MONEY_CHANGED".to_string(), Vec::new()));
            Ok(Value::Integer(1))
        })?,
    )?;

    // GetSendMailMoney() and GetSendMailCOD() (`0x4ae150`, `0x4ae1c0`): the stored amounts.
    g.set(
        "GetSendMailMoney",
        lua.create_function(|lua, ()| {
            Ok(i64::from(
                lua.app_data_ref::<Model>()
                    .expect("model app_data")
                    .mail_send_money,
            ))
        })?,
    )?;
    g.set(
        "GetSendMailCOD",
        lua.create_function(|lua, ()| {
            Ok(i64::from(
                lua.app_data_ref::<Model>()
                    .expect("model app_data")
                    .mail_send_cod,
            ))
        })?,
    )?;

    // SetSendMailCOD(copper) (`0x4ae180`): a non-number raises; with no attached item nothing
    // happens; there is no purse or sign check and no return value.
    g.set(
        "SetSendMailCOD",
        lua.create_function(|lua, copper: Value| {
            let n = crate::script::binding_abi::number_arg(
                lua,
                copper,
                "Usage: SetSendMailCOD(amount)",
            )? as u32;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.mail_send_items.iter().all(Option::is_none) {
                return Ok(());
            }
            model.mail_send_cod = n;
            model
                .pending_events
                .push(("SEND_MAIL_COD_CHANGED".to_string(), Vec::new()));
            Ok(())
        })?,
    )?;

    // GetSendMailItem([index]): name, texture, stackCount, quality of a send slot (12340
    // `0x570750`, MailFrame.lua:822), slot 1 without an index. The reference answers
    // `nil, nil, 0, 0` with nothing attached and before the template loads, and quality -1 for an
    // item whose template has none; this answers the cached quality, or 0.
    g.set(
        "GetSendMailItem",
        lua.create_function(|lua, args: MultiValue| {
            let slot = send_slot_arg(lua, args.front())?;
            // A whole-stack pickup records no count, so the size comes from the source slot.
            let item = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                slot.and_then(|i| model.mail_send_items[i].clone())
                    .map(|it| {
                        let count = cursor::held_count(&model, &it);
                        (it, count)
                    })
            };
            let Some((it, count)) = item else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Integer(0),
                    Value::Integer(0),
                ]));
            };
            let name = cursor::item_link_name(it.link.as_deref());
            let name = if name.is_empty() {
                Value::Nil
            } else {
                Value::String(lua.create_string(&name)?)
            };
            let texture = match &it.texture {
                Some(t) => Value::String(lua.create_string(t)?),
                None => Value::Nil,
            };
            // A number on every path, as in the reference.
            let quality = Value::Integer(i64::from(it.quality.unwrap_or(0)));
            Ok(MultiValue::from_vec(vec![
                name,
                texture,
                Value::Integer(i64::from(count)),
                quality,
            ]))
        })?,
    )?;

    // GetSendMailPrice() (12340 `0x571b00`): 30 copper per attachment, 30 with none, plus the
    // enclosed money, which MailFrame.lua:871 compares with GetMoney. Labelled: an uncarried
    // stationery's and the package's prices, which native also adds, are not added here.
    g.set(
        "GetSendMailPrice",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let items = model.mail_send_items.iter().flatten().count().max(1) as i64;
            Ok(Value::Integer(
                30 * items + i64::from(model.mail_send_money),
            ))
        })?,
    )?;

    // ClickSendMailItemButton([index [, clearItem]]) (12340 `0x570550`): the slot is
    // `trunc(index - 1)`, the first free slot without an index. clearItem (a true boolean, the
    // stock right-click) returns the slot's item to its bag: the slot clears, its lock lifts and
    // MAIL_SEND_INFO_UPDATE fires. Otherwise the slot is a cursor drop target.
    g.set(
        "ClickSendMailItemButton",
        lua.create_function(|lua, args: MultiValue| {
            let slot = match lua.coerce_number(args.front().cloned().unwrap_or(Value::Nil))? {
                Some(n) => usize::try_from((n - 1.0).trunc() as i64).ok(),
                None => None,
            };
            let explicit = args.front().is_some_and(|v| !matches!(v, Value::Nil));
            let clear = matches!(args.get(1), Some(Value::Boolean(true)));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if clear {
                if let Some(item) = slot
                    .filter(|i| *i < SEND_SLOTS)
                    .and_then(|i| model.mail_send_items[i].take())
                {
                    cursor::queue_lock_changed(&mut model, item.bag, item.slot);
                    model
                        .pending_events
                        .push(("MAIL_SEND_INFO_UPDATE".to_string(), Vec::new()));
                }
                return Ok(());
            }
            let target = if explicit {
                slot.filter(|i| *i < SEND_SLOTS)
            } else {
                model.mail_send_items.iter().position(Option::is_none)
            };
            click_send_mail_item(&mut model, target);
            Ok(())
        })?,
    )?;

    // ── The stationery family ──
    // The reference rebuilds the list (`0x4ad970`) in `GetNumStationeries` (`0x4ae1f0`), on world
    // entry and on the last item-query answer; the app recomputes it every frame.

    g.set(
        "GetNumStationeries",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.mail_stationeries.len() as i64)
        })?,
    )?;

    // `GetStationeryInfo(index)` (`0x4ae230`): a non-number raises its Usage; three values
    // always, three nils out of range.
    g.set(
        "GetStationeryInfo",
        lua.create_function(|lua, index: Value| {
            let index = crate::script::binding_abi::number_arg(
                lua,
                index,
                "Usage: GetStationeryInfo(index)",
            )?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let row = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| model.mail_stationeries.get(i));
            let Some(row) = row else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&row.name)?),
                Value::String(lua.create_string(&row.icon)?),
                row.cost
                    .map_or(Value::Nil, |c| Value::Integer(i64::from(c))),
            ]))
        })?,
    )?;

    // `SelectStationery(index)` (`0x4ae380`): out of range is not an error, it stores 0.
    g.set(
        "SelectStationery",
        lua.create_function(|lua, index: Value| {
            let index = crate::script::binding_abi::number_arg(
                lua,
                index,
                "Usage: SelectStationery(index)",
            )?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.mail_stationery = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| model.mail_stationeries.get(i))
                .map_or(0, |row| row.id);
            Ok(())
        })?,
    )?;

    // `GetSelectedStationeryTexture()` (`0x4ae3f0`): nil for no selection or an unknown id.
    g.set(
        "GetSelectedStationeryTexture",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let id = model.mail_stationery;
            let tex = (id != 0)
                .then(|| model.mail_stationeries.iter().find(|r| r.id == id))
                .flatten();
            Ok(match tex {
                Some(row) => Value::String(lua.create_string(&row.texture)?),
                None => Value::Nil,
            })
        })?,
    )?;

    // ClearSendMail() (`0x4adee0`): the compose reset with its events (`0x4acdc0(1)`), queued in
    // its order; zero returns.
    g.set(
        "ClearSendMail",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let lock = clear_compose(&mut model);
            model.pending_events.extend(lock);
            for event in [
                "SEND_MAIL_MONEY_CHANGED",
                "SEND_MAIL_COD_CHANGED",
                "MAIL_SEND_SUCCESS",
            ] {
                model.pending_events.push((event.to_string(), Vec::new()));
            }
            Ok(MultiValue::new())
        })?,
    )?;

    // GetNumPackages() (`0x4ae430`): `Package.dbc`'s raw row count, unfiltered.
    g.set(
        "GetNumPackages",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.mail_packages.len() as i64)
        })?,
    )?;

    // GetPackageInfo(index) (`0x4ae450`) → name, icon, cost for a 1-based row; out of range it
    // pushes nil, nil, 0 (`0x4ae525`), and a non-number raises.
    g.set(
        "GetPackageInfo",
        lua.create_function(|lua, index: Value| {
            let index =
                crate::script::binding_abi::number_arg(lua, index, "Usage: GetPackageInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let row = usize::try_from(index.wrapping_sub(1))
                .ok()
                .and_then(|i| model.mail_packages.get(i));
            Ok(match row {
                Some(row) => MultiValue::from_vec(vec![
                    Value::String(lua.create_string(&row.name)?),
                    Value::String(lua.create_string(&row.icon)?),
                    Value::Integer(i64::from(row.cost)),
                ]),
                None => MultiValue::from_vec(vec![Value::Nil, Value::Nil, Value::Integer(0)]),
            })
        })?,
    )?;

    // SelectPackage(index) (`0x4ae550`): stores the 1-based row's id, or 0 out of range
    // (`0x4adc90`); a non-number raises. Zero returns.
    g.set(
        "SelectPackage",
        lua.create_function(|lua, index: Value| {
            let index =
                crate::script::binding_abi::number_arg(lua, index, "Usage: SelectPackage(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.mail_package = usize::try_from(index.wrapping_sub(1))
                .ok()
                .and_then(|i| model.mail_packages.get(i))
                .map_or(0, |row| row.id);
            Ok(MultiValue::new())
        })?,
    )?;

    Ok(())
}

/// `ClickSendMailItemButton` on send slot `target` as a cursor drop target, firing `CURSOR_UPDATE`
/// and `ITEM_LOCK_CHANGED` like a bag slot: a held bag item goes in and the slot's previous item
/// comes up on the cursor; an empty cursor picks the slot's item up; the slot's own item dropped
/// back on it just releases the cursor. `None` (no free slot, or an index past the slots) keeps
/// the cursor as it is.
fn click_send_mail_item(model: &mut Model, target: Option<usize>) {
    let Some(target) = target else {
        return;
    };
    match model.cursor.take() {
        Some(CursorPayload::Item(item)) => {
            let same = model.mail_send_items[target]
                .as_ref()
                .is_some_and(|it| it.bag == item.bag && it.slot == item.slot);
            if same {
                // `0x519280`: the slot's own item is put back; the cursor empties.
                cursor::queue_cursor_update(model);
                return;
            }
            if !can_attach(model, &item) {
                model.cursor = Some(CursorPayload::Item(item));
                return;
            }
            let (bag, slot) = (item.bag, item.slot);
            let previous = model.mail_send_items[target].replace(item);
            cursor::queue_lock_changed(model, bag, slot);
            if let Some(prev) = previous {
                let (pbag, pslot) = (prev.bag, prev.slot);
                model.cursor = Some(CursorPayload::Item(prev));
                cursor::queue_lock_changed(model, pbag, pslot);
            }
            cursor::queue_cursor_update(model);
            // `MAIL_SEND_INFO_UPDATE` (`0x570606`): the tab re-reads the items and postage.
            model
                .pending_events
                .push(("MAIL_SEND_INFO_UPDATE".to_string(), Vec::new()));
        }
        None => {
            if let Some(item) = model.mail_send_items[target].take() {
                let (bag, slot) = (item.bag, item.slot);
                model.cursor = Some(CursorPayload::Item(item));
                cursor::queue_cursor_update(model);
                cursor::queue_lock_changed(model, bag, slot);
                model
                    .pending_events
                    .push(("MAIL_SEND_INFO_UPDATE".to_string(), Vec::new()));
            }
        }
        Some(other) => {
            model.cursor = Some(other);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    /// A send needs a stationery: select the default, as the stock `SendMailFrame_Reset` does.
    fn select_default_stationery(s: &mut UiScript) {
        s.set_mail_stationeries(vec![super::StationeryView {
            id: 41,
            name: "Default Stationery".into(),
            icon: "Interface\\Icons\\INV_Letter_15".into(),
            cost: Some(0),
            texture: "STATIONERYTEST".into(),
        }]);
        s.run("SelectStationery(1)").unwrap();
    }

    fn row(item_id: u32) -> MailInboxRow {
        MailInboxRow {
            package_icon: (item_id != 0).then(|| "Interface\\Icons\\INV_Misc_Bag_08".to_string()),
            stationery_icon: Some("Interface\\Icons\\INV_Letter_15".to_string()),
            sender: Some("Thrall".into()),
            subject: "Warchief's orders".into(),
            money: 5000,
            cod: 0,
            days_left: 29.5,
            item_count: if item_id != 0 { 3 } else { 0 },
            was_read: false,
            was_returned: false,
            text_created: false,
            can_reply: true,
            is_gm: false,
            body: Some("Lok'tar.".into()),
            stationery_texture: "STATIONERYTEST".into(),
            is_invoice: false,
            invoice: None,
            has_body: true,
            item_id,
            item_name: (item_id != 0).then(|| "Linen Cloth".to_string()),
            item_texture: (item_id != 0)
                .then(|| "Interface\\Icons\\INV_Fabric_Linen_01".to_string()),
            item_quality: (item_id != 0).then_some(1),
            can_delete: false, // a player's unreturned mail is returned, not deleted
            item_random_property_id: 0,
        }
    }

    /// A 1.12-style inbox as build 12340's store holds it: each row's item as attachment slot 0.
    fn wrath(st: MailState) -> crate::script::WrathMailInbox {
        crate::script::WrathMailInbox {
            total_count: st.inbox.len() as u32,
            rows: st
                .inbox
                .into_iter()
                .map(|header| crate::script::WrathMailRow {
                    attachments: (header.item_id != 0)
                        .then(|| crate::script::WrathMailAttachment {
                            slot: 0,
                            entry: header.item_id,
                            count: header.item_count.max(1),
                            name: header.item_name.clone(),
                            texture: header.item_texture.clone(),
                            quality: header.item_quality,
                            ..Default::default()
                        })
                        .into_iter()
                        .collect(),
                    header,
                    kind: 0,
                    sender_guid: 0,
                })
                .collect(),
        }
    }

    fn state() -> MailState {
        MailState {
            inbox: vec![row(2589), row(0)],
        }
    }

    #[test]
    fn an_invoice_has_no_letter_body() {
        let mut s = UiScript::new().unwrap();
        let mut st = state();
        st.inbox[0].is_invoice = true;
        st.inbox[0].body = Some("6C:10000:10000:25:500".into());
        s.set_wrath_mail(Some(wrath(st)));

        assert_eq!(
            s.arity("GetInboxText(1)").unwrap(),
            4,
            "four values, invoice or not"
        );
        assert_eq!(
            s.eval::<String>("return tostring((GetInboxText(1)))")
                .unwrap(),
            "nil",
            "the bookkeeping is never handed back as a letter body"
        );
        assert_eq!(
            s.eval::<String>("return tostring((GetInboxText(2)))")
                .unwrap(),
            "Lok'tar."
        );
    }

    #[test]
    fn a_buyer_invoice_is_the_literal_token_buyer() {
        let mut s = UiScript::new().unwrap();
        let mut st = state();
        st.inbox[0].is_invoice = true;
        st.inbox[0].invoice = Some(MailInvoice {
            seller: false,
            item_name: "Small Blue Pouch".into(),
            player_name: "Onewarrior".into(),
            bid: 9_000,
            buyout: 10_000,
            deposit: 0,
            consignment: 0,
            ..Default::default()
        });
        s.set_wrath_mail(Some(wrath(st)));
        assert_eq!(
            s.eval::<String>("return (GetInboxInvoiceInfo(1))").unwrap(),
            "buyer"
        );
    }

    #[test]
    fn inbox_header_reads_the_reference_tuple() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetInboxNumItems()").unwrap(), 0);
        assert!(s
            .eval::<bool>("return GetInboxHeaderInfo(1) == nil")
            .unwrap());

        s.set_wrath_mail(Some(wrath(state())));
        assert_eq!(s.eval::<i64>("return GetInboxNumItems()").unwrap(), 2);

        let (pkg, sta, sender, subject, money, cod, days): (
            String,
            String,
            String,
            String,
            i64,
            i64,
            f64,
        ) = s
            .eval("local a,b,c,d,e,f,g = GetInboxHeaderInfo(1)\nreturn a,b,c,d,e,f,g")
            .unwrap();
        assert_eq!(pkg, "Interface\\Icons\\INV_Misc_Bag_08");
        assert_eq!(sta, "Interface\\Icons\\INV_Letter_15");
        assert_eq!(
            (sender.as_str(), subject.as_str()),
            ("Thrall", "Warchief's orders")
        );
        assert_eq!((money, cod), (5000, 0));
        assert!((days - 29.5).abs() < 1e-3);
        assert!(s
            .eval::<bool>(
                "local a,b,c,d,e,f,g, has, read, ret, tc, reply, gm = GetInboxHeaderInfo(1)\n\
                 return has == 3 and read == nil and ret == nil and tc == nil and reply == 1 and gm == nil",
            )
            .unwrap());

        assert!(s
            .eval::<bool>(
                "local pkg, sta, s, subj, m, c, d, has = GetInboxHeaderInfo(2)\n\
                 return pkg == nil and has == nil",
            )
            .unwrap());
    }

    #[test]
    fn get_inbox_text_returns_body_and_queues_open() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_mail(Some(wrath(state())));
        let (body, tex, takeable, invoice): (String, String, Value, Value) =
            s.eval("return GetInboxText(1)").unwrap();
        assert_eq!(body, "Lok'tar.");
        assert_eq!(tex, "STATIONERYTEST");
        assert_eq!(takeable, Value::Integer(1)); // has a body to fetch
        assert!(matches!(invoice, Value::Nil));
        // The open is queued once per drain.
        s.run("GetInboxText(1)").unwrap();
        assert_eq!(s.take_mail_opens(), vec![1]);
        assert!(s.take_mail_opens().is_empty(), "drained");
    }

    #[test]
    fn get_inbox_item_and_can_delete() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_mail(Some(wrath(state())));
        let (name, _tex, count, quality, canuse): (String, String, i64, i64, i64) =
            s.eval("return GetInboxItem(1)").unwrap();
        assert_eq!(
            (name.as_str(), count, quality, canuse),
            ("Linen Cloth", 3, 1, 1)
        );
        assert!(s.eval::<bool>("return GetInboxItem(2) == nil").unwrap());
        // A player's unreturned mail is returnable, not deletable.
        assert!(s
            .eval::<bool>("return InboxItemCanDelete(1) == nil")
            .unwrap());
    }

    #[test]
    fn intents_queue_and_drain() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_mail(Some(wrath(state())));
        s.run("CheckInbox()").unwrap();
        assert!(s.take_mail_check_inbox());
        assert!(!s.take_mail_check_inbox(), "drained");

        s.run("TakeInboxItem(1) TakeInboxMoney(1) DeleteInboxItem(2) ReturnInboxItem(1) TakeInboxTextItem(1)")
            .unwrap();
        assert_eq!(s.take_mail_take_items(), vec![(1, None)]);
        assert_eq!(s.take_mail_take_money(), vec![1]);
        assert_eq!(s.take_mail_deletes(), vec![2]);
        assert_eq!(s.take_mail_returns(), vec![1]);
        assert_eq!(s.take_mail_take_texts(), vec![1]);
        assert!(s.take_mail_take_texts().is_empty(), "drained");

        s.run("CloseMail()").unwrap();
        assert!(s.take_mail_close());
    }

    #[test]
    fn send_folds_money_cod_and_returns_true_from_setmoney() {
        let mut s = UiScript::new().unwrap();
        s.set_money(5_000);
        assert!(s.take_mail_send().is_none());
        assert!(s
            .eval::<bool>("return SetSendMailMoney(9999) == nil")
            .unwrap());
        assert_eq!(s.take_ui_errors(), vec!["ERR_NOT_ENOUGH_MONEY"]);
        assert!(s
            .eval::<bool>("return SetSendMailMoney(1234) == 1")
            .unwrap());
        // COD with no attached item: no store, no event.
        s.run("SetSendMailCOD(50)").unwrap();
        assert_eq!(s.eval::<i64>("return GetSendMailCOD()").unwrap(), 0);
        select_default_stationery(&mut s);
        s.run("SendMail('Jaina', 'Hi', 'body text')").unwrap();
        let req = s.take_mail_send().expect("a send was queued");
        assert_eq!(req.target, "Jaina");
        assert_eq!(req.subject, "Hi");
        assert_eq!(req.body, "body text");
        assert_eq!((req.money, req.cod), (1234, 0));
        assert_eq!(req.items, vec![]);
        assert!(s.take_mail_send().is_none(), "drained");
        assert_eq!(
            s.eval::<i64>("return GetSendMailPrice()").unwrap(),
            30 + 1234,
            "postage plus the enclosed money (0x571b00)"
        );
        assert!(
            s.run("SetSendMailMoney(nil)").is_err(),
            "a non-number raises"
        );
        assert!(s.run("SetSendMailCOD({})").is_err());
    }

    #[test]
    fn attach_from_cursor_and_get_send_item() {
        use crate::script::cursor::{CursorItem, CursorPayload};
        let mut s = UiScript::new().unwrap();
        assert!(s
            .eval::<bool>("local n = GetSendMailItem()\nreturn n == nil")
            .unwrap());

        s.model_mut().cursor = Some(CursorPayload::Item(CursorItem {
            bar_placeable: true,
            bag: 0,
            slot: 5,
            item_id: 2589,
            texture: Some("Interface\\Icons\\INV_Fabric_Linen_01".into()),
            link: Some("|cff...|Hitem:2589|h[Linen Cloth]|h|r".into()),
            count: Some(7),
            quality: Some(1),
            equip_slots: Vec::new(),
        }));
        s.run("ClickSendMailItemButton()").unwrap();
        assert!(s.eval::<bool>("return not CursorHasItem()").unwrap());
        let (name, _tex, count, quality): (String, String, i64, i64) =
            s.eval("return GetSendMailItem()").unwrap();
        assert_eq!((name.as_str(), count, quality), ("Linen Cloth", 7, 1));

        select_default_stationery(&mut s);
        s.run("SendMail('Alt', 'stuff', '')").unwrap();
        assert_eq!(s.take_mail_send().unwrap().items, vec![(0, 0, 5)]);

        // An empty cursor dropped nowhere in particular picks nothing up (`0x570550` with -1).
        s.run("ClickSendMailItemButton()").unwrap();
        assert!(s.eval::<bool>("return not CursorHasItem()").unwrap());
        // Clicking the slot with an empty cursor detaches it.
        s.run("ClickSendMailItemButton(1)").unwrap();
        assert!(s.eval::<bool>("return CursorHasItem()").unwrap());
        assert!(s
            .eval::<bool>("local n = GetSendMailItem()\nreturn n == nil")
            .unwrap());
    }

    /// Build 12340's twelve send slots (`0xbeb038`): a slotless drop takes the first free slot,
    /// a slot click swaps with the cursor, right-click returns the item to its bag, and the send
    /// lists every attachment by its slot.
    #[test]
    fn twelve_send_slots_drop_swap_clear_and_send() {
        use crate::script::cursor::{CursorItem, CursorPayload};
        let mut s = UiScript::new().unwrap();
        let hold = |s: &UiScript, slot: u32| {
            s.model_mut().cursor = Some(CursorPayload::Item(CursorItem {
                bar_placeable: true,
                bag: 0,
                slot,
                item_id: 2589 + slot,
                texture: None,
                link: Some(format!("|Hitem:{}|h[Item {slot}]|h", 2589 + slot)),
                count: Some(1),
                quality: Some(1),
                equip_slots: Vec::new(),
            }));
        };
        hold(&s, 5);
        s.run("ClickSendMailItemButton()").unwrap();
        hold(&s, 6);
        s.run("ClickSendMailItemButton()").unwrap();
        assert_eq!(
            s.eval::<String>("return GetSendMailItem(2)").unwrap(),
            "Item 6"
        );
        // A slot click with a held item swaps.
        hold(&s, 7);
        s.run("ClickSendMailItemButton(2)").unwrap();
        assert_eq!(
            s.eval::<String>("return GetSendMailItem(2)").unwrap(),
            "Item 7"
        );
        assert!(
            s.eval::<bool>("return CursorHasItem()").unwrap(),
            "item 6 came up"
        );
        s.model_mut().cursor = None;
        // Right-click returns slot 1's item to its bag.
        s.run("ClickSendMailItemButton(1, true)").unwrap();
        assert!(s.eval::<bool>("return GetSendMailItem(1) == nil").unwrap());
        assert!(s.eval::<bool>("return not CursorHasItem()").unwrap());
        assert_eq!(s.eval::<i64>("return GetSendMailPrice()").unwrap(), 30);
        hold(&s, 9);
        s.run("ClickSendMailItemButton(12)").unwrap();
        assert_eq!(s.eval::<i64>("return GetSendMailPrice()").unwrap(), 60);
        select_default_stationery(&mut s);
        s.run("SendMail('Alt', 'stuff', '')").unwrap();
        assert_eq!(
            s.take_mail_send().unwrap().items,
            vec![(1, 0, 7), (11, 0, 9)]
        );
        s.drop_send_mail_item(11);
        assert!(s.eval::<bool>("return GetSendMailItem(12) == nil").unwrap());
        assert!(s.eval::<bool>("return GetSendMailItem(13) == nil").unwrap());
    }

    #[test]
    fn clear_send_item_resets_the_form() {
        use crate::script::cursor::{CursorItem, CursorPayload};
        let mut s = UiScript::new().unwrap();
        s.model_mut().cursor = Some(CursorPayload::Item(CursorItem {
            bar_placeable: true,
            bag: 0,
            slot: 1,
            item_id: 1,
            texture: None,
            link: None,
            count: None,
            quality: None,
            equip_slots: Vec::new(),
        }));
        s.run("ClickSendMailItemButton()").unwrap();
        s.run("SetSendMailMoney(99) SetSendMailCOD(5)").unwrap();
        s.reset_compose_tab();
        select_default_stationery(&mut s);
        s.run("SendMail('x','y','z')").unwrap();
        let req = s.take_mail_send().unwrap();
        assert_eq!((req.money, req.cod, req.items.len()), (0, 0, 0));
    }

    /// `ClearSendMail()` (`0x4adee0`) is the compose reset with its events, the attachment's lock
    /// first, then the three in `0x4acdc0`'s order; stationery and package clear with the rest.
    #[test]
    fn clear_send_mail_resets_the_form_and_fires_its_events() {
        use crate::script::cursor::{CursorItem, CursorPayload};
        let mut s = UiScript::new().unwrap();
        select_default_stationery(&mut s);
        s.set_mail_packages(vec![PackageView {
            id: 2,
            name: "Test Package".into(),
            icon: "Interface\\Icons\\INV_BOX_04".into(),
            cost: 10,
        }]);
        s.run("SelectPackage(1)").unwrap();
        s.model_mut().cursor = Some(CursorPayload::Item(CursorItem {
            bar_placeable: true,
            bag: 0,
            slot: 3,
            item_id: 1,
            texture: None,
            link: None,
            count: None,
            quality: None,
            equip_slots: Vec::new(),
        }));
        s.run("ClickSendMailItemButton() SetSendMailMoney(99) SetSendMailCOD(5)")
            .unwrap();
        s.tick(0.0);
        s.run(
            r#"FIRED = {}
               local f = CreateFrame("Frame")
               for _, e in ipairs({ "ITEM_LOCK_CHANGED", "SEND_MAIL_MONEY_CHANGED",
                             "SEND_MAIL_COD_CHANGED", "MAIL_SEND_SUCCESS" }) do
                   f:RegisterEvent(e)
               end
               f:SetScript("OnEvent", function(self, event) table.insert(FIRED, event) end)"#,
        )
        .unwrap();
        assert_eq!(
            s.eval::<i64>("local n = function(...) return arg.n end return n(ClearSendMail())")
                .unwrap(),
            0
        );
        s.tick(0.0);
        assert_eq!(
            s.eval::<Vec<String>>("return FIRED").unwrap(),
            [
                "ITEM_LOCK_CHANGED",
                "SEND_MAIL_MONEY_CHANGED",
                "SEND_MAIL_COD_CHANGED",
                "MAIL_SEND_SUCCESS"
            ]
        );
        assert!(s
            .eval::<bool>("return GetSendMailItem() == nil and GetSendMailMoney() == 0")
            .unwrap());
        s.run("SendMail('x', 'y', 'z')").unwrap();
        assert!(s.take_mail_send().is_none(), "the stationery cleared too");
        select_default_stationery(&mut s);
        s.run("SendMail('x', 'y', 'z')").unwrap();
        assert_eq!(s.take_mail_send().map(|r| r.package), Some(0));
    }

    /// `Package.dbc`'s one shipped row, through the three package verbs (`0x4ae430`, `0x4ae450`,
    /// `0x4ae550`).
    #[test]
    fn packages_list_describe_and_select_by_row() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetNumPackages()").unwrap(), 0);
        s.set_mail_packages(vec![PackageView {
            id: 2,
            name: "Test Package".into(),
            icon: "Interface\\Icons\\INV_BOX_04".into(),
            cost: 10,
        }]);
        assert_eq!(s.eval::<i64>("return GetNumPackages()").unwrap(), 1);
        let (name, icon, cost): (String, String, i64) = s.eval("return GetPackageInfo(1)").unwrap();
        assert_eq!(
            (name.as_str(), icon.as_str(), cost),
            ("Test Package", "Interface\\Icons\\INV_BOX_04", 10)
        );
        for miss in ["0", "2", "-1"] {
            assert!(
                s.eval::<bool>(&format!(
                    "local a, b, c = GetPackageInfo({miss}) return a == nil and b == nil and c == 0"
                ))
                .unwrap(),
                "{miss}"
            );
        }
        for call in ["GetPackageInfo()", "SelectPackage('x')"] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(err.contains("Usage: "), "{call}: {err}");
        }

        select_default_stationery(&mut s);
        s.run("SelectPackage(1) SendMail('x', 'y', 'z')").unwrap();
        assert_eq!(s.take_mail_send().map(|r| r.package), Some(2));
        s.run("SelectPackage(5) SendMail('x', 'y', 'z')").unwrap();
        assert_eq!(
            s.take_mail_send().map(|r| r.package),
            Some(0),
            "out of range stores 0"
        );
    }

    #[test]
    fn clearing_the_mail_empties_it() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_mail(Some(wrath(state())));
        s.set_mail(None);
        assert_eq!(s.eval::<i64>("return GetInboxNumItems()").unwrap(), 0);
        assert!(s.eval::<bool>("return HasNewMail() == nil").unwrap());
    }

    #[test]
    fn has_new_mail_reads_the_pushed_flag() {
        let mut s = UiScript::new().unwrap();
        assert!(s.eval::<bool>("return HasNewMail() == nil").unwrap());
        s.set_has_new_mail(true);
        assert!(s.eval::<bool>("return HasNewMail() == 1").unwrap());
        s.set_has_new_mail(false);
        assert!(s.eval::<bool>("return HasNewMail() == nil").unwrap());
    }
}

#[cfg(test)]
mod stationery_tests {
    use super::StationeryView;
    use crate::script::UiScript;

    fn seated() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_mail_stationeries(vec![
            StationeryView {
                id: 41,
                name: "Default Stationery".into(),
                icon: "Interface\\Icons\\INV_Letter_15".into(),
                cost: Some(0),
                texture: "STATIONERYTEST".into(),
            },
            StationeryView {
                id: 64,
                name: "Valentine Stationery".into(),
                icon: "Interface\\Icons\\INV_ValentinesCard02".into(),
                cost: None,
                texture: "STATIONERY_VAL".into(),
            },
        ]);
        s
    }

    #[test]
    fn the_stationery_family_answers_like_the_client() {
        let s = seated();
        assert_eq!(s.eval::<i64>("return GetNumStationeries()").unwrap(), 2);
        assert_eq!(
            s.eval::<(String, String, i64)>("return GetStationeryInfo(1)")
                .unwrap(),
            (
                "Default Stationery".into(),
                "Interface\\Icons\\INV_Letter_15".into(),
                0
            )
        );
        assert!(
            s.eval::<bool>("local n, t, c = GetStationeryInfo(\"2\") return n == \"Valentine Stationery\" and c == nil")
                .unwrap(),
            "a carried paper costs nil; a numeric string is an index"
        );
        assert!(
            s.eval::<bool>(
                "local n, t, c = GetStationeryInfo(3) return n == nil and t == nil and c == nil"
            )
            .unwrap(),
            "past the end: three nils"
        );
        for bad in [
            "GetStationeryInfo()",
            "GetStationeryInfo(\"x\")",
            "SelectStationery(nil)",
        ] {
            let err = s.run(bad).expect_err(bad).to_string();
            assert!(err.contains("Usage: "), "{bad}: {err}");
        }
        assert!(s
            .eval::<bool>("return GetSelectedStationeryTexture() == nil")
            .unwrap());
        s.run("SelectStationery(2)").unwrap();
        assert_eq!(
            s.eval::<String>("return GetSelectedStationeryTexture()")
                .unwrap(),
            "STATIONERY_VAL"
        );
        s.run("SelectStationery(9)").unwrap();
        assert!(
            s.eval::<bool>("return GetSelectedStationeryTexture() == nil")
                .unwrap(),
            "out of range is a deselect, not an error"
        );
    }

    #[test]
    fn a_send_needs_a_stationery_and_carries_its_id() {
        let mut s = seated();
        s.run("SendMail(\"Bob\", \"hi\", \"body\")").unwrap();
        assert!(
            s.take_mail_send().is_none(),
            "no selection: silently nothing"
        );
        s.run("SelectStationery(1) SendMail(\"Bob\", \"hi\", \"body\")")
            .unwrap();
        assert_eq!(s.take_mail_send().map(|r| r.stationery), Some(41));
        s.reset_compose_tab_silently();
        s.run("SendMail(\"Bob\", \"hi\", \"body\")").unwrap();
        assert!(s.take_mail_send().is_none());
    }

    /// Build 12340 answers ten values, and a sale pending is `"seller_temp_invoice"`.
    #[test]
    fn a_wrath_invoice_has_ten_values_and_the_pending_type() {
        use super::{MailInboxRow, MailInvoice};
        let mut s = UiScript::new().unwrap();
        let header = MailInboxRow {
            is_invoice: true,
            invoice: Some(MailInvoice {
                seller: true,
                item_name: "Linen Cloth".into(),
                player_name: "Buyer".into(),
                bid: 9000,
                buyout: 10_000,
                deposit: 25,
                consignment: 450,
                pending: true,
                money_delay: 3600,
                eta_hour: 14,
                eta_min: 35,
            }),
            ..Default::default()
        };
        s.set_wrath_mail(Some(super::wrath::Inbox {
            total_count: 1,
            rows: vec![super::wrath::Row {
                header,
                ..Default::default()
            }],
        }));
        assert!(s
            .eval::<bool>(
                "local t, _, p, _, _, _, c, d, h, m = GetInboxInvoiceInfo(1) \
                 return select('#', GetInboxInvoiceInfo(1)) == 10 and t == 'seller_temp_invoice' \
                 and p == 'Buyer' and c == 450 and d == 3600 and h == 14 and m == 35 \
                 and select('#', GetInboxInvoiceInfo(2)) == 10"
            )
            .unwrap());
    }
}
