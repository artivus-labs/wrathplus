//! Build-12340 globals FriendsFrame.lua (the friends, ignore/mute, guild-control and guild-log
//! panes) and MoneyFrame.lua's guild-bank money calls need (one Wow.exe round:
//! `/tmp/azb-ui-friends-native.log`).
//!
//! - Battle.net: every binding asks for a connected session before its usage line, and a 3.3.5a
//!   realm never opens one, so they all do nothing.
//! - The voice mute list: voice chat is off, so it is empty (`GetNumMutes` 0 with no voice
//!   system, `0x6b45c0`).
//! - The guild bank's tabs, money and per-rank rights, and the guild event log, are host-published
//!   ([`WrathGuildBankView`], [`WrathGuildEvent`]); edits to the rank being staged
//!   (`GuildControlSetRank`) change the staged rights, which App sends with the rank save.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// One purchased guild-bank tab as the player sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildBankTab {
    /// The tab's name; empty answers `GUILDBANK_TAB_NUMBER` with the tab's number.
    pub name: String,
    /// The icon's file name under `Interface\Icons`; empty is the question mark.
    pub icon: String,
}

/// One rank's row of 12340's rank table (`0xc21e60`, stride 0x38).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildBankRankRights {
    /// Copper per day (`GetGuildBankWithdrawLimit` answers it in gold).
    pub money_per_day: u32,
    /// Per tab: flags (1 view, 2 deposit, 4 update text, `0xc21e68`) and the item limit
    /// (`0xc21e80`).
    pub tabs: [(u32, u32); 6],
}

/// The guild bank as the host publishes it, vault open or not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildBankView {
    /// The vault is open: the bank guid (`0xc1dc10`) is set.
    pub open: bool,
    /// Copper in the bank (`0xc1dc20`).
    pub money: u64,
    /// Purchased tabs in order (`0xc1dc0c` of them).
    pub tabs: Vec<WrathGuildBankTab>,
    /// Item withdrawals the player has left today (`0xc1dc30`).
    pub remaining_withdrawals: u32,
    /// Each rank's rights, rank 0 (the guild master) first, from the roster and our own row from
    /// `MSG_GUILD_PERMISSIONS`; the rank being edited is staged from these.
    pub rank_rights: Vec<WrathGuildBankRankRights>,
    /// Per purchased tab, its occupied slots (`super::wrath_guild_bank`).
    pub slots: Vec<Vec<super::wrath_guild_bank::WrathGuildBankSlot>>,
    /// The money log, newest first (25 rows natively).
    pub money_log: Vec<super::wrath_guild_bank::WrathGuildBankMoneyLog>,
    /// Per tab, its item log, newest first (25 rows natively).
    pub tab_logs: Vec<Vec<super::wrath_guild_bank::WrathGuildBankItemLog>>,
    /// Per tab, its info text.
    pub tab_text: Vec<String>,
    /// Copper the player may still withdraw today (`0xc1dc34`); -1 is unlimited.
    pub withdraw_money_remaining: i64,
}

/// One guild event-log row (`0xc211e0`, stride 8 dwords).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathGuildEvent {
    /// 1 invite, 2 join, 3 promote, 4 demote, 5 remove, 6 quit; anything else is "none".
    pub kind: u8,
    pub player1: Option<String>,
    pub player2: Option<String>,
    /// The rank name for a promote or demote.
    pub rank: Option<String>,
    /// Seconds since the event.
    pub seconds_ago: u32,
}

