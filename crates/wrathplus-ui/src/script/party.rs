//! The party and raid bindings. The app pushes a roster snapshot ([`UiScript::set_party`]) that
//! the getters read; the verbs queue a [`PartyRequest`] the app drains
//! ([`UiScript::take_party_requests`]) into the send. Per-member game state is the unit snapshot
//! under `party1`..`party4` ([`super::unit::UnitState`]).
//!
//! The raid roster ([`PartyState::raid`]) is one list, the player included, because the
//! reference's count (`0xb713e0`) bounds the very array `GetRaidRosterInfo` indexes (`0xb712a8`).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::number_arg;
use super::Model;

/// One party member; everything else about them is the unit snapshot under their `partyN` token.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyMemberInfo {
    pub name: String,
    /// The identity `UnitInParty` matches any token against; `0` never matches.
    pub guid: u64,
}

/// One raid roster row, `GetRaidRosterInfo`'s first nine returns in push order (`0x573690`). The
/// binding, not the row, applies the subgroup's +1 and the offline zone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RaidMemberInfo {
    /// Return 1, from the name cache (`0x573722`). Empty means not cached, which the reference
    /// answers with the miss tuple, as for an empty slot (`0x57372b je 0x5739e6`).
    pub name: String,
    /// The identity `UnitInRaid`'s row walk (`0x608130`) matches a token's guid against; `0`
    /// never matches.
    pub guid: u64,
    /// Return 2, exposed as stored (`0x57374b fild [edi+0xc]`): 0 member, 1 assistant, 2 leader,
    /// the convention addons read (`ChatLog.lua:351-353`).
    pub rank: u32,
    /// Return 3, stored 0-based as the wire's `GroupMemberEntry::flags` bits 0-2 carry it; the
    /// binding exposes it 1-based (`0x57375a add ecx,0x1`).
    pub subgroup: u32,
    /// Return 4, `0` until an object or a stats packet carries it, as in the reference's third arm.
    pub level: u32,
    /// Return 5, the localized class name.
    pub class: Option<String>,
    /// Return 6, the class token (`"WARRIOR"`), the one unlocalized string.
    pub class_file: Option<String>,
    /// Return 7 while [`Self::online`]; offline, the binding pushes the `PLAYER_OFFLINE` global
    /// instead and never reads this.
    pub zone: Option<String>,
    /// Return 8, which also picks return 7's offline arm.
    pub online: bool,
    /// Return 9, unnamed: `1` for a streamed member whose health (`[obj+0x110 +0x40]`) is 0 or
    /// below, or an unstreamed one whose roster status (`[+0x18]`) has both `0x1` and `0x4`,
    /// vmangos's `ONLINE` and `DEAD`, from which the app fills it. Addons read it as `isDead`.
    pub ninth: bool,
}

/// The party and raid roster snapshot the app pushes whole when it changes
/// ([`UiScript::set_party`]); the default is ungrouped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyState {
    /// The other party members in `party1`..`party4` order, empty when ungrouped. The player is
    /// never in it, as in `SMSG_GROUP_LIST`; in a raid it is our own subgroup's slice.
    pub members: Vec<PartyMemberInfo>,
    /// `GetPartyLeaderIndex`: `0` the player, `1..=4` that `members` slot.
    pub leader_index: u32,
    /// The cached group-leader guid (the reference's `[0xbd1968]:[0xbd196c]`), `0` when
    /// ungrouped, fed from the wire rather than derived from [`Self::leader_index`].
    /// `UnitIsPartyLeader` (`0x60c770`) compares a resolved token's guid with it and, unlike
    /// `IsPartyLeader` (`0x52ccd0`), has no zero guard, so an unresolvable token answers `1` while
    /// solo.
    pub leader_guid: u64,
    /// The player's own guid (`0x4d3790`, the object manager's `+0xc0`), `0` out of world. With
    /// [`Self::leader_guid`] it is the ready-check timeout's leader gate, a bare guid compare, so a
    /// solo player never matches.
    pub own_guid: u64,
    /// The whole raid roster, 1-based for `GetRaidRosterInfo`, empty outside a raid. It includes
    /// the player, as the reference's array does, so `UnitInRaid("player")` answers `1`;
    /// `GetNumRaidMembers()` is its length.
    pub raid: Vec<RaidMemberInfo>,
    /// `GetLootMethod`'s method: `"freeforall"`, `"roundrobin"`, `"master"`, `"group"` or
    /// `"needbeforegreed"`. Empty, the default, reads as `"freeforall"`, the reference's answer
    /// before any group.
    pub loot_method: String,
    /// The master looter as a party index (`0` the player, `1..=4` a `members` slot),
    /// `GetLootMethod`'s second return.
    pub master_looter: Option<u32>,
    /// The master looter's guid (`0xbd1970`), `0` for none: `GetLootMethod`'s third return finds
    /// it in [`Self::raid`] and `GetRaidRosterInfo`'s eleventh compares it.
    pub master_looter_guid: u64,
    /// `GetLootThreshold`'s item quality, `2..=4` once grouped.
    pub loot_threshold: u32,
}

