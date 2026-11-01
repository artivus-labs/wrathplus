//! Build 12340: Get 0x52cf60/0x52ca00/0x572950, Set 0x52e1b0, Clear 0x52df40.
//! Assignment state comes only from the roster echo, not optimistic local writes.
use super::super::{binding_abi, clock, Model, PartyRequest};
use mlua::{Lua, MultiValue, Value};

fn assignment(lua: &Lua, value: Value) -> mlua::Result<u8> {
    let name = binding_abi::string_arg(
        lua,
        value,
        "Usage: Get/Set/ClearPartyAssignment(\"assignment\" [,\"raidmember\"] [,exactMatch])",
    )?;
    if name.eq_ignore_ascii_case("MAINTANK") {
        Ok(0)
    } else if name.eq_ignore_ascii_case("MAINASSIST") {
        Ok(1)
    } else {
        Err(mlua::Error::RuntimeError("Invalid Party assignment".into()))
    }
}
fn guid(m: &Model, token: &str, raid: bool, exact: bool) -> Option<u64> {
    let resolved = m
        .unit_guids
        .resolve(token)
        .ok()
        .flatten()
        .or_else(|| m.unit(token).map(|u| u.guid))
        .filter(|g| *g != 0);
    if let Some(g) = resolved {
        let own = m.unit_guids.player != 0 && g == m.unit_guids.player
            || m.unit("player").is_some_and(|p| p.guid == g);
        return (own
            || m.party.members.iter().any(|p| p.guid == g)
            || raid && m.party.raid.iter().any(|p| p.guid == g))
        .then_some(g);
    }
    // TODO shared non-exact resolver 0x520190 -> 0x51ffe0: enumerate held players,
    // filter 5=self/party (0x52c680), 6=self/party/raid (0x512a00); match names
    // case-insensitively, preserving realm qualification. Rank by squared 3D
    // distance to player/controlled-unit anchor, +10000 if health<=0 OR dynamic
    // flags 0x20 OR stand-state 7. Self uses float at 0xA02A3C. Enumeration tie
    // order and controlled anchor remain unaudited; never substitute roster-first.
    // See https://github.com/artivus-labs/wrathplus/wiki/WRATH-PARTY-ASSIGNMENT-RESOLVER; no new snapshot in this unit.
    if !exact || token.is_empty() {
        return None;
    }
    // UnitPopup passes full names and exactMatch=1. Within self/party/raid a
    // full (including realm, if supplied) name is unique: ranking cannot affect it.
    // Compare the complete supplied name; never strip a realm or accept a prefix.
    let mut matches = Vec::new();
    if let Some(p) = m.unit("player") {
        if p.name
            .as_deref()
            .is_some_and(|n| n.eq_ignore_ascii_case(token))
        {
            matches.push(p.guid);
        }
    }
    matches.extend(
        m.party
            .members
            .iter()
            .filter(|p| p.name.eq_ignore_ascii_case(token))
            .map(|p| p.guid),
    );
    if raid {
        matches.extend(
            m.party
                .raid
                .iter()
                .filter(|p| p.name.eq_ignore_ascii_case(token))
                .map(|p| p.guid),
        );
    }
    matches.retain(|g| *g != 0);
    matches.sort_unstable();
    matches.dedup();
    (matches.len() == 1).then(|| matches[0])
}
fn exact_match(lua: &Lua, args: &MultiValue) -> bool {
    // Native 0x815500 parses this as a numeric flag, not Lua truthiness.
    binding_abi::coerced_number(lua, args.get(2).cloned()) != 0.0
}

