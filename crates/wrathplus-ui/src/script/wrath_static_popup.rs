//! Build-12340 globals StaticPopup.lua's dialogs call that the 1.12 surface lacks (one Wow.exe
//! round: `/tmp/azb-ui-popup-native.log`, `/tmp/azb-ui-popup-rest{,2}.log`,
//! `/tmp/azb-ui-gettext-native.log`). The equipment-set dialogs' APIs are Wire B's
//! (`wrath_equipment.rs`).
//!
//! The dialogs' answers go out as [`WrathPopupRequest`]s, labelled capture-only until App sends
//! them; the state the dialogs read is published by the host ([`WrathPopupState`]) and answers the
//! native "nothing pending" shapes until it is. Battle.net bindings answer the closed session; the
//! voice mute list is inert, as voice chat is off.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// One dialog answer for App, in call order. Capture-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathPopupRequest {
    /// `AcceptArenaTeam()` (`0x51ac30` → `0x6d4840`).
    AcceptArenaTeam,
    /// `DeclineArenaTeam()` (`0x51ac60` → `0x6d48b0`).
    DeclineArenaTeam,
    /// `ArenaTeamDisband(team)` (`0x516130`, CMSG `0x355`); 1-based slot, 0 for none.
    ArenaTeamDisband { team: u8 },
    /// `AcceptLevelGrant()` (`0x523360` → `0x521b00`). Declining only clears the pending grant.
    AcceptLevelGrant,
    /// `BattlefieldMgrEntryInviteResponse(id, accept)` (`0x54e160`).
    BattlefieldMgrEntryInvite { id: i32, accept: bool },
    /// `BattlefieldMgrQueueInviteResponse(id, accept)` (`0x54e1c0`).
    BattlefieldMgrQueueInvite { id: i32, accept: bool },
    /// `BuyGuildBankTab()` (`0x5a5d30`): the next tab, App checking the price against money.
    BuyGuildBankTab,
    /// `CancelSummon()` (`0x51b110`, CMSG `0x2AC` with the summoner and 0).
    CancelSummon,
    /// `ComplainChat(lineID)` or `ComplainChat(name [, text])` (`0x50ed00`).
    ComplainChat {
        line_id: Option<i32>,
        name: Option<String>,
        text: Option<String>,
    },
    /// `DeclineInvite("channel")` (`0x4ffcc0` → `0x4fe460(0x410)`): the channel invite's answer,
    /// `CMSG_DECLINE_CHANNEL_INVITE` with the name. Labelled: the name resolver (`0x4fe160`) is
    /// not read, so the argument goes out as given and an empty one sends nothing.
    DeclineInvite { channel: String },
    /// `DepositGuildBankMoney(copper)` (`0x5a5e20`), already capped at the player's money.
    DepositGuildBankMoney { copper: u32 },
    /// `EndBoundTradeable(type)` (`0x5233d0`): "itemenchant", "item" or "spellenchant".
    EndBoundTradeable { kind: String },
    /// `EndRefund(type)` (`0x523370`): 1 item enchant, 2 item.
    EndRefund { kind: i32 },
    /// `ContainerRefundItemPurchase(bag, slot [, isEquipped])` (`0x5d91b0`, CMSG `0x4B4`).
    RefundItemPurchase { bag: i64, slot: u32, equipped: bool },
    /// `GMResponseResolve()` (`0x5ad1b0`).
    GmResponseResolve,
    /// `PlaceGlyphInSocket(socket)` (`0x5b75a0`); 1-based socket.
    PlaceGlyphInSocket { socket: u8 },
    /// `RemoveGlyphFromSocket(socket)` (`0x5b7660`, CMSG `0x48A` with the 0-based socket); only
    /// for a socket the host reports filled.
    RemoveGlyphFromSocket { socket: u8 },
    /// `ReplaceTradeEnchant()` (`0x510b80` → `0x80c5f0`).
    ReplaceTradeEnchant,
    /// `RespondInstanceLock(accept)` (`0x613ed0`, CMSG `0x13F`).
    RespondInstanceLock { accept: bool },
    /// `SetFriendNotes(index | name, notes)` (`0x6b7c90`).
    SetFriendNotes {
        index: Option<u32>,
        name: Option<String>,
        note: String,
    },
    /// `SetLFGBootVote(yes)` (`0x554a50`).
    SetLfgBootVote { yes: bool },
}