/// Outbound party and raid intents, drained by the app ([`UiScript::take_party_requests`]) into
/// the matching send.
#[derive(Clone, Debug, PartialEq)]
pub enum PartyRequest {
    /// Wrath assignment intent, resolved by the VM; roster flags await the server echo.
    Assignment {
        assignment: u8,
        apply: bool,
        guid: u64,
    },
    /// `AcceptGroup()`.
    Accept,
    /// `DeclineGroup()`.
    Decline,
    /// `LeaveParty()`.
    Leave,
    /// `InviteByName(name)`.
    InviteName(String),
    /// `InviteToParty(unit)`, by unit token; the app resolves it to a name.
    InviteUnit(String),
    /// `UninviteFromParty(unit)`, by unit token.
    UninviteUnit(String),
    /// Build 12340's `UninviteUnit(unitOrName [, reason])` (`0x51a7a0`): the token or member name
    /// resolved to a guid (`0x60abf0`), then `CMSG_GROUP_UNINVITE_GUID` with the reason
    /// (`0x6d43c0`).
    UninviteGuid { guid: u64, reason: String },
    /// `PromoteToPartyLeader(unit)`, by unit token.
    PromoteUnit(String),
    /// `SetLootMethod(method[, masterName])`, validated in the VM; the app resolves the master
    /// looter's name to a roster member.
    LootMethod {
        method: String,
        master_name: Option<String>,
        /// The current threshold, which 12340 sends with every method (`0x52de1d`); the binding
        /// takes no threshold argument.
        threshold: Option<u32>,
    },
    /// `SetLootThreshold(n)`.
    LootThreshold(u32),
    /// `SetRaidTarget(unit, index)`: icon 1 to 8, or 0 to clear; the app resolves the token to a
    /// guid for `MSG_RAID_TARGET_UPDATE`.
    SetRaidTarget { unit: String, index: u8 },
    // ── The raid-management verbs ───────────────────────────────────────────────────────────
    /// `ConvertToRaid()`, leader only (`CMSG_GROUP_RAID_CONVERT`).
    ConvertToRaid,
    /// `SetRaidSubgroup(index, group)`, both 1-based. The wire (`CMSG_GROUP_CHANGE_SUB_GROUP`)
    /// takes a name and a 0-based subgroup, which the app resolves.
    SetSubgroup { index: u32, group: u32 },
    /// `SwapRaidSubgroup(index, other)`: trade two raid rows' subgroups
    /// (`CMSG_GROUP_SWAP_SUB_GROUP`, two names on the wire).
    SwapSubgroup { index: u32, other: u32 },
    /// `PromoteByName(name)` (`CMSG_GROUP_SET_LEADER`, whose guid the app resolves).
    PromoteName(String),
    /// `PromoteToAssistant(name)` / `DemoteAssistant(name)` (`CMSG_GROUP_ASSISTANT_LEADER`).
    AssistantLeader { name: String, grant: bool },
    /// `UninviteFromRaid(index)`: a 1-based raid row the app resolves to the name
    /// `CMSG_GROUP_UNINVITE` takes.
    UninviteRaid(u32),
    /// `DoReadyCheck()`, leader only (`MSG_RAID_READY_CHECK`, empty body).
    ReadyCheckStart,
    /// `ConfirmReadyCheck(ready)` (`MSG_RAID_READY_CHECK`, one byte).
    ReadyCheckAnswer(bool),
}

impl super::UiScript {
    /// Push the roster snapshot, replacing it, and run `SMSG_GROUP_LIST`'s ready-check leg over the
    /// new roster. The app fires the `PARTY_*` events.
    pub fn set_party(&mut self, state: PartyState) {
        self.model_mut().party = state;
        // The ready-check leg (`0x4ba5f0`, from the `0x7d` handler): a member who left drops their
        // pending flag, and nobody left pending force-closes the check.
        let lua = self.lua();
        let mut model = self.model_mut();
        if model.ready_check.deadline.is_some() {
            let roster: Vec<u64> = model.party.raid.iter().map(|m| m.guid).collect();
            model.ready_check.unanswered.retain(|g| roster.contains(g));
            if model.ready_check.unanswered.is_empty() {
                ready_check_force_close(lua, &mut model);
            }
        }
    }

    /// The `MSG_RAID_READY_CHECK` open form arrived (handler `0x4ba360`). The leader force-closes
    /// if nobody is left pending; anyone else arms the 30 s deadline (`0x4ba535`), which their
    /// leader-gated tick never acts on. The app fires the `READY_CHECK` popup.
    pub fn ready_check_request(&mut self, we_lead: bool) {
        let lua = self.lua();
        let now = clock(lua);
        let mut model = self.model_mut();
        if we_lead {
            if model.ready_check.deadline.is_some() && !ready_check_pending_online(&model) {
                ready_check_force_close(lua, &mut model);
            }
        } else {
            model.ready_check.deadline = Some(now + READY_CHECK_SECONDS);
        }
    }

    /// Drain the ready-check lines composed since the last call; the reference prints each as a
    /// `CHAT_MSG_SYSTEM` line (`0x49a870(text, 10)`).
    pub fn take_ready_check_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.model_mut().ready_check.lines)
    }

    /// Drain the party/loot intents queued since the last call.
    pub fn take_party_requests(&mut self) -> Vec<PartyRequest> {
        std::mem::take(&mut self.model_mut().party_requests)
    }
}

/// `GetRaidRosterInfo`'s first nine values for one row, or the miss tuple's for `None`; build
/// 12340's `0x573690` answers fourteen on every path (`mov eax,0xe`), `wrath_group.rs` adding the
/// last five. Out of range, an empty slot and a name-cache miss all reach `0x5739e6`, which pushes
/// `nil, 0, 1, 1` and ten `nil`s. Takes the row by value because every arm re-enters Lua, and a
/// callback drops its `app_data` borrow before that.
fn raid_roster_info(lua: &Lua, row: Option<RaidMemberInfo>) -> mlua::Result<MultiValue> {
    let Some(m) = row else {
        return Ok(MultiValue::from_vec(vec![
            Value::Nil,
            Value::Integer(0),
            Value::Integer(1),
            Value::Integer(1),
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
        ]));
    };
    let opt_str = |s: &Option<String>| match s {
        Some(s) => lua.create_string(s).map(Value::String),
        None => Ok(Value::Nil),
    };
    // Return 7: the zone while online, else the `PLAYER_OFFLINE` global read from the VM
    // (`0x703bf0`), so a translated GlobalStrings.lua translates it; a missing global pushes `nil`
    // (`0x6f3890`).
    let zone = if m.online {
        opt_str(&m.zone)?
    } else {
        match lua.globals().get::<Value>("PLAYER_OFFLINE") {
            Ok(v @ Value::String(_)) => v,
            _ => Value::Nil,
        }
    };
    let flag = |b: bool| if b { Value::Integer(1) } else { Value::Nil };
    Ok(MultiValue::from_vec(vec![
        lua.create_string(&m.name).map(Value::String)?,
        Value::Integer(i64::from(m.rank)),
        // Stored 0-based, exposed 1-based (`0x4bb61a inc eax`).
        Value::Integer(i64::from(m.subgroup) + 1),
        Value::Integer(i64::from(m.level)),
        opt_str(&m.class)?,
        opt_str(&m.class_file)?,
        zone,
        flag(m.online),
        flag(m.ninth),
    ]))
}

