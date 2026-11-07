//! Build-12340 inbox view. Keep attachment slots separate from the 1.12 single-item row.
use super::MailInboxRow;

/// Resolved item metadata for one native attachment slot (1-based in Lua).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attachment {
    pub slot: u8,
    pub guid_low: u32,
    pub entry: u32,
    pub random_property: i32,
    pub suffix_factor: u32,
    pub count: u32,
    pub name: Option<String>,
    pub texture: Option<String>,
    pub quality: Option<u32>,
    /// Native nine-field hyperlink, resolved by the app; absent until metadata/level are known.
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub header: MailInboxRow,
    pub attachments: Vec<Attachment>,
    /// The mail's message type (`+0x4`; 0 a player's mail), which the complaint check reads.
    pub kind: u8,
    /// The sender's full GUID (`+0x8`), which the complaint check compares and records.
    pub sender_guid: u64,
}

/// One mail-family intent for App, in call order. Labelled capture-only: App sends the native
/// packets (`0x245`/`0x246` auto-loot, `0x3c7` complaint then `0x23a` list refresh).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrathMailRequest {
    /// `AutoLootMailItem(index)` (`0x571450` → `0x570410`): take the row's money, then its items
    /// one at a time (`0x570270`, which checks free bag space and continues per take result).
    AutoLoot { row: u32 },
    /// `ComplainInboxItem(index)` (`0x571350` → `0x56faf0`) on a complainable row whose sender
    /// was not already reported: `CMSG_COMPLAIN` (type 0, sender, mail id), then the list query.
    Complain { row: u32, sender: u64 },
}

/// Build 12340's mail-family host state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Family {
    requests: Vec<WrathMailRequest>,
    /// `SetSendMailShowing` (`0xbeb1b9`).
    pub(crate) send_showing: bool,
    /// The pending send-item lock `RespondMailLockSendItem` answers (`0xbeb1b8`).
    pub(super) send_locked: bool,
    /// `GetLatestThreeSenders`' names (`0xbeaff8`, three rows).
    latest_senders: Vec<String>,
    /// Senders already reported (`0x6b3700`/`0x6b3750`). Labelled: per VM here.
    complained: std::collections::HashSet<u64>,
    /// The player's GUID, which a complaint may not name.
    self_guid: u64,
}

impl crate::script::UiScript {
    /// Drain the mail-family requests in call order.
    pub fn take_wrath_mail_requests(&mut self) -> Vec<WrathMailRequest> {
        std::mem::take(&mut self.model_mut().wrath_mail_family.requests)
    }

    /// Whether the send tab is showing (`SetSendMailShowing`), for the bag-click attach path.
    pub fn wrath_mail_send_showing(&self) -> bool {
        self.model_ref().wrath_mail_family.send_showing
    }

    /// Arm the send-item lock App reports with `MAIL_LOCK_SEND_ITEMS(slot, link)`; the player's
    /// answer arrives through `RespondMailLockSendItem`.
    pub fn set_wrath_mail_send_locked(&mut self, locked: bool) {
        self.model_mut().wrath_mail_family.send_locked = locked;
    }

    /// The up-to-three senders `GetLatestThreeSenders` answers, already filtered to delivered
    /// rows (delay <= 0) and resolved by App: a player's name, `GM_EMAIL_NAME`, the auction
    /// house's, a creature's or a game object's (`0x56d960`).
    pub fn set_wrath_mail_latest_senders(&mut self, names: Vec<String>) {
        self.model_mut().wrath_mail_family.latest_senders = names;
    }

    /// The player's GUID for the complaint check.
    pub fn set_wrath_mail_self_guid(&mut self, guid: u64) {
        self.model_mut().wrath_mail_family.self_guid = guid;
    }
}

/// `0x56fa70`: a player's mail (type 0), not returned, not GM stationery, from someone other
/// than the player.
fn can_complain(model: &super::Model, index: i32) -> bool {
    let Some(row) = usize::try_from(index)
        .ok()
        .and_then(|i| model.wrath_mail.as_ref()?.rows.get(i))
    else {
        return false;
    };
    let fam = &model.wrath_mail_family;
    row.kind == 0
        && !row.header.was_returned
        && !row.header.is_gm
        && row.sender_guid != fam.self_guid
}

/// A 1-based row argument, rounded, as the 12340 bindings read it; `None` is the usage error.
fn row_arg(lua: &mlua::Lua, v: Option<&mlua::Value>) -> mlua::Result<Option<i32>> {
    Ok(lua
        .coerce_number(v.cloned().unwrap_or(mlua::Value::Nil))?
        .map(|n| (n.round_ties_even() as i64 as i32).wrapping_sub(1)))
}

