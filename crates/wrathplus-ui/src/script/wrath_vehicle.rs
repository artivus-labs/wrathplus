//! Build 12340's vehicle UI bindings (`VehicleMenuBar.lua`, `PlayerFrame.lua`, the party and raid
//! frames), over the app's vehicle view (`/tmp/azb-exp-vehicle-clean.log`,
//! `/tmp/azb-exp-veh{2,3}-clean.log`).
//!
//! A unit is seated (`UnitInVehicle`) while its passenger link's state is 3 and using a vehicle
//! (`UnitUsingVehicle`) while it is non-zero; an out-of-range party member answers from its
//! roster flag `0x200` (`0x6133d0`, `0x6134a0`). Seats are numbered flat over the root vehicle's
//! eight slots, depth first: a slot holding an accessory vehicle stands for that vehicle's own
//! seats (`0x757550`, `0x757470`). The player's own seat gates exit and switching
//! (`0x5fb560`: seated, the active mover the vehicle or the player, the seat's flag set).
//!
//! The aim names read the aiming unit (`0x5f9d20`): the vehicle when the player's seat controls
//! it, else the player. Its `Vehicle.dbc` flags `0x400`/`0x800` make the angle and power
//! adjustable; `0x40` gives it its own pitch range, else ±π/2.
//!
//! Unresolved, labelled: `VehicleAimUpStart`/`Stop` and `VehicleAimDownStart`/`Stop` set the
//! mover's pitch-up/down movement flags (`0x5fa170`/`0x5fa450` with `0x400`/`0x800`), a
//! continuous pitch the mover does not model; they do nothing here.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::flag;
use super::Model;

/// `VEHICLE_SEAT_FLAG_CAN_CONTROL`, `_CAN_ENTER_OR_EXIT`, `_CAN_SWITCH`.
const SEAT_CAN_CONTROL: u32 = 0x800;
const SEAT_CAN_EXIT: u32 = 0x0200_0000;
const SEAT_CAN_SWITCH: u32 = 0x0400_0000;
/// The seat flag `UnitHasVehicleUI` reads (`0x613755`).
const SEAT_HAS_UI: u32 = 0x2000_0000;
/// `VehicleSeat` flags B: the passenger can be ejected.
const SEAT_B_EJECTABLE: u32 = 0x20;
/// `Vehicle.dbc` flags: a custom pitch range, an adjustable angle, an adjustable power.
const VEHICLE_CUSTOM_PITCH: u32 = 0x40;
const VEHICLE_AIM_ANGLE: u32 = 0x400;
const VEHICLE_AIM_POWER: u32 = 0x800;
/// The default pitch range (`0x9f1ff4`, `0x9e8d88`), the step (`0x9e3004`) and the degenerate
/// range test (`0x9e8cd0`).
const PITCH_MIN: f32 = -std::f32::consts::FRAC_PI_2;
const PITCH_MAX: f32 = std::f32::consts::FRAC_PI_2;
const AIM_STEP: f32 = 0.1;
const RANGE_EPSILON: f32 = 1.0e-4;

/// One flat seat of a root vehicle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleSeatView {
    /// The vehicle that owns the slot: the root or an accessory.
    pub vehicle: u64,
    /// The slot on that vehicle, 0-7.
    pub slot: u8,
    /// The seat's `VehicleSeat` flags and flags B.
    pub flags: u32,
    pub flags_b: u32,
    /// The seat controls its vehicle, and that vehicle is the root.
    pub root: bool,
    /// The passenger's guid and name, `None` for an empty seat.
    pub occupant: Option<u64>,
    pub occupant_name: Option<String>,
}

impl VehicleSeatView {
    fn control(&self) -> &'static str {
        match (self.flags & SEAT_CAN_CONTROL != 0, self.root) {
            (false, _) => "None",
            (true, true) => "Root",
            (true, false) => "Child",
        }
    }
}

/// One unit's vehicle state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitVehicleView {
    /// The passenger link's state is 3 (or the roster flag for an unstreamed member).
    pub seated: bool,
    /// The passenger link's state is non-zero (or the roster flag).
    pub using: bool,
    /// Seated, with the unit's charm the vehicle (`0x613570`).
    pub controlling: bool,
    /// The unit's seat flags and UI skin (`0x613600`), `None` without a resolvable seat.
    pub seat: Option<(u32, i32)>,
    /// The root vehicle's flat seats, for a streamed unit in a vehicle.
    pub seats: Vec<VehicleSeatView>,
}

