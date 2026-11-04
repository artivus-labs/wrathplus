//! Build 12340's character-state readers over a snapshot the app pushes each frame: the active
//! player's movement and area predicates, the unit speed, pitch, feign-death, trivial and
//! sanctuary readers, `GetPowerRegen`, the resurrect offerer, and the two death/skill verbs.
//! Evidence: `azb-live/ghidra/ui/apistate{,2,3}.log` (`ApiState*.java`).
//!
//! The player predicates open on the active player object (`0x4d3790` → `0x4d4db0`) and answer 1
//! or nil; with no player each is nil, except `IsFlyableArea`, whose no-player arm reads the area
//! alone (`0x51a1d0`). The unit readers resolve through `0x60abf0`, which never raises: a token
//! naming nothing reads as no unit, and a group member's name names that member.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{flag, string_arg};
use super::{Model, UiScript, UnitState};

/// The active player's state the predicates read, pushed whole by the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCharState {
    /// The active player object exists; without it the predicates answer nil and `GetPowerRegen`
    /// `0, 0` (`0x612bbc`).
    pub has_player: bool,
    /// `UNIT_FIELD_MOUNTDISPLAYID`, the mount model `0x740450` applies to `[player+0x9c0]`, which
    /// `IsMounted` (`0x6125a0`) needs above 0.
    pub mount_display: u32,
    /// `IsFlying` (`0x612500`): `MOVEMENTFLAG_FLYING` (`0x2000000`) on the movement word, the
    /// vehicle's while seated on one (its transport link of type 3).
    pub flying: bool,
    /// `IsSwimming` (`0x6124a0`): `[player+0xa30] & 0x200000`, the in-water bit `0x730d10` sets
    /// from `MOVEMENTFLAG_SWIMMING` or a liquid deeper than 0.75 × the collision height.
    pub swimming: bool,
    /// `0x71b7f0` → `0x77fbf0`, which `IsOutdoors` (`0x612360`) answers and `IsIndoors`
    /// (`0x612300`) negates: in a WMO group, `WMOAreaTable` flag 4 outdoors, flag 2 indoors, else
    /// the group's exterior bit; on terrain, `AreaTable` flag `0x4000000` outdoors, `0x2000000`
    /// indoors, else outdoors.
    pub outdoors: bool,
    /// `IsFlyableArea` (`0x612260`): the area's `AreaTable` flag `0x400` without `0x20000000`
    /// (`0x51a1d0`); with a player, also not Northrend (map 571) without Cold Weather Flying
    /// (spell 54197, `0x7261b0`).
    pub flyable: bool,
    /// `IsStealthed` (`0x612610`): `UNIT_FIELD_BYTES_1` byte 2 bit `0x2` (descriptor `+0x112`).
    pub stealthed: bool,
    /// `GetPowerRegen` (`0x612b40`): `(regen, casting regen)` for the active power type.
    pub power_regen: (f64, f64),
    /// `ResurrectGetOfferer` (`0x515950`): the name-cache name of the offer's caster (`0xc9eab8`),
    /// `None` without an offer or before the name answers.
    pub resurrect_offerer: Option<String>,
    /// The four totem slots' guids (`0xbd0b08`, stride `0x20`), 0 for empty: `TargetTotem`.
    pub totems: [u64; 4],
    /// `GetInstanceDifficulty`'s `0xbd0894`, 0-based: `SMSG_INSTANCE_DIFFICULTY`'s first word
    /// (`0x526530`, opcode `0x33b`), zeroed at world entry (`0x52b550`).
    pub instance_difficulty: u32,
    /// The ignore list's guids (`+0xd490`, 50 slots), for `IsIgnored` by unit.
    pub ignore_guids: Vec<u64>,
}

/// The per-unit state the unit readers need beyond the token snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WrathUnitExtra {
    /// `GetUnitSpeed` (`0x613290`): the movement's current speed, yd/s (`[[unit+0xd8]+0x8c]`).
    pub speed: f32,
    /// `GetUnitPitch` (`0x613330`): the unit's pitch, radians (vtable `+0x14c`).
    pub pitch: f32,
    /// `UnitIsPVPSanctuary` (`0x60cf20`): `UNIT_FIELD_BYTES_2` byte 1 bit `0x08`.
    pub sanctuary: bool,
}

