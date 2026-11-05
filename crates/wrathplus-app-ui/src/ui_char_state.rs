//! Build 12340's character-state feed for the UI ([`WrathCharState`], [`WrathUnitExtra`]): the
//! active player's mount, movement, in-water, outdoor and flyable state, power regen, resurrect
//! offerer, totem guids, instance difficulty and ignore guids, and each pushed unit's speed,
//! pitch and sanctuary bit; and the `HearthAndResurrectFromArea` send.

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use wrathplus_assets::coords::bevy_to_wow;
use wrathplus_protocol::types::Field;
use wrathplus_ui::script::{wrath_power_regen, UiScript, WrathCharState, WrathUnitExtra};
use wrathplus_world::world_point::{Subject, WorldPoint};

use crate::app_core::movement::{may_swim, move_flags, swim_enter_depth};
use crate::app_core::player::Player;
use crate::app_core::schedule::{UiFeed, UiInput};
use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::net::wrath_vehicle::VehicleState;
use crate::net::{ClientCommand, GuidIndex, NetCommands};

/// `AreaTable` flags `0x51a1d0` and `0x77fbf0` read: flyable, no-fly, inside, outside.
const AREA_FLAG_FLYABLE: u32 = 0x400;
const AREA_FLAG_NO_FLY_ZONE: u32 = 0x2000_0000;
const AREA_FLAG_INSIDE: u32 = 0x0200_0000;
const AREA_FLAG_OUTSIDE: u32 = 0x0400_0000;
/// `WMOAreaTable` flags `0x77fbf0` reads: outdoors, indoors.
const WMO_AREA_OUTDOORS: u32 = 0x4;
const WMO_AREA_INDOORS: u32 = 0x2;
/// `0x7261b0`'s Northrend gate: map 571 without Cold Weather Flying (54197) is not flyable.
const MAP_NORTHREND: u32 = 571;
const COLD_WEATHER_FLYING: u32 = 54197;

/// `0x77fbf0`, the outdoor test: inside a WMO group (`interior` holds the group's own
/// `WMOAreaTable` row flags, if it has a row) flag 4 is outdoors and flag 2 indoors, else the
/// group's exterior bit, which an interior claim lacks; on terrain the area's `AreaTable` flags,
/// `0x4000000` outdoors and `0x2000000` indoors, else (or with no row) outdoors.
pub(crate) fn outdoors(interior: Option<Option<u32>>, area_flags: Option<u32>) -> bool {
    match interior {
        Some(row) => match row {
            Some(f) if f & WMO_AREA_OUTDOORS != 0 => true,
            Some(f) if f & WMO_AREA_INDOORS != 0 => false,
            _ => false,
        },
        None => match area_flags {
            Some(f) if f & AREA_FLAG_OUTSIDE != 0 => true,
            Some(f) if f & AREA_FLAG_INSIDE != 0 => false,
            _ => true,
        },
    }
}

/// `0x51a1d0`: the area's row (`0xbd0810`, the subzone), else the zone's (`0xbd080c`), has the
/// flyable flag without the no-fly flag; then, with a player, `0x7261b0`'s Northrend gate.
pub(crate) fn flyable(area_flags: Option<u32>, player: Option<(u32, bool)>) -> bool {
    let area =
        area_flags.is_some_and(|f| f & AREA_FLAG_FLYABLE != 0 && f & AREA_FLAG_NO_FLY_ZONE == 0);
    area && !player.is_some_and(|(map, cold_weather)| map == MAP_NORTHREND && !cold_weather)
}

/// `0x730d10`'s in-water bit (`[unit+0xa30] & 0x200000`, `IsSwimming`'s read): `SWIMMING` on
/// the movement word, or the liquid more than 0.75 × the collision height over the feet for a
/// unit that may swim (`0x4f53d0`) and rides no transport.
pub(crate) fn in_water(
    flags: u32,
    depth: Option<f32>,
    height: f32,
    unit_flags: u32,
    riding: bool,
) -> bool {
    flags & move_flags::SWIMMING != 0
        || (depth.is_some_and(|d| swim_enter_depth(height) < d) && may_swim(unit_flags) && !riding)
}

/// The world-side reads the feed takes.
#[derive(SystemParam)]
pub(crate) struct CharSources<'w, 's> {
    self_q: Query<'w, 's, (Entity, &'static ObjectStore), With<SelfPlayer>>,
    stores: Query<'w, 's, &'static ObjectStore>,
    player: Option<Res<'w, Player>>,
    range: crate::spell::RangeUnits<'w, 's>,
    index: Option<Res<'w, GuidIndex>>,
    vehicle: Option<Res<'w, VehicleState>>,
    world: WorldPoint<'w, 's>,
    areas: Option<Res<'w, crate::area::AreaTableRes>>,
    map: Option<Res<'w, wrathplus_world::world_map::CurrentMap>>,
    actions: Option<Res<'w, crate::app_core::state::PlayerActions>>,
}

/// The session-side reads the feed takes.
#[derive(SystemParam)]
pub(crate) struct SessionSources<'w> {
    death: Option<Res<'w, crate::app_core::death::DeathNet>>,
    names: Option<Res<'w, crate::names::NameCache>>,
    commands: Res<'w, NetCommands>,
    totems: Option<Res<'w, crate::ui_totems::NativeTotems>>,
    lock: Option<Res<'w, crate::ui_static_popup::InstanceLock>>,
    social: Option<Res<'w, crate::app_core::interaction::SocialState>>,
}