/// The aiming unit's `Vehicle.dbc` row and current pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleAimView {
    pub flags: u32,
    pub pitch_min: f32,
    pub pitch_max: f32,
    /// The aiming unit's pitch, radians.
    pub pitch: f32,
}

impl VehicleAimView {
    fn range(&self) -> (f32, f32) {
        if self.flags & VEHICLE_CUSTOM_PITCH != 0 {
            (self.pitch_min, self.pitch_max)
        } else {
            (PITCH_MIN, PITCH_MAX)
        }
    }

    /// `0x5fb3a0`: the requested pitch clamped to the range.
    fn clamp(&self, pitch: f32) -> f32 {
        let (lo, hi) = self.range();
        if pitch < lo {
            lo
        } else if hi <= pitch {
            hi
        } else {
            pitch
        }
    }
}

/// A `VehicleUIIndicator.dbc` row and its `VehicleUIIndSeat` rows, in table order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleIndicatorView {
    pub texture: String,
    /// `(virtual seat index, x, y)`.
    pub seats: Vec<(u32, f32, f32)>,
}

/// One battlefield vehicle (`0xbea5b8` list, `GetBattlefieldVehicleInfo` `0x54c4d0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BattlefieldVehicleView {
    /// The position on the displayed map, `None` off it (the getter then answers nothing).
    pub uv: Option<(f32, f32)>,
    pub name: Option<String>,
    /// `UNIT_FLAG_POSSESSED`.
    pub possessed: bool,
    /// `"Drive"`, `"Fly"`, `"Idle"`, `"Airship Horde"` or `"Airship Alliance"` by the vehicle's UI
    /// locomotion type, for a vehicle whose `Vehicle.dbc` flags have `0x10000000`; else `None`.
    pub vehicle_type: Option<String>,
    /// The facing, radians.
    pub facing: f32,
    /// The vehicle is the player's own.
    pub is_player: bool,
    pub alive: bool,
}

/// What the app pushes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathVehicles {
    pub units: HashMap<u64, UnitVehicleView>,
    /// The player's seat flags when `0x5fb560` admits them: seated, and the active mover the
    /// vehicle or the player.
    pub player_seat_flags: Option<u32>,
    /// The active mover is the root vehicle (`CanEjectPassengerFromSeat`'s `0x74c650` test).
    pub drives_root: bool,
    /// The aiming unit's row, `None` when it has no vehicle row.
    pub aim: Option<VehicleAimView>,
    /// There is an active mover (`[0xca1238]`).
    pub has_mover: bool,
    /// The battlefield vehicle list, at most 40.
    pub battlefield: Vec<BattlefieldVehicleView>,
    /// The possess bar's spell (`0xc234e0`, picked by `0x5d62a0`), `None` without one.
    pub possess_spell: Option<u32>,
    /// That spell's row as `GetPossessInfo` reads it; `None` without a spell or its row.
    pub possess: Option<PossessView>,
}

/// The possess bar's two slots (`GetPossessInfo` `0x5d5820`): the spell, then its cancel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PossessView {
    /// The spell's `SpellIcon.dbc` path: its active icon when it has one, else its icon.
    pub texture: Option<String>,
    pub name: String,
    /// `SpellIcon.dbc` row 0x2b5, the cancel slot's art.
    pub cancel_texture: Option<String>,
    /// `Attributes` lacks 0x80000000 (cannot be cancelled), so the cancel slot shows.
    pub cancelable: bool,
}

/// A vehicle request for the app's outbox.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VehicleCall {
    Exit,
    NextSeat,
    PrevSeat,
    SwitchSeat {
        vehicle: u64,
        seat: u8,
    },
    Eject {
        passenger: u64,
    },
    /// The aiming unit's pitch, radians, already clamped.
    AimPitch(f32),
}

impl super::UiScript {
    pub fn set_wrath_vehicles(&mut self, vehicles: WrathVehicles) {
        self.model_mut().wrath_vehicles = vehicles;
    }

    /// The indicator table, pushed once.
    pub fn set_vehicle_indicators(&mut self, indicators: HashMap<u32, VehicleIndicatorView>) {
        self.model_mut().vehicle_indicators = indicators;
    }

    pub fn take_vehicle_calls(&mut self) -> Vec<VehicleCall> {
        std::mem::take(&mut self.model_mut().vehicle_calls)
    }
}

fn unit_view<'a>(model: &'a Model, token: &str) -> Option<&'a UnitVehicleView> {
    let guid = model.unit(token)?.guid;
    model.wrath_vehicles.units.get(&guid)
}

