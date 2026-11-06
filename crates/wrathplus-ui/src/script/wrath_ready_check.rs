//! Build-12340 ready-check presentation. Stock ReadyCheck.lua:11-40 consumes
//! READY_CHECK(initiator,timeLeft)/READY_CHECK_FINISHED; unit frames query status.
//! Kept separate from the Vanilla leader-owned timeout and AFK chat synthesis.
use super::{Model, PartyRequest, ScriptValue, UiScript};
use mlua::Lua;
use std::collections::HashMap;

/// The ready check's length, `READY_CHECK`'s `timeLeft`.
const READY_CHECK_SECS: f64 = 30.0;

#[derive(Default)]
pub(crate) struct State {
    active: bool,
    statuses: HashMap<u64, &'static str>,
    /// `GetReadyCheckTimeLeft`'s end (`0xbeb61c`) on the `GetTime` clock: the start plus 30000 ms
    /// (`0x574d50`, `MSG_RAID_READY_CHECK`), cleared by the last answer and at the finish.
    pub(crate) ends_at: Option<f64>,
}
fn token(model: &Model, guid: u64) -> Option<String> {
    if guid != 0 && guid == model.party.own_guid {
        return Some("player".into());
    }
    if let Some(i) = model.party.raid.iter().position(|m| m.guid == guid) {
        return Some(format!("raid{}", i + 1));
    }
    model
        .party
        .members
        .iter()
        .position(|m| m.guid == guid)
        .map(|i| format!("party{}", i + 1))
}
impl UiScript {
    pub fn wrath_ready_check_start(&mut self, initiator: u64) {
        if self.model_ref().wrath_ready_check.active {
            self.fire_event("READY_CHECK_FINISHED", vec![ScriptValue::Bool(true)]);
        }
        let initiator_token = {
            let mut model = self.model_mut();
            let mut statuses = HashMap::new();
            for guid in model
                .party
                .raid
                .iter()
                .map(|m| m.guid)
                .chain(model.party.members.iter().map(|m| m.guid))
                .chain([model.party.own_guid])
            {
                if guid != 0 {
                    statuses.insert(guid, "waiting");
                }
            }
            statuses.insert(initiator, "ready");
            model.wrath_ready_check = State {
                active: true,
                statuses,
                ends_at: Some(model.now + READY_CHECK_SECS),
            };
            token(&model, initiator)
        };
        // Self initiators still receive the event: stock hides their listener, not
        // the whole ready-check display. Unknown GUIDs must not become the leader.
        self.fire_event(
            "READY_CHECK",
            vec![
                initiator_token.map_or(ScriptValue::Nil, ScriptValue::Str),
                ScriptValue::Number(READY_CHECK_SECS),
            ],
        );
    }
    pub fn wrath_ready_check_confirm(&mut self, guid: u64, state: u8) {
        let unit = {
            let mut model = self.model_mut();
            if !model.wrath_ready_check.active {
                return;
            }
            model
                .wrath_ready_check
                .statuses
                .insert(guid, if state == 0 { "notready" } else { "ready" });
            // The last answer ends the timer (`0x574880`/`0x5744f0` stamp `now - 1`, and the timer
            // pass `0x573390` zeroes it), so the time left reads 0.
            let check = &mut model.wrath_ready_check;
            if !check.statuses.values().any(|s| *s == "waiting") {
                check.ends_at = None;
            }
            token(&model, guid)
        };
        if let Some(unit) = unit {
            self.fire_event(
                "READY_CHECK_CONFIRM",
                vec![ScriptValue::Str(unit), ScriptValue::Int(i64::from(state))],
            );
        }
    }
    pub fn wrath_ready_check_finish(&mut self) {
        {
            let mut model = self.model_mut();
            model.wrath_ready_check.active = false;
            model.wrath_ready_check.ends_at = None;
        }
        // Keep final statuses for stock unit-frame fade callbacks. The next start
        // replaces them; no Vanilla AFK summary or leader-only timer runs here.
        self.fire_event("READY_CHECK_FINISHED", vec![]);
    }
}
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetReadyCheckStatus",
        lua.create_function(|lua, unit: String| {
            let model = lua.app_data_ref::<Model>().expect("model");
            let guid = model
                .unit_guid(&unit)
                .or_else(|| model.unit(&unit).map(|u| u.guid))
                .or_else(|| {
                    if unit == "player" {
                        return Some(model.party.own_guid);
                    }
                    unit.strip_prefix("raid")
                        .and_then(|n| n.parse::<usize>().ok())
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|i| model.party.raid.get(i))
                        .map(|m| m.guid)
                        .or_else(|| {
                            unit.strip_prefix("party")
                                .and_then(|n| n.parse::<usize>().ok())
                                .and_then(|n| n.checked_sub(1))
                                .and_then(|i| model.party.members.get(i))
                                .map(|m| m.guid)
                        })
                });
            Ok(guid.and_then(|g| model.wrath_ready_check.statuses.get(&g).copied()))
        })?,
    )?;
    lua.globals().set(
        "DoReadyCheck",
        lua.create_function(|lua, ()| {
            lua.app_data_mut::<Model>()
                .expect("model")
                .party_requests
                .push(PartyRequest::ReadyCheckStart);
            Ok(())
        })?,
    )?;
    lua.globals()
        .set("CheckReadyCheckTime", lua.create_function(|_, ()| Ok(()))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{PartyState, RaidMemberInfo};
    #[test]
    fn assistant_initiator_status_confirm_preemption_and_finish_are_native() {
        let mut s = UiScript::new().unwrap();
        s.set_party(PartyState {
            own_guid: 7,
            leader_guid: 99,
            raid: vec![
                RaidMemberInfo {
                    guid: 7,
                    name: "Assistant".into(),
                    ..Default::default()
                },
                RaidMemberInfo {
                    guid: 99,
                    name: "Leader".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        s.run("events={}; local f=CreateFrame('Frame'); for _,e in ipairs({'READY_CHECK','READY_CHECK_CONFIRM','READY_CHECK_FINISHED'}) do f:RegisterEvent(e) end; f:SetScript('OnEvent',function(_,e,...) table.insert(events,{e,...}) end)").unwrap();
        s.wrath_ready_check_start(7);
        s.run("assert(events[1][1]=='READY_CHECK' and events[1][2]=='player' and events[1][3]==30); assert(GetReadyCheckStatus('player')=='ready'); assert(GetReadyCheckStatus('raid2')=='waiting')").unwrap();
        s.wrath_ready_check_confirm(99, 0);
        s.run("assert(GetReadyCheckStatus('raid2')=='notready'); assert(events[2][1]=='READY_CHECK_CONFIRM' and events[2][2]=='raid2')").unwrap();
        s.wrath_ready_check_start(99);
        s.run("assert(events[3][1]=='READY_CHECK_FINISHED' and events[3][2]==true); assert(events[4][2]=='raid2'); assert(GetReadyCheckStatus('player')=='waiting')").unwrap();
        s.wrath_ready_check_finish();
        s.run(
            "assert(events[5][1]=='READY_CHECK_FINISHED'); DoReadyCheck(); CheckReadyCheckTime()",
        )
        .unwrap();
        assert_eq!(s.take_party_requests(), vec![PartyRequest::ReadyCheckStart]);
        assert!(s.take_ready_check_lines().is_empty());
        assert!(!s.model_ref().wrath_ready_check.active);
    }
}