#[derive(Default)]
struct State {
    player: WrathCharState,
    units: HashMap<u64, WrathUnitExtra>,
    hearth_and_resurrect: usize,
    /// The mount display a `Dismount()` cleared locally: `0x7412e0` sets `[player+0xa30] |=
    /// 0x10000000` (or drops the model) as it sends, so `IsMounted` reads nil until the display
    /// moves; a new display clears it (`0x740450`).
    dismounted: Option<u32>,
}

/// `GetPowerRegen`'s base rate per `(power type, casting)` (`0xb6ba80`, lazily filled by
/// `0x4f5300`): rage and runic power decay 12.5 out of combat, focus regains 5 and energy 10.
const REGEN_BASE: [[f32; 2]; 7] = [
    [0.0, 0.0],
    [-12.5, 0.0],
    [5.0, 5.0],
    [10.0, 10.0],
    [0.0, 0.0],
    [0.0, 0.0],
    [-12.5, 0.0],
];

/// `0x9e1134`, added to each `GetPowerRegen` value (`0x612b7e`).
const REGEN_NUDGE: f32 = 0.001;

/// `GetPowerRegen`'s pair for a power type and its descriptor modifiers
/// (`UNIT_FIELD_POWER_REGEN_FLAT_MODIFIER` and `..._INTERRUPTED_FLAT_MODIFIER`): `0x4f5390` adds the
/// base rate, then the binding adds [`REGEN_NUDGE`].
pub fn power_regen(power_type: u8, flat: f32, interrupted: f32) -> (f64, f64) {
    let base = REGEN_BASE
        .get(usize::from(power_type))
        .copied()
        .unwrap_or_default();
    let value = |b: f32, m: f32| f64::from(b) + f64::from(m) + f64::from(REGEN_NUDGE);
    (value(base[0], flat), value(base[1], interrupted))
}

impl UiScript {
    /// Push the active player's state.
    pub fn set_wrath_char_state(&mut self, state: WrathCharState) {
        let mut s = self.lua.app_data_mut::<State>().expect("char state");
        if s.dismounted != Some(state.mount_display) {
            s.dismounted = None;
        }
        s.player = state;
    }

    /// Push the per-unit extras, by guid, for the units the tokens name.
    pub fn set_wrath_unit_extras(&mut self, units: HashMap<u64, WrathUnitExtra>) {
        self.lua.app_data_mut::<State>().expect("char state").units = units;
    }

    /// The guids of every unit snapshot pushed, by token or by guid: the units
    /// [`Self::set_wrath_unit_extras`] serves.
    pub fn pushed_unit_guids(&self) -> Vec<u64> {
        let model = self.model_ref();
        let mut guids: Vec<u64> = model
            .units_by_lower
            .values()
            .chain(model.units_by_guid.values())
            .map(|u| u.guid)
            .filter(|&g| g != 0)
            .collect();
        guids.sort_unstable();
        guids.dedup();
        guids
    }

    /// Take the `HearthAndResurrectFromArea()` calls: each sends `CMSG_HEARTH_AND_RESURRECT`.
    pub fn take_wrath_hearth_and_resurrect(&mut self) -> usize {
        std::mem::take(
            &mut self
                .lua
                .app_data_mut::<State>()
                .expect("char state")
                .hearth_and_resurrect,
        )
    }
}

/// The pushed totem guid in a 1-based slot, 0 when empty.
pub(crate) fn totem_guid(lua: &Lua, slot: usize) -> u64 {
    let s = lua.app_data_ref::<State>().expect("char state");
    slot.checked_sub(1)
        .and_then(|i| s.player.totems.get(i).copied())
        .unwrap_or(0)
}

/// `UNIT_FLAG_TAXI_FLIGHT`: `0x7412e0` sends nothing on a flight path.
const UNIT_FLAG_TAXI_FLIGHT: u32 = 0x10_0000;

/// `Dismount()` (`0x7412e0`): queue the send and, off a flight path, clear the mount locally at
/// once, so a later `IsMounted()` in the same chunk reads nil.
pub(crate) fn dismount(lua: &Lua) {
    let taxi = {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.script_calls.push(super::ScriptCall::Dismount);
        let me = model.unit("player");
        me.is_some_and(|u| u.flags & UNIT_FLAG_TAXI_FLIGHT != 0)
    };
    if !taxi {
        let mut s = lua.app_data_mut::<State>().expect("char state");
        s.dismounted = Some(s.player.mount_display).filter(|&d| d > 0);
    }
}

/// Read the pushed player state under a short borrow.
pub(crate) fn with_char<T>(lua: &Lua, f: impl FnOnce(&WrathCharState) -> T) -> T {
    f(&lua.app_data_ref::<State>().expect("char state").player)
}