fn string_arg(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_str().ok()?.to_string()),
        Value::Integer(i) => Some(i.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `lua_isnumber`, then the rounded value.
fn number(lua: &Lua, v: &Value) -> mlua::Result<Option<f64>> {
    lua.coerce_number(v.clone())
}

fn index(lua: &Lua, v: &Value) -> mlua::Result<Option<i64>> {
    Ok(number(lua, v)?.map(|n| n.round_ties_even() as i64))
}

fn push(lua: &Lua, call: VehicleCall) {
    lua.app_data_mut::<Model>()
        .expect("model")
        .vehicle_calls
        .push(call);
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// The 1-based flat seat of the unit's root vehicle.
fn seat_at(view: &UnitVehicleView, index: i64) -> Option<&VehicleSeatView> {
    view.seats.get(usize::try_from(index.checked_sub(1)?).ok()?)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let set = |name: &str, f: fn(&Lua, MultiValue) -> mlua::Result<MultiValue>| {
        g.set(name, lua.create_function(f)?)
    };

    for (name, usage, seated) in [
        ("UnitInVehicle", "Usage: UnitInVehicle(\"unit\")", true),
        (
            "UnitUsingVehicle",
            "Usage: UnitUsingVehicle(\"unit\")",
            false,
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let Some(token) = string_arg(&arg(&args, 0)) else {
                    return Err(mlua::Error::runtime(usage));
                };
                let model = lua.app_data_ref::<Model>().expect("model");
                let on =
                    unit_view(&model, &token)
                        .is_some_and(|v| if seated { v.seated } else { v.using });
                Ok(flag(on))
            })?,
        )?;
    }
    set("UnitControllingVehicle", |lua, args| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let on = string_arg(&arg(&args, 0))
            .and_then(|t| unit_view(&model, &t))
            .is_some_and(|v| v.controlling);
        one(Value::Boolean(on))
    })?;
    set("UnitHasVehicleUI", |lua, args| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let on = string_arg(&arg(&args, 0))
            .and_then(|t| unit_view(&model, &t))
            .and_then(|v| v.seat)
            .is_some_and(|(flags, _)| flags & SEAT_HAS_UI != 0);
        one(Value::Boolean(on))
    })?;
    // "Natural" or "Mechanical", "" past them; nothing without a seat (`0x6137d0`).
    set("UnitVehicleSkin", |lua, args| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let Some((_, skin)) = string_arg(&arg(&args, 0))
            .and_then(|t| unit_view(&model, &t))
            .and_then(|v| v.seat)
        else {
            return Ok(MultiValue::new());
        };
        let name = match skin {
            0 => "Natural",
            1 => "Mechanical",
            _ => "",
        };
        one(Value::String(lua.create_string(name)?))
    })?;
    set("UnitVehicleSeatCount", |lua, args| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let n = string_arg(&arg(&args, 0))
            .and_then(|t| unit_view(&model, &t))
            .map_or(0, |v| v.seats.len());
        one(Value::Integer(n as i64))
    })?;
    // controlType, occupantName, occupantRealm, ejectable, canSwitchSeats; nothing off the seats
    // (`0x6138c0`). The realm is always nil.
    set("UnitVehicleSeatInfo", |lua, args| {
        let i = index(lua, &arg(&args, 1))?.unwrap_or(0);
        let model = lua.app_data_ref::<Model>().expect("model");
        let Some(seat) = string_arg(&arg(&args, 0))
            .and_then(|t| unit_view(&model, &t))
            .and_then(|v| seat_at(v, i))
        else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(seat.control())?),
            match &seat.occupant_name {
                Some(n) if seat.occupant.is_some() => Value::String(lua.create_string(n)?),
                _ => Value::Nil,
            },
            Value::Nil,
            Value::Boolean(seat.flags_b & SEAT_B_EJECTABLE != 0),
            Value::Boolean(seat.flags & SEAT_CAN_SWITCH != 0),
        ]))
    })?;
    // The player only: an empty seat that admits switching, from a seat that lets us switch
    // (`0x6139b0` → `0x74ca90`).
    set("UnitSwitchToVehicleSeat", |lua, args| {
        let i = index(lua, &arg(&args, 1))?.unwrap_or(0);
        let call = {
            let model = lua.app_data_ref::<Model>().expect("model");
            let me = model.unit("player").map(|u| u.guid);
            let token = string_arg(&arg(&args, 0));
            let view = token
                .as_deref()
                .and_then(|t| model.unit(t))
                .filter(|u| Some(u.guid) == me)
                .and_then(|u| model.wrath_vehicles.units.get(&u.guid));
            let ours = model
                .wrath_vehicles
                .player_seat_flags
                .is_some_and(|f| f & SEAT_CAN_SWITCH != 0);
            view.and_then(|v| seat_at(v, i))
                .filter(|s| ours && s.occupant.is_none() && s.flags & SEAT_CAN_SWITCH != 0)
                .map(|s| VehicleCall::SwitchSeat {
                    vehicle: s.vehicle,
                    seat: s.slot,
                })
        };
        if let Some(call) = call {
            push(lua, call);
        }
        Ok(MultiValue::new())
    })?;
    for (name, mask, boolean) in [
        ("CanExitVehicle", SEAT_CAN_EXIT, false),
        ("CanSwitchVehicleSeats", SEAT_CAN_SWITCH, false),
        ("CanSwitchVehicleSeat", SEAT_CAN_SWITCH, true),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let model = lua.app_data_ref::<Model>().expect("model");
                let on = model
                    .wrath_vehicles
                    .player_seat_flags
                    .is_some_and(|f| f & mask != 0);
                Ok(if boolean {
                    Value::Boolean(on)
                } else {
                    flag(on)
                })
            })?,
        )?;
    }
    // Leave, or `SPELL_FAILED_CANT_DO_THAT_RIGHT_NOW` from a seat that cannot (`0x5fb660`).
    set("VehicleExit", |lua, _| {
        let mut model = lua.app_data_mut::<Model>().expect("model");
        let v = &model.wrath_vehicles;
        if !v.has_mover {
            return Ok(MultiValue::new());
        }
        if v.player_seat_flags.is_some_and(|f| f & SEAT_CAN_EXIT != 0) {
            model.vehicle_calls.push(VehicleCall::Exit);
        } else {
            model.ui_errors.push("SPELL_FAILED_CANT_DO_THAT_RIGHT_NOW");
        }
        Ok(MultiValue::new())
    })?;
    for (name, call) in [
        ("VehicleNextSeat", VehicleCall::NextSeat),
        ("VehiclePrevSeat", VehicleCall::PrevSeat),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let mut model = lua.app_data_mut::<Model>().expect("model");
                let v = &model.wrath_vehicles;
                if v.has_mover
                    && v.player_seat_flags
                        .is_some_and(|f| f & SEAT_CAN_SWITCH != 0)
                {
                    model.vehicle_calls.push(call);
                }
                Ok(())
            })?,
        )?;
    }
    // The root vehicle we drive, a passenger in the seat, and the seat's ejectable flag.
    set("CanEjectPassengerFromSeat", |lua, args| {
        let Some(i) = index(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                "Usage: CanEjectPassengerFromSeat(seatIndex)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let v = &model.wrath_vehicles;
        let seat = model
            .unit("player")
            .and_then(|u| v.units.get(&u.guid))
            .and_then(|u| seat_at(u, i));
        let on = v.has_mover
            && v.drives_root
            && seat.is_some_and(|s| s.occupant.is_some() && s.flags_b & SEAT_B_EJECTABLE != 0);
        one(Value::Boolean(on))
    })?;
    set("EjectPassengerFromSeat", |lua, args| {
        let Some(i) = index(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                "Usage: EjectPassengerFromSeat(seatIndex)",
            ));
        };
        let passenger = {
            let model = lua.app_data_ref::<Model>().expect("model");
            let v = &model.wrath_vehicles;
            model
                .unit("player")
                .and_then(|u| v.units.get(&u.guid))
                .and_then(|u| seat_at(u, i))
                .and_then(|s| s.occupant)
                .filter(|_| v.has_mover)
        };
        if let Some(passenger) = passenger {
            push(lua, VehicleCall::Eject { passenger });
        }
        Ok(MultiValue::new())
    })?;
    // backgroundTexture, numSeatIndicators; nothing for an id without a row (`0x614e60`).
    set("GetVehicleUIIndicator", |lua, args| {
        let Some(id) = index(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                "Usage: GetVehicleUIIndicator(indicatorID)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let Some(ind) = u32::try_from(id)
            .ok()
            .filter(|&id| id > 0)
            .and_then(|id| model.vehicle_indicators.get(&id))
        else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(&ind.texture)?),
            Value::Integer(ind.seats.len() as i64),
        ]))
    })?;
    // virtualSeatIndex, xPos, yPos (`0x614ef0`).
    set("GetVehicleUIIndicatorSeat", |lua, args| {
        let (Some(id), Some(i)) = (index(lua, &arg(&args, 0))?, index(lua, &arg(&args, 1))?) else {
            return Err(mlua::Error::runtime(
                "Usage: GetVehicleUIIndicatorSeat(indicatorID, indicatorSeatIndex)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let seat = u32::try_from(id)
            .ok()
            .and_then(|id| model.vehicle_indicators.get(&id))
            .and_then(|ind| ind.seats.get(usize::try_from(i.checked_sub(1)?).ok()?));
        let Some(&(index, x, y)) = seat else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            Value::Integer(i64::from(index)),
            Value::Number(f64::from(x)),
            Value::Number(f64::from(y)),
        ]))
    })?;
    for (name, mask) in [
        ("IsVehicleAimAngleAdjustable", VEHICLE_AIM_ANGLE),
        ("IsVehicleAimPowerAdjustable", VEHICLE_AIM_POWER),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let model = lua.app_data_ref::<Model>().expect("model");
                Ok(flag(
                    model
                        .wrath_vehicles
                        .aim
                        .is_some_and(|a| a.flags & mask != 0),
                ))
            })?,
        )?;
    }
    set("VehicleAimGetAngle", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let pitch = model.wrath_vehicles.aim.map_or(0.0, |a| a.pitch);
        one(Value::Number(f64::from(pitch)))
    })?;
    // `(pitch - min) / (max - min)`, 0 over a degenerate range (`0x5f9e60`).
    set("VehicleAimGetNormAngle", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let n = model.wrath_vehicles.aim.map_or(0.0, |a| {
            let (lo, hi) = a.range();
            if (hi - lo).abs() <= RANGE_EPSILON {
                0.0
            } else {
                (a.pitch - lo) / (hi - lo)
            }
        });
        one(Value::Number(f64::from(n)))
    })?;
    // Only an angle-adjustable vehicle takes a requested angle (`0x5fb820`, `0x5fb8c0`).
    for (name, normalized) in [
        ("VehicleAimRequestAngle", false),
        ("VehicleAimRequestNormAngle", true),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let Some(n) = number(lua, &arg(&args, 0))? else {
                    return Ok(());
                };
                let aim = lua
                    .app_data_ref::<Model>()
                    .expect("model")
                    .wrath_vehicles
                    .aim;
                if let Some(a) = aim.filter(|a| a.flags & VEHICLE_AIM_ANGLE != 0) {
                    let (lo, hi) = a.range();
                    let pitch = if normalized {
                        lo + (hi - lo) * n as f32
                    } else {
                        n as f32
                    };
                    push(lua, VehicleCall::AimPitch(a.clamp(pitch)));
                }
                Ok(())
            })?,
        )?;
    }
    // The pitch plus or minus the step, 0.1 radians by default (`0x5fb770`, `0x5fb7d0`).
    for (name, sign) in [
        ("VehicleAimIncrement", 1.0f32),
        ("VehicleAimDecrement", -1.0),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let step = number(lua, &arg(&args, 0))?.map_or(AIM_STEP, |n| n as f32);
                let aim = lua
                    .app_data_ref::<Model>()
                    .expect("model")
                    .wrath_vehicles
                    .aim;
                if let Some(a) = aim {
                    push(lua, VehicleCall::AimPitch(a.clamp(a.pitch + sign * step)));
                }
                Ok(())
            })?,
        )?;
    }
    // GetPossessInfo(index) (`0x5d5820`): slot 1 the spell, 2 its cancel, else three nils; only an
    // index equal to 0, a non-number included, raises (`0x5d5843`), and the rest truncate.
    set("GetPossessInfo", |lua, args| {
        let n = number(lua, &arg(&args, 0))?.unwrap_or(0.0);
        let Some(slot) = (n != 0.0).then_some(n as i64) else {
            return Err(mlua::Error::runtime("Usage: GetPossessInfo(index)"));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let possess = model
            .wrath_vehicles
            .possess
            .as_ref()
            .filter(|_| model.unit("player").is_some_and(|u| u.exists));
        let text = |s: &Option<String>| -> mlua::Result<Value> {
            Ok(match s {
                Some(t) => Value::String(lua.create_string(t)?),
                None => Value::Nil,
            })
        };
        let (texture, enabled) = match (possess, slot) {
            (Some(p), 1) => (&p.texture, true),
            (Some(p), 2) => (&p.cancel_texture, p.cancelable),
            _ => return Ok(MultiValue::from_vec(vec![Value::Nil; 3])),
        };
        let name = possess.map_or(Value::Nil, |p| {
            lua.create_string(&p.name).map_or(Value::Nil, Value::String)
        });
        Ok(MultiValue::from_vec(vec![
            text(texture)?,
            name,
            Value::Boolean(enabled),
        ]))
    })?;
    // IsPossessBarVisible() (`0x5a8820`): a boolean, false without a possess spell (`0xc234e0`, the
    // slot GetPossessInfo reads). Labelled: the spell's `AttributesEx7 & 0x1000` (attribute 0xec,
    // which hides the bar) is not loaded, so a present spell shows it.
    set("IsPossessBarVisible", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model");
        Ok(MultiValue::from_vec(vec![Value::Boolean(
            model.wrath_vehicles.possess.is_some(),
        )]))
    })?;
    set("GetNumBattlefieldVehicles", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model");
        one(Value::Integer(model.wrath_vehicles.battlefield.len() as i64))
    })?;
    // x, y, name, isPossessed, vehicleType, orientation, isPlayer, isAlive; nothing for a vehicle
    // off the displayed map or an index off the list (`0x54c4d0`).
    set("GetBattlefieldVehicleInfo", |lua, args| {
        let Some(i) = index(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                "Usage: GetBattlefieldVehicleInfo(index)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let Some((v, (x, y))) = usize::try_from(i - 1)
            .ok()
            .and_then(|i| model.wrath_vehicles.battlefield.get(i))
            .and_then(|v| Some((v, v.uv?)))
        else {
            return Ok(MultiValue::new());
        };
        let text = |s: &Option<String>| -> mlua::Result<Value> {
            Ok(match s {
                Some(t) => Value::String(lua.create_string(t)?),
                None => Value::Nil,
            })
        };
        Ok(MultiValue::from_vec(vec![
            Value::Number(f64::from(x)),
            Value::Number(f64::from(y)),
            text(&v.name)?,
            Value::Boolean(v.possessed),
            text(&v.vehicle_type)?,
            Value::Number(f64::from(v.facing)),
            Value::Boolean(v.is_player),
            Value::Boolean(v.alive),
        ]))
    })?;
    // GameTooltip:SetPossession(slot) (`0x6257c0`): slot 1 shows the possess spell; 1 when it
    // renders, else nil.
    let tooltips: mlua::Table = lua.named_registry_value(super::tooltip::REG_TOOLTIP_METHODS)?;
    tooltips.set(
        "SetPossession",
        lua.create_function(|lua, (this, slot): (mlua::Table, Value)| {
            let Some(slot) = index(lua, &slot)? else {
                let name: Option<String> = this
                    .get::<mlua::Function>("GetName")
                    .and_then(|f| f.call::<Option<String>>(this.clone()))
                    .ok()
                    .flatten();
                return Err(mlua::Error::runtime(format!(
                    "Usage: {}:SetPossession(slot)",
                    name.as_deref().unwrap_or("<unnamed>")
                )));
            };
            let spell = lua
                .app_data_ref::<Model>()
                .expect("model")
                .wrath_vehicles
                .possess_spell;
            let Some(spell) = spell.filter(|_| slot == 1) else {
                return Ok(Value::Nil);
            };
            let known = super::tooltip_spell::spell_view_of(lua, spell, false).is_some();
            super::tooltip_spell::set_spell_by_id(
                lua,
                &this,
                spell,
                None,
                super::tooltip_spell::SpellRenderOpts::default(),
                None,
            )?;
            Ok(flag(known))
        })?,
    )?;
    for name in [
        "VehicleAimUpStart",
        "VehicleAimUpStop",
        "VehicleAimDownStart",
        "VehicleAimDownStop",
    ] {
        g.set(name, lua.create_function(|_, _: MultiValue| Ok(()))?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{UiScript, UnitState};

    const ME: u64 = 0x10;
    const CAR: u64 = 0xF150_0000_0000_0007;
    const TURRET: u64 = 0xF150_0000_0000_0008;
    const PAL: u64 = 0x11;

    fn seat(
        vehicle: u64,
        slot: u8,
        flags: u32,
        root: bool,
        who: Option<(u64, &str)>,
    ) -> VehicleSeatView {
        VehicleSeatView {
            vehicle,
            slot,
            flags,
            flags_b: if slot == 1 { SEAT_B_EJECTABLE } else { 0 },
            root,
            occupant: who.map(|w| w.0),
            occupant_name: who.map(|w| w.1.to_string()),
        }
    }

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        for (token, guid) in [("player", ME), ("party1", PAL)] {
            s.set_unit(
                token,
                Some(UnitState {
                    exists: true,
                    guid,
                    ..Default::default()
                }),
            );
        }
        let seats = vec![
            seat(
                CAR,
                0,
                SEAT_CAN_CONTROL | SEAT_CAN_EXIT | SEAT_CAN_SWITCH,
                true,
                Some((ME, "Me")),
            ),
            seat(CAR, 1, SEAT_CAN_SWITCH, false, Some((PAL, "Pal"))),
            seat(TURRET, 0, SEAT_CAN_CONTROL | SEAT_CAN_SWITCH, false, None),
        ];
        let mut units = HashMap::new();
        units.insert(
            ME,
            UnitVehicleView {
                seated: true,
                using: true,
                controlling: true,
                seat: Some((SEAT_HAS_UI, 1)),
                seats: seats.clone(),
            },
        );
        units.insert(
            PAL,
            UnitVehicleView {
                seated: false,
                using: true,
                ..Default::default()
            },
        );
        s.set_wrath_vehicles(WrathVehicles {
            units,
            player_seat_flags: Some(SEAT_CAN_CONTROL | SEAT_CAN_EXIT | SEAT_CAN_SWITCH),
            drives_root: true,
            aim: Some(VehicleAimView {
                flags: VEHICLE_AIM_ANGLE | VEHICLE_CUSTOM_PITCH,
                pitch_min: -0.5,
                pitch_max: 1.5,
                pitch: 0.5,
            }),
            has_mover: true,
            battlefield: Vec::new(),
            possess_spell: None,
            possess: None,
        });
        let mut ind = HashMap::new();
        ind.insert(
            3,
            VehicleIndicatorView {
                texture: "Interface\\Vehicles\\SeatIndicator\\Car".into(),
                seats: vec![(1, 0.5, 0.25), (2, 0.5, 0.75)],
            },
        );
        s.set_vehicle_indicators(ind);
        s
    }

    #[test]
    fn unit_predicates_and_the_flat_seats_read_the_view() {
        let s = vm();
        s.run(
            r##"
            assert(UnitInVehicle("player") == 1 and UnitUsingVehicle("player") == 1)
            assert(UnitInVehicle("party1") == nil and UnitUsingVehicle("party1") == 1)
            assert(UnitControllingVehicle("player") == true and UnitControllingVehicle("party1") == false)
            assert(UnitHasVehicleUI("player") == true and UnitHasVehicleUI("target") == false)
            assert(UnitVehicleSkin("player") == "Mechanical" and select("#", UnitVehicleSkin("party1")) == 0)
            assert(UnitVehicleSeatCount("player") == 3 and UnitVehicleSeatCount("party1") == 0)
            local c, n, r, e, sw = UnitVehicleSeatInfo("player", 2)
            assert(c == "None" and n == "Pal" and r == nil and e == true and sw == true)
            assert(UnitVehicleSeatInfo("player", 1) == "Root" and UnitVehicleSeatInfo("player", 3) == "Child")
            assert(select("#", UnitVehicleSeatInfo("player", 4)) == 0)
            assert(not pcall(UnitInVehicle))
            assert(CanExitVehicle() == 1 and CanSwitchVehicleSeats() == 1 and CanSwitchVehicleSeat() == true)
            assert(CanEjectPassengerFromSeat(2) == true and CanEjectPassengerFromSeat(1) == false)
            local t, k = GetVehicleUIIndicator(3)
            assert(k == 2 and select("#", GetVehicleUIIndicator(4)) == 0)
            local i, x, y = GetVehicleUIIndicatorSeat(3, 2)
            assert(i == 2 and x == 0.5 and y == 0.75)
            assert(IsVehicleAimAngleAdjustable() == 1 and IsVehicleAimPowerAdjustable() == nil)
            assert(math.abs(VehicleAimGetNormAngle() - 0.5) < 1e-6)
            "##,
        )
        .unwrap();
    }

    #[test]
    fn battlefield_vehicles_answer_only_on_the_map() {
        let mut s = vm();
        let mut v = s.model_mut().wrath_vehicles.clone();
        v.battlefield = vec![
            BattlefieldVehicleView {
                uv: Some((0.25, 0.5)),
                name: Some("Siege Engine".into()),
                possessed: true,
                vehicle_type: Some("Drive".into()),
                facing: 1.5,
                is_player: false,
                alive: true,
            },
            BattlefieldVehicleView::default(),
        ];
        s.set_wrath_vehicles(v);
        s.run(
            r##"
            assert(GetNumBattlefieldVehicles() == 2)
            local x, y, n, p, t, o, me, alive = GetBattlefieldVehicleInfo(1)
            assert(x == 0.25 and y == 0.5 and n == "Siege Engine" and p == true and t == "Drive")
            assert(o == 1.5 and me == false and alive == true)
            assert(select("#", GetBattlefieldVehicleInfo(2)) == 0)
            assert(select("#", GetBattlefieldVehicleInfo(3)) == 0)
            assert(not pcall(GetBattlefieldVehicleInfo))
            "##,
        )
        .unwrap();
    }

    #[test]
    fn set_possession_shows_slot_one_only() {
        let mut s = vm();
        s.run("T = CreateFrame('GameTooltip', 'PossTip', UIParent, 'GameTooltipTemplate')")
            .ok();
        s.run("T = T or CreateFrame('GameTooltip', 'PossTip')")
            .unwrap();
        assert!(
            s.eval::<bool>("return T:SetPossession(1) == nil").unwrap(),
            "no spell"
        );
        let mut v = s.model_mut().wrath_vehicles.clone();
        v.possess_spell = Some(5);
        s.set_wrath_vehicles(v);
        assert!(
            s.eval::<bool>("return T:SetPossession(2) == nil").unwrap(),
            "slot 2 shows nothing"
        );
        s.run("T:SetPossession(1)").unwrap();
        assert!(s.run("T:SetPossession()").is_err());
    }

    #[test]
    fn possess_info_is_the_spell_then_its_cancel() {
        let mut s = vm();
        s.run("assert(select('#', GetPossessInfo(1)) == 3 and GetPossessInfo(1) == nil)")
            .unwrap();
        assert!(s
            .eval::<bool>("return IsPossessBarVisible() == false")
            .unwrap());
        let mut v = s.model_mut().wrath_vehicles.clone();
        v.possess = Some(PossessView {
            texture: Some("Interface\\Icons\\Spell_Shadow_ShadowWordDominate".into()),
            name: "Mind Control".into(),
            cancel_texture: Some("Interface\\Icons\\Spell_ChargeNegative".into()),
            cancelable: false,
        });
        s.set_wrath_vehicles(v);
        s.run(
            r##"
            local t, n, e = GetPossessInfo(1.9)
            assert(t == "Interface\\Icons\\Spell_Shadow_ShadowWordDominate" and n == "Mind Control" and e == true)
            t, n, e = GetPossessInfo(2)
            assert(t == "Interface\\Icons\\Spell_ChargeNegative" and n == "Mind Control" and e == false)
            assert(select("#", GetPossessInfo(3)) == 3 and GetPossessInfo(3) == nil)
            assert(not pcall(GetPossessInfo) and not pcall(GetPossessInfo, 0))
            assert(IsPossessBarVisible() == true)
            "##,
        )
        .unwrap();
    }

    #[test]
    fn the_verbs_queue_their_requests_in_call_order() {
        let mut s = vm();
        s.run(
            "VehicleExit() VehicleNextSeat() VehiclePrevSeat() UnitSwitchToVehicleSeat('player', 3) \
             UnitSwitchToVehicleSeat('player', 2) UnitSwitchToVehicleSeat('party1', 3) \
             EjectPassengerFromSeat(2) VehicleAimRequestNormAngle(1) VehicleAimRequestAngle(-9) \
             VehicleAimIncrement() VehicleAimDecrement(0.25) VehicleAimUpStart()",
        )
        .unwrap();
        let calls = s.take_vehicle_calls();
        assert_eq!(
            calls[..5],
            [
                VehicleCall::Exit,
                VehicleCall::NextSeat,
                VehicleCall::PrevSeat,
                VehicleCall::SwitchSeat {
                    vehicle: TURRET,
                    seat: 0
                },
                VehicleCall::Eject { passenger: PAL },
            ]
        );
        let pitches: Vec<f32> = calls[5..]
            .iter()
            .map(|c| match c {
                VehicleCall::AimPitch(p) => *p,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(pitches, [1.5, -0.5, 0.6, 0.25]);
        // A seat that cannot exit refuses with the client's error.
        let mut v = s.model_mut().wrath_vehicles.clone();
        v.player_seat_flags = Some(0);
        s.set_wrath_vehicles(v);
        s.run("VehicleExit() VehicleNextSeat()").unwrap();
        assert!(s.take_vehicle_calls().is_empty());
        assert_eq!(s.take_ui_errors(), ["SPELL_FAILED_CANT_DO_THAT_RIGHT_NOW"]);
    }
}
