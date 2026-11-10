//! Build-12340 guild bank window (Blizzard_GuildBankUI and the census callers in MerchantFrame
//! and UIParent): slots, logs, tab text and the bank gestures, over the bank Wire B feeds
//! ([`super::WrathGuildBankView`], one Wow.exe round: `/tmp/azb-ui-gbank-native.log`).
//!
//! Gestures go out as [`WrathGuildBankRequest`]s (capture-only until App sends them); the bank
//! state is whatever the host publishes. The selected tab is local, as natively (`0xc1dc0d`).

use mlua::{Lua, MultiValue, Table, Value};

use super::cursor::{CursorGuildBankItem, CursorPayload};
use super::{Model, WrathGuildBankRequest};

/// One occupied bank slot, as `SMSG_GUILD_BANK_LIST` carries it (AC `GuildBankItemInfo`), with
/// the icon and nine-field link App resolves from the item cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildBankSlot {
    /// 0-based, below 98.
    pub slot: u8,
    pub item_id: u32,
    pub count: i32,
    pub random_property_id: i32,
    /// The suffix factor.
    pub random_property_seed: i32,
    pub enchant_id: i32,
    pub charges: i32,
    pub flags: i32,
    /// `(socket index, gem enchant id)` pairs.
    pub gems: Vec<(u8, i32)>,
    pub texture: Option<String>,
    pub link: Option<String>,
    /// Locked by a pending move (`GetGuildBankItemInfo`'s third value).
    pub locked: bool,
}

/// One money-log row (`0xc11070`, 25 rows).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildBankMoneyLog {
    /// 4 deposit, 5 withdraw, 8 withdrawForTab, 9 buyTab, anything else repair.
    pub kind: u8,
    pub player: Option<String>,
    pub copper: i32,
    pub seconds_ago: u32,
}

/// One item-log row (`0xc0f900`, 25 rows per tab).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildBankItemLog {
    /// 1 deposit, 2 withdraw, 3 move out of this tab, 7 move into it; anything else unknown.
    pub kind: u8,
    pub player: Option<String>,
    /// The item's link; `None` (template not cached) answers "".
    pub link: Option<String>,
    pub count: i32,
    /// The other tab of a move, 1-based.
    pub other_tab: u8,
    pub seconds_ago: u32,
}