/// One guild-bank intent for App. Capture-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathGuildBankRequest {
    /// `PickupGuildBankMoney(copper)` (`0x5a4330`): onto the cursor, at most the bank's money.
    PickupMoney { copper: u32 },
    /// `SummonFriend(name)` (`0x523260`): a recruited friend's summon.
    SummonFriend { name: String },
    /// `QueryGuildBankTab(tab)` (`0x5a5ce0`), 1-based.
    QueryTab { tab: u8 },
    /// `QueryGuildBankText(tab)` (`0x5a7110`).
    QueryText { tab: u8 },
    /// `QueryGuildBankLog(tab)` (`0x5a6d50`); tab 7 is the money log.
    QueryLog { tab: u8 },
    /// `PickupGuildBankItem(tab, slot)` (`0x5a6200`), both 1-based, with a bag item on the cursor:
    /// the deposit (`0x5a50c0`). App reads the held item and clears the cursor.
    PickupItem { tab: u8, slot: u8 },
    /// `PickupGuildBankItem` with a bank item on the cursor (mode 11): the move `0x5a5410`, all
    /// 1-based; `count` the split amount, `None` the whole stack.
    MoveItem {
        from_tab: u8,
        from_slot: u8,
        to_tab: u8,
        to_slot: u8,
        count: Option<u32>,
    },
    /// A held bank item dropped on a bag slot (`PickupContainerItem`, `0x5d7ff0`'s mode-11 arm,
    /// withdraw `0x6db880`).
    DropOnBag {
        tab: u8,
        slot: u8,
        bag: i64,
        bag_slot: u32,
        count: Option<u32>,
    },
    /// A held bank item dropped on an equipment slot (`PickupInventoryItem`; labelled: its
    /// mode-11 arm `0x5e85d0` is unread).
    DropOnEquip {
        tab: u8,
        slot: u8,
        equip_slot: u32,
        count: Option<u32>,
    },
    /// `AutoStoreGuildBankItem(tab, slot)` (`0x5a63f0`): withdraw into the bags.
    AutoStoreItem { tab: u8, slot: u8 },
    /// `SetGuildBankText(tab, text)` (`0x5a7180`).
    SetText { tab: u8, text: String },
    /// `SetGuildBankTabInfo(tab, name, icon)` (`0x5a6a00`).
    SetTabInfo { tab: u8, name: String, icon: String },
    /// `CloseGuildBankFrame()` (`0x5a4500`).
    Close,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    pub(super) bank: WrathGuildBankView,
    events: Vec<WrathGuildEvent>,
    /// The staged rights of the rank being edited, by 0-based rank.
    staged: Vec<WrathGuildBankRankRights>,
    pub(super) requests: Vec<WrathGuildBankRequest>,
    /// The selected bank tab, 0-based (`0xc1dc0d`).
    pub(super) current_tab: u8,
}

impl super::UiScript {
    /// Publish the guild bank whole; the staged rank rights restart when its `rank_rights` move,
    /// as a packet overwrites the edits 12340's setters made in place.
    pub fn set_wrath_guild_bank(&mut self, bank: WrathGuildBankView) {
        let mut model = self.model_mut();
        if model.wrath_friends.bank.rank_rights != bank.rank_rights {
            model.wrath_friends.staged = bank.rank_rights.clone();
        }
        model.wrath_friends.bank = bank;
    }

    /// Publish the guild event log, newest first as the server sends it.
    pub fn set_wrath_guild_events(&mut self, events: Vec<WrathGuildEvent>) {
        self.model_mut().wrath_friends.events = events;
    }

    /// The staged bank rights of 0-based `rank`, the wire's rank id, for App's rank save
    /// (`GuildControlSaveRank`).
    pub fn wrath_guild_bank_staged_rights(&self, rank: u32) -> Option<WrathGuildBankRankRights> {
        let model = self.model_ref();
        let rank = usize::try_from(rank).ok()?;
        model.wrath_friends.staged.get(rank).cloned()
    }

