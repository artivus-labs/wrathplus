//! Build-12340 channel getters, native 0x504f30/0x505190 and display-list controls.
use super::{
    wrath_channel_display::{entries, selected_index},
    wrath_channel_roster::{Identity, WrathChannelRosterMember, WrathChannelRosterRequest},
    Model,
};
use mlua::{Lua, MultiValue, Value};

fn index(lua: &Lua, value: Value) -> Option<usize> {
    let number = lua.coerce_number(value).ok().flatten()?;
    usize::try_from((number as i64 as i32).wrapping_sub(1)).ok()
}
fn flag(set: bool) -> Value {
    if set {
        Value::Integer(1)
    } else {
        Value::Nil
    }
}
fn count(n: usize) -> Value {
    if n == 0 {
        Value::Nil
    } else {
        Value::Integer(n as i64)
    }
}
fn localized(lua: &Lua, key: &str) -> mlua::Result<Value> {
    let value: Value = lua.globals().get(key)?;
    Ok(if matches!(value, Value::String(_)) {
        value
    } else {
        Value::String(lua.create_string(key)?)
    })
}
fn select(model: &mut Model, identity: Identity) -> bool {
    let changed = model.wrath_channel_roster.select(identity.clone());
    if let Identity::Synthetic(kind) = identity {
        let party = &model.party;
        let rows = if kind == 2 {
            let mut rows = vec![WrathChannelRosterMember {
                guid: party.own_guid,
                name: model.unit("player").and_then(|p| p.name.clone()),
                flags: if party.own_guid == party.leader_guid {
                    5
                } else {
                    4
                },
            }];
            rows.extend(party.members.iter().map(|p| WrathChannelRosterMember {
                guid: p.guid,
                name: Some(p.name.clone()),
                flags: if p.guid == party.leader_guid { 5 } else { 4 },
            }));
            rows
        } else {
            party
                .raid
                .iter()
                .map(|p| WrathChannelRosterMember {
                    guid: p.guid,
                    name: Some(p.name.clone()),
                    flags: match p.rank {
                        2 => 7,
                        1 => 6,
                        _ => 4,
                    },
                })
                .collect()
        };
        model.wrath_channel_roster.set_synthetic(rows);
    }
    changed
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // SetChannelWatch("channel") (`0x4ffc30` → `0x4fe460(0x3ef)`) and ClearChannelWatch()
    // (`0x4ffc50`, `0x3F3` with no body): the channel through the verbs' resolver `0x4fe160`.
    g.set(
        "SetChannelWatch",
        lua.create_function(|lua, value: Value| {
            let Some(name) = lua.coerce_string(value)?.map(|s| s.to_string_lossy()) else {
                return Err(mlua::Error::runtime("Usage: SetChannelWatch(\"channel\")"));
            };
            let resolved = {
                let m = lua.app_data_ref::<Model>().expect("model");
                super::channel::resolve_channel(&m, &name)
            };
            if let Some(name) = resolved {
                let mut m = lua.app_data_mut::<Model>().expect("model");
                m.wrath_channel_roster
                    .requests
                    .push(WrathChannelRosterRequest::Watch { name });
            }
            Ok(())
        })?,
    )?;
    g.set(
        "ClearChannelWatch",
        lua.create_function(|lua, ()| {
            let mut m = lua.app_data_mut::<Model>().expect("model");
            m.wrath_channel_roster
                .requests
                .push(WrathChannelRosterRequest::ClearWatch);
            Ok(())
        })?,
    )?;
    g.set(
        "GetNumDisplayChannels",
        lua.create_function(|lua, ()| {
            let m = lua.app_data_ref::<Model>().expect("model");
            Ok(entries(&m).len())
        })?,
    )?;
    g.set(
        "GetChannelDisplayInfo",
        lua.create_function(|lua, value: Value| {
            let i =
                super::binding_abi::number_arg(lua, value, "Usage: GetChannelDisplayInfo(index)")?;
            let m = lua.app_data_ref::<Model>().expect("model");
            let Some(row) = usize::try_from(i.wrapping_sub(1))
                .ok()
                .and_then(|i| entries(&m).get(i).cloned())
            else {
                return Ok(MultiValue::new());
            };
            if let Some(category) = row.header {
                return Ok(MultiValue::from_vec(vec![
                    localized(lua, &row.name)?,
                    flag(true),
                    flag(m.wrath_channel_display.collapsed[category]),
                    Value::Nil,
                    count(row.count),
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ]));
            }
            let synthetic = matches!(row.identity, Some(Identity::Synthetic(_)));
            let id = m
                .wrath_channel_display
                .channels
                .iter()
                .find(|c| c.name == row.name)
                .map_or(0, |c| c.channel_id);
            let name = if synthetic {
                localized(lua, &row.name)?
            } else {
                // `0x4fdf50(slot, 1)`: a DBC channel's Shortcut, whatever the record's state.
                let name = super::channel::listed_name(&m, &row.name, id);
                Value::String(lua.create_string(name)?)
            };
            let category = if synthetic {
                "CHANNEL_CATEGORY_GROUP"
            } else if id != 0 {
                "CHANNEL_CATEGORY_WORLD"
            } else {
                "CHANNEL_CATEGORY_CUSTOM"
            };
            Ok(MultiValue::from_vec(vec![
                name,
                Value::Nil,
                Value::Nil,
                row.slot.map_or(Value::Nil, |n| Value::Integer(n as i64)),
                count(row.count),
                flag(row.active),
                Value::String(lua.create_string(category)?),
                Value::Nil,
                Value::Nil,
            ]))
        })?,
    )?;
    g.set(
        "GetSelectedDisplayChannel",
        lua.create_function(|lua, ()| {
            let m = lua.app_data_ref::<Model>().expect("model");
            Ok(selected_index(&m))
        })?,
    )?;
    g.set(
        "SetSelectedDisplayChannel",
        lua.create_function(|lua, value: Value| {
            let Some(i) = index(lua, value) else {
                return Ok(());
            };
            let mut m = lua.app_data_mut::<Model>().expect("model");
            if let Some(identity) = entries(&m).get(i).and_then(|r| r.identity.clone()) {
                select(&mut m, identity);
            }
            Ok(())
        })?,
    )?;
    for (name, collapse) in [
        ("CollapseChannelHeader", true),
        ("ExpandChannelHeader", false),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, value: Value| {
                let Some(i) = index(lua, value) else {
                    return Ok(());
                };
                let mut m = lua.app_data_mut::<Model>().expect("model");
                if let Some(category) = entries(&m).get(i).and_then(|r| r.header) {
                    m.wrath_channel_display.collapsed[category] = collapse;
                    m.pending_events.push(("CHANNEL_UI_UPDATE".into(), vec![]));
                }
                Ok(())
            })?,
        )?;
    }
    g.set(
        "GetNumChannelMembers",
        lua.create_function(|lua, value: Value| {
            let Some(i) = index(lua, value) else {
                return Ok(Value::Nil);
            };
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let Some(identity) = entries(&m).get(i).and_then(|r| r.identity.clone()) else {
                return Ok(Value::Nil);
            };
            select(&mut m, identity);
            Ok(if m.wrath_channel_roster.ready() {
                count(m.wrath_channel_roster.members.len())
            } else {
                Value::Nil
            })
        })?,
    )?;
    g.set(
        "GetChannelRosterInfo",
        lua.create_function(|lua, (channel, row): (Value, Value)| {
            let (Some(i), Some(n)) = (index(lua, channel), index(lua, row)) else {
                return Ok(MultiValue::new());
            };
            let mut m = lua.app_data_mut::<Model>().expect("model");
            let Some(identity) = entries(&m).get(i).and_then(|r| r.identity.clone()) else {
                return Ok(MultiValue::new());
            };
            if select(&mut m, identity) || !m.wrath_channel_roster.ready() {
                return Ok(MultiValue::new());
            }
            let Some(row) = m.wrath_channel_roster.members.get(n) else {
                return Ok(MultiValue::new());
            };
            let name = match row.name.as_deref() {
                Some(name) if !name.is_empty() => Value::String(lua.create_string(name)?),
                _ => localized(lua, "UNKNOWN")?,
            };
            let mut out = vec![name];
            out.extend([1, 2, 0x20, 0x40, 0x80].map(|bit| flag(row.flags & bit != 0)));
            Ok(MultiValue::from_vec(out))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{
        PartyMemberInfo, PartyState, UiScript, UnitState, WrathChannelRosterMember,
        WrathDisplayChannel,
    };
    fn script() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_display_channels(vec![
            WrathDisplayChannel {
                slot: 1,
                name: "World".into(),
                channel_id: 0,
                active: true,
                custom_state: 0,
            },
            WrathDisplayChannel {
                slot: 2,
                name: "General".into(),
                channel_id: 1,
                active: true,
                custom_state: 0,
            },
        ]);
        s
    }
    #[test]
    fn display_indices_include_headers_and_collapse_without_losing_identity() {
        let mut s = script();
        s.run("assert(GetNumDisplayChannels()==5); assert(GetChannelDisplayInfo(3)=='General'); assert(GetChannelDisplayInfo(5)=='World'); SetSelectedDisplayChannel(5); assert(GetSelectedDisplayChannel()==5); CollapseChannelHeader(2); assert(GetNumDisplayChannels()==4); assert(GetSelectedDisplayChannel()==4); ExpandChannelHeader(2); assert(GetSelectedDisplayChannel()==5)").unwrap();
        assert_eq!(s.take_wrath_channel_roster_requests().len(), 2);
        s.run("GetNumChannelMembers(5); GetChannelRosterInfo(5,1)")
            .unwrap();
        assert!(s.take_wrath_channel_roster_requests().is_empty());
    }
    #[test]
    fn addon_watch_calls_send_as_given_and_clear_is_bodyless() {
        use crate::script::WrathChannelRosterRequest as R;
        let mut s = script();
        s.run("SetChannelWatch('Trade'); SetChannelWatch(''); ClearChannelWatch()")
            .unwrap();
        assert!(s.run("SetChannelWatch()").is_err());
        assert_eq!(
            s.take_wrath_channel_roster_requests(),
            vec![
                R::Watch {
                    name: "Trade".into()
                },
                R::ClearWatch
            ]
        );
    }
    #[test]
    fn userlist_deltas_patch_the_selected_roster() {
        use crate::script::WrathChannelRosterDelta as D;
        let mut s = script();
        s.run("GetNumChannelMembers(5)").unwrap();
        assert!(
            s.apply_wrath_channel_roster_delta("World", D::Remove { guid: 1 }, 0, 0)
                .is_none(),
            "before the roster"
        );
        let rev = s
            .receive_wrath_channel_roster(
                1,
                "World",
                0,
                vec![WrathChannelRosterMember {
                    guid: 2,
                    flags: 0,
                    name: Some("Zed".into()),
                }],
            )
            .unwrap();
        assert!(s
            .apply_wrath_channel_roster_delta("General", D::Add { guid: 3, flags: 0 }, 0, 2)
            .is_none());
        assert_eq!(
            s.apply_wrath_channel_roster_delta("world", D::Add { guid: 3, flags: 1 }, 0, 2),
            Some(rev)
        );
        s.run("assert(GetNumChannelMembers(5)==nil)").unwrap();
        assert!(s.resolve_wrath_channel_roster_name(rev, 3, "Amy".into()));
        s.apply_wrath_channel_roster_delta("World", D::Update { guid: 2, flags: 2 }, 0, 2);
        s.run("assert(GetNumChannelMembers(5)==2); local n,o=GetChannelRosterInfo(5,1); assert(n=='Amy' and o==1); local n,o,m=GetChannelRosterInfo(5,2); assert(n=='Zed' and m==1)").unwrap();
        s.apply_wrath_channel_roster_delta("World", D::Remove { guid: 3 }, 0, 1);
        s.run("assert(GetNumChannelMembers(5)==1); assert(GetChannelRosterInfo(5,1)=='Zed')")
            .unwrap();
    }

    #[test]
    fn native_arity_numeric_flags_names_and_pending_gate() {
        let mut s = script();
        s.run("assert(select('#',GetChannelRosterInfo())==0); assert(select('#',GetChannelRosterInfo(true,1))==0); assert(select('#',GetNumChannelMembers())==1); assert(GetNumChannelMembers()==nil); assert(GetNumChannelMembers('5')==nil)").unwrap();
        let rev = s
            .receive_wrath_channel_roster(
                1,
                "World",
                0,
                vec![
                    WrathChannelRosterMember {
                        guid: 1,
                        flags: 0xe3,
                        name: None,
                    },
                    WrathChannelRosterMember {
                        guid: 2,
                        flags: 8,
                        name: Some("Zed".into()),
                    },
                ],
            )
            .unwrap();
        s.run("assert(GetNumChannelMembers(5)==nil); assert(select('#',GetChannelRosterInfo(5,1))==0)").unwrap();
        assert!(s.resolve_wrath_channel_roster_name(rev, 1, "Alpha".into()));
        s.run("assert(GetNumChannelMembers(5)==2); local n,o,m,mut,a,e=GetChannelRosterInfo('5','1'); assert(n=='Alpha' and o==1 and m==1 and mut==1 and a==1 and e==1); assert(type(o)=='number'); local n,o,m,mut=GetChannelRosterInfo(5,2); assert(n=='Zed' and o==nil and m==nil and mut==nil); assert(select('#',GetChannelRosterInfo(5,3))==0); assert(select('#',GetChannelRosterInfo(5,1))==6)").unwrap();
        assert!(s
            .receive_wrath_channel_roster(0, "World", 0, vec![])
            .is_none());
        s.reset_wrath_channel_roster();
        assert!(!s.resolve_wrath_channel_roster_name(rev, 1, "Stale".into()));
    }
    #[test]
    fn synthetic_party_rows_use_real_names_and_leader_flag() {
        let mut s = script();
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                name: Some("Zed".into()),
                exists: true,
                ..Default::default()
            }),
        );
        s.set_party(PartyState {
            own_guid: 1,
            leader_guid: 1,
            members: vec![PartyMemberInfo {
                guid: 2,
                name: "Alpha".into(),
            }],
            ..Default::default()
        });
        s.run("assert(GetNumChannelMembers(2)==2); local n,o,m=GetChannelRosterInfo(2,1); assert(n=='Alpha' and o==nil and m==nil); local n,o,m=GetChannelRosterInfo(2,2); assert(n=='Zed' and o==1 and m==nil)").unwrap();
        assert!(s.take_wrath_channel_roster_requests().is_empty());
    }
}

