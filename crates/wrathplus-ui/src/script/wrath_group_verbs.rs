//! Build 12340's leader, officer and loot verbs, which read the group globals directly: the leader
//! guid `[0xbd1968]`, the party slots at `0xbd1948` and the raid roster (`0xbeb568`, rank at
//! `+0xc`), compared with the active player's guid (`0x4d3790`).

use mlua::{Lua, Value};

use crate::script::binding_abi::flag;
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

/// A message table entry (`0x5216f0`, table `0xac8af0`) of kind 0: a system chat line, `%s`
/// filled with `arg`.
fn system_line(lua: &Lua, key: &str, arg: &str) -> mlua::Result<()> {
    let text = match lua.globals().get::<Value>(key)? {
        Value::String(s) => s.to_string_lossy().replacen("%s", arg, 1),
        _ => key.to_string(),
    };
    crate::script::tick::fire_event_into(
        lua,
        "CHAT_MSG_SYSTEM",
        vec![crate::script::ScriptValue::Str(text)],
    );
    Ok(())
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
    // ERR_NOT_IN_GROUP (`0x50`) and not leading ERR_NOT_LEADER (`0x54`), both system lines;
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
            if !grouped {
                return system_line(lua, "ERR_NOT_IN_GROUP", "");
            }
            if !leader {
                return system_line(lua, "ERR_NOT_LEADER", "");
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
                    return system_line(lua, "ERR_TARGET_NOT_IN_GROUP_S", &name);
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
    Ok(())
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
