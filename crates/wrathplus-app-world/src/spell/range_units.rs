//! The units `GetMinMaxRange` (`0x7ff480`) reads, built from the world: the caster, a target unit,
//! and the caster's auto-attack target, each as its combat reach, player bit and motion
//! ([`wrathplus_formats::RangeUnit`]).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use wrathplus_formats::{RangeUnit, UnitMotion};
use wrathplus_protocol::types::ObjectType;
use wrathplus_protocol::MoveSpeeds;

use crate::app_core::movement::move_flags;
use crate::app_core::player::Player;
use crate::app_core::session::Embodied;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::creature_anim::select::unify;
use crate::creature_anim::Engaged;
use crate::motion::CreatureSwimming;
use crate::motion::RemoteMotion;
use crate::motion::Spline;
use crate::motion::UnitMoveModes;
use crate::net::{current_speed, GuidIndex, UnitSpeeds};

/// The descriptor's default combat reach, before a unit's store streams.
pub const DEFAULT_REACH: f32 = 1.5;

/// One unit's motion as the moving test reads it: `flags` is the movement word, and the current
/// speed is `CMovement::GetCurrentSpeed` (`0x987570`), [`speed_12340`] over the unit's speeds and
/// spline. A unit with no speeds streamed reads no current speed here, so it is not running; every
/// unit's create block applies its speeds, so none is seen that way.
pub(crate) fn unit_motion(
    flags: u32,
    speeds: Option<&MoveSpeeds>,
    spline: Option<&Spline>,
) -> UnitMotion {
    let Some(speeds) = speeds else {
        return UnitMotion {
            flags,
            ..UnitMotion::default()
        };
    };
    let speed = speed_12340(speeds, flags, spline.map(spline_speed));
    UnitMotion {
        flags,
        speed,
        walk_speed: speeds.walk,
    }
}

/// What a unit's range inputs are read from: its descriptor, and the state its movement flags and
/// speeds come from ([`unify`], [`UnitSpeeds`]).
type UnitParts = (
    &'static ObjectStore,
    Option<&'static RemoteMotion>,
    Option<&'static Spline>,
    Option<&'static UnitMoveModes>,
    Option<&'static UnitSpeeds>,
    Has<CreatureSwimming>,
    Has<Embodied>,
);

/// The reader of the range units: any streamed unit, our own caster, and a unit's auto-attack
/// target.
#[derive(SystemParam)]
pub struct RangeUnits<'w, 's> {
    items: Option<Res<'w, crate::items::Items>>,
    units: Query<'w, 's, UnitParts>,
    caster: Query<'w, 's, Entity, With<SelfPlayer>>,
    /// The auto-attack target guid, build-12340 `[unit+0xa20/+0xa24]`.
    engaged: Query<'w, 's, &'static Engaged>,
    index: Option<Res<'w, GuidIndex>>,
    /// The flags word of the body we drive, [`Player::move_flags`].
    player: Option<Res<'w, Player>>,
}

impl RangeUnits<'_, '_> {
    /// Equipped ranged weapon's item-query range scale (`12340 0x7ff480`).
    pub(crate) fn ranged_mod(&self) -> Option<f32> {
        let caster = self.caster.iter().next()?;
        let store = self.units.get(caster).ok()?.0;
        let guid = store.0.player_inv_slot(17)?;
        let entity = *self.index.as_ref()?.0.get(&guid)?;
        let item = self.units.get(entity).ok()?.0;
        let entry = item.0.object_entry()?;
        Some(
            self.items
                .as_ref()?
                .template_cached(entry)?
                .ranged_mod_range,
        )
    }

    /// The unit `entity` is, `None` before its descriptor streams. The body we drive reads the
    /// word our controller builds, one answer for the wire and the local gates; any other unit's
    /// is its relayed or spline-implied flags with the modes the server granted.
    pub fn unit(&self, entity: Entity) -> Option<RangeUnit> {
        let (store, remote, spline, modes, speeds, swimming, driven) =
            self.units.get(entity).ok()?;
        if !matches!(
            store.0.object_type(),
            Some(ObjectType::Unit | ObjectType::Player)
        ) {
            return None;
        }
        let flags = if driven {
            self.player.as_deref().map_or(0, Player::move_flags)
        } else {
            unify(None, remote, spline, swimming, modes).flags
        };
        Some(RangeUnit {
            reach: store.0.unit_combat_reach(),
            player: store.0.object_type() == Some(ObjectType::Player),
            motion: unit_motion(flags, speeds.map(|s| &s.0), spline),
        })
    }

    /// The caster: our own player, at the default reach and standing until its descriptor streams.
    pub fn caster(&self) -> RangeUnit {
        self.caster
            .iter()
            .next()
            .and_then(|e| self.unit(e))
            .unwrap_or(RangeUnit::still(DEFAULT_REACH))
    }

    /// The auto-attack target of `entity`, the unit `GetMinMaxRange` looks up itself for its melee
    /// arm (build 12340 `0x7ff502`–`0x7ff559`). `None` with no target engaged or before it streams, where the
    /// arm reads the caster's own reach (same arm).
    pub fn attack_target(&self, entity: Entity) -> Option<RangeUnit> {
        let guid = self.engaged.get(entity).ok()?.0;
        let target = *self.index.as_ref()?.0.get(&guid)?;
        self.unit(target)
    }

    /// The auto-attack target of our own player.
    pub fn caster_attack_target(&self) -> Option<RangeUnit> {
        self.attack_target(self.caster.iter().next()?)
    }

    /// Build 12340's `GetUnitSpeed` (`0x613290`) and `GetUnitPitch` (`0x613330`) reads of `entity`:
    /// [`speed_12340`] over its movement word, speeds and spline, and its pitch (the mover pitch
    /// for the body we drive, the reported pitch for any other). `None` before its descriptor.
    pub fn speed_and_pitch(&self, entity: Entity) -> Option<(f32, f32)> {
        let (_, remote, spline, modes, speeds, swimming, driven) = self.units.get(entity).ok()?;
        let (flags, pitch) = if driven {
            let player = self.player.as_deref();
            (
                player.map_or(0, Player::move_flags),
                player.map_or(0.0, Player::mover_pitch),
            )
        } else {
            let flags = unify(None, remote, spline, swimming, modes).flags;
            (flags, remote.map_or(0.0, |r| r.pitch))
        };
        let spline_speed = spline.map(spline_speed);
        let speed = speeds.map_or(0.0, |s| speed_12340(&s.0, flags, spline_speed));
        Some((speed, pitch))
    }
}

/// A spline's speed as `0x987570` reads it: its length over its duration, 0 for a zero duration
/// (`0x98759b`).
fn spline_speed(spline: &Spline) -> f32 {
    if spline.duration.is_zero() {
        0.0
    } else {
        spline.speed()
    }
}

/// `CMovement`'s current speed as build 12340 refreshes it (`0x987570`, into `+0x8c`): 0 without a
/// direction or vertical bit (`0xc0000f`); a live spline's length over its duration (`0x987584`);
/// else [`current_speed`]'s flight, swim, walk or run arm.
pub fn speed_12340(s: &MoveSpeeds, flags: u32, spline_speed: Option<f32>) -> f32 {
    use move_flags as f;
    match spline_speed {
        Some(speed) if flags & (f::ANY_MOVE | f::ASCENDING | f::DESCENDING) != 0 => speed,
        _ => current_speed(s, flags),
    }
}

#[cfg(test)]
#[path = "range_units_tests.rs"]
mod tests;
