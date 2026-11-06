//! Build-12340 group-only ABI. Source: AC Group.h flags/roles and Group.cpp
//! SendUpdateToPlayer; stock RaidFrame consumes the extended roster tuple.
use super::super::Model;
use mlua::{Function, Lua, MultiValue, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    pub guid: u64,
    pub flags: u8,
    pub roles: u8,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub members: Vec<Member>,
    pub dungeon_difficulty: Option<u8>,
    pub raid_difficulty: Option<u8>,
    /// The group's LFG dungeon, a full packed entry, 0 for none (`DAT_00bd19a4`): IsPartyLFG,
    /// IsInLFGDungeon and the backfill getters.
    pub lfg_dungeon_entry: u32,
    /// The LFG status (`DAT_00bd1998`); backfill needs 1..3.
    pub lfg_status: u8,
    /// The LFG flags byte (`DAT_00bd1989`); backfill needs bit 2 clear.
    pub lfg_flags: u8,
    /// HasLFGRestrictions (`DAT_00bd19a8`).
    pub lfg_restricted: bool,
}
impl super::super::UiScript {
    /// The group list's state. The difficulty fields stay [`Self::set_difficulties`]': they are
    /// the player's answer, grouped or not, not the list's.
    pub fn set_wrath_group(&mut self, group: Group) {
        let mut model = self.model_mut();
        let old = &model.wrath_group;
        let (dungeon_difficulty, raid_difficulty) = (old.dungeon_difficulty, old.raid_difficulty);
        model.wrath_group = Group {
            dungeon_difficulty,
            raid_difficulty,
            ..group
        };
    }

    /// The effective dungeon and raid difficulty `GetDungeonDifficulty`/`GetRaidDifficulty`
    /// answer, 1-based, whether grouped or not (build 12340's `0xbd0898`/`0xbd1978` pair and its
    /// raid twin), from the native difficulty feed.
    pub fn set_difficulties(&mut self, dungeon: u8, raid: u8) {
        let mut model = self.model_mut();
        model.wrath_group.dungeon_difficulty = Some(dungeon);
        model.wrath_group.raid_difficulty = Some(raid);
    }
}
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let roster: Function = g.get("GetRaidRosterInfo")?;
    g.set(
        "GetRaidRosterInfo",
        lua.create_function(move |lua, index: Value| {
            let mut values: MultiValue = roster.call(index.clone())?;
            let n = super::super::binding_abi::number_arg(
                lua,
                index,
                "Usage: GetRaidRosterInfo(index)",
            )?;
            let member = {
                let model = lua.app_data_ref::<Model>().expect("model");
                usize::try_from(n.wrapping_sub(1))
                    .ok()
                    .and_then(|i| model.party.raid.get(i))
                    .and_then(|r| model.wrath_group.members.iter().find(|m| m.guid == r.guid))
                    .cloned()
            };
            // Return 10: main-tank/main-assist assignment; 11: master looter.
            let assignment = member.as_ref().and_then(|m| {
                if m.flags & 2 != 0 {
                    Some("MAINTANK")
                } else if m.flags & 4 != 0 {
                    Some("MAINASSIST")
                } else {
                    None
                }
            });
            values.push_back(match assignment {
                Some(a) => Value::String(lua.create_string(a)?),
                None => Value::Nil,
            });
            let master = {
                let model = lua.app_data_ref::<Model>().expect("model");
                let guid = model.party.master_looter_guid;
                usize::try_from(n.wrapping_sub(1))
                    .ok()
                    .and_then(|i| model.party.raid.get(i))
                    .is_some_and(|r| guid != 0 && r.guid == guid)
            };
            values.push_back(Value::Boolean(master));
            Ok(values)
        })?,
    )?;
    g.set(
        "UnitGroupRolesAssigned",
        // UnitGroupRolesAssigned(unit) → isTank, isHealer, isDamage: build 12340's three booleans
        // (PartyMemberFrame.lua:223, PlayerFrame.lua:250), from the member's role bits 2, 4, 8.
        lua.create_function(|lua, token: Option<String>| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let guid = token.as_ref().and_then(|t| model.unit(t)).map(|u| u.guid);
            let bits = guid
                .and_then(|g| model.wrath_group.members.iter().find(|m| m.guid == g))
                .map_or(0, |m| m.roles);
            Ok((bits & 2 != 0, bits & 4 != 0, bits & 8 != 0))
        })?,
    )?;
    for (name, raid) in [("GetDungeonDifficulty", false), ("GetRaidDifficulty", true)] {
        g.set(
            name,
            lua.create_function(move |lua, ()| {
                let m = lua.app_data_ref::<Model>().expect("model");
                Ok(if raid {
                    m.wrath_group.raid_difficulty
                } else {
                    m.wrath_group.dungeon_difficulty
                })
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{
        PartyState, RaidMemberInfo, UiScript, UnitState, WrathGroup, WrathGroupMember,
    };
    #[test]
    fn wrath_group_tuple_roles_and_difficulty_use_pushed_native_data() {
        let mut s = UiScript::new().unwrap();
        s.set_party(PartyState {
            raid: vec![RaidMemberInfo {
                guid: 7,
                name: "Ally".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        s.set_unit(
            "raid1",
            Some(UnitState {
                guid: 7,
                exists: true,
                ..Default::default()
            }),
        );
        s.set_wrath_group(WrathGroup {
            members: vec![WrathGroupMember {
                guid: 7,
                flags: 2,
                roles: 2,
            }],
            // The list's own copy is not the answer: `set_difficulties` owns it.
            dungeon_difficulty: Some(9),
            ..Default::default()
        });
        s.set_difficulties(2, 4);
        assert!(s.eval::<bool>("local a,b,c,d,e,f,g,h,i,j,k=GetRaidRosterInfo(1); return a=='Ally' and j=='MAINTANK' and k==false and select('#', UnitGroupRolesAssigned('raid1'))==3 and UnitGroupRolesAssigned('raid1')==true and select(2, UnitGroupRolesAssigned('raid1'))==false and GetDungeonDifficulty()==2 and GetRaidDifficulty()==4").unwrap());
        assert_eq!(s.arity("GetRaidRosterInfo(1)").unwrap(), 11);
        s.set_wrath_group(Default::default());
        assert!(s
            .eval::<bool>(
                "return GetDungeonDifficulty()==2 and not (UnitGroupRolesAssigned('raid1') or select(2, UnitGroupRolesAssigned('raid1')) or select(3, UnitGroupRolesAssigned('raid1')))"
            )
            .unwrap());
    }
}