/// A player predicate: nil without the player, else the field as 1 or nil.
fn player_flag(lua: &Lua, pick: fn(&WrathCharState) -> bool) -> Value {
    let s = lua.app_data_ref::<State>().expect("char state");
    flag(s.player.has_player && pick(&s.player))
}

/// `0x60a630`, `0x60abf0`'s name arm: the name before a `-target` suffix, matched case-blind
/// against `UnitName` of `player`, then the raid members (else the party, else the arena
/// opponents), then `pet` and those members' pets; each `-target` then walks one target
/// (`0x60aaa0`). The token it names, `None` when nobody bears the name.
fn token_for_name(model: &Model, arg: &str) -> Option<String> {
    let (name, suffix) = arg.split_at(arg.find('-').unwrap_or(arg.len()));
    let hops = suffix.matches("-target").count();
    if name.is_empty() || suffix != "-target".repeat(hops) {
        return None;
    }
    let party = &model.party;
    let (group, size) = if !party.raid.is_empty() {
        ("raid", party.raid.len())
    } else if !party.members.is_empty() {
        ("party", party.members.len())
    } else {
        ("arena", 5)
    };
    let members = (1..=size).map(|i| format!("{group}{i}"));
    let pets = (1..=size).map(|i| format!("{group}pet{i}"));
    let base = std::iter::once("player".to_string())
        .chain(members)
        .chain(std::iter::once("pet".to_string()))
        .chain(pets)
        .find(|t| {
            let n = model.unit(t).and_then(|u| u.name.as_deref());
            n.is_some_and(|n| n.eq_ignore_ascii_case(name))
        })?;
    Some(format!("{base}{}", "target".repeat(hops)))
}

/// The token `0x60abf0` resolves an argument to: a unit token as given, else a group member's
/// name ([`token_for_name`]). It never raises.
pub(crate) fn resolve_unit_arg(lua: &Lua, arg: &str) -> Option<String> {
    if super::unit::check_unit_token(lua, &Some(arg.to_string())).is_ok() {
        return Some(arg.to_string());
    }
    token_for_name(&lua.app_data_ref::<Model>().expect("model app_data"), arg)
}

/// The held unit an argument names through `0x60abf0`, mapped by `f`; `None` for no unit.
fn held_unit<T>(lua: &Lua, arg: &str, f: impl FnOnce(&Model, &UnitState) -> T) -> Option<T> {
    let token = resolve_unit_arg(lua, arg)?;
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    let unit = model.unit(&token).filter(|u| u.exists && u.has_object)?;
    Some(f(&model, unit))
}

/// A unit's extra, resolved through its token's guid.
fn extra(lua: &Lua, token: &str) -> Option<WrathUnitExtra> {
    let guid = held_unit(lua, token, |_, u| u.guid)?;
    let s = lua.app_data_ref::<State>().expect("char state");
    Some(s.units.get(&guid).copied().unwrap_or_default())
}

/// `0x512a30`: the guid is us or our pet (`0x52d310`), a party member or their pet, or a raid
/// member (`0x573200`).
fn in_own_group(model: &Model, guid: u64) -> bool {
    if guid == 0 {
        return false;
    }
    let party = &model.party;
    let token_guid = |t: &str| model.unit(t).map(|u| u.guid);
    guid == party.own_guid
        || token_guid("player") == Some(guid)
        || token_guid("pet") == Some(guid)
        || party.members.iter().any(|m| m.guid == guid)
        || party.raid.iter().any(|m| m.guid == guid)
        || (1..=4).any(|i| token_guid(&format!("partypet{i}")) == Some(guid))
}