/// Register the party and raid globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetNumPartyMembers() → the other members, never counting the player; 0 when ungrouped.
    g.set(
        "GetNumPartyMembers",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.party.members.len() as i64)
        })?,
    )?;

    // GetNumRaidMembers() → the raid roster's length, 0 outside a raid.
    g.set(
        "GetNumRaidMembers",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.party.raid.len() as i64)
        })?,
    )?;

    // GetRaidRosterInfo(index) (`0x573690`), 1-based: name, rank, subgroup, level, class,
    // fileName, zone, online and isDead here; `wrath_group.rs` adds the last five. Its only raise
    // is a non-number argument (`lua_isnumber` at `0x5736a0`, the Usage at `0x5736ac`).
    g.set(
        "GetRaidRosterInfo",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: GetRaidRosterInfo(index)")?;
            let row = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                // One unsigned compare after the decrement (`0x5736e6 sub eax,0x1`, `0x5736f2 jae`)
                // catches an index of 0 or below and one past the count; `dec` wraps, and
                // `wrapping_sub` keeps that true of `i32::MIN`.
                usize::try_from(index.wrapping_sub(1))
                    .ok()
                    .and_then(|i| model.party.raid.get(i))
                    // A name-cache miss shares that same arm (`0x57372b je 0x5739e6`).
                    .filter(|m| !m.name.is_empty())
                    .cloned()
            };
            raid_roster_info(lua, row)
        })?,
    )?;

    // GetPartyLeaderIndex() → 0 (the player leads) or 1..4 (that party slot leads).
    g.set(
        "GetPartyLeaderIndex",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.party.leader_index))
        })?,
    )?;

    // GetLootMethod() (`0x52cd90`) → method, partyMaster, raidMaster: always three. partyMaster is
    // 0 for the player or a party slot 1..4; raidMaster the raid roster index (`0xbeb568`); each
    // nil when the master is not there.
    g.set(
        "GetLootMethod",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            // Never grouped, the reference reads zero-filled cells past `.data`'s end, and method
            // 0 is `freeforall`.
            let method = if model.party.loot_method.is_empty() {
                "freeforall"
            } else {
                model.party.loot_method.as_str()
            };
            let master = match model.party.master_looter {
                Some(idx) => Value::Integer(i64::from(idx)),
                None => Value::Nil,
            };
            let guid = model.party.master_looter_guid;
            let raid = match model
                .party
                .raid
                .iter()
                .position(|m| guid != 0 && m.guid == guid)
            {
                Some(i) => Value::Integer(i as i64 + 1),
                None => Value::Nil,
            };
            Ok((Value::String(lua.create_string(method)?), master, raid))
        })?,
    )?;

    // GetLootThreshold() → the loot quality threshold.
    g.set(
        "GetLootThreshold",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.party.loot_threshold))
        })?,
    )?;

    // The party verbs queue a `PartyRequest` and return nothing.
    g.set(
        "AcceptGroup",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::Accept);
            Ok(())
        })?,
    )?;
    g.set(
        "DeclineGroup",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::Decline);
            Ok(())
        })?,
    )?;
    g.set(
        "LeaveParty",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::Leave);
            Ok(())
        })?,
    )?;
    g.set(
        "InviteByName",
        lua.create_function(|lua, name: String| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::InviteName(name));
            Ok(())
        })?,
    )?;
    g.set(
        "InviteToParty",
        lua.create_function(|lua, unit: String| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::InviteUnit(unit));
            Ok(())
        })?,
    )?;
    g.set(
        "UninviteFromParty",
        lua.create_function(|lua, unit: String| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::UninviteUnit(unit));
            Ok(())
        })?,
    )?;
    g.set(
        "PromoteToPartyLeader",
        lua.create_function(|lua, unit: String| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::PromoteUnit(unit));
            Ok(())
        })?,
    )?;
    g.set(
        "SetLootThreshold",
        lua.create_function(|lua, n: u32| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::LootThreshold(n));
            Ok(())
        })?,
    )?;
    // SetRaidTarget(unit, index) is the engine verb. `SetRaidTargetIcon` is FrameXML's toggle
    // around it (`TargetFrame.lua:486`), and `TargetFrame.lua:488`, `:490` and `Bindings.xml:1160`
    // call it bare.
    g.set(
        "SetRaidTarget",
        lua.create_function(|lua, (unit, index): (String, u8)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .party_requests
                .push(PartyRequest::SetRaidTarget { unit, index });
            Ok(())
        })?,
    )?;

    // ── The raid-management verbs ────────────────────────────────────────────
    // Each only marshals its arguments into a client call or a send (`SetRaidSubgroup 0x4bb990`,
    // `SwapRaidSubgroup 0x4bbb00`, `PromoteToAssistant 0x4bbd20`, `UninviteFromRaid 0x48a580`,
    // `SetRaidRosterSelection 0x4bb820`), so the verbs here queue a request.
    // SetRaidSubgroup(index, group) / SwapRaidSubgroup(index, other), the drag's two drops: a
    // non-number raises the usage error rather than queueing a zero.
    g.set(
        "SetRaidSubgroup",
        lua.create_function(|lua, (index, group): (Value, Value)| {
            let index = number_arg(lua, index, "Usage: SetRaidSubgroup(index, subgroup)")?;
            let group = number_arg(lua, group, "Usage: SetRaidSubgroup(index, subgroup)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::SetSubgroup {
                index: index.max(0) as u32,
                group: group.max(0) as u32,
            });
            Ok(())
        })?,
    )?;
    g.set(
        "SwapRaidSubgroup",
        lua.create_function(|lua, (index, other): (Value, Value)| {
            let index = number_arg(lua, index, "Usage: SwapRaidSubgroup(index1, index2)")?;
            let other = number_arg(lua, other, "Usage: SwapRaidSubgroup(index1, index2)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::SwapSubgroup {
                index: index.max(0) as u32,
                other: other.max(0) as u32,
            });
            Ok(())
        })?,
    )?;
    g.set(
        "PromoteByName",
        lua.create_function(|lua, name: String| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.party_requests.push(PartyRequest::PromoteName(name));
            Ok(())
        })?,
    )?;
    for (binding, grant) in [("PromoteToAssistant", true), ("DemoteAssistant", false)] {
        g.set(
            binding,
            lua.create_function(move |lua, name: String| {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                model
                    .party_requests
                    .push(PartyRequest::AssistantLeader { name, grant });
                Ok(())
            })?,
        )?;
    }
    g.set(
        "UninviteFromRaid",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: UninviteFromRaid(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .party_requests
                .push(PartyRequest::UninviteRaid(index.max(0) as u32));
            Ok(())
        })?,
    )?;
    g.set(
        "DoReadyCheck",
        lua.create_function(|lua, ()| {
            let now = clock(lua);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // The worker `0x4bb1d0` flags every roster member but the caller as pending
            // (`0x4bb24c`, `0x4bb258`) and arms the 30 s deadline (`0x4bb2b6`), then sends. A
            // party has no raid roster, so nothing is flagged and the leader's echo closes it.
            let own = model.party.own_guid;
            model.ready_check.unanswered = model
                .party
                .raid
                .iter()
                .map(|m| m.guid)
                .filter(|g| *g != own)
                .collect();
            model.ready_check.deadline = Some(now + READY_CHECK_SECONDS);
            model.party_requests.push(PartyRequest::ReadyCheckStart);
            Ok(())
        })?,
    )?;
    // CheckReadyCheckTime() (`0x4bc120` → `0x4bb310`), called every frame from stock
    // `UIParent.xml`'s `OnUpdate`, alone drives the 30 s expiry. Once an armed deadline passes, and
    // only on the leader's client, it disarms and prints `RAID_MEMBERS_AFK` with the pending names
    // joined by `", "`, or `READY_CHECK_NO_AFK`, as a `CHAT_MSG_SYSTEM` line. Both strings are read
    // raw from `_G` with an empty default (`0x704350`), not through `GetText`; nothing is sent.
    g.set(
        "CheckReadyCheckTime",
        lua.create_function(|lua, ()| {
            let now = clock(lua);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            ready_check_tick(lua, &mut model, now);
            Ok(())
        })?,
    )?;
    // ConfirmReadyCheck(ready) reads its argument as truth: the reference's No button passes no
    // argument, Yes passes `1`.
    g.set(
        "ConfirmReadyCheck",
        lua.create_function(|lua, ready: Value| {
            let ready = !matches!(ready, Value::Nil | Value::Boolean(false));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .party_requests
                .push(PartyRequest::ReadyCheckAnswer(ready));
            Ok(())
        })?,
    )?;
    // SetRaidRosterSelection(index) / GetRaidRosterSelection(): a client-side cursor; the
    // reference's `0x4bb820` writes a global and sends nothing.
    g.set(
        "SetRaidRosterSelection",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: SetRaidRosterSelection(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.raid_selection = i64::from(index);
            Ok(())
        })?,
    )?;
    g.set(
        "GetRaidRosterSelection",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.raid_selection)
        })?,
    )?;

    Ok(())
}

