//! Build 12340's small game queries over state the VM already holds: the vehicle control seat,
//! `TargetNearest` and `TargetTotem`, the instance difficulty, the real group leader, the ignore
//! list, the ready check's time left, the quest panel's daily and weekly flags, and the greeting
//! lists' levels. Evidence: `azb-live/ghidra/ui/apistate{,2,3}.log` (`ApiState*.java`).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{bool_or_default, flag, number_arg};
use super::calls::{NearestMode, ScriptCall};
use super::wrath_char_state::with_char;
use super::Model;

/// `VEHICLE_SEAT_FLAG_CAN_CONTROL`, `UnitInVehicleControlSeat`'s bit (`0x613700`).
const SEAT_CAN_CONTROL: u32 = 0x800;
/// The seat bit `IsUsingVehicleControls` passes `0x5fb560` (`0x5fb979 push 0x800000`).
const SEAT_USES_CONTROLS: u32 = 0x0080_0000;
/// `QUEST_FLAGS_DAILY` and `QUEST_FLAGS_WEEKLY` on the open panel's flags (`0xc0d6b4`).
const QUEST_FLAGS_DAILY: u32 = 0x1000;
const QUEST_FLAGS_WEEKLY: u32 = 0x8000;
/// `TargetTotem`'s slot count, the bound of its range error (`0x5262e0`).
const TOTEM_SLOTS: i32 = 4;

fn model(lua: &Lua) -> mlua::AppDataRef<'_, Model> {
    lua.app_data_ref::<Model>().expect("model app_data")
}

fn push_call(lua: &Lua, call: ScriptCall) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .script_calls
        .push(call);
}

/// `0x6b52e0` over the ignore list (`+0xd490`): the argument as a name first (`0x6701f0`, the
/// listed names being the list's guids resolved), then as a unit token (`0x60abf0`) by guid.
fn ignored(lua: &Lua, model: &Model, arg: &str) -> bool {
    let listed = model
        .social
        .ignores
        .iter()
        .any(|n| n.eq_ignore_ascii_case(arg));
    listed
        || model
            .unit_guid(arg)
            .is_some_and(|g| with_char(lua, |c| c.ignore_guids.contains(&g)))
}

/// A greeting list's row at a 1-based index, truncated (`0x58c031`, the x87 control word set to
/// chop).
fn greeting_row(model: &Model, index: i32, active: bool) -> Option<&super::QuestGreetingInfo> {
    let q = model.quest.as_ref()?;
    let list = if active {
        &q.active_info
    } else {
        &q.available_info
    };
    list.get(usize::try_from(index).ok()?.checked_sub(1)?)
}