impl CharSources<'_, '_> {
    fn entity(&self, guid: u64) -> Option<Entity> {
        self.index.as_ref()?.0.get(&guid).copied()
    }

    /// The leaf area's `AreaTable` flags, else its zone's.
    fn area_flags(&self) -> Option<u32> {
        let areas = &self.areas.as_ref()?.0;
        let leaf = areas.get(self.world.area()?)?;
        Some(leaf.flags)
    }

    fn outdoors(&self) -> bool {
        let interior = self
            .world
            .area_interior_rows()
            .map(|(_, group, _)| group.map(|row| row.flags));
        outdoors(interior, self.area_flags())
    }

    fn player_state(&self) -> WrathCharState {
        let Ok((me, store)) = self.self_q.single() else {
            return WrathCharState {
                flyable: flyable(self.area_flags(), None),
                outdoors: true,
                ..Default::default()
            };
        };
        let flags = |e: Entity| self.range.unit(e).map_or(0, |u| u.motion.flags);
        // `0x612500`: seated on a vehicle (transport link type 3), the vehicle's word.
        let mover = self
            .vehicle
            .as_ref()
            .and_then(|v| v.vehicle)
            .and_then(|g| self.entity(g))
            .unwrap_or(me);
        let own = flags(me);
        let (depth, height, riding) = self.player.as_deref().map_or((None, 0.0, false), |p| {
            let wow = bevy_to_wow(p.pos);
            let depth = self
                .world
                .liquid_at(Subject::Player, wow)
                .map(|hit| hit.surface_z - wow[2]);
            (depth, p.collision_height.0, p.ride.is_some())
        });
        let cold_weather = self
            .actions
            .as_ref()
            .is_some_and(|a| a.spells.contains(&COLD_WEATHER_FLYING));
        let map = self.map.as_ref().map_or(0, |m| m.0);
        let power = store.0.unit_power_type();
        let modifier = |field| store.0.get_float(field, u16::from(power)).unwrap_or(0.0);
        WrathCharState {
            has_player: true,
            mount_display: store.0.unit_mount_display_id(),
            flying: flags(mover) & move_flags::FLYING != 0,
            swimming: in_water(own, depth, height, store.0.unit_flags(), riding),
            outdoors: self.outdoors(),
            flyable: flyable(self.area_flags(), Some((map, cold_weather))),
            stealthed: store.0.unit_is_stealthed(),
            power_regen: wrath_power_regen(
                power,
                modifier(Field::UnitPowerRegenFlatModifier),
                modifier(Field::UnitPowerRegenInterruptedFlatModifier),
            ),
            ..Default::default()
        }
    }

    fn unit_extras(&self, guids: Vec<u64>) -> HashMap<u64, WrathUnitExtra> {
        guids
            .into_iter()
            .filter_map(|guid| {
                let e = self.entity(guid)?;
                let (speed, pitch) = self.range.speed_and_pitch(e).unwrap_or_default();
                let sanctuary = self.stores.get(e).is_ok_and(|s| s.0.unit_is_in_sanctuary());
                Some((
                    guid,
                    WrathUnitExtra {
                        speed,
                        pitch,
                        sanctuary,
                    },
                ))
            })
            .collect()
    }
}

impl SessionSources<'_> {
    fn fill(&self, state: &mut WrathCharState) {
        // `0x515950`: the name cache by the offer's caster guid, asked once on a miss
        // (`0x67d770`); a creature caster has no player name, so it reads nil.
        state.resurrect_offerer = self
            .death
            .as_ref()
            .and_then(|d| d.resurrect.as_ref())
            .filter(|o| wrathplus_protocol::guid::is_player(o.caster))
            .and_then(|o| self.names.as_ref()?.resolve(o.caster, &self.commands))
            .map(str::to_string);
        if let Some(t) = self.totems.as_ref() {
            for (out, slot) in state.totems.iter_mut().zip(t.0.iter()) {
                *out = slot.map_or(0, |s| s.guid);
            }
        }
        state.instance_difficulty = self.lock.as_ref().and_then(|l| l.difficulty).unwrap_or(0);
        state.ignore_guids = self
            .social
            .as_ref()
            .map(|s| s.ignores.clone())
            .unwrap_or_default();
    }
}

fn feed_char_state(
    script: Option<NonSendMut<UiScript>>,
    world: CharSources,
    session: SessionSources,
) {
    let Some(mut script) = script else {
        return;
    };
    let mut state = world.player_state();
    session.fill(&mut state);
    script.set_wrath_char_state(state);
    let guids = script.pushed_unit_guids();
    script.set_wrath_unit_extras(world.unit_extras(guids));
}

/// `HearthAndResurrectFromArea()`'s `CMSG_HEARTH_AND_RESURRECT`, one per call.
fn drain_char_state(script: Option<NonSendMut<UiScript>>, commands: Res<NetCommands>) {
    let Some(mut script) = script else {
        return;
    };
    for _ in 0..script.take_wrath_hearth_and_resurrect() {
        let _ = commands.0.send(ClientCommand::NativeDeath(
            wrathplus_protocol::wrath_death::Command::HearthAndResurrect,
        ));
    }
}

/// The character-state feed and its drain.
pub struct UiCharStatePlugin;

impl Plugin for UiCharStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                feed_char_state
                    .in_set(UiFeed)
                    .after(crate::app_core::schedule::UnitFeed)
                    // The ignore list `IsIgnored` reads lands first.
                    .after(crate::ui_social::feed::feed_social),
                drain_char_state.after(UiInput),
            ),
        );
    }
}

#[cfg(test)]
#[path = "ui_char_state_tests.rs"]
mod tests;
