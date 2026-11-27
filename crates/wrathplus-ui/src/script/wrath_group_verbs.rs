//! Build 12340's leader, officer and loot verbs, which read the group globals directly: the leader
//! guid `[0xbd1968]`, the party slots at `0xbd1948` and the raid roster (`0xbeb568`, rank at
//! `+0xc`), compared with the active player's guid (`0x4d3790`).

use mlua::{Lua, MultiValue, Value};

use crate::script::binding_abi::{bool_or_default, flag};
use crate::script::{Model, PartyRequest};

/// The local player leads: `[0xbd1968]` is non-zero and equals our guid (`0x52cce3`), no raid
/// flag read.
fn leads(m: &Model) -> bool {
    let me = m.local_guid();
    me != 0 && m.party.leader_guid == me
}

/// The local player's raid row has a rank above 0 (`0x549670`, `[rec+0xc] > 0`): an assistant or
/// the leader.
fn officer(m: &Model) -> bool {
    let me = m.local_guid();
    m.party
        .raid
        .iter()
        .any(|r| me != 0 && r.guid == me && r.rank > 0)
}

/// The loot methods `SetLootMethod` matches whole and case-insensitively (`0x52dcae`-`0x52dd25`),
/// in their wire order.
const METHODS: [&str; 5] = [
    "freeforall",
    "roundrobin",
    "master",
    "group",
    "needbeforegreed",
];

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // IsPartyLeader (`0x52ccd0`): the leader guid is non-zero and ours. IsRaidLeader (`0x573a60`):
    // ours equals it, with no zero test and no raid flag, so a party leader answers 1 to both, and
    // so does a VM with neither guid known.
    g.set(
        "IsPartyLeader",
        lua.create_function(|lua, ()| {
            Ok(flag(leads(&lua.app_data_ref::<Model>().expect("model"))))
        })?,
    )?;
    g.set(
        "IsRaidLeader",
        lua.create_function(|lua, ()| {
            let m = lua.app_data_ref::<Model>().expect("model");
            Ok(flag(m.party.leader_guid == m.local_guid()))
        })?,
    )?;
    // IsRaidOfficer (`0x573b00`): our raid row's rank above 0.
    g.set(
        "IsRaidOfficer",
        lua.create_function(|lua, ()| {
            Ok(flag(officer(&lua.app_data_ref::<Model>().expect("model"))))
        })?,
    )?;
    // GetPartyMember(n) (`0x52c1d0`): `lua_isnumber`, truncated (`0x52c1f8`), and `n - 1` below 4
    // unsigned, else the same `Usage:` raises; 1 when that slot holds a guid.
    g.set(
        "GetPartyMember",
        lua.create_function(|lua, n: Value| {
            let usage = "Usage: GetPartyMember(1-4)";
            let slot = match lua.coerce_number(n)? {
                Some(n) => (n as i64 as u32).wrapping_sub(1),
                None => return Err(mlua::Error::runtime(usage)),
            };
            if slot >= 4 {
                return Err(mlua::Error::runtime(usage));
            }
            let m = lua.app_data_ref::<Model>().expect("model");
            let held = m.party.members.get(slot as usize);
            Ok(flag(held.is_some_and(|p| p.guid != 0)))
        })?,
    )?;
    // ConvertToRaid (`0x574a00`): silent unless party slot 1 is filled, we lead, and our level is
    // 10 or more (`0x574a59`); then `CMSG_GROUP_RAID_CONVERT` (`0x28e`).
    g.set(
        "ConvertToRaid",
        lua.create_function(|lua, ()| {
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let filled = m.party.members.first().is_some_and(|p| p.guid != 0);
            if filled && leads(&m) && m.player_req.level >= 10 {
                m.party_requests.push(PartyRequest::ConvertToRaid);
            }
            Ok(())
        })?,
    )?;
    // SetLootMethod("method" [, master [, exactMatch]]) (`0x52dc20`), in its order: not grouped is
    // ERR_NOT_IN_GROUP (`0x50`) and not leading ERR_NOT_LEADER (`0x54`), both chat-kind messages;
    // `lua_isstring(1)` else `Usage:`; a method no whole compare matches raises "Invalid loot
    // method"; `master` wants a non-empty name (else ERR_SPECIFY_MASTER_LOOTER, `0xfc`) naming us
    // or a member (`0x520190`, then `0x52c680`/`0x5726f0`; else ERR_TARGET_NOT_IN_GROUP_S, `0x51`).
    // The send carries the current threshold (`[0xacc6cc]`, `0x52de1d`): there is no threshold
    // argument. Labelled: the name is matched whole; `0x520190`'s non-exact nearest-match leg is
    // not built.
    g.set(
        "SetLootMethod",
        lua.create_function(|lua, (method, master): (Value, Value)| {
            let (grouped, leader) = {
                let m = lua.app_data_ref::<Model>().expect("model");
                let grouped = !m.party.members.is_empty() || !m.party.raid.is_empty();
                (grouped, leads(&m))
            };
            if !grouped || !leader {
                let key = if grouped {
                    "ERR_NOT_LEADER"
                } else {
                    "ERR_NOT_IN_GROUP"
                };
                lua.app_data_mut::<Model>()
                    .expect("model")
                    .ui_errors
                    .push(key);
                return Ok(());
            }
            let usage = r#"Usage: SetLootMethod("method" [,master])"#;
            let method = crate::script::binding_abi::string_arg(lua, method, usage)?;
            let Some(method) = METHODS.iter().find(|m| m.eq_ignore_ascii_case(&method)) else {
                return Err(mlua::Error::runtime("Invalid loot method"));
            };
            let mut master_name = None;
            if *method == "master" {
                let name = crate::script::unit::token::token_text(lua, &master).unwrap_or_default();
                if name.is_empty() {
                    let mut m = lua.app_data_mut::<Model>().expect("model");
                    m.ui_errors.push("ERR_SPECIFY_MASTER_LOOTER");
                    return Ok(());
                }
                let found = {
                    let m = lua.app_data_ref::<Model>().expect("model");
                    let is = |n: &str| n.eq_ignore_ascii_case(&name);
                    is(&m.player_record.name)
                        || m.party.members.iter().any(|p| is(&p.name))
                        || m.party.raid.iter().any(|p| is(&p.name))
                };
                if !found {
                    let mut m = lua.app_data_mut::<Model>().expect("model");
                    m.ui_errors_s.push(("ERR_TARGET_NOT_IN_GROUP_S", name));
                    return Ok(());
                }
                master_name = Some(name);
            }
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let threshold = Some(m.party.loot_threshold);
            m.party_requests.push(PartyRequest::LootMethod {
                method: method.to_string(),
                master_name,
                threshold,
            });
            Ok(())
        })?,
    )?;
    // SetLootThreshold(threshold) (`0x52de60`), in its order: not grouped is ERR_NOT_IN_GROUP
    // (`0x50`) and not leading ERR_NOT_LEADER (`0x54`); then `isnumber` (the usage line's typo is
    // 12340's, `0xa07ffc`), `ftol`, and 2 to 6 (`0xa07fc4`). The send keeps the method and master.
    g.set(
        "SetLootThreshold",
        lua.create_function(|lua, n: Value| {
            let mut m = lua.app_data_mut::<Model>().expect("model");
            if !grouped(&m) {
                m.ui_errors.push("ERR_NOT_IN_GROUP");
                return Ok(());
            }
            if !leads(&m) {
                m.ui_errors.push("ERR_NOT_LEADER");
                return Ok(());
            }
            let usage = "Usage: SetLooThreshold(threshold)";
            let n = crate::script::binding_abi::ftol_arg(lua, n, usage)?;
            if !(2..=6).contains(&n) {
                return Err(mlua::Error::runtime(
                    "SetLootThreshold(): threshold must be between 2 and 6",
                ));
            }
            // Labelled: 12340 also needs the active player object (`0x4038f0`, `0x52def8`).
            m.party_requests.push(PartyRequest::LootThreshold(n as u32));
            Ok(())
        })?,
    )?;
    // ConfirmReadyCheck([ready]) (`0x5740c0`): only grouped with the check's deadline `[0xbeb61c]`
    // still ahead; the flag by `0x815500` with default false, so 0 and "0" are not ready.
    g.set(
        "ConfirmReadyCheck",
        lua.create_function(|lua, args: MultiValue| {
            let mut m = lua.app_data_mut::<Model>().expect("model");
            if grouped(&m) && m.wrath_ready_check.ends_at.is_some_and(|end| end > m.now) {
                let ready = bool_or_default(args.front(), false);
                m.party_requests.push(PartyRequest::ReadyCheckAnswer(ready));
            }
            Ok(())
        })?,
    )?;
    // PromoteToAssistant(name [, exactMatch]) (`0x573ef0`) and DemoteAssistant (`0x573fd0`):
    // `lua_isstring(1)` else the usage line; the text through `0x520190` (players, filter 6, exact
    // by `0x815500(L, 2, 0)`), and a guid found sends CMSG_GROUP_ASSISTANT_LEADER (`0x28f`) with
    // the flag 1 or 0. A miss prints nothing.
    for (verb, grant, usage) in [
        (
            "PromoteToAssistant",
            true,
            "Usage: PromoteToAssistant(name)",
        ),
        ("DemoteAssistant", false, "Usage: DemoteAssistant(name)"),
    ] {
        g.set(
            verb,
            lua.create_function(move |lua, args: MultiValue| {
                let who = args.front().cloned().unwrap_or(Value::Nil);
                let who = crate::script::binding_abi::string_arg(lua, who, usage)?;
                let exact = bool_or_default(args.get(1), false);
                let mut m = lua.app_data_mut::<Model>().expect("model");
                if let Some(guid) =
                    super::wrath_party_assignment::resolve_member(&m, &who, true, exact)
                {
                    m.party_requests
                        .push(PartyRequest::AssistantLeader { guid, grant });
                }
                Ok(())
            })?,
        )?;
    }
    // SetRaidRosterSelection(index) (`0x572bc0`): truncated; the guid of raid row `index`, or 0
    // past the roster (`0x572c3e`). GetRaidRosterSelection (`0x572c50`): that guid's row now, or 0.
    g.set(
        "SetRaidRosterSelection",
        lua.create_function(|lua, index: Value| {
            let usage = "Usage: SetRaidRosterSelection(index)";
            let index = crate::script::binding_abi::number_arg(lua, index, usage)?;
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let row = usize::try_from(index.wrapping_sub(1)).ok();
            m.raid_selection = row.and_then(|i| m.party.raid.get(i)).map_or(0, |r| r.guid);
            Ok(())
        })?,
    )?;
    g.set(
        "GetRaidRosterSelection",
        lua.create_function(|lua, ()| {
            let m = lua.app_data_ref::<Model>().expect("model");
            let guid = m.raid_selection;
            let row = m
                .party
                .raid
                .iter()
                .position(|r| guid != 0 && r.guid == guid);
            Ok(row.map_or(0, |i| i as i64 + 1))
        })?,
    )?;
    Ok(())
}

/// In a group: a party member or a raid roster (`0x52de63`-`0x52de76`).
fn grouped(m: &Model) -> bool {
    !m.party.members.is_empty() || !m.party.raid.is_empty()
}

/// `DoReadyCheck`'s gate (`0x5732b0`): in a raid the leader or an officer (`0x572740`), else the
/// party leader.
pub(crate) fn may_ready_check(m: &Model) -> bool {
    if m.party.raid.is_empty() {
        leads(m)
    } else {
        leads(m) || officer(m)
    }
}