/// A greeting row's level, 0 outside the list.
fn greeting_level(model: &Model, index: i32, active: bool) -> i32 {
    greeting_row(model, index, active).map_or(0, |row| row.level)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // UnitInVehicleControlSeat(unit) (`0x613700`): a Lua boolean, the unit's seat (`0x613600`)
    // flags `& 0x800`; false with no seat. No usage check.
    g.set(
        "UnitInVehicleControlSeat",
        lua.create_function(|lua, token: Value| {
            let token = super::binding_abi::optional_string(lua, &token);
            let model = model(lua);
            let seat = token
                .and_then(|t| model.unit_guid(&t))
                .and_then(|guid| model.wrath_vehicles.units.get(&guid))
                .and_then(|v| v.seat);
            Ok(seat.is_some_and(|(flags, _)| flags & SEAT_CAN_CONTROL != 0))
        })?,
    )?;
    // IsUsingVehicleControls() (`0x5fb970`): the player's admitted seat (`0x5fb560`) has
    // `0x800000`; 1 or nil.
    g.set(
        "IsUsingVehicleControls",
        lua.create_function(|lua, _: MultiValue| {
            let flags = model(lua).wrath_vehicles.player_seat_flags;
            Ok(flag(flags.is_some_and(|f| f & SEAT_USES_CONTROLS != 0)))
        })?,
    )?;
    // TargetNearest([reverse]) (`0x525a90`): mode 0 of the cycler `0x524fc0`, whose candidate
    // filter `0x518e40` accepts any unit for it; `reverse` through `0x815500`. Labelled: the
    // protected gate (`[0xbeaf4c]`, `[0xbeaf44] & 0x20`) is not modelled, as for the other
    // `TargetNearest*` verbs.
    g.set(
        "TargetNearest",
        lua.create_function(|lua, args: MultiValue| {
            let reverse = bool_or_default(args.front(), false);
            let mode = NearestMode::Any;
            push_call(lua, ScriptCall::TargetNearest { mode, reverse });
            Ok(())
        })?,
    )?;
    // TargetTotem(slot) (`0x5262e0`): the slot's totem guid into `SetSelection` (`0x524bf0`); an
    // empty slot does nothing, a slot outside 1..4 raises.
    g.set(
        "TargetTotem",
        lua.create_function(|lua, slot: Value| {
            let slot = number_arg(lua, slot, "Usage: TargetTotem(slot)")?;
            if !(1..=TOTEM_SLOTS).contains(&slot) {
                return Err(mlua::Error::runtime(format!(
                    "Totem slot must be in range of 1 to {TOTEM_SLOTS}"
                )));
            }
            let guid = super::wrath_char_state::totem_guid(lua, slot as usize);
            if guid != 0 {
                push_call(lua, ScriptCall::TargetGuid(guid));
            }
            Ok(())
        })?,
    )?;
    // GetInstanceDifficulty() (`0x515750`): the 0-based difficulty `0xbd0894` plus one.
    g.set(
        "GetInstanceDifficulty",
        lua.create_function(|lua, _: MultiValue| {
            let difficulty = with_char(lua, |c| c.instance_difficulty);
            Ok(f64::from(difficulty) + 1.0)
        })?,
    )?;
    // IsRealPartyLeader() (`0x52cd30`): the group leader guid (`0xbd1990`) is non-zero and ours.
    // IsRealRaidLeader() (`0x573ab0`) compares the same pair with no zero guard.
    g.set(
        "IsRealPartyLeader",
        lua.create_function(|lua, _: MultiValue| {
            let party = &model(lua).party;
            Ok(flag(
                party.leader_guid != 0 && party.leader_guid == party.own_guid,
            ))
        })?,
    )?;
    g.set(
        "IsRealRaidLeader",
        lua.create_function(|lua, _: MultiValue| {
            let party = &model(lua).party;
            Ok(flag(party.leader_guid == party.own_guid))
        })?,
    )?;
    // IsIgnored(name or unit) (`0x6b6c60`) and IsIgnoredOrMuted (`0x6b6d80`, adding the voice mute
    // list `+0xd7c0` through `0x6b5350`): 1 or nil; nil names nobody. The mute list is empty, as
    // this client has no voice chat. Unbuilt: the complaint auto-ignore list `+0xdaf0` that
    // `0x6b3700` reads while `SMSG_FEATURE_SYSTEM_STATUS` sets `[0xbceffc] == 2` (`0x6e2e90`, opcode
    // `0x3c9`) has no writer here (no `ComplainChat`), so it reads empty.
    for name in ["IsIgnored", "IsIgnoredOrMuted"] {
        g.set(
            name,
            lua.create_function(|lua, arg: Value| {
                let arg = super::binding_abi::optional_string(lua, &arg);
                let model = model(lua);
                Ok(flag(arg.is_some_and(|a| {
                    !a.is_empty() && ignored(lua, &model, &a)
                })))
            })?,
        )?;
    }
    // GetReadyCheckTimeLeft() (`0x572c80`): 0 outside a group (no raid, no party1), else the
    // whole seconds to the check's end (`0xbeb61c`), 0 once passed or with none.
    g.set(
        "GetReadyCheckTimeLeft",
        lua.create_function(|lua, _: MultiValue| {
            let model = model(lua);
            let grouped = !model.party.raid.is_empty() || !model.party.members.is_empty();
            let left = match model.wrath_ready_check.ends_at {
                Some(end) if grouped && end > model.now => {
                    ((end - model.now) * 1000.0) as u32 / 1000
                }
                _ => 0,
            };
            Ok(f64::from(left))
        })?,
    )?;
    for (name, bit) in [
        ("QuestIsDaily", QUEST_FLAGS_DAILY),
        ("QuestIsWeekly", QUEST_FLAGS_WEEKLY),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                Ok(flag(model(lua).wrath_quests.panel_flags & bit != 0))
            })?,
        )?;
    }
    // IsAvailableQuestTrivial(index) (`0x58d370`) and IsActiveQuestTrivial(index) (`0x58d420`):
    // `0x58aba0` on the row's level, 0 outside the list, so an index past it reads trivial once
    // the player outgrows level 0; 1 or nil.
    for (name, usage, active) in [
        (
            "IsAvailableQuestTrivial",
            "Usage: IsAvailableQuestTrivial(index)",
            false,
        ),
        (
            "IsActiveQuestTrivial",
            "Usage: IsActiveQuestTrivial(index)",
            true,
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let index = number_arg(lua, index, usage)?;
                let model = model(lua);
                let level = greeting_level(&model, index, active);
                Ok(flag(super::quest_log::quest_trivial(&model, level)))
            })?,
        )?;
    }
    // GetAvailableQuestInfo(index) (`0x58dbc0`): trivial as above, then daily and repeatable, all
    // 1 or nil; outside the list the trivial test still runs on level 0.
    g.set(
        "GetAvailableQuestInfo",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: GetAvailableQuestInfo(index)")?;
            let model = model(lua);
            let level = greeting_level(&model, index, false);
            let row = greeting_row(&model, index, false);
            Ok((
                flag(super::quest_log::quest_trivial(&model, level)),
                flag(row.is_some_and(|r| r.daily)),
                flag(row.is_some_and(|r| r.repeatable)),
            ))
        })?,
    )?;
    // GetActiveLevel(index) (`0x58c010`) and GetAvailableLevel(index) (`0x58bf70`): the greeting
    // row's level, 0 outside the list. The usage strings are the client's own, doubled `Get`.
    for (name, usage, active) in [
        ("GetActiveLevel", "Usage: GetGetActiveLevel(index)", true),
        (
            "GetAvailableLevel",
            "Usage: GetGetAvailableLevel(index)",
            false,
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let index = number_arg(lua, index, usage)?;
                Ok(f64::from(greeting_level(&model(lua), index, active)))
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "wrath_game_queries_tests.rs"]
mod tests;
