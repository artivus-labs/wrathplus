//! Build 12340: Get 0x52cf60/0x52ca00/0x572950, Set 0x52e1b0, Clear 0x52e400 (body 0x52df40).
//! Assignment state comes only from the roster echo, not optimistic local writes.
use super::super::unit::token::{sstr_eq_i_n, NOBODY};
use super::super::{binding_abi, clock, Model, PartyRequest};
use mlua::{Lua, MultiValue, Value};

const GET_USAGE: &str = "Usage: GetPartyAssignment(\"assignment\" [,\"raidmember\"] [,exactMatch])";

/// `MAINTANK` 0 or `MAINASSIST` 1, compared whole and case-insensitively (`0x76e780`).
fn assignment(lua: &Lua, value: Value, usage: &'static str, invalid: &str) -> mlua::Result<u8> {
    let name = binding_abi::string_arg(lua, value, usage)?;
    if name.eq_ignore_ascii_case("MAINTANK") {
        Ok(0)
    } else if name.eq_ignore_ascii_case("MAINASSIST") {
        Ok(1)
    } else {
        Err(mlua::Error::RuntimeError(invalid.into()))
    }
}

/// A candidate's rank when it is our own unit, the anchor (`0x52006a`), or a groupmate the client
/// does not hold, ranked from the name cache (`0x520331`): `2.5e9`, behind any held unit.
const FAR: f32 = 2.5e9;
/// Added for a held unit at 0 health or `UNIT_DYNFLAG_DEAD` (`0x5200dd`).
const DEAD_PENALTY: f32 = 10_000.0;

/// `0x520190`'s pass over players for `filter` 5 (us and the party) or, with `raid`, 6 (and the
/// raid). The unit resolver goes first (`0x5201e5`, flag 1: no text is the selection) and decides
/// unless it answers nobody; then the candidate whose name starts with the text (whole when
/// `exact`) and ranks lowest wins, strictly, in `0x520190`'s order: unheld party rows, unheld raid
/// rows, then the held units. Held is a guid with a `unit_reach` d², from us rather than from a
/// unit we control (`0x6dd060`). Labelled: stand state 7 is not read for the dead penalty, and
/// the object manager's order among equal ranks is taken to be the roster's. At most 41 rows per
/// call, nothing per frame.
pub(crate) fn resolve_member(m: &Model, text: &str, raid: bool, exact: bool) -> Option<u64> {
    let me = m.unit("player").map_or(m.unit_guids.player, |p| p.guid);
    let member = |g: u64| {
        (g != 0
            && (g == me
                || m.party.members.iter().any(|p| p.guid == g)
                || raid && m.party.raid.iter().any(|p| p.guid == g)))
        .then_some(g)
    };
    let token = if text.is_empty() {
        Some(m.unit_guid("target").unwrap_or(0))
    } else {
        let r = m.resolve_token(text);
        r.ok.then_some(r.guid)
    };
    if let Some(g) = token.filter(|&g| g != NOBODY) {
        return member(g);
    }
    let n = if exact {
        usize::MAX
    } else {
        text.chars().count()
    };
    let matches = |name: &str| sstr_eq_i_n(name.as_bytes(), text.as_bytes(), n);
    let party = m
        .party
        .members
        .iter()
        .take(4)
        .map(|p| (p.guid, p.name.as_str()));
    let raid_rows = m.party.raid.iter().filter(|_| raid);
    let rows: Vec<_> = party
        .chain(raid_rows.map(|p| (p.guid, p.name.as_str())))
        .collect();
    let held = |g: u64| m.unit_reach.get(&g).map(|r| r.dist_sq as f32);
    let mut best: Option<(f32, u64)> = None;
    let mut offer = |g: u64, name: Option<&str>, rank: f32| {
        if g != 0 && name.is_some_and(matches) && best.is_none_or(|(b, _)| rank < b) {
            best = Some((rank, g));
        }
    };
    for &(g, name) in rows.iter().filter(|(g, _)| *g != me && held(*g).is_none()) {
        offer(g, Some(name), FAR);
    }
    let unit_name = |g: u64| m.unit_by_guid(g).and_then(|u| u.name.as_deref());
    offer(
        me,
        unit_name(me).or(Some(m.player_record.name.as_str())),
        FAR,
    );
    for &(g, name) in &rows {
        let (Some(d2), true) = (held(g), g != me) else {
            continue;
        };
        let dead = m
            .unit_by_guid(g)
            .is_some_and(|u| u.dead || u.dynamic_flags & 0x20 != 0);
        offer(
            g,
            unit_name(g).or(Some(name)),
            d2 + if dead { DEAD_PENALTY } else { 0.0 },
        );
    }
    best.map(|(_, g)| g)
}