/// A pending instance lock: `GetInstanceLockTimeRemaining`'s source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathInstanceLock {
    /// When the lock is forced, on the GetTime clock.
    pub expires_at: f64,
    /// Whether accepting extends an existing lock.
    pub extending: bool,
    pub encounters_completed: u32,
    pub encounters_total: u32,
    /// The lock's completed encounters, one bit per DungeonEncounter.dbc bit index, for
    /// `GetInstanceLockTimeRemainingEncounter`.
    pub completed_mask: u32,
}

/// The state StaticPopup's dialogs read, as the host publishes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathPopupState {
    /// `CannotBeResurrected` (`0x802270`, player flag `+0xf47 & 4`).
    pub cannot_be_resurrected: bool,
    /// `IsFalling` (`0x612430`, movement flags 0x1000 without 0x800).
    pub falling: bool,
    /// `IsOutOfBounds` (`0x6123c0`).
    pub out_of_bounds: bool,
    /// `PlayerCanTeleport` (`0x60d8f0`): none of the blocking movement flags.
    pub can_teleport: bool,
    /// Units controlling another (`UnitIsControlling`, `0x613c90`), by GUID.
    pub controlling: Vec<u64>,
    /// `GetGuildBankTabCost` (`0x5a6b20`): the next tab's price, `None` with no tab to buy.
    pub guild_bank_tab_cost: Option<u32>,
    /// `GetInstanceLockTimeRemaining` (`0x516340`).
    pub instance_lock: Option<WrathInstanceLock>,
    /// Filled glyph sockets, 1-based (`RemoveGlyphFromSocket`'s gate).
    pub filled_glyph_sockets: Vec<u8>,
}

/// The requests and published state.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    requests: Vec<WrathPopupRequest>,
    state: WrathPopupState,
}

impl State {
    /// `IsFalling` as published, for the verbs that refuse while falling.
    pub(crate) fn falling(&self) -> bool {
        self.state.falling
    }
}

impl super::UiScript {
    /// Drain StaticPopup's dialog answers in call order.
    pub fn take_wrath_popup_requests(&mut self) -> Vec<WrathPopupRequest> {
        std::mem::take(&mut self.model_mut().wrath_popup.requests)
    }

    /// Publish the state StaticPopup's dialogs read, whole.
    pub fn set_wrath_popup_state(&mut self, state: WrathPopupState) {
        self.model_mut().wrath_popup.state = state;
    }
}

fn push(lua: &Lua, r: WrathPopupRequest) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .wrath_popup
        .requests
        .push(r);
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

fn truth(args: &MultiValue, i: usize) -> bool {
    super::binding_abi::bool_or_default(args.get(i), false)
}