/// The age split every 12340 log answers with: years of 8760 hours, months of 720, days of 24.
pub(super) fn age(seconds_ago: u32) -> (i64, i64, i64, i64) {
    let hours = i64::from(seconds_ago / 3600);
    let rest = hours % 8760 % 720;
    (hours / 8760, hours % 8760 / 720, rest / 24, rest % 24)
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// A truncated 1-based argument as 0-based (`fistp` under `or 0xc00`, as `0x5a686a` and
/// `0x564fae` read theirs), `None` when not a number or below 1.
fn index(lua: &Lua, v: &Value) -> Option<Option<usize>> {
    let n = lua.coerce_number(v.clone()).ok().flatten()?;
    Some(usize::try_from(i64::from(super::binding_abi::ftol(n)) - 1).ok())
}

fn int(n: impl Into<i64>) -> Value {
    Value::Integer(n.into())
}

fn s(lua: &Lua, v: &Option<String>) -> mlua::Result<Value> {
    Ok(match v {
        Some(v) => Value::String(lua.create_string(v)?),
        None => Value::Nil,
    })
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn push(lua: &Lua, r: WrathGuildBankRequest) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .wrath_friends
        .requests
        .push(r);
}

/// The slot at 0-based `(tab, slot)`.
fn slot_at(model: &Model, tab: usize, slot: usize) -> Option<WrathGuildBankSlot> {
    model
        .wrath_friends
        .bank
        .slots
        .get(tab)?
        .iter()
        .find(|s| usize::from(s.slot) == slot && s.item_id != 0)
        .cloned()
}

/// Whether the cursor holds the bank item at 0-based `(tab, slot)`.
fn held_from(model: &Model, tab: Option<usize>, slot: Option<usize>) -> bool {
    matches!((&model.cursor, tab, slot), (Some(CursorPayload::GuildBankItem(h)), Some(t), Some(s))
        if usize::from(h.tab) == t + 1 && usize::from(h.slot) == s + 1)
}

/// The player's rank's flags on 0-based `tab` include `bit` (`0x5a41c0`: 1 view, 2 deposit).
fn tab_right(model: &Model, tab: usize, bit: u32) -> bool {
    super::wrath_friends_frame::own_bank_rank(model)
        .and_then(|r| r.tabs.get(tab))
        .is_some_and(|(flags, _)| flags & bit != 0)
}

/// A kind-0 message: the global string as a system chat line.
fn system_line(lua: &Lua, key: &str) -> mlua::Result<()> {
    let message = match lua.globals().get::<Value>(key)? {
        Value::String(s) => s.to_string_lossy(),
        _ => key.to_string(),
    };
    super::tick::fire_event_into(
        lua,
        "CHAT_MSG_SYSTEM",
        vec![super::ScriptValue::Str(message)],
    );
    Ok(())
}

/// Whether the player's rank holds `right`, and with `allowance` also a non-zero money word.
fn rank_right(model: &Model, right: u32, allowance: bool) -> bool {
    let guild = &model.guild;
    guild.in_guild
        && guild.rights & right != 0
        && (!allowance
            || super::wrath_friends_frame::own_bank_rank(model)
                .is_some_and(|r| r.money_per_day != 0))
}

/// The item icons the tab-icon chooser lists (`0xbeafa8`, `0xbeafa4` of them), bare file names
/// under `Interface\Icons`; the host builds the list.
#[derive(Default)]
struct MacroItemIcons(Vec<String>);

impl super::UiScript {
    /// Publish the item-icon list `GetNumMacroItemIcons`/`GetMacroItemIconInfo` read, in order.
    pub fn set_macro_item_icons(&mut self, icons: Vec<String>) {
        self.lua.set_app_data(MacroItemIcons(icons));
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    use WrathGuildBankRequest as R;
    lua.set_app_data(MacroItemIcons::default());
    // GetNumMacroItemIcons() (`0x5664e0`) and GetMacroItemIconInfo(index) (`0x564f60`): the
    // count, and the `index`th icon's `Interface\Icons\` path, "" out of range or for an empty
    // name.
    set(lua, "GetNumMacroItemIcons", |lua, _| {
        let n = lua
            .app_data_ref::<MacroItemIcons>()
            .map_or(0, |i| i.0.len());
        Ok(MultiValue::from_vec(vec![int(n as i64)]))
    })?;
    set(lua, "GetMacroItemIconInfo", |lua, args| {
        let Some(i) = index(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: GetMacroItemIconInfo(index)"));
        };
        let path = i
            .and_then(|i| {
                let icons = lua.app_data_ref::<MacroItemIcons>()?;
                icons.0.get(i).filter(|n| !n.is_empty()).cloned()
            })
            .map_or_else(String::new, |n| format!("Interface\\Icons\\{n}"));
        Ok(MultiValue::from_vec(vec![Value::String(
            lua.create_string(&path)?,
        )]))
    })?;
    let two = |lua: &Lua, args: &MultiValue, usage: &'static str| match (
        index(lua, &arg(args, 0)),
        index(lua, &arg(args, 1)),
    ) {
        (Some(t), Some(s)) => Ok((t, s)),
        _ => Err(mlua::Error::runtime(usage)),
    };

    // GetGuildBankItemInfo(tab, slot) (`0x5a6000`) → texture, count, locked; the miss is
    // nil, 0, nil.
    set(lua, "GetGuildBankItemInfo", move |lua, args| {
        let (t, sl) = two(lua, &args, "Usage: GetGuildBankItemInfo(tab, slot)")?;
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let hit = t
            .zip(sl)
            .filter(|(t, _)| *t < 6)
            .and_then(|(t, s)| slot_at(&model, t, s));
        Ok(match hit {
            Some(slot) if model.unit_guids.player != 0 => MultiValue::from_vec(vec![
                s(lua, &slot.texture)?,
                int(slot.count),
                super::binding_abi::flag(slot.locked || held_from(&model, t, sl)),
            ]),
            _ => MultiValue::from_vec(vec![Value::Nil, int(0), Value::Nil]),
        })
    })?;
    // GetGuildBankItemLink(tab, slot) (`0x5a4d30`): nothing until the link is resolved.
    set(lua, "GetGuildBankItemLink", move |lua, args| {
        let (t, sl) = two(lua, &args, "Usage: GetGuildBankItemLink(tab, slot)")?;
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let link = t
            .zip(sl)
            .filter(|(t, _)| *t < 6)
            .and_then(|(t, s)| slot_at(&model, t, s))
            .and_then(|slot| slot.link);
        Ok(match link {
            Some(link) => MultiValue::from_vec(vec![Value::String(lua.create_string(&link)?)]),
            None => MultiValue::new(),
        })
    })?;

    set(lua, "GetCurrentGuildBankTab", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![int(i64::from(
            model.wrath_friends.current_tab,
        ) + 1)]))
    })?;
    // SetCurrentGuildBankTab(tab) (`0x5a43c0`): truncated, 1..6 only.
    set(lua, "SetCurrentGuildBankTab", |lua, args| {
        let Some(n) = lua.coerce_number(arg(&args, 0))? else {
            return Err(mlua::Error::runtime("Usage: SetCurrentTab(tab)"));
        };
        let tab = n.trunc() as i64;
        if (1..=6).contains(&tab) {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .wrath_friends
                .current_tab = (tab - 1) as u8;
        }
        none()
    })?;

    // ── gestures ──
    for (global, usage) in [
        ("QueryGuildBankTab", "Usage: QueryGuildBankTab(tab)"),
        // 12340 raises the log's usage string here too (`0xa161b0`, pushed at `0x5a7129`).
        ("QueryGuildBankText", "Usage: QueryGuildBankLog(tab)"),
        ("QueryGuildBankLog", "Usage: QueryGuildBankLog(tab)"),
    ] {
        set(lua, global, move |lua, args| {
            let Some(t) = index(lua, &arg(&args, 0)) else {
                return Err(mlua::Error::runtime(usage));
            };
            if let Some(tab) = t.filter(|t| *t < 7) {
                let tab = tab as u8 + 1;
                push(
                    lua,
                    match global {
                        "QueryGuildBankTab" => R::QueryTab { tab },
                        "QueryGuildBankText" => R::QueryText { tab },
                        _ => R::QueryLog { tab },
                    },
                );
            }
            none()
        })?;
    }
    // PickupGuildBankItem(tab, slot) (`0x5a6200`), for a player: a held bag item deposits
    // (`0x5a50c0`, App's half); a held bank item moves (`0x5a5410`), refused with
    // ERR_GUILD_PERMISSIONS (message 0x5f, kind 0) without deposit rights on the target tab; an
    // empty cursor takes the slot's item as mode 11 (`0x520d30`), locking it client-side.
    set(lua, "PickupGuildBankItem", move |lua, args| {
        let (t, sl) = two(lua, &args, "Usage: PickupGuildBankItem(tab, slot)")?;
        let Some((t, s)) = t.zip(sl).filter(|(t, s)| *t < 6 && *s < 98) else {
            return none();
        };
        let (tab, slot) = (t as u8 + 1, s as u8 + 1);
        let mut refused = false;
        {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.unit_guids.player == 0 {
                return none();
            }
            match model.cursor.take() {
                Some(CursorPayload::Item(held)) => {
                    model.cursor = Some(CursorPayload::Item(held));
                    model
                        .wrath_friends
                        .requests
                        .push(R::PickupItem { tab, slot });
                }
                Some(CursorPayload::GuildBankItem(held)) => {
                    if held.tab == tab && held.slot == slot {
                        super::cursor::queue_cursor_update(&mut model);
                    } else if !tab_right(&model, t, 2) {
                        model.cursor = Some(CursorPayload::GuildBankItem(held));
                        refused = true;
                    } else {
                        model.wrath_friends.requests.push(R::MoveItem {
                            from_tab: held.tab,
                            from_slot: held.slot,
                            to_tab: tab,
                            to_slot: slot,
                            count: held.count,
                        });
                        super::cursor::queue_cursor_update(&mut model);
                    }
                }
                None => {
                    if let Some(item) = slot_at(&model, t, s).filter(|i| !i.locked) {
                        model.cursor = Some(CursorPayload::GuildBankItem(CursorGuildBankItem {
                            tab,
                            slot,
                            item_id: item.item_id,
                            texture: item.texture,
                            link: item.link,
                            count: None,
                        }));
                        super::cursor::queue_cursor_update(&mut model);
                    }
                }
                other => model.cursor = other,
            }
        }
        if refused {
            system_line(lua, "ERR_GUILD_PERMISSIONS")?;
        }
        none()
    })?;
    set(lua, "AutoStoreGuildBankItem", move |lua, args| {
        let (t, sl) = two(lua, &args, "Usage: AutoStoreGuildBankItem(tab, slot)")?;
        if let Some((tab, slot)) = t.zip(sl).filter(|(t, s)| *t < 6 && *s < 98) {
            push(
                lua,
                R::AutoStoreItem {
                    tab: tab as u8 + 1,
                    slot: slot as u8 + 1,
                },
            );
        }
        none()
    })?;
    // SplitGuildBankItem(tab, slot, amount) (`0x5a6600`): 1..stack of an unlocked slot goes on the
    // cursor as mode 11 with that count, the whole stack as a plain pickup; no request.
    set(lua, "SplitGuildBankItem", move |lua, args| {
        let (t, sl) = two(lua, &args, "Usage: SplitGuildBankItem(tab, slot, amount)")?;
        let Some(n) = lua.coerce_number(arg(&args, 2))? else {
            return Err(mlua::Error::runtime(
                "Usage: SplitGuildBankItem(tab, slot, amount)",
            ));
        };
        let amount = n.trunc() as i64;
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        if model.unit_guids.player == 0 {
            return none();
        }
        let hit = t
            .zip(sl)
            .filter(|(t, s)| *t < 6 && *s < 98)
            .and_then(|(t, s)| slot_at(&model, t, s).map(|slot| (t, s, slot)))
            .filter(|(_, _, slot)| !slot.locked && amount > 0 && amount <= i64::from(slot.count));
        if let Some((t, s, item)) = hit {
            let count = (amount < i64::from(item.count)).then_some(amount as u32);
            model.cursor = Some(CursorPayload::GuildBankItem(CursorGuildBankItem {
                tab: t as u8 + 1,
                slot: s as u8 + 1,
                item_id: item.item_id,
                texture: item.texture,
                link: item.link,
                count,
            }));
            super::cursor::queue_cursor_update(&mut model);
        }
        none()
    })?;
    set(lua, "SetGuildBankText", |lua, args| {
        let (Some(t), Some(text)) = (
            index(lua, &arg(&args, 0)),
            lua.coerce_string(arg(&args, 1))?
                .map(|s| s.to_string_lossy()),
        ) else {
            return Err(mlua::Error::runtime("Usage: SetGuildBankText(tab, text)"));
        };
        if let Some(tab) = t.filter(|t| *t < 6) {
            push(
                lua,
                R::SetText {
                    tab: tab as u8 + 1,
                    text,
                },
            );
        }
        none()
    })?;
    // SetGuildBankTabInfo(tab, name, icon) (`0x5a6a00`).
    set(lua, "SetGuildBankTabInfo", |lua, args| {
        let Some(t) = index(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: SetGuildBankTabInfo(tab, name, iconIndex)",
            ));
        };
        let name = lua
            .coerce_string(arg(&args, 1))?
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let icon = lua
            .coerce_string(arg(&args, 2))?
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        if let Some(tab) = t.filter(|t| *t < 6) {
            push(
                lua,
                R::SetTabInfo {
                    tab: tab as u8 + 1,
                    name,
                    icon,
                },
            );
        }
        none()
    })?;
    set(lua, "CloseGuildBankFrame", |lua, _| {
        push(lua, R::Close);
        none()
    })?;

    // ── logs and text ──
    set(lua, "GetNumGuildBankMoneyTransactions", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![int(
            model.wrath_friends.bank.money_log.len().min(25) as i64,
        )]))
    })?;
    // GetGuildBankMoneyTransaction(index) (`0x5a4e50`) → type, name, amount, years, months,
    // days, hours.
    set(lua, "GetGuildBankMoneyTransaction", |lua, args| {
        let Some(i) = index(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetGuildBankMoneyTransaction(index)",
            ));
        };
        let row = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            i.filter(|i| *i < 25)
                .and_then(|i| model.wrath_friends.bank.money_log.get(i).cloned())
                .unwrap_or_default()
        };
        let kind = match row.kind {
            4 => "deposit",
            5 => "withdraw",
            8 => "withdrawForTab",
            9 => "buyTab",
            _ => "repair",
        };
        let (y, m, d, h) = age(row.seconds_ago);
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(kind)?),
            s(lua, &row.player)?,
            int(row.copper),
            int(y),
            int(m),
            int(d),
            int(h),
        ]))
    })?;
    set(lua, "GetNumGuildBankTransactions", |lua, args| {
        let Some(t) = index(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetNumGuildBankTransactions(tab)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let n = t
            .filter(|t| *t < 7)
            .and_then(|t| model.wrath_friends.bank.tab_logs.get(t))
            .map_or(0, |l| l.len().min(25));
        Ok(MultiValue::from_vec(vec![int(n as i64)]))
    })?;
    // GetGuildBankTransaction(tab, index) (`0x5a6dc0`) → type, name, itemLink, count, then for a
    // move the source and destination tabs, then years, months, days, hours.
    set(lua, "GetGuildBankTransaction", move |lua, args| {
        let (t, i) = two(lua, &args, "Usage: GetGuildBankTransaction(tab, index)")?;
        let (row, has_player) = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let row = t
                .zip(i)
                .filter(|(t, i)| *t < 7 && *i < 25)
                .and_then(|(t, i)| model.wrath_friends.bank.tab_logs.get(t)?.get(i).cloned());
            (row, model.unit_guids.player != 0)
        };
        let (Some(row), true) = (row, has_player) else {
            return none();
        };
        let tab = t.unwrap_or(0) as i64 + 1;
        let kind = match row.kind {
            1 => "deposit",
            2 => "withdraw",
            3 | 7 => "move",
            _ => "unknown",
        };
        let mut out = vec![
            Value::String(lua.create_string(kind)?),
            s(lua, &row.player)?,
            Value::String(lua.create_string(row.link.as_deref().unwrap_or(""))?),
            int(row.count),
        ];
        match row.kind {
            3 => out.extend([int(tab), int(i64::from(row.other_tab))]),
            7 => out.extend([int(i64::from(row.other_tab)), int(tab)]),
            _ => out.extend([Value::Nil, Value::Nil]),
        }
        let (y, m, d, h) = age(row.seconds_ago);
        out.extend([int(y), int(m), int(d), int(h)]);
        Ok(MultiValue::from_vec(out))
    })?;
    // GetGuildBankText(tab) (`0x5a45e0`): a purchased tab's text, nil otherwise.
    set(lua, "GetGuildBankText", |lua, args| {
        let Some(t) = index(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: GetGuildBankText(tab)"));
        };
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let bank = &model.wrath_friends.bank;
        let text = t
            .filter(|t| *t < 6 && *t < bank.tabs.len())
            .map(|t| bank.tab_text.get(t).cloned().unwrap_or_default());
        Ok(MultiValue::from_vec(vec![s(lua, &text)?]))
    })?;

    // ── rights ──
    set(lua, "GetGuildBankWithdrawMoney", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![Value::Number(
            model.wrath_friends.bank.withdraw_money_remaining as f64,
        )]))
    })?;
    // CanWithdrawGuildBankMoney() (`0x5a5f60`): the withdraw-gold right (0x80000) and a non-zero
    // money word (`0x5a5fa2`). CanGuildBankRepair() (`0x5cc200`): the repair right (0x40000) alone
    // (`0x5cc23a`).
    for (global, right, allowance) in [
        ("CanWithdrawGuildBankMoney", 0x80000u32, true),
        ("CanGuildBankRepair", 0x40000, false),
    ] {
        set(lua, global, move |lua, _| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(MultiValue::from_vec(vec![super::binding_abi::flag(
                rank_right(&model, right, allowance),
            )]))
        })?;
    }
    // CanEditGuildTabInfo(tab) (`0x5cc290`): flag 0x4 (update text) of the player's own rank on
    // the truncated tab (`0x5cc325`); nil with no player or a tab past six.
    set(lua, "CanEditGuildTabInfo", |lua, args| {
        let tab =
            super::binding_abi::ftol_arg(lua, arg(&args, 0), "Usage: CanEditGuildTabInfo(tab)")?;
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let tab = usize::try_from(i64::from(tab) - 1).ok().filter(|t| *t < 6);
        Ok(MultiValue::from_vec(vec![super::binding_abi::flag(
            model.unit_guids.player != 0 && tab.is_some_and(|t| tab_right(&model, t, 4)),
        )]))
    })?;
    // GameTooltip:SetGuildBankItem(tab, slot) (`0x6304a0`): the slot's item, its suffix, enchant
    // and gems carried by the slot's link; an empty slot shows nothing. Labelled: the reference
    // builds the instance tooltip from the slot's fields (`0x6277f0`), the same ones the link
    // encodes.
    let tooltip: Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    tooltip.set(
        "SetGuildBankItem",
        lua.create_function(move |lua, args: MultiValue| {
            let this = match args.front() {
                Some(Value::Table(t)) => t.clone(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "Invalid tab or slot in SetGuildBankItem",
                    ))
                }
            };
            let rest: MultiValue = args.iter().skip(1).cloned().collect();
            let (Some(t), Some(sl)) = two(lua, &rest, "Invalid tab or slot in SetGuildBankItem")?
            else {
                return Ok(());
            };
            let link = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                (t < 6)
                    .then(|| slot_at(&model, t, sl))
                    .flatten()
                    .and_then(|s| s.link)
            };
            if let Some(link) = link {
                mlua::ObjectLike::call_method::<()>(&this, "SetHyperlink", link)?;
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_guild_bank_tests.rs"]
mod tests;