/// `0x815500(L, 3, 0)`: the reference's boolean coercion, not Lua truthiness.
fn exact_match(args: &MultiValue) -> bool {
    binding_abi::bool_or_default(args.get(2), false)
}

/// The refusal after Set/Clear's pass fails (`0x52e30f`-`0x52e3ae`): a held unit the text names
/// is no groupmate (`0x1d2`); else a text that is neither empty nor `target` names nobody
/// (`0x13a`); else there is no target (`0xc7`). Labelled: the second pass's name leg over every
/// held unit (`typemask 8`) is not built, only its unit-resolver leg.
fn unresolved_key(m: &Model, text: &str) -> &'static str {
    let r = m.resolve_token(if text.is_empty() { "target" } else { text });
    if r.ok && m.unit_reach.contains_key(&r.guid) {
        "ERR_MEETING_STONE_TARGET_NOT_IN_PARTY"
    } else if !text.is_empty() && !text.eq_ignore_ascii_case("target") {
        "ERR_UNIT_NOT_FOUND"
    } else {
        "ERR_GENERIC_NO_TARGET"
    }
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
            let a = assignment(
                lua,
                args.front().cloned().unwrap_or(Value::Nil),
                GET_USAGE,
                "GetPartyAssignment(): Invalid party assignment",
            )?;
            let member = args
                .get(1)
                .and_then(|v| binding_abi::optional_string(lua, v));
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let bit = 2 << a;
            if let Some(token) = member {
                let raid = !m.party.raid.is_empty();
                let Some(g) = resolve_member(&m, &token, raid, exact_match(&args)) else {
                    // `DisplayError(0x51, name)` and no returns (`0x52ca92`).
                    m.ui_errors_s.push(("ERR_TARGET_NOT_IN_GROUP_S", token));
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
    for (name, apply, usage) in [
        (
            "SetPartyAssignment",
            true,
            "Usage: SetPartyAssignment(\"assignment\" [,\"raidmember\"] [,exactMatch])",
        ),
        (
            "ClearPartyAssignment",
            false,
            "Usage: ClearPartyAssignment(\"assignment\" [,\"raidmember\"] [,exactMatch])",
        ),
    ] {
        lua.globals().set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let now = clock::now(lua);
                // Both refusals are kind 0 in the 12340 message table (`ERR_NOT_IN_GROUP` entry
                // at `0xac9130`, `ERR_NOT_LEADER` at `0xac9180`): a system chat line, which the
                // message sink raises with the chat event's full argument list.
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
                let mut m = lua.app_data_mut::<Model>().expect("model");
                if let Some(key) = refusal {
                    m.ui_errors.push(key);
                    return Ok(());
                }
                let a = assignment(
                    lua,
                    args.front().cloned().unwrap_or(Value::Nil),
                    usage,
                    "Invalid Party assignment",
                )?;
                if m.assignment_sent_at.is_some_and(|t| now - t < 0.750) {
                    return Ok(());
                }
                let g = if let Some(token) = args
                    .get(1)
                    .and_then(|v| binding_abi::optional_string(lua, v))
                {
                    let Some(g) = resolve_member(&m, &token, true, exact_match(&args)) else {
                        let key = unresolved_key(&m, &token);
                        m.ui_errors.push(key);
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
        // The resolver reads the guid table (`0x60abf0`), the frames the snapshots.
        s.set_unit_guids(&crate::script::UnitGuids {
            player: 1,
            target: 2,
            party: [2, 0, 0, 0],
            raid: if raid { vec![1, 2] } else { vec![] },
            wrath: true,
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
            let mut s = fixture(raid);
            assert_eq!(
                s.eval::<i64>("return GetPartyAssignment('maintank')")
                    .unwrap(),
                if raid { 2 } else { 1 }
            );
            assert!(s.eval::<bool>("return GetPartyAssignment('MAINTANK','target') == 1 and GetPartyAssignment('MAINASSIST','target') == nil and GetPartyAssignment('MAINASSIST','player') == 1").unwrap());
            // A plain name matches its first letters (`0x51ffe0`), as `/maintank` passes it.
            assert!(s
                .eval::<bool>("return GetPartyAssignment('MAINTANK','al') == 1")
                .unwrap());
            assert_eq!(
                s.arity("GetPartyAssignment('MAINTANK','missing')").unwrap(),
                0
            );
            assert_eq!(
                s.take_ui_errors_s(),
                vec![("ERR_TARGET_NOT_IN_GROUP_S", "missing".to_string())]
            );
            let err = |call: &str| s.run(call).unwrap_err().to_string();
            assert!(err("GetPartyAssignment('bad')")
                .contains("GetPartyAssignment(): Invalid party assignment"));
            assert!(err("GetPartyAssignment()").contains(super::GET_USAGE));
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
        s.eval::<()>("SetPartyAssignment('MAINTANK','Stranger')")
            .unwrap();
        assert!(s.take_party_requests().is_empty());
        assert_eq!(s.take_ui_errors(), vec!["ERR_UNIT_NOT_FOUND"]);
        let err = s.run("SetPartyAssignment()").unwrap_err().to_string();
        assert!(err.contains("Usage: SetPartyAssignment("), "{err}");
        // Solo, both verbs refuse through the message table, never a bare CHAT_MSG_SYSTEM.
        let mut solo = UiScript::new().unwrap();
        solo.run("SetPartyAssignment('MAINTANK') ClearPartyAssignment('MAINTANK')")
            .unwrap();
        assert_eq!(solo.take_ui_errors(), vec!["ERR_NOT_IN_GROUP"; 2]);
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
        s.eval::<()>("SetPartyAssignment('MAINASSIST','Stranger-Realm',1); SetPartyAssignment('MAINASSIST','Ally',1)").unwrap();
        assert!(s.take_party_requests().is_empty(), "exact: the whole name");
        // Not exact (`0x815500` reads 0 as false): the first letters, so the realm can go.
        s.eval::<()>("SetPartyAssignment('MAINASSIST','Ally',0)")
            .unwrap();
        assert_eq!(s.take_party_requests().len(), 1);
        s.tick(0.75);
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

    /// `0x520190`'s ranks: a held groupmate by d² (+10000 dead), ahead of an unheld one and of us
    /// (both 2.5e9), and the first unheld row before us on a tie.
    #[test]
    fn the_name_pass_takes_the_nearest_living_match() {
        let mut s = fixture(true);
        let mut p = s.model_mut().party.clone();
        p.members[0].name = "Alba".into();
        p.raid.push(crate::script::RaidMemberInfo {
            guid: 3,
            name: "Albert".into(),
            ..Default::default()
        });
        s.set_party(p);
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                name: Some("Alfred".into()),
                exists: true,
                ..Default::default()
            }),
        );
        let m = |s: &mut UiScript, text| super::resolve_member(&s.model_mut(), text, true, false);
        assert_eq!(
            m(&mut s, "al"),
            Some(2),
            "the first unheld row ties us and wins"
        );
        assert_eq!(m(&mut s, "alf"), Some(1), "us, by name");
        let reach = |d| crate::script::UnitReach {
            dist_sq: d,
            inspectable: true,
        };
        s.set_unit_reach([(3, reach(400.0)), (2, reach(100.0))].into());
        assert_eq!(m(&mut s, "al"), Some(2), "the nearest held");
        for token in ["party1", "raid2", "target"] {
            let dead = UnitState {
                guid: 2,
                dead: true,
                exists: true,
                ..Default::default()
            };
            s.set_unit(token, Some(dead));
        }
        assert_eq!(m(&mut s, "al"), Some(3), "a dead one ranks 10000 back");
        assert_eq!(m(&mut s, "ALBERT"), Some(3));
        assert_eq!(m(&mut s, "zed"), None);
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