/// The ready-check deadline, `0x7530` ms after the arm (`0x4ba535`, `0x4bb2b6`).
const READY_CHECK_SECONDS: f64 = 30.0;

/// The ready-check state: the armed deadline (`[0xb713f4]`), the members whose pending flag
/// (`[entry+0x158]`) is set, and summary lines not yet pushed to chat. Outside [`PartyState`]
/// because a roster push replaces that and this survives it.
#[derive(Debug, Default)]
pub(crate) struct ReadyCheckState {
    pub(crate) deadline: Option<f64>,
    pub(crate) unanswered: Vec<u64>,
    pub(crate) lines: Vec<String>,
}

/// `GetTime()`'s clock, the session seconds `UiScript::tick` advances.
fn clock(lua: &Lua) -> f64 {
    crate::script::clock::now(lua)
}

/// The timeout worker `0x4bb310`.
fn ready_check_tick(lua: &Lua, model: &mut Model, now: f64) {
    let Some(deadline) = model.ready_check.deadline else {
        return;
    };
    if now < deadline {
        return;
    }
    if model.party.own_guid != model.party.leader_guid {
        return;
    }
    model.ready_check.deadline = None;
    // A pending member the roster cannot name is skipped (`0x55f080`, its query arm off).
    let names: Vec<&str> = model
        .ready_check
        .unanswered
        .iter()
        .filter_map(|g| model.party.raid.iter().find(|m| m.guid == *g))
        .map(|m| m.name.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    let raw = |key: &str| lua.globals().get::<String>(key).unwrap_or_default();
    let text = if names.is_empty() {
        raw("READY_CHECK_NO_AFK")
    } else {
        raw("RAID_MEMBERS_AFK").replacen("%s", &names.join(", "), 1)
    };
    model.ready_check.lines.push(text);
}

/// The `0x322` handler's close test (`0x4ba498`-`0x4ba4cc`): a member holds the check open only
/// while pending and online (`[entry+0x18]` bit 0, `SMSG_GROUP_LIST`'s online bit). An empty
/// roster closes at once (`0x4ba3ea`).
fn ready_check_pending_online(model: &Model) -> bool {
    model
        .ready_check
        .unanswered
        .iter()
        .any(|g| model.party.raid.iter().any(|m| m.guid == *g && m.online))
}

/// The force-close sites (`0x4ba4d4`, `0x4bacb4`): an armed deadline becomes `now - 1` and the
/// worker runs at once.
fn ready_check_force_close(lua: &Lua, model: &mut Model) {
    if model.ready_check.deadline.is_none() {
        return;
    }
    let now = model.now;
    model.ready_check.deadline = Some(now - 1.0);
    ready_check_tick(lua, model, now);
}

#[cfg(test)]
mod tests {
    use crate::script::{PartyMemberInfo, PartyRequest, PartyState, UiScript};

    fn two_member_party() -> PartyState {
        PartyState {
            members: vec![
                PartyMemberInfo {
                    name: "Alice".into(),
                    guid: 0xA11CE,
                },
                PartyMemberInfo {
                    name: "Bob".into(),
                    guid: 0xB0B,
                },
            ],
            leader_index: 1, // Alice (party1) leads
            leader_guid: 0xA11CE,
            own_guid: 0x5E1F,
            raid: Vec::new(),
            loot_method: "group".into(),
            master_looter: None,
            master_looter_guid: 0,
            loot_threshold: 2,
        }
    }

    #[test]
    fn read_natives_report_the_pushed_roster() {
        let mut s = UiScript::new().unwrap();
        s.set_party(two_member_party());

        assert_eq!(s.eval::<i64>("return GetNumPartyMembers()").unwrap(), 2);
        assert_eq!(s.eval::<i64>("return GetNumRaidMembers()").unwrap(), 0);
        assert_eq!(s.eval::<i64>("return GetPartyMember(1)").unwrap(), 1);
        assert_eq!(s.eval::<i64>("return GetPartyMember(2)").unwrap(), 1);
        assert!(s.eval::<bool>("return GetPartyMember(3) == nil").unwrap());
        assert!(
            s.run("GetPartyMember(0)").is_err(),
            "0x52c215: out of 1-4 raises"
        );
        assert_eq!(s.eval::<i64>("return GetPartyLeaderIndex()").unwrap(), 1);
        assert!(s.eval::<bool>("return IsPartyLeader() == nil").unwrap());
        let (method, master) = s
            .eval::<(String, Option<i64>)>("return GetLootMethod()")
            .unwrap();
        assert_eq!(method, "group");
        assert_eq!(master, None);
        assert_eq!(s.eval::<i64>("return GetLootThreshold()").unwrap(), 2);
    }

    #[test]
    fn is_party_leader_reports_when_the_player_leads() {
        let mut s = UiScript::new().unwrap();
        let mut party = two_member_party();
        (party.leader_index, party.leader_guid) = (0, party.own_guid); // the player leads
        s.set_party(party);
        assert_eq!(s.eval::<i64>("return IsPartyLeader()").unwrap(), 1);
    }

    #[test]
    fn get_loot_method_reports_the_assigned_master() {
        let mut s = UiScript::new().unwrap();
        let mut party = two_member_party();
        party.loot_method = "master".into();
        party.master_looter = Some(2); // Bob (party2)
        s.set_party(party.clone());
        let (method, master) = s.eval::<(String, i64)>("return GetLootMethod()").unwrap();
        assert_eq!(method, "master");
        assert_eq!(master, 2);
        // Build 12340's third return: the master's raid roster index, nil outside the roster.
        assert_eq!(s.arity("GetLootMethod()").unwrap(), 3);
        assert_eq!(
            s.eval::<Option<i64>>("local _, _, r = GetLootMethod() return r")
                .unwrap(),
            None
        );
        party.master_looter_guid = 0xb0b;
        party.raid = vec![
            super::RaidMemberInfo {
                guid: 0xa,
                ..Default::default()
            },
            super::RaidMemberInfo {
                guid: 0xb0b,
                ..Default::default()
            },
        ];
        s.set_party(party);
        assert_eq!(
            s.eval::<Option<i64>>("local _, _, r = GetLootMethod() return r")
                .unwrap(),
            Some(2)
        );
    }

    #[test]
    fn empty_state_reports_the_solo_player_shape() {
        let s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetNumPartyMembers()").unwrap(), 0);
        assert!(s.eval::<bool>("return GetPartyMember(1) == nil").unwrap());
        assert!(s.eval::<bool>("return IsPartyLeader() == nil").unwrap());
        let (method, master) = s
            .eval::<(String, Option<i64>)>("return GetLootMethod()")
            .unwrap();
        // Never grouped reads method 0, `freeforall`; the `group` above is a real party's.
        assert_eq!(method, "freeforall");
        assert_eq!(master, None);
    }

    // ── The raid trio (`UnitInRaid`, `GetRaidRosterInfo`, `IsRaidLeader`) ────────────────────

    fn raider(name: &str, guid: u64) -> crate::script::RaidMemberInfo {
        crate::script::RaidMemberInfo {
            name: name.into(),
            guid,
            rank: 0,
            subgroup: 0,
            level: 60,
            class: Some("Warrior".into()),
            class_file: Some("WARRIOR".into()),
            zone: Some("Molten Core".into()),
            online: true,
            ninth: false,
        }
    }

    fn ten_player_raid() -> PartyState {
        let mut raid: Vec<crate::script::RaidMemberInfo> = (0u64..10)
            .map(|i| raider(&format!("Raider{i}"), 0x100 + i))
            .collect();
        raid[0].rank = 2; // the leader
        raid[0].name = "Me".into();
        raid[3].subgroup = 1; // stored 0-based, exposed 2
        PartyState {
            raid,
            ..Default::default()
        }
    }

    /// Build 12340 answers fourteen on every path (`mov eax,0xe`, `0x5739c7`, `0x5739dc`).
    #[test]
    fn get_raid_roster_info_returns_fourteen_values_on_every_path() {
        let mut s = UiScript::new().unwrap();
        s.set_party(ten_player_raid());

        for index in ["1", "10", "0", "-1", "41", "9999"] {
            assert_eq!(
                s.arity(&format!("GetRaidRosterInfo({index})")).unwrap(),
                14,
                "index {index} must still be a fourteen-value answer"
            );
        }
        let s = UiScript::new().unwrap();
        assert_eq!(s.arity("GetRaidRosterInfo(1)").unwrap(), 14);
    }

    /// `0x5739e6` pushes `nil, 0, 1, 1` and ten nils.
    #[test]
    fn get_raid_roster_info_misses_with_the_fixed_tuple() {
        let mut s = UiScript::new().unwrap();
        s.set_party(ten_player_raid());
        let miss = "local a,b,c,d,e,f,g,h,i = GetRaidRosterInfo(99) \
                    return a == nil, b, c, d, e == nil, f == nil, g == nil, h == nil, i == nil";
        let (a, b, c, d, e, f, g, h, i): (bool, i64, i64, i64, bool, bool, bool, bool, bool) =
            s.eval(miss).unwrap();
        assert!(a, "name is nil");
        assert_eq!(
            (b, c, d),
            (0, 1, 1),
            "rank 0, subgroup 1, level 1 — numbers"
        );
        assert!(e && f && g && h && i, "the last five are nil");

        // An in-range member with no cached name takes the same arm (`0x57372b je 0x5739e6`).
        let mut party = ten_player_raid();
        party.raid[1].name = String::new();
        s.set_party(party);
        let (name_nil, rank): (bool, i64) = s
            .eval("local a,b = GetRaidRosterInfo(2) return a == nil, b")
            .unwrap();
        assert!(name_nil && rank == 0, "an uncached name is a full miss");
    }

    #[test]
    fn get_raid_roster_info_reports_the_pushed_row() {
        let mut s = UiScript::new().unwrap();
        s.set_party(ten_player_raid());
        let (name, rank, subgroup, level, class, file, zone, online, ninth): (
            String,
            i64,
            i64,
            i64,
            String,
            String,
            String,
            i64,
            Option<i64>,
        ) = s
            .eval("local a,b,c,d,e,f,g,h,i = GetRaidRosterInfo(1) return a,b,c,d,e,f,g,h,i")
            .unwrap();
        assert_eq!(name, "Me");
        assert_eq!(rank, 2, "as stored, no adjustment");
        assert_eq!(subgroup, 1, "stored 0 → exposed 1 (`0x57375a add ecx,0x1`)");
        assert_eq!(level, 60);
        assert_eq!((class.as_str(), file.as_str()), ("Warrior", "WARRIOR"));
        assert_eq!(zone, "Molten Core");
        assert_eq!(online, 1);
        assert_eq!(ninth, None, "1/nil, never true/false");
        // Subgroup is the one adjusted field: member 4 is stored in subgroup 1 and reads 2.
        assert_eq!(
            s.eval::<i64>("local _,_,g = GetRaidRosterInfo(4) return g")
                .unwrap(),
            2
        );
    }

    /// Return 8 goes nil on the same arm (`0x573858`).
    #[test]
    fn get_raid_roster_info_puts_player_offline_in_the_zone_slot() {
        let mut s = UiScript::new().unwrap();
        let mut party = ten_player_raid();
        party.raid[1].online = false;
        party.raid[1].zone = Some("Molten Core".into()); // ignored on the offline arm
        s.set_party(party);
        // The binding reads whatever `PLAYER_OFFLINE` the VM holds.
        s.run(r#"PLAYER_OFFLINE = "Offline""#).unwrap();
        let (zone, online): (String, Option<i64>) = s
            .eval("local _,_,_,_,_,_,g,h = GetRaidRosterInfo(2) return g, h")
            .unwrap();
        assert_eq!(zone, "Offline");
        assert_eq!(online, None);
    }

    /// The raise at `0x5736b2` goes through `luaL_error` (`0x84f280`), which does not return.
    #[test]
    fn get_raid_roster_info_raises_only_on_a_non_number() {
        let s = UiScript::new().unwrap();
        let err = s.arity("GetRaidRosterInfo({})").unwrap_err();
        assert!(
            format!("{err}").contains("Usage: GetRaidRosterInfo(index)"),
            "got {err}"
        );
        // A numeric string coerces (`lua_isnumber`) and does not raise.
        assert_eq!(s.arity(r#"GetRaidRosterInfo("3")"#).unwrap(), 14);
    }

    /// 12340 pushes the 0-based roster row (`0x608130`, `fild` at `0x60ca65`): 0 is a hit.
    #[test]
    fn unit_in_raid_answers_the_zero_based_row() {
        let mut s = UiScript::new().unwrap();
        s.set_party(ten_player_raid());
        s.set_unit("player", Some(unit(true, 0x100)));
        s.set_unit("target", Some(unit(true, 0x105))); // roster row 6
        s.set_unit("mouseover", Some(unit(true, 0xDEAD)));
        s.set_unit_guids(&crate::script::UnitGuids {
            player: 0x100,
            target: 0x105,
            mouseover: 0xDEAD,
            held: std::collections::HashMap::from([(0xDEAD, 0)]),
            ..Default::default()
        });

        assert_eq!(s.eval::<i64>(r#"return UnitInRaid("player")"#).unwrap(), 0);
        assert_eq!(s.eval::<i64>(r#"return UnitInRaid("target")"#).unwrap(), 5);
        assert!(s
            .eval::<bool>(r#"return UnitInRaid("mouseover") == nil"#)
            .unwrap());
        // Nil for a missing, wrong-typed, unmatched or unparsed token: the resolver (`0x60abf0`)
        // never raises, and a number is a token like any other.
        for call in ["UnitInRaid()", "UnitInRaid(nil)", r#"UnitInRaid("party3")"#] {
            assert!(s.eval::<bool>(&format!("return {call} == nil")).unwrap());
        }
        for call in ["UnitInRaid(7)", r#"UnitInRaid("nosuchtoken")"#] {
            assert!(s.eval::<bool>(&format!("return {call} == nil")).unwrap());
        }
    }

    /// `0x573a60` reads the same leader guid as `0x52ccd0`, and no raid flag.
    #[test]
    fn is_raid_leader_is_true_for_a_party_leader() {
        let mut s = UiScript::new().unwrap();
        let mut party = two_member_party();
        (party.leader_index, party.leader_guid) = (0, party.own_guid); // we lead, no raid roster
        s.set_party(party);
        assert!(
            s.eval::<i64>("return GetNumRaidMembers()").unwrap() == 0,
            "no raid, and that is exactly the point"
        );
        assert_eq!(s.eval::<i64>("return IsRaidLeader()").unwrap(), 1);
        assert_eq!(s.eval::<i64>("return IsPartyLeader()").unwrap(), 1);

        s.set_party(two_member_party()); // leader_index 1 = Alice
        assert!(s.eval::<bool>("return IsRaidLeader() == nil").unwrap());
        assert!(s.eval::<bool>("return IsPartyLeader() == nil").unwrap());

        // Neither guid known: `0x573a60` has no zero test and answers 1, where `0x52ccd0` tests
        // the leader guid (`0x52cce3`).
        let s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return IsRaidLeader()").unwrap(), 1);
        assert!(s.eval::<bool>("return IsPartyLeader() == nil").unwrap());
    }

    #[test]
    fn get_num_raid_members_bounds_the_roster_it_indexes() {
        let mut s = UiScript::new().unwrap();
        s.set_party(ten_player_raid());
        assert_eq!(s.eval::<i64>("return GetNumRaidMembers()").unwrap(), 10);
        assert!(s
            .eval::<bool>("return GetRaidRosterInfo(GetNumRaidMembers()) ~= nil")
            .unwrap());
        assert!(s
            .eval::<bool>("return GetRaidRosterInfo(GetNumRaidMembers() + 1) == nil")
            .unwrap());
    }

    #[test]
    fn intent_natives_queue_the_exact_request_sequence() {
        let mut s = UiScript::new().unwrap();
        assert!(s.take_party_requests().is_empty());

        s.run("AcceptGroup()").unwrap();
        s.run("DeclineGroup()").unwrap();
        s.run("LeaveParty()").unwrap();
        s.run(r#"InviteByName("Bob")"#).unwrap();
        s.run(r#"InviteToParty("target")"#).unwrap();
        s.run(r#"UninviteFromParty("party2")"#).unwrap();
        s.run(r#"PromoteToPartyLeader("party2")"#).unwrap();
        let mut party = two_member_party();
        party.leader_guid = party.own_guid;
        s.set_party(party);
        s.run(r#"SetLootMethod("master", "Bob")"#).unwrap();
        s.run(r#"SetLootThreshold(3)"#).unwrap();

        assert_eq!(
            s.take_party_requests(),
            vec![
                PartyRequest::Accept,
                PartyRequest::Decline,
                PartyRequest::Leave,
                PartyRequest::InviteName("Bob".into()),
                PartyRequest::InviteUnit("target".into()),
                PartyRequest::UninviteUnit("party2".into()),
                PartyRequest::PromoteUnit("party2".into()),
                PartyRequest::LootMethod {
                    method: "master".into(),
                    master_name: Some("Bob".into()),
                    threshold: Some(2),
                },
                PartyRequest::LootThreshold(3),
            ]
        );
        assert!(s.take_party_requests().is_empty());
    }

    /// `0x52dc20`'s order: not grouped (`0x50`) and not leading (`0x54`) are system lines and
    /// queue nothing; then the `Usage:` gate and the whole-string method ("Invalid loot method");
    /// `master` needs a non-empty name (`0xfc`, a UI error) of us or a member (`0x51`). The send
    /// carries the current threshold; a third argument is `exactMatch`, never a threshold.
    #[test]
    fn set_loot_method_checks_the_group_the_leader_and_the_master_in_order() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"SetLootMethod("freeforall")"#).unwrap();
        s.set_party(two_member_party());
        s.run(r#"SetLootMethod("freeforall")"#).unwrap();
        assert!(s.take_party_requests().is_empty(), "solo, then not leading");
        let mut party = two_member_party();
        (party.leader_guid, party.loot_threshold) = (party.own_guid, 3);
        s.set_party(party);
        assert!(s
            .run("SetLootMethod()")
            .unwrap_err()
            .to_string()
            .contains("Usage"));
        let bad = s.run(r#"SetLootMethod("free")"#).unwrap_err().to_string();
        assert!(bad.contains("Invalid loot method"), "{bad}");
        s.run(r#"SetLootMethod("master", "") SetLootMethod("master", "Zed")"#)
            .unwrap();
        assert_eq!(s.take_ui_errors(), ["ERR_SPECIFY_MASTER_LOOTER"]);
        s.run(r#"SetLootMethod("GROUP", nil, 4) SetLootMethod("master", "bob")"#)
            .unwrap();
        let req = |method: &str, master_name: Option<&str>| PartyRequest::LootMethod {
            method: method.into(),
            master_name: master_name.map(Into::into),
            threshold: Some(3),
        };
        assert_eq!(
            s.take_party_requests(),
            [req("group", None), req("master", Some("bob"))]
        );
    }

    #[test]
    fn set_raid_target_icon_queues_the_token_and_index() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"SetRaidTarget("target", 8)"#).unwrap();
        s.run(r#"SetRaidTarget("party2", 0)"#).unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![
                PartyRequest::SetRaidTarget {
                    unit: "target".into(),
                    index: 8,
                },
                PartyRequest::SetRaidTarget {
                    unit: "party2".into(),
                    index: 0,
                },
            ]
        );
    }

    /// `0x573b00`: our raid row (`0x549670`) with a rank above 0 (`0x573b16`).
    #[test]
    fn is_raid_officer_reads_our_raid_rows_rank() {
        let mut s = UiScript::new().unwrap();
        assert!(s.eval::<bool>("return IsRaidOfficer() == nil").unwrap());
        let mut raid = ten_player_raid();
        raid.own_guid = raid.raid[1].guid;
        raid.raid[1].rank = 1;
        s.set_party(raid.clone());
        assert_eq!(s.eval::<i64>("return IsRaidOfficer()").unwrap(), 1);
        raid.raid[1].rank = 0;
        s.set_party(raid);
        assert!(s.eval::<bool>("return IsRaidOfficer() == nil").unwrap());
    }

    // ── The identity predicates ─────────────────────────────────────────────

    fn unit(exists: bool, guid: u64) -> crate::script::UnitState {
        crate::script::UnitState {
            exists,
            guid,
            ..Default::default()
        }
    }

    /// Pushes `u` as the target beside the guid table the app feeds with it, `g` with `u`'s guid
    /// in the target global (`[0xbd07b0]`).
    fn set_target(s: &mut UiScript, g: &crate::script::UnitGuids, u: crate::script::UnitState) {
        let target = u.guid;
        s.set_unit("target", Some(u));
        s.set_unit_guids(&crate::script::UnitGuids {
            target,
            ..g.clone()
        });
    }

    #[test]
    fn unit_is_unit_compares_guids_and_tokens() {
        let mut s = UiScript::new().unwrap();
        let (player, target, party) = (0x10, 0x10, [0x20, 0, 0, 0]);
        let g = crate::script::UnitGuids {
            player,
            target,
            party,
            ..Default::default()
        };
        s.set_unit_guids(&g);
        assert_eq!(
            s.eval::<i64>(r#"return UnitIsUnit("target", "player")"#)
                .unwrap(),
            1
        );
        assert_eq!(
            s.eval::<i64>(r#"return UnitIsUnit("player", "player")"#)
                .unwrap(),
            1
        );
        assert!(s
            .eval::<bool>(r#"return UnitIsUnit("party1", "player") == nil"#)
            .unwrap());
        // Zero guids never match across tokens.
        s.set_unit_guids(&crate::script::UnitGuids { target: 0, ..g });
        assert!(s
            .eval::<bool>(r#"return UnitIsUnit("target", "mouseover") == nil"#)
            .unwrap());
        assert!(s
            .eval::<bool>(r#"return UnitIsUnit("pet", "player") == nil"#)
            .unwrap());
    }

    /// A member's own guid, or a member's pet: the party's for `UnitPlayerOrPetInParty`, the raid
    /// roster's for `UnitPlayerOrPetInRaid`.
    #[test]
    fn unit_player_or_pet_in_party_reads_the_owner() {
        let mut s = UiScript::new().unwrap();
        s.set_party(two_member_party());
        let (player, pet, party) = (0x10, 0x11, [0xA11CE, 0xB0B, 0, 0]);
        let party_pets = [0xC0FFEE, 0, 0, 0];
        let g = crate::script::UnitGuids {
            player,
            pet,
            party,
            party_pets,
            ..Default::default()
        };
        set_target(&mut s, &g, unit(true, 0xC0FFEE));
        assert_eq!(
            s.eval::<i64>(r#"return UnitPlayerOrPetInParty("target")"#)
                .unwrap(),
            1,
            "Alice's pet is party1's (`0x52c8c0`)"
        );
        // The slots hold the other members only: we and our pet are not in them.
        let us = "return UnitPlayerOrPetInParty('player') == nil \
                  and UnitPlayerOrPetInParty('pet') == nil";
        assert!(s.eval::<bool>(us).unwrap());
        let raid = "return UnitPlayerOrPetInRaid('target') == nil \
                    and UnitPlayerOrPetInRaid({}) == nil";
        assert!(
            s.eval::<bool>(raid).unwrap(),
            "no raid: the raid twin says nil"
        );
        let mut strangers_pet = unit(true, 0xBEEF);
        strangers_pet.owner = 0xDEAD;
        set_target(&mut s, &g, strangers_pet);
        assert!(s
            .eval::<bool>(r#"return UnitPlayerOrPetInParty("target") == nil"#)
            .unwrap());
        // A member's own guid still answers, as UnitInParty does.
        set_target(&mut s, &g, unit(true, 0xB0B));
        assert_eq!(
            s.eval::<i64>(r#"return UnitPlayerOrPetInParty("target")"#)
                .unwrap(),
            1
        );
        // A raid row's pet (`0x573200`): raidpet1 is row 1's.
        s.set_party(ten_player_raid());
        let raid = crate::script::UnitGuids {
            raid: ten_player_raid().raid.iter().map(|m| m.guid).collect(),
            raid_pets: vec![0xC0FFEE],
            ..g.clone()
        };
        let mut raiders_pet = unit(true, 0xC0FFEE);
        raiders_pet.owner = ten_player_raid().raid[0].guid;
        set_target(&mut s, &raid, raiders_pet);
        assert_eq!(
            s.eval::<i64>(r#"return UnitPlayerOrPetInRaid("target")"#)
                .unwrap(),
            1
        );
        s.set_party(crate::script::PartyState::default());
        assert!(s
            .eval::<bool>(r#"return UnitPlayerOrPetInParty("target") == nil"#)
            .unwrap());
    }

    #[test]
    fn unit_in_party_matches_roster_guids() {
        let mut s = UiScript::new().unwrap();
        s.set_party(two_member_party());
        s.set_unit("player", Some(unit(true, 0x10)));
        s.set_unit("party1", Some(unit(true, 0xA11CE)));
        let g = crate::script::UnitGuids {
            player: 0x10,
            party: [0xA11CE, 0, 0, 0],
            ..Default::default()
        };
        set_target(&mut s, &g, unit(true, 0xA11CE));
        assert_eq!(s.eval::<i64>(r#"return UnitInParty("target")"#).unwrap(), 1);
        assert_eq!(s.eval::<i64>(r#"return UnitInParty("party1")"#).unwrap(), 1);
        set_target(&mut s, &g, unit(true, 0xDEAD));
        assert!(s
            .eval::<bool>(r#"return UnitInParty("target") == nil"#)
            .unwrap());
        // A raid-only member is in no party slot (`0x52c680`).
        s.set_party(ten_player_raid());
        set_target(&mut s, &g, unit(true, ten_player_raid().raid[5].guid));
        assert!(s
            .eval::<bool>(r#"return UnitInParty("target") == nil"#)
            .unwrap());
        // Ungrouped, we are still in our own party: 1 for our guid (`0x52c69c`).
        s.set_party(crate::script::PartyState::default());
        assert_eq!(s.eval::<i64>(r#"return UnitInParty("player")"#).unwrap(), 1);
    }

    #[test]
    fn unit_can_cooperate_needs_a_friendly_player() {
        let mut s = UiScript::new().unwrap();
        let mut friendly = unit(true, 0x30);
        friendly.is_player = true;
        friendly.reaction = 5;
        s.set_unit("target", Some(friendly.clone()));
        assert_eq!(
            s.eval::<i64>(r#"return UnitCanCooperate("player", "target")"#)
                .unwrap(),
            1
        );
        let mut hostile = friendly.clone();
        hostile.reaction = 2;
        s.set_unit("target", Some(hostile));
        assert!(s
            .eval::<bool>(r#"return UnitCanCooperate("player", "target") == nil"#)
            .unwrap());
        let mut npc = friendly;
        npc.is_player = false;
        s.set_unit("target", Some(npc));
        assert!(s
            .eval::<bool>(r#"return UnitCanCooperate("player", "target") == nil"#)
            .unwrap());
    }

    #[test]
    fn the_raid_verbs_queue_what_they_name() {
        let mut s = UiScript::new().unwrap();
        let mut party = two_member_party();
        party.leader_guid = party.own_guid;
        s.set_party(party);
        s.model_mut().player_req.level = 10;
        s.run(
            r#"
            ConvertToRaid()
            SetRaidSubgroup(7, 3)
            SwapRaidSubgroup(7, 12)
            PromoteByName("Alice")
            PromoteToAssistant("Bob")
            DemoteAssistant("Bob")
            UninviteFromRaid(9)
            DoReadyCheck()
            "#,
        )
        .unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![
                PartyRequest::ConvertToRaid,
                PartyRequest::SetSubgroup { index: 7, group: 3 },
                PartyRequest::SwapSubgroup {
                    index: 7,
                    other: 12
                },
                PartyRequest::PromoteName("Alice".into()),
                PartyRequest::AssistantLeader {
                    name: "Bob".into(),
                    grant: true
                },
                PartyRequest::AssistantLeader {
                    name: "Bob".into(),
                    grant: false
                },
                PartyRequest::UninviteRaid(9),
                PartyRequest::ReadyCheckStart,
            ]
        );
        assert!(s.take_party_requests().is_empty(), "the drain empties");
        // `0x574a59`: below level 10 ConvertToRaid sends nothing.
        s.model_mut().player_req.level = 9;
        s.run("ConvertToRaid()").unwrap();
        assert!(s.take_party_requests().is_empty());
    }

    /// The reference's No button passes no argument, Yes passes `1`.
    #[test]
    fn confirm_ready_check_treats_an_absent_argument_as_not_ready() {
        let mut s = UiScript::new().unwrap();
        s.run("ConfirmReadyCheck(1) ConfirmReadyCheck() ConfirmReadyCheck(false) ConfirmReadyCheck(0)")
            .unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![
                PartyRequest::ReadyCheckAnswer(true),
                PartyRequest::ReadyCheckAnswer(false),
                PartyRequest::ReadyCheckAnswer(false),
                // `0` is true in Lua.
                PartyRequest::ReadyCheckAnswer(true),
            ]
        );
    }

    #[test]
    fn the_raid_roster_selection_is_a_local_cursor() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetRaidRosterSelection()").unwrap(), 0);
        s.run("SetRaidRosterSelection(11)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetRaidRosterSelection()").unwrap(),
            11
        );
        assert!(
            s.take_party_requests().is_empty(),
            "nothing about a selection goes on the wire"
        );
    }

    #[test]
    fn get_raid_target_index_reads_the_fed_mark() {
        let mut s = UiScript::new().unwrap();
        let mut marked = unit(true, 0x40);
        marked.raid_target = 8;
        s.set_unit("target", Some(marked));
        assert_eq!(
            s.eval::<i64>(r#"return GetRaidTargetIndex("target")"#)
                .unwrap(),
            8
        );
        s.set_unit("target", Some(unit(true, 0x40)));
        assert!(s
            .eval::<bool>(r#"return GetRaidTargetIndex("target") == nil"#)
            .unwrap());
    }

    // ── The ready-check timeout ─────────────────────────────────────────────

    /// A member's client arms the deadline, never clears it, and never prints.
    #[test]
    fn the_tick_is_inert_for_a_non_leader() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"READY_CHECK_NO_AFK = "No players are AFK""#)
            .unwrap();
        s.set_party(two_member_party()); // Alice leads; we are 0x5E1F
        s.set_now(100.0);
        s.ready_check_request(false);
        s.set_now(130.0);
        s.run("CheckReadyCheckTime()").unwrap();
        assert!(s.take_ready_check_lines().is_empty());
        assert_eq!(s.model_mut().ready_check.deadline, Some(130.0));
    }
}