fn flags(m: &Model, guid: u64) -> u8 {
    m.wrath_group
        .members
        .iter()
        .find(|p| p.guid == guid)
        .map_or(0, |p| p.flags)
}
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetPartyAssignment",
        lua.create_function(|lua, args: MultiValue| {
            let a = assignment(lua, args.front().cloned().unwrap_or(Value::Nil))?;
            let member = args
                .get(1)
                .and_then(|v| binding_abi::optional_string(lua, v));
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let bit = 2 << a;
            if let Some(token) = member {
                let Some(g) = guid(
                    &m,
                    &token,
                    !m.party.raid.is_empty(),
                    exact_match(lua, &args),
                ) else {
                    m.ui_errors.push("PLAYER_NOT_FOUND");
                    return Ok(MultiValue::new());
                };
                return Ok(MultiValue::from_vec(vec![if flags(&m, g) & bit != 0 {
                    Value::Integer(1)
                } else {
                    Value::Nil
                }]));
            }
            let guids: Vec<_> = if m.party.raid.is_empty() {
                m.party.members.iter().map(|p| p.guid).collect()
            } else {
                m.party.raid.iter().map(|p| p.guid).collect()
            };
            Ok(MultiValue::from_vec(
                guids
                    .iter()
                    .enumerate()
                    .filter_map(|(i, g)| {
                        (flags(&m, *g) & bit != 0).then_some(Value::Integer((i + 1) as i64))
                    })
                    .collect(),
            ))
        })?,
    )?;
    for (name, apply) in [
        ("SetPartyAssignment", true),
        ("ClearPartyAssignment", false),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let now = clock::now(lua);
                // Both refusals are kind 0 in the 12340 message table (`ERR_NOT_IN_GROUP` entry
                // at `0xac9130`, `ERR_NOT_LEADER` at `0xac9180`): a system chat line, not a red
                // UI error (kind 2).
                let refusal = {
                    let m = lua.app_data_ref::<Model>().expect("model");
                    let own = m.unit("player").map_or(m.unit_guids.player, |p| p.guid);
                    if m.party.members.is_empty() && m.party.raid.is_empty() {
                        Some("ERR_NOT_IN_GROUP")
                    } else if own == 0
                        || (own != m.party.leader_guid
                            && !(m.party.raid.iter().any(|p| p.guid == own)
                                && flags(&m, own) & 1 != 0))
                    {
                        Some("ERR_NOT_LEADER")
                    } else {
                        None
                    }
                };
                if let Some(key) = refusal {
                    let message = match lua.globals().get::<Value>(key)? {
                        Value::String(s) => s.to_string_lossy(),
                        _ => key.to_string(),
                    };
                    super::super::tick::fire_event_into(
                        lua,
                        "CHAT_MSG_SYSTEM",
                        vec![super::super::ScriptValue::Str(message)],
                    );
                    return Ok(());
                }
                let mut m = lua.app_data_mut::<Model>().expect("model");
                let a = assignment(lua, args.front().cloned().unwrap_or(Value::Nil))?;
                if m.assignment_sent_at.is_some_and(|t| now - t < 0.750) {
                    return Ok(());
                }
                let g = if let Some(token) = args
                    .get(1)
                    .and_then(|v| binding_abi::optional_string(lua, v))
                {
                    let Some(g) = guid(&m, &token, true, exact_match(lua, &args)) else {
                        m.ui_errors.push("PLAYER_NOT_FOUND");
                        return Ok(());
                    };
                    g
                } else {
                    0
                };
                m.assignment_sent_at = Some(now);
                m.party_requests.push(PartyRequest::Assignment {
                    assignment: a,
                    apply,
                    guid: g,
                });
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{
        PartyMemberInfo, PartyRequest, PartyState, RaidMemberInfo, UiScript, UnitState, WrathGroup,
        WrathGroupMember,
    };
    fn fixture(raid: bool) -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_party(PartyState {
            leader_guid: 1,
            members: vec![PartyMemberInfo {
                guid: 2,
                name: "Ally".into(),
            }],
            raid: if raid {
                vec![
                    RaidMemberInfo {
                        guid: 1,
                        ..Default::default()
                    },
                    RaidMemberInfo {
                        guid: 2,
                        ..Default::default()
                    },
                ]
            } else {
                vec![]
            },
            ..Default::default()
        });
        for (token, guid) in [("player", 1), ("party1", 2), ("raid2", 2), ("target", 2)] {
            s.set_unit(
                token,
                Some(UnitState {
                    guid,
                    is_player: true,
                    has_object: true,
                    exists: true,
                    ..Default::default()
                }),
            );
        }
        s.set_wrath_group(WrathGroup {
            members: vec![
                WrathGroupMember {
                    guid: 1,
                    flags: 4,
                    roles: 0,
                },
                WrathGroupMember {
                    guid: 2,
                    flags: 2,
                    roles: 0,
                },
            ],
            ..Default::default()
        });
        s
    }
    #[test]
    fn query_returns_indices_or_numeric_one_nil_and_unresolved_zero_arity() {
        for raid in [false, true] {
            let s = fixture(raid);
            assert_eq!(
                s.eval::<i64>("return GetPartyAssignment('maintank')")
                    .unwrap(),
                if raid { 2 } else { 1 }
            );
            assert!(s.eval::<bool>("return GetPartyAssignment('MAINTANK','target') == 1 and GetPartyAssignment('MAINASSIST','target') == nil and GetPartyAssignment('MAINASSIST','player') == 1").unwrap());
            assert_eq!(s.arity("GetPartyAssignment('MAINTANK','Ally')").unwrap(), 0);
            assert_eq!(
                s.arity("GetPartyAssignment('MAINTANK','missing')").unwrap(),
                0
            );
            assert!(s
                .eval::<bool>(
                    "return not pcall(GetPartyAssignment, 'bad') and not pcall(GetPartyAssignment)"
                )
                .unwrap());
        }
    }
    #[test]
    fn sends_full_guid_zero_guid_clear_and_throttles_without_optimistic_writes() {
        let mut s = fixture(false);
        assert_eq!(
            s.arity("SetPartyAssignment('MAINASSIST','TaRgEt')")
                .unwrap(),
            0
        );
        assert_eq!(
            s.take_party_requests(),
            vec![PartyRequest::Assignment {
                assignment: 1,
                apply: true,
                guid: 2
            }]
        );
        assert!(s.eval::<bool>("ClearPartyAssignment('MAINTANK'); return GetPartyAssignment('MAINASSIST','target')==nil").unwrap());
        assert!(s.take_party_requests().is_empty());
        s.tick(0.75);
        s.eval::<()>("ClearPartyAssignment('MAINTANK')").unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![PartyRequest::Assignment {
                assignment: 0,
                apply: false,
                guid: 0
            }]
        );
        s.tick(0.75);
        s.eval::<()>("SetPartyAssignment('MAINTANK','Ally')")
            .unwrap();
        assert!(s.take_party_requests().is_empty());
        // Failed resolution must not consume the throttle.
        s.eval::<()>("SetPartyAssignment('MAINTANK','party1')")
            .unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![PartyRequest::Assignment {
                assignment: 0,
                apply: true,
                guid: 2
            }]
        );
    }
    #[test]
    fn exact_full_roster_names_resolve_but_strangers_and_prefixes_do_not() {
        let mut s = fixture(true);
        let mut p = s.model_mut().party.clone();
        p.members[0].name = "Ally-Realm".into();
        p.raid[1].name = "Ally-Realm".into();
        s.set_party(p);
        assert!(s
            .eval::<bool>("return GetPartyAssignment('MAINTANK','ally-realm',1)==1")
            .unwrap());
        s.eval::<()>("SetPartyAssignment('MAINASSIST','Ally-Realm',1)")
            .unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![PartyRequest::Assignment {
                assignment: 1,
                apply: true,
                guid: 2
            }]
        );
        s.tick(0.75);
        s.eval::<()>("SetPartyAssignment('MAINASSIST','Stranger-Realm',1); SetPartyAssignment('MAINASSIST','Ally',1); SetPartyAssignment('MAINASSIST','Ally-Realm'); SetPartyAssignment('MAINASSIST','Ally',0)").unwrap();
        assert!(s.take_party_requests().is_empty());
        s.eval::<()>("ClearPartyAssignment('MAINTANK','Ally-Realm',1)")
            .unwrap();
        assert_eq!(
            s.take_party_requests(),
            vec![PartyRequest::Assignment {
                assignment: 0,
                apply: false,
                guid: 2
            }]
        );
    }

    #[test]
    fn permissions_and_server_echo_flags_are_authoritative() {
        let mut s = fixture(true);
        let mut p = s.model_mut().party.clone();
        p.leader_guid = 2;
        s.set_party(p);
        s.eval::<()>("SetPartyAssignment('MAINTANK','raid2')")
            .unwrap();
        assert!(s.take_party_requests().is_empty());
        s.set_wrath_group(WrathGroup {
            members: vec![WrathGroupMember {
                guid: 1,
                flags: 1,
                roles: 0,
            }],
            ..Default::default()
        });
        s.eval::<()>("SetPartyAssignment('MAINTANK','raid2')")
            .unwrap();
        assert_eq!(s.take_party_requests().len(), 1);
        s.set_party(Default::default());
        s.tick(1.0);
        s.eval::<()>("ClearPartyAssignment('MAINTANK')").unwrap();
        assert!(s.take_party_requests().is_empty());
    }
}