#[cfg(test)]
mod lifetime_tests {
    use crate::script::{
        PartyState, RaidMemberInfo, UiScript, WrathChannelRosterMember, WrathDisplayChannel,
    };
    #[test]
    fn raid_and_battleground_indices_and_roster_changes_use_current_membership() {
        let mut s = UiScript::new().unwrap();
        let mut party = PartyState {
            own_guid: 1,
            leader_guid: 1,
            raid: vec![
                RaidMemberInfo {
                    guid: 1,
                    name: "Leader".into(),
                    rank: 2,
                    ..Default::default()
                },
                RaidMemberInfo {
                    guid: 2,
                    name: "Assistant".into(),
                    rank: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        s.set_party(party.clone());
        s.run("assert(GetNumDisplayChannels()==4); assert(GetNumChannelMembers(2)==2); local n,o,m=GetChannelRosterInfo(2,1); assert(n=='Assistant' and o==nil and m==1); local n,o,m=GetChannelRosterInfo(2,2); assert(n=='Leader' and o==1 and m==1)").unwrap();
        party.raid.remove(1);
        s.set_party(party);
        s.run("assert(GetNumChannelMembers(2)==1); assert(GetChannelRosterInfo(2,1)=='Leader')")
            .unwrap();
        s.set_instance_type(Some(3));
        s.run("CHAT_MSG_BATTLEGROUND='Battleground'; assert(GetNumDisplayChannels()==5); assert(GetChannelDisplayInfo(3)=='Battleground'); assert(GetNumChannelMembers(3)==1)").unwrap();
    }
    #[test]
    fn leave_invalidates_pending_names_and_hidden_custom_rows_do_not_shift_as_joined_slots() {
        let mut s = UiScript::new().unwrap();
        let row = |slot, name: &str, state| WrathDisplayChannel {
            slot,
            name: name.into(),
            channel_id: 0,
            active: state == 0,
            custom_state: state,
        };
        s.set_wrath_display_channels(vec![row(8, "Hidden", 1), row(9, "World", 0)]);
        s.run("assert(GetNumDisplayChannels()==4); assert(GetChannelDisplayInfo(4)=='World'); assert(select(4,GetChannelDisplayInfo(4))==9); SetSelectedDisplayChannel(4)").unwrap();
        let rev = s
            .receive_wrath_channel_roster(
                1,
                "World",
                0,
                vec![WrathChannelRosterMember {
                    guid: 2,
                    flags: 0,
                    name: None,
                }],
            )
            .unwrap();
        s.set_wrath_display_channels(vec![row(8, "Hidden", 1)]);
        assert!(!s.resolve_wrath_channel_roster_name(rev, 2, "Old".into()));
        s.run("assert(GetSelectedDisplayChannel()==nil); assert(GetNumDisplayChannels()==3)")
            .unwrap();
    }
    #[test]
    fn hiding_confirmed_selected_custom_row_invalidates_pending_revision() {
        let mut s = UiScript::new().unwrap();
        let mut row = WrathDisplayChannel {
            slot: 9,
            name: "Custom".into(),
            channel_id: 0,
            active: true,
            custom_state: 0,
        };
        s.set_wrath_display_channels(vec![row.clone()]);
        s.run("SetSelectedDisplayChannel(4)").unwrap();
        let rev = s
            .receive_wrath_channel_roster(
                1,
                "Custom",
                0,
                vec![WrathChannelRosterMember {
                    guid: 2,
                    flags: 0,
                    name: None,
                }],
            )
            .unwrap();
        // Explicit leave hides the custom row before native confirmation changes.
        row.custom_state = 1;
        s.set_wrath_display_channels(vec![row]);
        assert!(!s.resolve_wrath_channel_roster_name(rev, 2, "Stale".into()));
        s.run("assert(GetSelectedDisplayChannel()==nil); assert(GetNumDisplayChannels()==3)")
            .unwrap();
        assert!(s
            .receive_wrath_channel_roster(1, "Custom", 0, vec![])
            .is_none());
    }
    #[test]
    fn suspended_custom_activity_uses_raw_custom_state_not_confirmation() {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_display_channels(vec![WrathDisplayChannel {
            slot: 9,
            name: "Custom".into(),
            channel_id: 0,
            active: false,
            custom_state: 0,
        }]);
        let rows = super::super::wrath_channel_display::entries(&s.model_mut());
        assert!(rows[3].active);
        s.run("SetSelectedDisplayChannel(4); assert(GetSelectedDisplayChannel()==4)")
            .unwrap();
    }
}