fn has_player(lua: &Lua) -> bool {
    lua.app_data_ref::<Model>()
        .expect("model app_data")
        .unit_guids
        .player
        != 0
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

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn flag(on: bool) -> Value {
    super::binding_abi::flag(on)
}

/// A simple capture-only verb with no arguments.
fn verb(lua: &Lua, name: &str, r: WrathPopupRequest) -> mlua::Result<()> {
    set(lua, name, move |lua, _| {
        push(lua, r.clone());
        none()
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    use WrathPopupRequest as R;

    // ── arena, level grant, summons, instance locks, GM, trade ──
    set(lua, "AcceptArenaTeam", |lua, _| {
        if has_player(lua) {
            push(lua, R::AcceptArenaTeam);
        }
        none()
    })?;
    set(lua, "DeclineArenaTeam", |lua, _| {
        if has_player(lua) {
            push(lua, R::DeclineArenaTeam);
        }
        none()
    })?;
    set(lua, "ArenaTeamDisband", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: ArenaTeamDisband(team)"));
        };
        let n = n.round_ties_even() as i64;
        let team = if (1..=3).contains(&n) { n as u8 } else { 0 };
        push(lua, R::ArenaTeamDisband { team });
        none()
    })?;
    verb(lua, "AcceptLevelGrant", R::AcceptLevelGrant)?;
    // DeclineLevelGrant() (`0x517d20`) only forgets the pending grant; nothing is sent.
    set(lua, "DeclineLevelGrant", |_, _| none())?;
    verb(lua, "CancelSummon", R::CancelSummon)?;
    verb(lua, "GMResponseResolve", R::GmResponseResolve)?;
    verb(lua, "ReplaceTradeEnchant", R::ReplaceTradeEnchant)?;
    set(lua, "DeclineInvite", |lua, args| {
        let Some(channel) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime("Usage: DeclineInvite(\"channel\")"));
        };
        if !channel.is_empty() {
            push(lua, R::DeclineInvite { channel });
        }
        none()
    })?;
    // PartyLFGStartBackfill() (`0x55a0c0`): the group's dungeon becomes the whole selection and
    // the ordinary join goes out (`wrath_lfg`'s queue).
    set(lua, "PartyLFGStartBackfill", |lua, _| {
        super::wrath_lfg::start_backfill(lua);
        none()
    })?;
    set(lua, "RespondInstanceLock", |lua, args| {
        push(
            lua,
            R::RespondInstanceLock {
                accept: truth(&args, 0),
            },
        );
        none()
    })?;
    set(lua, "SetLFGBootVote", |lua, args| {
        push(
            lua,
            R::SetLfgBootVote {
                yes: truth(&args, 0),
            },
        );
        none()
    })?;
    for (global, entry) in [
        ("BattlefieldMgrEntryInviteResponse", true),
        ("BattlefieldMgrQueueInviteResponse", false),
    ] {
        set(lua, global, move |lua, args| {
            let id = number(lua, &arg(&args, 0)).map_or(0, |n| n.trunc() as i64 as i32);
            let accept = truth(&args, 1);
            push(
                lua,
                if entry {
                    R::BattlefieldMgrEntryInvite { id, accept }
                } else {
                    R::BattlefieldMgrQueueInvite { id, accept }
                },
            );
            none()
        })?;
    }

    // ── guild bank ──
    set(lua, "BuyGuildBankTab", |lua, _| {
        if has_player(lua) {
            push(lua, R::BuyGuildBankTab);
        }
        none()
    })?;
    // DepositGuildBankMoney(copper): more than the player holds is a game error, no request.
    set(lua, "DepositGuildBankMoney", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: DepositGuildBankMoney(money)"));
        };
        let copper = n.round_ties_even() as i64 as u32;
        let money = lua
            .globals()
            .get::<mlua::Function>("GetMoney")?
            .call::<Option<f64>>(())?;
        if money.is_some_and(|m| f64::from(copper) <= m) {
            push(lua, R::DepositGuildBankMoney { copper });
        }
        none()
    })?;
    set(lua, "GetGuildBankTabCost", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        one(match model.wrath_popup.state.guild_bank_tab_cost {
            Some(c) if c != 0 && model.unit_guids.player != 0 => Value::Integer(i64::from(c)),
            _ => Value::Nil,
        })
    })?;

    // ── refunds and bound-trade items ──
    set(lua, "ContainerRefundItemPurchase", |lua, args| {
        let (b, s) = (arg(&args, 0), arg(&args, 1));
        let (Some(bag), Some(slot)) = (number(lua, &b), number(lua, &s)) else {
            return Err(mlua::Error::runtime(
                "Usage: ContainerRefundItemPurchase(index, slot[, isEquipped])",
            ));
        };
        let equipped = number(lua, &arg(&args, 2)).is_some_and(|n| n.trunc() != 0.0);
        push(
            lua,
            R::RefundItemPurchase {
                bag: bag.trunc() as i64,
                slot: slot.trunc() as i64 as u32,
                equipped,
            },
        );
        none()
    })?;
    set(lua, "EndBoundTradeable", |lua, args| {
        let Some(kind) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(r#"Usage: EndBoundTradeable("type")"#));
        };
        push(lua, R::EndBoundTradeable { kind });
        none()
    })?;
    set(lua, "EndRefund", |lua, args| {
        if let Some(n) = number(lua, &arg(&args, 0)) {
            let kind = n.trunc() as i64 as i32;
            if kind == 1 || kind == 2 {
                push(lua, R::EndRefund { kind });
            }
        }
        none()
    })?;

    // ── chat complaints, friends, glyphs ──
    set(lua, "ComplainChat", |lua, args| {
        let first = arg(&args, 0);
        let request =
            match &first {
                Value::Integer(_) | Value::Number(_) => R::ComplainChat {
                    line_id: number(lua, &first).map(|n| n.round_ties_even() as i64 as i32),
                    name: None,
                    text: None,
                },
                Value::String(_) => R::ComplainChat {
                    line_id: None,
                    name: text(lua, &first)?,
                    text: text(lua, &arg(&args, 1))?,
                },
                _ => return Err(mlua::Error::runtime(
                    "Usage: ComplainChat(lineID) or ComplainChat(name) or ComplainChat(name, text)",
                )),
            };
        push(lua, request);
        none()
    })?;
    set(lua, "SetFriendNotes", |lua, args| {
        let Some(note) = text(lua, &arg(&args, 1))? else {
            return Err(mlua::Error::runtime(
                r#"Usage: SetFriendNotes(["name"] or [index], ["notes"])"#,
            ));
        };
        let first = arg(&args, 0);
        let request = match &first {
            Value::Integer(_) | Value::Number(_) => {
                let i = number(lua, &first).unwrap_or(0.0).round_ties_even() as i64;
                // 100 friend rows (`0x6b7c90`).
                if !(1..=100).contains(&i) {
                    return none();
                }
                R::SetFriendNotes {
                    index: Some(i as u32),
                    name: None,
                    note,
                }
            }
            Value::String(_) => R::SetFriendNotes {
                index: None,
                name: text(lua, &first)?,
                note,
            },
            _ => {
                return Err(mlua::Error::runtime(
                    r#"Usage: SetFriendNotes(["name"] or [index], ["notes"])"#,
                ))
            }
        };
        push(lua, request);
        none()
    })?;
    set(lua, "PlaceGlyphInSocket", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: PlaceGlyphInSocket(index)"));
        };
        let n = n.round_ties_even() as i64;
        if (1..=6).contains(&n) && has_player(lua) {
            push(lua, R::PlaceGlyphInSocket { socket: n as u8 });
        }
        none()
    })?;
    set(lua, "RemoveGlyphFromSocket", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: RemoveGlyphFromSocket(index)"));
        };
        let n = n.round_ties_even() as i64;
        let filled = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            (1..=6).contains(&n)
                && model.unit_guids.player != 0
                && model
                    .wrath_popup
                    .state
                    .filled_glyph_sockets
                    .contains(&(n as u8))
        };
        if filled {
            push(lua, R::RemoveGlyphFromSocket { socket: n as u8 });
        }
        none()
    })?;

    // ── Battle.net and voice: inert ──
    for global in [
        "BNDeclineFriendInvite",
        "BNRemoveFriend",
        "BNSetBlocked",
        "BNSetFriendNote",
        "AddMute",
    ] {
        set(lua, global, |_, _| none())?;
    }

    // ── state readers ──
    set(lua, "CannotBeResurrected", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        one(flag(
            model.unit_guids.player != 0 && model.wrath_popup.state.cannot_be_resurrected,
        ))
    })?;
    set(lua, "IsFalling", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        one(flag(
            model.unit_guids.player != 0 && model.wrath_popup.state.falling,
        ))
    })?;
    set(lua, "IsOutOfBounds", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        one(flag(
            model.unit_guids.player != 0 && model.wrath_popup.state.out_of_bounds,
        ))
    })?;
    // PlayerCanTeleport() pushes a boolean (`0x84e4d0`).
    set(lua, "PlayerCanTeleport", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        one(Value::Boolean(model.wrath_popup.state.can_teleport))
    })?;
    set(lua, "UnitIsControlling", |lua, args| {
        let token = text(lua, &arg(&args, 0))?.unwrap_or_default();
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let guid = model.unit_guid(&token);
        one(flag(guid.is_some_and(|g| {
            model.wrath_popup.state.controlling.contains(&g)
        })))
    })?;
    // GetInstanceLockTimeRemaining() → seconds left (whole), extending, encountersCompleted,
    // encountersTotal; 0, false, 0, 0 with nothing pending.
    set(lua, "GetInstanceLockTimeRemaining", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let (secs, extending, done, total) = match &model.wrath_popup.state.instance_lock {
            Some(l) if l.expires_at > model.now => (
                (l.expires_at - model.now).trunc() as i64,
                l.extending,
                l.encounters_completed,
                l.encounters_total,
            ),
            _ => (0, false, 0, 0),
        };
        Ok(MultiValue::from_vec(vec![
            Value::Integer(secs),
            Value::Boolean(extending),
            Value::Integer(i64::from(done)),
            Value::Integer(i64::from(total)),
        ]))
    })?;

    // GetInstanceLockTimeRemainingEncounter(index) (`0x516410` → `0x5538b0`): the current map
    // and difficulty's index-th encounter under the pending lock's mask; three nils past the end
    // or without a lock.
    set(lua, "GetInstanceLockTimeRemainingEncounter", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetInstanceLockTimeRemainingEncounter(encounterIndex)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let hit = model
            .wrath_popup
            .state
            .instance_lock
            .as_ref()
            .filter(|l| l.expires_at > model.now)
            .and_then(|l| {
                let i = usize::try_from(n.trunc() as i64 - 1).ok()?;
                super::wrath_lfg::lock_encounter(&model, i, l.completed_mask)
            });
        Ok(MultiValue::from_vec(match hit {
            Some((name, icon, killed)) => vec![
                Value::String(lua.create_string(&name)?),
                match icon {
                    Some(i) => Value::String(lua.create_string(&i)?),
                    None => Value::Nil,
                },
                Value::Boolean(killed),
            ],
            None => vec![Value::Nil, Value::Nil, Value::Nil],
        }))
    })?;

    // ── loot and strings ──
    // ConfirmLootSlot(slot) (`0x589590`) is LootSlot's confirmation path (same usage line).
    set(lua, "ConfirmLootSlot", |lua, args| {
        let Some(n) = number(lua, &arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: LootSlot(slot)"));
        };
        let slot = n.round_ties_even() as i64;
        if let Ok(slot) = u32::try_from(slot) {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .loot_confirms
                .push(slot);
        }
        none()
    })?;
    // GetText(token [, gender [, ordinal]]) (`0x81b720` → `0x819d40`): the global string, its
    // `_FEMALE` form for gender 3 when the strings carry one, else the token itself. Labelled:
    // the ordinal variant is not resolved.
    set(lua, "GetText", |lua, args| {
        let Some(token) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                r#"Usage: GetText("token" [,gender] [,ordinal])"#,
            ));
        };
        let gender = number(lua, &arg(&args, 1)).map_or(0, |n| n.round_ties_even() as i64);
        let g = lua.globals();
        if gender == 3 {
            if let Value::String(s) = g.get::<Value>(format!("{token}_FEMALE"))? {
                return one(Value::String(s));
            }
        }
        one(match g.get::<Value>(token.as_str())? {
            v @ Value::String(_) => v,
            _ => Value::String(lua.create_string(&token)?),
        })
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    fn vm() -> UiScript {
        let s = UiScript::new().unwrap();
        s.model_mut().unit_guids.player = 0x10;
        s
    }

    #[test]
    fn dialog_answers_go_to_the_outbox() {
        let mut s = vm();
        s.run(
            "AcceptArenaTeam() ArenaTeamDisband(2) DeclineLevelGrant() CancelSummon() \
             RespondInstanceLock(1) SetLFGBootVote() BattlefieldMgrQueueInviteResponse(7, true) \
             ComplainChat(42) ComplainChat('Bob', 'spam') SetFriendNotes(3, 'n') \
             SetFriendNotes(200, 'x') ContainerRefundItemPurchase(0, 4) EndRefund(2) EndRefund(5) \
             PlaceGlyphInSocket(1) RemoveGlyphFromSocket(2) AddMute('x') BNRemoveFriend(1) \
             DeclineInvite('Trade') DeclineInvite('')",
        )
        .unwrap();
        assert_eq!(
            s.take_wrath_popup_requests(),
            vec![
                R::AcceptArenaTeam,
                R::ArenaTeamDisband { team: 2 },
                R::CancelSummon,
                R::RespondInstanceLock { accept: true },
                R::SetLfgBootVote { yes: false },
                R::BattlefieldMgrQueueInvite {
                    id: 7,
                    accept: true
                },
                R::ComplainChat {
                    line_id: Some(42),
                    name: None,
                    text: None
                },
                R::ComplainChat {
                    line_id: None,
                    name: Some("Bob".into()),
                    text: Some("spam".into())
                },
                R::SetFriendNotes {
                    index: Some(3),
                    name: None,
                    note: "n".into()
                },
                R::RefundItemPurchase {
                    bag: 0,
                    slot: 4,
                    equipped: false
                },
                R::EndRefund { kind: 2 },
                R::PlaceGlyphInSocket { socket: 1 },
                R::DeclineInvite {
                    channel: "Trade".into()
                },
            ],
            "an unfilled socket removes nothing; voice and Battle.net are inert"
        );
        assert!(s.run("ComplainChat()").is_err());
        assert!(s.run("ArenaTeamDisband()").is_err());
    }

    use WrathPopupRequest as R;

    #[test]
    fn readers_answer_published_state_and_native_misses() {
        let mut s = vm();
        assert!(s
            .eval::<bool>("return GetGuildBankTabCost() == nil and IsFalling() == nil")
            .unwrap());
        assert_eq!(
            s.eval::<(i64, bool, i64, i64)>("return GetInstanceLockTimeRemaining()")
                .unwrap(),
            (0, false, 0, 0)
        );
        s.model_mut().unit_guids.target = 0x22;
        s.set_now(100.0);
        s.set_wrath_popup_state(WrathPopupState {
            falling: true,
            cannot_be_resurrected: true,
            can_teleport: true,
            controlling: vec![0x22],
            guild_bank_tab_cost: Some(1_000_000),
            instance_lock: Some(WrathInstanceLock {
                expires_at: 160.5,
                extending: true,
                encounters_completed: 2,
                encounters_total: 5,
                completed_mask: 0b11,
            }),
            filled_glyph_sockets: vec![2],
            ..Default::default()
        });
        s.set_wrath_lfg_location(Some((533, 1)));
        s.set_wrath_lfg_encounters(std::sync::Arc::new(vec![
            crate::script::WrathLfgEncounter {
                map: 533,
                difficulty: 1,
                bit: 1,
                name: "Patchwerk".into(),
                icon: None,
            },
            crate::script::WrathLfgEncounter {
                map: 533,
                difficulty: 1,
                bit: 2,
                name: "Grobbulus".into(),
                icon: None,
            },
        ]));
        assert!(s
            .eval::<bool>("return select(1, GetInstanceLockTimeRemainingEncounter(1)) == 'Patchwerk' and select(3, GetInstanceLockTimeRemainingEncounter(1)) == true and select(3, GetInstanceLockTimeRemainingEncounter(2)) == false and select('#', GetInstanceLockTimeRemainingEncounter(3)) == 3")
            .unwrap());
        assert_eq!(
            s.eval::<i64>("return GetGuildBankTabCost()").unwrap(),
            1_000_000
        );
        assert!(s
            .eval::<bool>("return IsFalling() == 1 and CannotBeResurrected() == 1")
            .unwrap());
        assert!(s
            .eval::<bool>("return PlayerCanTeleport() == true and IsOutOfBounds() == nil")
            .unwrap());
        assert!(s
            .eval::<bool>(
                "return UnitIsControlling('target') == 1 and UnitIsControlling('player') == nil"
            )
            .unwrap());
        assert_eq!(
            s.eval::<(i64, bool, i64, i64)>("return GetInstanceLockTimeRemaining()")
                .unwrap(),
            (60, true, 2, 5)
        );
        s.run("RemoveGlyphFromSocket(2)").unwrap();
        assert_eq!(
            s.take_wrath_popup_requests(),
            vec![R::RemoveGlyphFromSocket { socket: 2 }]
        );
    }

    #[test]
    fn get_text_picks_the_female_form_and_falls_back_to_the_token() {
        let s = vm();
        s.run("TITLE = 'Hero' TITLE_FEMALE = 'Heroine'").unwrap();
        assert_eq!(
            s.eval::<String>("return GetText('TITLE', 3)").unwrap(),
            "Heroine"
        );
        assert_eq!(
            s.eval::<String>("return GetText('TITLE', 2)").unwrap(),
            "Hero"
        );
        assert_eq!(s.eval::<String>("return GetText('NOPE')").unwrap(), "NOPE");
        assert!(s.run("GetText()").is_err());
    }

    #[test]
    fn confirm_loot_slot_confirms() {
        let mut s = vm();
        s.run("ConfirmLootSlot(2)").unwrap();
        assert_eq!(s.take_loot_confirms(), vec![2]);
    }
}