/// `0x6de130` (`UnitIsTrivial`): not a player, and more than the grey band below the player:
/// `0xa31bb8` by player level / 5, 8 past the table.
fn unit_trivial(player_level: u32, unit: &UnitState) -> bool {
    const BAND: [u32; 8] = [4, 4, 5, 5, 6, 6, 7, 7];
    if unit.is_player {
        return false;
    }
    let band = BAND.get(player_level as usize / 5).copied().unwrap_or(8);
    let gap = i64::from(player_level) - i64::from(unit.level);
    gap > 0 && gap > i64::from(band)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let g = lua.globals();
    // IsMounted(): a mount display and no local dismount since it was applied.
    g.set(
        "IsMounted",
        lua.create_function(|lua, _: MultiValue| {
            let s = lua.app_data_ref::<State>().expect("char state");
            let p = &s.player;
            Ok(flag(
                p.has_player && p.mount_display > 0 && s.dismounted != Some(p.mount_display),
            ))
        })?,
    )?;
    let predicates: [(&str, fn(&WrathCharState) -> bool); 5] = [
        ("IsFlying", |p| p.flying),
        ("IsSwimming", |p| p.swimming),
        ("IsIndoors", |p| !p.outdoors),
        ("IsOutdoors", |p| p.outdoors),
        ("IsStealthed", |p| p.stealthed),
    ];
    for (name, pick) in predicates {
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| Ok(player_flag(lua, pick)))?,
        )?;
    }
    // Pushed with its no-player arm already applied, so it answers without a player.
    g.set(
        "IsFlyableArea",
        lua.create_function(|lua, _: MultiValue| {
            let s = lua.app_data_ref::<State>().expect("char state");
            Ok(flag(s.player.flyable))
        })?,
    )?;
    // GetUnitSpeed(unit), GetUnitPitch(unit): a number, 0 for no unit; a non-string raises.
    for (name, usage, pitch) in [
        ("GetUnitSpeed", "Usage: GetUnitSpeed(\"unit\")", false),
        ("GetUnitPitch", "Usage: GetUnitPitch(\"unit\")", true),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, token: Value| {
                let token = string_arg(lua, token, usage)?;
                let e = extra(lua, &token).unwrap_or_default();
                Ok(f64::from(if pitch { e.pitch } else { e.speed }))
            })?,
        )?;
    }
    // UnitIsFeignDeath(unit): `UNIT_DYNAMIC_FLAGS & 0x20` (descriptor `+0x124`), shown only for
    // our own group (`0x512a30`), so a feigning stranger reads nil.
    g.set(
        "UnitIsFeignDeath",
        lua.create_function(|lua, token: Value| {
            let token = string_arg(lua, token, "Usage: UnitIsFeignDeath(\"unit\")")?;
            let hit = held_unit(lua, &token, |model, u| {
                u.dynamic_flags & 0x20 != 0 && in_own_group(model, u.guid)
            });
            Ok(flag(hit == Some(true)))
        })?,
    )?;
    g.set(
        "UnitIsTrivial",
        lua.create_function(|lua, token: Value| {
            let token = string_arg(lua, token, "Usage: UnitIsTrivial(\"unit\")")?;
            let hit = held_unit(lua, &token, |model, u| {
                let player = model.unit("player").filter(|p| p.has_object);
                player.is_some_and(|p| unit_trivial(p.level, u))
            });
            Ok(flag(hit == Some(true)))
        })?,
    )?;
    // UnitIsPVPSanctuary(unit): no usage check; nil names no unit.
    g.set(
        "UnitIsPVPSanctuary",
        lua.create_function(|lua, token: Value| {
            let token = super::binding_abi::optional_string(lua, &token);
            let hit = token
                .and_then(|t| extra(lua, &t))
                .is_some_and(|e| e.sanctuary);
            Ok(flag(hit))
        })?,
    )?;
    g.set(
        "GetPowerRegen",
        lua.create_function(|lua, _: MultiValue| {
            let s = lua.app_data_ref::<State>().expect("char state");
            Ok(if s.player.has_player {
                s.player.power_regen
            } else {
                (0.0, 0.0)
            })
        })?,
    )?;
    g.set(
        "ResurrectGetOfferer",
        lua.create_function(|lua, _: MultiValue| {
            let s = lua.app_data_ref::<State>().expect("char state");
            Ok(s.player.resurrect_offerer.clone())
        })?,
    )?;
    // HearthAndResurrectFromArea() (`0x54c7a0`): `CMSG_HEARTH_AND_RESURRECT` (0x49c), empty, with no
    // gate of its own; answers nothing.
    g.set(
        "HearthAndResurrectFromArea",
        lua.create_function(|lua, _: MultiValue| {
            lua.app_data_mut::<State>()
                .expect("char state")
                .hearth_and_resurrect += 1;
            Ok(())
        })?,
    )?;
    // AcceptSkillUps() (`0x5ce5d0`): sends `CMSG_SKILL_BUY_RANK` (0x220) with each pending
    // skill-up, only while the pending flag `0xc22b00` is set. Not modelled: nothing in this
    // client writes that list (AzerothCore ignores the opcode), so it never sends; answers nothing.
    g.set(
        "AcceptSkillUps",
        lua.create_function(|_, _: MultiValue| Ok(()))?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_char_state_tests.rs"]
mod tests;