    /// Drain the guild-bank intents in call order.
    pub fn take_wrath_guild_bank_requests(&mut self) -> Vec<WrathGuildBankRequest> {
        std::mem::take(&mut self.model_mut().wrath_friends.requests)
    }
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn number(lua: &Lua, v: &Value) -> Option<f64> {
    lua.coerce_number(v.clone()).ok().flatten()
}

fn text(lua: &Lua, v: &Value) -> mlua::Result<Option<String>> {
    Ok(lua.coerce_string(v.clone())?.map(|s| s.to_string_lossy()))
}

fn flag(on: bool) -> Value {
    super::binding_abi::flag(on)
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

/// The player's own rank's row of the rank table, which the bank predicates read.
pub(super) fn own_bank_rank(model: &Model) -> Option<&WrathGuildBankRankRights> {
    let rank = usize::try_from(model.guild.rank_index).ok()?;
    model.wrath_friends.bank.rank_rights.get(rank)
}

/// The 0-based rank `GuildControlSetRank` loaded, if any.
fn edited_rank(model: &Model) -> Option<usize> {
    usize::try_from(model.guild_control.rank.checked_sub(1)?).ok()
}

/// The global string `key`, or the key itself when the strings are not loaded.
fn global_string(lua: &Lua, key: &str) -> mlua::Result<String> {
    Ok(match lua.globals().get::<Value>(key)? {
        Value::String(v) => v.to_string_lossy(),
        _ => key.to_string(),
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // ── Battle.net and the voice mute list: inert ──
    for global in [
        "BNAcceptFriendInvite",
        "BNGetBlockedInfo",
        "BNGetBlockedToonInfo",
        "BNGetFOFInfo",
        "BNGetFriendToonInfo",
        "BNGetNumFOF",
        "BNGetNumFriendToons",
        "BNGetSelectedBlock",
        "BNGetSelectedFriend",
        "BNGetSelectedToonBlock",
        "BNRequestFOFInfo",
        "BNSendFriendInvite",
        "BNSendFriendInviteByID",
        "BNSetCustomMessage",
        "BNSetSelectedBlock",
        "BNSetSelectedFriend",
        "BNSetSelectedToonBlock",
        "DelMute",
    ] {
        set(lua, global, |_, _| none())?;
    }
    set(lua, "GetNumMutes", |_, _| {
        Ok(MultiValue::from_vec(vec![int(0)]))
    })?;
    // GetMuteName(index) (`0x6b4760`): an empty list answers the localized UNKNOWN.
    set(lua, "GetMuteName", |lua, args| {
        if number(lua, &arg(&args, 0)).is_none() {
            return Err(mlua::Error::runtime("Usage: GetMuteName(index)"));
        }
        let unknown = global_string(lua, "UNKNOWN")?;
        Ok(MultiValue::from_vec(vec![Value::String(
            lua.create_string(&unknown)?,
        )]))
    })?;
    // GetSelectedMute() (`0x6b4a40`): the selection's index + 1, 0 with none.
    set(lua, "GetSelectedMute", |_, _| {
        Ok(MultiValue::from_vec(vec![int(0)]))
    })?;
    set(lua, "SetSelectedMute", |lua, args| {
        if number(lua, &arg(&args, 0)).is_none() {
            return Err(mlua::Error::runtime("Usage: SetSelectedMute(index)"));
        }
        none()
    })?;

    // ── guild bank ──
    set(lua, "GetNumGuildBankTabs", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![int(
            model.wrath_friends.bank.tabs.len() as i64,
        )]))
    })?;
    // GetGuildBankTabInfo(tab) (`0x5a67f0`) → name, icon, isViewable, canDeposit,
    // numWithdrawals, remainingWithdrawals; the miss is nil, nil, nil, nil, 0, 0.
    set(lua, "GetGuildBankTabInfo", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: GetGuildBankTabInfo(tab)"));
        };
        let tab = n.round_ties_even() as i64;
        let (row, rights, remaining, has_player) = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let bank = &model.wrath_friends.bank;
            let i = usize::try_from(tab - 1).ok().filter(|i| *i < 6);
            let row = i.and_then(|i| bank.tabs.get(i).cloned());
            // The own rank's tab words (`0x5a41c0`, `0x5a41f0`), not the vault's.
            let rights = i
                .zip(own_bank_rank(&model))
                .map_or((0, 0), |(i, r)| r.tabs[i]);
            (
                row,
                rights,
                bank.remaining_withdrawals,
                model.unit_guids.player != 0,
            )
        };
        if !has_player {
            return none();
        }
        let Some(t) = row else {
            return Ok(MultiValue::from_vec(vec![
                Value::Nil,
                Value::Nil,
                Value::Nil,
                Value::Nil,
                int(0),
                int(0),
            ]));
        };
        let name = if t.name.is_empty() {
            global_string(lua, "GUILDBANK_TAB_NUMBER")?.replacen("%d", &tab.to_string(), 1)
        } else {
            t.name.clone()
        };
        let icon = if t.icon.is_empty() {
            "INV_Misc_QuestionMark"
        } else {
            t.icon.as_str()
        };
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(&name)?),
            Value::String(lua.create_string(format!("Interface\\Icons\\{icon}"))?),
            flag(rights.0 & 1 != 0),
            flag(rights.0 & 2 != 0),
            int(rights.1),
            int(remaining),
        ]))
    })?;
    set(lua, "GetGuildBankMoney", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![Value::Number(
            model.wrath_friends.bank.money as f64,
        )]))
    })?;
    set(lua, "PickupGuildBankMoney", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: PickupGuildBankMoney(amount)"));
        };
        let copper = n.round_ties_even() as i64 as u32;
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        if copper != 0 && u64::from(copper) <= model.wrath_friends.bank.money {
            model
                .wrath_friends
                .requests
                .push(WrathGuildBankRequest::PickupMoney { copper });
        }
        none()
    })?;
    // GetGuildBankTabPermissions(tab) (`0x5cb560`) → canView, canDeposit, canUpdateText,
    // numWithdrawals for the rank being edited.
    set(lua, "GetGuildBankTabPermissions", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetGuildBankTabPermissions(tab)",
            ));
        };
        let tab = n.round_ties_even() as i64;
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let (flags, withdrawals) = edited_rank(&model)
            .and_then(|r| model.wrath_friends.staged.get(r))
            .zip(usize::try_from(tab - 1).ok().filter(|i| *i < 6))
            .map_or((0, 0), |(rights, t)| rights.tabs[t]);
        Ok(MultiValue::from_vec(vec![
            flag(flags & 1 != 0),
            flag(flags & 2 != 0),
            flag(flags & 4 != 0),
            int(withdrawals),
        ]))
    })?;
    // GetGuildBankWithdrawLimit() (`0x5cb700`): the edited rank's signed money word, divided by
    // 10000 only when not negative (`0x5cb71e`), so the guild master's unlimited word answers -1.
    set(lua, "GetGuildBankWithdrawLimit", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let word = edited_rank(&model)
            .and_then(|r| model.wrath_friends.staged.get(r))
            .map_or(0, |r| r.money_per_day as i32);
        Ok(MultiValue::from_vec(vec![int(if word >= 0 {
            word / 10_000
        } else {
            word
        })]))
    })?;
    // SetGuildBankTabPermissions(tab, index, enabled) (`0x5cb470`): index 1 view, 2 deposit,
    // 3 update text, on the edited rank's staged rights.
    set(lua, "SetGuildBankTabPermissions", |lua, args| {
        let (Some(t), Some(i)) = (number(lua, &arg(&args, 0)), number(lua, &arg(&args, 1))) else {
            return Err(mlua::Error::runtime(
                "Usage: SetGuildTabPermissions(tab, index, enabled)",
            ));
        };
        let enabled = super::binding_abi::bool_or_default(args.get(2), false);
        let mask = match i.trunc() as i64 {
            1 => 1,
            2 => 2,
            3 => 4,
            _ => 0,
        };
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let tab = usize::try_from(t.round_ties_even() as i64 - 1)
            .ok()
            .filter(|t| *t < 6);
        if let (Some(r), Some(tab)) = (edited_rank(&model), tab) {
            if let Some(rights) = model.wrath_friends.staged.get_mut(r) {
                if enabled {
                    rights.tabs[tab].0 |= mask;
                } else {
                    rights.tabs[tab].0 &= !mask;
                }
            }
        }
        none()
    })?;
    // SetGuildBankTabWithdraw(tab, amount) (`0x5cb760`): at most 100000 a day.
    set(lua, "SetGuildBankTabWithdraw", |lua, args| {
        let (Some(t), Some(a)) = (number(lua, &arg(&args, 0)), number(lua, &arg(&args, 1))) else {
            return Err(mlua::Error::runtime(
                "Usage: SetGuildBankTabWithdraw(tab, amount)",
            ));
        };
        let amount = (a.trunc().max(0.0) as u64).min(100_000) as u32;
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let tab = usize::try_from(t.round_ties_even() as i64 - 1)
            .ok()
            .filter(|t| *t < 6);
        if let (Some(r), Some(tab)) = (edited_rank(&model), tab) {
            if let Some(rights) = model.wrath_friends.staged.get_mut(r) {
                rights.tabs[tab].1 = amount;
            }
        }
        none()
    })?;
    // SetGuildBankWithdrawLimit(amount) (`0x5cb6a0`): the truncated gold figure, stored raw; an
    // unsigned compare caps it at 1000000000 (`0x5cb6d9`), so a negative one lands there too.
    set(lua, "SetGuildBankWithdrawLimit", |lua, args| {
        let Some(a) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: Script_SetGuildBankWithdrawLimit(amount)",
            ));
        };
        let amount = (super::binding_abi::ftol(a) as u32).min(1_000_000_000);
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        if let Some(r) = edited_rank(&model) {
            if let Some(rights) = model.wrath_friends.staged.get_mut(r) {
                rights.money_per_day = amount;
            }
        }
        none()
    })?;
    // GetCoinText(copper [, separator]) (`0x510c60`): the non-zero gold, silver and copper parts
    // through GOLD_AMOUNT/SILVER_AMOUNT/COPPER_AMOUNT, joined by the separator (", "). Labelled:
    // the formatter `0x7e7d80` was not decompiled; zero answers "0 Copper".
    set(lua, "GetCoinText", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetCoinText(amount [,separator])",
            ));
        };
        let sep = text(lua, &arg(&args, 1))?.unwrap_or_else(|| ", ".to_string());
        let copper = n.round_ties_even().max(0.0) as u64;
        let mut parts = Vec::new();
        for (key, value) in [
            ("GOLD_AMOUNT", copper / 10_000),
            ("SILVER_AMOUNT", copper / 100 % 100),
            ("COPPER_AMOUNT", copper % 100),
        ] {
            if value > 0 || (key == "COPPER_AMOUNT" && parts.is_empty()) {
                parts.push(global_string(lua, key)?.replacen("%d", &value.to_string(), 1));
            }
        }
        Ok(MultiValue::from_vec(vec![Value::String(
            lua.create_string(parts.join(&sep))?,
        )]))
    })?;

    // ── guild event log ──
    set(lua, "GetNumGuildEvents", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![int(
            model.wrath_friends.events.len() as i64,
        )]))
    })?;
    // GetGuildEventInfo(index) (`0x5ccd90`) → type, player1, player2, rank, then the time since
    // as years, months, days, hours (the bank logs' 365/30-day split, `0x5a4e50`).
    set(lua, "GetGuildEventInfo", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: GetGuildEventInfo(index)"));
        };
        let row = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            usize::try_from(n.round_ties_even() as i64 - 1)
                .ok()
                .and_then(|i| model.wrath_friends.events.get(i).cloned())
        };
        let Some(e) = row else {
            return none();
        };
        let kind = match e.kind {
            1 => "invite",
            2 => "join",
            3 => "promote",
            4 => "demote",
            5 => "remove",
            6 => "quit",
            _ => "none",
        };
        let (years, months, days, hours) = super::wrath_guild_bank::age(e.seconds_ago);
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(kind)?),
            s(lua, &e.player1)?,
            s(lua, &e.player2)?,
            s(lua, &e.rank)?,
            int(years),
            int(months),
            int(days),
            int(hours),
        ]))
    })?;

    // ── recruit-a-friend ──
    set(lua, "SummonFriend", |lua, args| {
        if let Some(name) = text(lua, &arg(&args, 0))?.filter(|n| !n.is_empty()) {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .wrath_friends
                .requests
                .push(WrathGuildBankRequest::SummonFriend { name });
        }
        none()
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_friends_frame_tests.rs"]
mod tests;
