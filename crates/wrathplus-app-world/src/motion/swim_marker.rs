//! Native depth law plus the explicitly approved AC server-seat interoperability exception.
use super::*;
use crate::app_core::movement::swim_enter_depth;
use crate::app_core::session::ObjectStore;
use crate::app_core::state::CollisionHeight;

/// A stamped unit re-enters [`mark_swimming_creatures`] only when its `Transform` or its
/// descriptors or collision height change (`0x730d90` reads the live height); an unstamped one
/// re-enters every frame. Live splines and marked swimmers revisit support/state each frame,
/// including path removal; the mark itself keeps a formerly splined player eligible to exit.
pub(super) type SwimMarkGate = Or<(
    Changed<Transform>,
    Changed<ObjectStore>,
    Changed<CollisionHeight>,
    Changed<Spline>,
    Changed<crate::motion::UnitMoveModes>,
    With<Spline>,
    With<CreatureSwimming>,
    Without<SwimEvaluated>,
)>;

type SwimMarkQuery = (
    Entity,
    &'static NetEntity,
    &'static Transform,
    Option<&'static CollisionHeight>,
    Option<&'static ObjectStore>,
    Has<CreatureSwimming>,
    Has<SwimEvaluated>,
    // [`ground_derived`]'s other inputs: the mark and the clamp must walk the same population.
    Option<&'static Spline>,
    Has<GroundClamped>,
    Has<crate::app_core::session::Embodied>,
    Has<crate::motion::RemoteMotion>,
    Option<&'static crate::motion::UnitMoveModes>,
);

/// The stamp [`mark_swimming_creatures`] leaves once a unit's room claim settles; 1.12 water is
/// static, so it holds until the unit moves or the `ColliderEpoch` bumps.
#[derive(Component)]
pub(in crate::motion) struct SwimEvaluated;

/// The exit band (yd) below the swim boundary: the player boundary's 1/36-yd hysteresis
/// (`0x7ff9d0`), an absolute distance, not scaled with collision height.
pub(super) const CREATURE_SWIM_EXIT_BAND: f32 = 1.0 / 36.0;

/// 12340's `0x730d10` decision for one creature:
///
/// ```text
/// enter iff  flags ∧ depth >  0.75·h              (0.75·h < depth, strict)
/// stay  iff  flags ∧ depth >= 0.75·h − 1/36       (the stop takes depth < 0.75·h − 1/36)
/// ```
///
/// A false flag term forces a stop at any depth (the `0x721290` arm), not only a refused start.
pub(in crate::motion) fn creature_swim_state(
    marked: bool,
    depth: f32,
    boundary: f32,
    unit_flags: u32,
) -> bool {
    if !crate::app_core::movement::may_swim(unit_flags) {
        return false; // both legs: without a permit a unit can neither start nor stay
    }
    if marked {
        depth >= boundary - CREATURE_SWIM_EXIT_BAND
    } else {
        depth > boundary
    }
}

/// Maintains [`CreatureSwimming`] on every [`ground_derived`] body from the water over its feet.
/// Chained before [`ground_clamp_creatures`] so a fresh mark exempts the clamp the same frame.
pub(in crate::motion) fn mark_swimming_creatures(
    mut commands: Commands,
    support: wrathplus_world::collision::WorldCollision,
    units: Query<SwimMarkQuery, SwimMarkGate>,
    stamped: Query<Entity, With<SwimEvaluated>>,
    world: wrathplus_world::world_point::WorldPoint,
    epoch: Res<wrathplus_world::collision::ColliderEpoch>,
    mut last_epoch: Local<Option<u64>>,
) {
    // A collider epoch change (liquid streams with the colliders) drops every stamp.
    let now = epoch.get();
    if last_epoch.replace(now) != Some(now) {
        for e in &stamped {
            commands.entity(e).remove::<SwimEvaluated>();
        }
    }
    for (
        e,
        net,
        t,
        collision,
        store,
        marked,
        evaluated,
        spline,
        clamped,
        embodied,
        relayed,
        modes,
    ) in &units
    {
        if !ground_derived(
            net.kind,
            spline.is_some(),
            clamped || marked,
            embodied,
            relayed,
        ) || relayed
        {
            continue; // GameObjects don't swim; a relayed player's flag arrives on the wire
        }
        // The unit's own room decides whose liquid answers (an interior's, not the ADT water over
        // it). An unsettled claim may not enter swim, but a stale mark may always clear.
        let who = wrathplus_world::world_point::Subject::Unit(e);
        if !marked && !world.room_settled(who) {
            continue; // unstamped, so retried until the claim lands
        }
        if !evaluated {
            commands.entity(e).insert(SwimEvaluated);
        }
        let wow = bevy_to_wow(t.translation);
        let depth = world
            .water_surface_at(who, wow)
            .map_or(f32::MIN, |s| s - wow[2]);
        let boundary = swim_enter_depth(collision.copied().unwrap_or_default().0);
        // Flags read 0 until the descriptor block lands; `Changed<ObjectStore>` re-asks then.
        let flags = store.map_or(0, |s| s.0.unit_flags());
        let swimming = creature_swim_state(marked, depth, boundary, flags)
            || (crate::app_core::movement::may_swim(flags)
                && world.room_settled(who)
                && collision
                    .is_some_and(|h| ac_surface_swimming(&support, t, spline, modes, h.0, depth)));
        if swimming != marked {
            if swimming {
                commands.entity(e).insert(CreatureSwimming);
            } else {
                commands.entity(e).remove::<CreatureSwimming>();
            }
        }
    }
}

/// Deliberate AC interoperability deviation from 12340's feet-only 0.75*h law.
/// AC `GetMinHeightInWater` seats at (1 - 4*1040/10202)*h without necessarily
/// broadcasting SWIMMING for playerbots. Admit ONLY that surface seat on a live ground
/// spline above measured deep support; never infer depth from missing collision.
/// See docs/WRATH-AC-SURFACE-SWIM.md for provenance, authority and numeric bounds.
fn ac_surface_swimming(
    collision: &wrathplus_world::collision::WorldCollision,
    t: &Transform,
    spline: Option<&Spline>,
    modes: Option<&crate::motion::UnitMoveModes>,
    height: f32,
    depth: f32,
) -> bool {
    const AC_IMMERSED_FRAC: f32 = 1.0 - 4.0 * 1040.0 / 10202.0;
    if !spline.is_some_and(|s| s.grounded && s.deck.is_none())
        || modes.is_some_and(|m| m.water_walking() || m.hovering())
        || !height.is_finite()
        || height <= 0.0
        || !depth.is_finite()
        || (depth - AC_IMMERSED_FRAC * height).abs() > CREATURE_SWIM_EXIT_BAND
    {
        return false;
    }
    let origin = t.translation + Vec3::Y * IDLE_UP_BAND;
    collision
        .ray_body(origin, Dir3::NEG_Y, IDLE_UP_BAND + GROUND_CLAMP_DOWN)
        .is_some_and(|hit| {
            let gap = hit.distance - IDLE_UP_BAND;
            hit.normal.y > 0.5 && gap > IDLE_UP_BAND && depth + gap > swim_enter_depth(height)
        })
}