fn install_family(lua: &mlua::Lua) -> mlua::Result<()> {
    use super::super::ScriptValue;
    use mlua::{MultiValue, Value};
    let g = lua.globals();

    // AutoLootMailItem(index): with a mailbox open and no take in flight, App takes the money
    // and every item. Mailbox and busy gating are App's.
    g.set(
        "AutoLootMailItem",
        lua.create_function(|lua, args: MultiValue| {
            let Some(row) = row_arg(lua, args.front())? else {
                return Err(mlua::Error::runtime("Usage: AutoLootMailItem(index)"));
            };
            if let Ok(row) = u32::try_from(row) {
                let mut model = lua.app_data_mut::<super::Model>().expect("model");
                model
                    .wrath_mail_family
                    .requests
                    .push(WrathMailRequest::AutoLoot { row: row + 1 });
            }
            Ok(())
        })?,
    )?;

    // CanComplainInboxItem(index) → 1 or nil. The usage line is the binding's own
    // ("InboxItemCanComplain", `0x5713c0`).
    g.set(
        "CanComplainInboxItem",
        lua.create_function(|lua, args: MultiValue| {
            let Some(row) = row_arg(lua, args.front())? else {
                return Err(mlua::Error::runtime("Usage: InboxItemCanComplain(index)"));
            };
            let model = lua.app_data_ref::<super::Model>().expect("model");
            Ok(super::flag(can_complain(&model, row)))
        })?,
    )?;

    // ComplainInboxItem(index): a sender already reported prints COMPLAINT_ADDED as a system
    // line; otherwise the sender is recorded, App sends the complaint, and CLOSE_INBOX_ITEM fires
    // with the 1-based index.
    g.set(
        "ComplainInboxItem",
        lua.create_function(|lua, args: MultiValue| {
            let Some(row) = row_arg(lua, args.front())? else {
                return Err(mlua::Error::runtime("Usage: ComplainInboxItem(index)"));
            };
            let sender = {
                let model = lua.app_data_ref::<super::Model>().expect("model");
                if !can_complain(&model, row) {
                    return Ok(());
                }
                model.wrath_mail.as_ref().expect("checked").rows[row as usize].sender_guid
            };
            let reported = {
                let mut model = lua.app_data_mut::<super::Model>().expect("model");
                let fam = &mut model.wrath_mail_family;
                if fam.complained.insert(sender) {
                    fam.requests.push(WrathMailRequest::Complain {
                        row: row as u32 + 1,
                        sender,
                    });
                    false
                } else {
                    true
                }
            };
            if reported {
                let text: Value = lua.globals().get("COMPLAINT_ADDED")?;
                let text = match text {
                    Value::String(s) => s.to_str()?.to_string(),
                    _ => "COMPLAINT_ADDED".to_string(),
                };
                super::super::tick::fire_event_into(
                    lua,
                    "CHAT_MSG_SYSTEM",
                    vec![ScriptValue::Str(text)],
                );
            } else {
                super::super::tick::fire_event_into(
                    lua,
                    "CLOSE_INBOX_ITEM",
                    vec![ScriptValue::Int(i64::from(row) + 1)],
                );
            }
            Ok(())
        })?,
    )?;

    // SetSendMailShowing(bool) (`0x56d570`): a real boolean only, else the usage line.
    g.set(
        "SetSendMailShowing",
        lua.create_function(|lua, args: MultiValue| {
            let Some(Value::Boolean(on)) = args.front() else {
                return Err(mlua::Error::runtime("Usage: SetSendMailShowing(bool)"));
            };
            lua.app_data_mut::<super::Model>()
                .expect("model")
                .wrath_mail_family
                .send_showing = *on;
            Ok(())
        })?,
    )?;

    // RespondMailLockSendItem(slot, keepItem) (`0x56f410`): only while a lock is pending, which
    // it clears. Keeping fires MAIL_UNLOCK_SEND_ITEMS; otherwise the slot's attachment is dropped
    // and unlocked, then MAIL_SEND_INFO_UPDATE and MAIL_UNLOCK_SEND_ITEMS fire.
    g.set(
        "RespondMailLockSendItem",
        lua.create_function(|lua, args: MultiValue| {
            let Some(slot) = row_arg(lua, args.front())? else {
                return Err(mlua::Error::runtime(
                    "Usage: RespondMailLockSendItem(slot, keepItem)",
                ));
            };
            let keep = super::super::binding_abi::bool_or_default(args.get(1), false);
            let mut model = lua.app_data_mut::<super::Model>().expect("model");
            if !std::mem::take(&mut model.wrath_mail_family.send_locked) {
                return Ok(());
            }
            // Queued in the native order, the item unlock (`0x513770`) first, as
            // `drop_send_mail_item` queues its MAIL_SEND_INFO_UPDATE.
            if !keep {
                let taken = usize::try_from(slot)
                    .ok()
                    .filter(|i| *i < super::SEND_SLOTS)
                    .and_then(|i| model.mail_send_items[i].take());
                if let Some(item) = taken {
                    super::super::cursor::queue_lock_changed(&mut model, item.bag, item.slot);
                }
                model
                    .pending_events
                    .push(("MAIL_SEND_INFO_UPDATE".to_string(), Vec::new()));
            }
            model
                .pending_events
                .push(("MAIL_UNLOCK_SEND_ITEMS".to_string(), Vec::new()));
            Ok(())
        })?,
    )?;

    // GetLatestThreeSenders() → up to three names, nothing when none (`0x56f3a0`).
    g.set(
        "GetLatestThreeSenders",
        lua.create_function(|lua, _ignored: MultiValue| {
            let model = lua.app_data_ref::<super::Model>().expect("model");
            let mut out = MultiValue::new();
            for name in model.wrath_mail_family.latest_senders.iter().take(3) {
                out.push_back(Value::String(lua.create_string(name)?));
            }
            Ok(out)
        })?,
    )?;
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inbox {
    pub total_count: u32,
    pub rows: Vec<Row>,
}

/// Wrath GetInboxItem(index, attachmentIndex): name, texture, count, quality, usable (stock MailFrame.lua:400).
pub(super) fn item(
    lua: &mlua::Lua,
    model: &super::Model,
    index: usize,
    slot: usize,
) -> mlua::Result<mlua::MultiValue> {
    use mlua::Value;
    let attachment = model
        .wrath_mail
        .as_ref()
        .and_then(|m| index.checked_sub(1).and_then(|i| m.rows.get(i)))
        .and_then(|r| {
            slot.checked_sub(1)
                .and_then(|s| r.attachments.iter().find(|a| usize::from(a.slot) == s))
        });
    let Some(a) = attachment else {
        return Ok(mlua::MultiValue::from_vec(vec![Value::Nil]));
    };
    let text = |s: &Option<String>| -> mlua::Result<Value> {
        Ok(match s {
            Some(s) => Value::String(lua.create_string(s)?),
            None => Value::Nil,
        })
    };
    Ok(mlua::MultiValue::from_vec(vec![
        text(&a.name)?,
        text(&a.texture)?,
        Value::Integer(i64::from(a.count)),
        Value::Integer(i64::from(a.quality.unwrap_or(0))),
        super::flag(super::super::item_stats::item_usable_by_id(model, a.entry)),
    ]))
}

/// Build-12340 0x56eca0; stock MailFrame.xml:353 uses the explicit attachment slot.
/// Unlike GetInboxItem, unavailable links return no Lua values (not one nil).
pub(in crate::script) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    install_family(lua)?;
    lua.globals().set(
        "GetInboxItemLink",
        lua.create_function(|lua, args: mlua::MultiValue| {
            let index = super::super::binding_abi::number_arg(
                lua,
                args.front().cloned().unwrap_or(mlua::Value::Nil),
                "Usage: GetInboxItemLink(messageIndex, attachIndex)",
            )?
            .wrapping_sub(1) as u32;
            // Native subtracts before truncating the second argument, unlike the first.
            let slot = lua
                .coerce_number(args.get(1).cloned().unwrap_or(mlua::Value::Nil))?
                .map(|n| (n - 1.0) as i64 as i32)
                .unwrap_or(-1);
            let model = lua.app_data_ref::<super::Model>().expect("model");
            let attachment = model
                .wrath_mail
                .as_ref()
                .and_then(|m| m.rows.get(index as usize))
                .and_then(|r| {
                    if slot == -1 {
                        r.attachments
                            .iter()
                            .filter(|a| a.slot < 16 && a.entry != 0)
                            .min_by_key(|a| a.slot)
                    } else if (0..16).contains(&slot) {
                        r.attachments
                            .iter()
                            .find(|a| i32::from(a.slot) == slot && a.entry != 0)
                    } else {
                        None
                    }
                });
            let mut values = mlua::MultiValue::new();
            if let Some(link) = attachment.and_then(|a| a.link.as_ref()) {
                values.push_back(mlua::Value::String(lua.create_string(link)?));
            }
            Ok(values)
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_link_argument_slot_and_zero_return_abi() {
        let mut script = crate::script::UiScript::new().unwrap();
        script.set_wrath_mail(Some(Inbox {
            rows: vec![Row {
                attachments: vec![
                    Attachment {
                        slot: 11,
                        entry: 2589,
                        link: Some("late".into()),
                        ..Default::default()
                    },
                    Attachment {
                        slot: 0,
                        entry: 0,
                        link: Some("empty".into()),
                        ..Default::default()
                    },
                    Attachment {
                        slot: 2,
                        entry: 2590,
                        link: Some("first".into()),
                        ..Default::default()
                    },
                    Attachment {
                        slot: 4,
                        entry: 2591,
                        ..Default::default()
                    },
                    Attachment {
                        slot: 16,
                        entry: 2592,
                        link: Some("outside".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }));
        for args in [
            "1",
            "'1.9'",
            "1,nil",
            "1,false",
            "1,{}",
            "1,'junk'",
            "1,0",
            "1,-0.5",
            "1,4294967296",
        ] {
            let link: String = script
                .lua()
                .load(format!("return GetInboxItemLink({args})"))
                .eval()
                .unwrap();
            assert_eq!(link, "first", "{args}");
        }
        for args in ["1,12", "1,'12.9'", "1.9,12", "1,13.0-0.1"] {
            let link: String = script
                .lua()
                .load(format!("return GetInboxItemLink({args})"))
                .eval()
                .unwrap();
            assert_eq!(link, "late", "{args}");
        }
        for args in ["0", "2", "-1", "1,1", "1,5", "1,17", "1,-1", "1,0.5"] {
            let values: mlua::MultiValue = script
                .lua()
                .load(format!("return GetInboxItemLink({args})"))
                .eval()
                .unwrap();
            assert!(values.is_empty(), "{args}");
        }
        for args in ["", "nil", "false", "{}", "'junk'"] {
            let error = script
                .lua()
                .load(format!("return GetInboxItemLink({args})"))
                .eval::<mlua::MultiValue>()
                .unwrap_err();
            assert!(error
                .to_string()
                .contains("Usage: GetInboxItemLink(messageIndex, attachIndex)"));
        }
        script.set_wrath_mail(None);
        assert!(script
            .lua()
            .load("return GetInboxItemLink(1)")
            .eval::<mlua::MultiValue>()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn first_occupied_unresolved_link_does_not_skip_to_resolved_slot() {
        let mut script = crate::script::UiScript::new().unwrap();
        script.set_wrath_mail(Some(Inbox {
            rows: vec![Row {
                attachments: vec![
                    Attachment {
                        slot: 0,
                        entry: 1,
                        ..Default::default()
                    },
                    Attachment {
                        slot: 1,
                        entry: 2,
                        link: Some("ready".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }));
        assert!(script
            .lua()
            .load("return GetInboxItemLink(1)")
            .eval::<mlua::MultiValue>()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn native_inbox_exposes_total_and_attachment_slot_without_stack_narrowing() {
        let mut script = crate::script::UiScript::new().unwrap();
        script.set_wrath_mail(Some(Inbox {
            total_count: 70,
            rows: vec![Row {
                header: MailInboxRow {
                    subject: "subject".into(),
                    body: Some("inline".into()),
                    item_count: 1,
                    package_icon: Some("NativeIcon".into()),
                    ..Default::default()
                },
                attachments: vec![Attachment {
                    slot: 11,
                    entry: 2589,
                    count: 1000,
                    name: Some("Linen".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }));
        let counts: (u32, u32) = script
            .lua()
            .load("return GetInboxNumItems()")
            .eval()
            .unwrap();
        assert_eq!(counts, (1, 70));
        let result: (String, mlua::Value, u32, u32, mlua::Value) = script
            .lua()
            .load("return GetInboxItem(1,12)")
            .eval()
            .unwrap();
        assert_eq!((result.0, result.2), ("Linen".into(), 1000));
        let (icon, quantity): (String, u32) = script.lua().load(
            "local icon,_,_,_,_,_,_,_,_,_,_,_,_,quantity = GetInboxHeaderInfo(1); return icon,quantity"
        ).eval().unwrap();
        assert_eq!((icon, quantity), ("NativeIcon".into(), 1000));
        let body: String = script.lua().load("return GetInboxText(1)").eval().unwrap();
        assert_eq!(body, "inline");
        assert!(script
            .lua()
            .load("return GetInboxItem(1,13)")
            .eval::<mlua::Value>()
            .unwrap()
            .is_nil());
        script.set_wrath_mail(None);
        assert_eq!(
            script
                .lua()
                .load("return GetInboxNumItems()")
                .eval::<(u32, u32)>()
                .unwrap(),
            (0, 0)
        );
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;
    use crate::script::UiScript;

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        let row = |kind: u8, sender: u64, gm: bool, returned: bool| Row {
            kind,
            sender_guid: sender,
            header: MailInboxRow {
                is_gm: gm,
                was_returned: returned,
                ..Default::default()
            },
            ..Default::default()
        };
        s.set_wrath_mail(Some(Inbox {
            total_count: 5,
            rows: vec![
                row(0, 0x42, false, false),
                row(2, 0x43, false, false),
                row(0, 0x44, true, false),
                row(0, 0x45, false, true),
                row(0, 0x10, false, false),
            ],
        }));
        s.set_wrath_mail_self_guid(0x10);
        s
    }

    #[test]
    fn take_inbox_item_records_the_attachment_slot() {
        let mut s = vm();
        s.run("TakeInboxItem(2, 3) TakeInboxItem(1)").unwrap();
        assert_eq!(s.take_mail_take_items(), vec![(2, Some(3)), (1, None)]);
        assert!(s.run("TakeInboxItem()").is_err());
    }

    #[test]
    fn complaints_gate_record_and_close_the_letter() {
        let mut s = vm();
        let can = |s: &UiScript, i: i64| {
            s.eval::<Option<i64>>(&format!("return CanComplainInboxItem({i})"))
                .unwrap()
                .is_some()
        };
        assert!(can(&s, 1));
        assert!(!can(&s, 2), "not a player's mail");
        assert!(!can(&s, 3), "GM stationery");
        assert!(!can(&s, 4), "returned");
        assert!(!can(&s, 5), "from the player");
        assert!(!can(&s, 9));
        s.run(
            "COMPLAINT_ADDED = 'added' SEEN = {} local f = CreateFrame('Frame') \
             f:RegisterEvent('CLOSE_INBOX_ITEM') f:RegisterEvent('CHAT_MSG_SYSTEM') \
             f:SetScript('OnEvent', function(_, e, a) SEEN[#SEEN + 1] = e .. ':' .. tostring(a) end) \
             ComplainInboxItem(1) ComplainInboxItem(1) ComplainInboxItem(2)",
        )
        .unwrap();
        assert_eq!(
            s.eval::<Vec<String>>("return SEEN").unwrap(),
            vec![
                "CLOSE_INBOX_ITEM:1".to_string(),
                "CHAT_MSG_SYSTEM:added".to_string()
            ]
        );
        assert_eq!(
            s.take_wrath_mail_requests(),
            vec![WrathMailRequest::Complain {
                row: 1,
                sender: 0x42
            }]
        );
        assert!(s.run("CanComplainInboxItem()").is_err());
    }

    #[test]
    fn auto_loot_showing_lock_and_senders() {
        let mut s = vm();
        s.run("AutoLootMailItem(2) AutoLootMailItem(0)").unwrap();
        assert_eq!(
            s.take_wrath_mail_requests(),
            vec![WrathMailRequest::AutoLoot { row: 2 }]
        );
        s.run("SetSendMailShowing(true)").unwrap();
        assert!(s.wrath_mail_send_showing());
        assert!(
            s.run("SetSendMailShowing(1)").is_err(),
            "a real boolean only"
        );

        s.run(
            "SEEN = {} local f = CreateFrame('Frame') \
             f:RegisterEvent('MAIL_UNLOCK_SEND_ITEMS') f:RegisterEvent('MAIL_SEND_INFO_UPDATE') \
             f:SetScript('OnEvent', function(_, e) SEEN[#SEEN + 1] = e end) \
             RespondMailLockSendItem(1, false)",
        )
        .unwrap();
        s.tick(0.0);
        assert_eq!(s.eval::<i64>("return #SEEN").unwrap(), 0, "no lock pending");
        s.set_wrath_mail_send_locked(true);
        s.run("RespondMailLockSendItem(1, false) RespondMailLockSendItem(1, true)")
            .unwrap();
        s.tick(0.0);
        assert_eq!(
            s.eval::<Vec<String>>("return SEEN").unwrap(),
            vec![
                "MAIL_SEND_INFO_UPDATE".to_string(),
                "MAIL_UNLOCK_SEND_ITEMS".to_string()
            ],
            "the answer clears the lock, so the second call does nothing"
        );

        assert_eq!(
            s.eval::<i64>("return select('#', GetLatestThreeSenders())")
                .unwrap(),
            0
        );
        s.set_wrath_mail_latest_senders(vec!["A".into(), "B".into(), "C".into(), "D".into()]);
        assert_eq!(
            s.eval::<Vec<String>>("return {GetLatestThreeSenders()}")
                .unwrap(),
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
    }
}
