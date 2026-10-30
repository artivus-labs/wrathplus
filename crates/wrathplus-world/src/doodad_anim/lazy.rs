//! The lazy palette-rig lane: a placed doodad claims its skin-palette slot at its first draw-gate
//! wake, not at spawn, keeps it while parked, and gives it back only under table pressure. An
//! emitter-only host has no skinned parts, gets no [`LazyRig`] and never takes a slot.

use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::mesh::MeshTag;
use bevy::prelude::*;

use crate::rig_palette::{RigPalettes, RigSkin};

use super::DoodadAnimHost;

/// Reaping starts below this slot headroom: room for the eager lanes and a stream-in burst.
pub(crate) const REAP_LOW_WATER: usize = 256;

/// Demotions per frame under pressure: enough to outpace a stream-in without a one-frame spike.
const REAP_PER_FRAME: usize = 64;

/// Seconds a host must be parked before it is reaped, so a camera swing and back does not churn.
const REAP_MIN_PARKED_SECS: f32 = 2.0;

/// On the anim-host root of a placement with skinned parts: what a wake's allocation needs.
#[derive(Component)]
pub(crate) struct LazyRig {
    /// The skeleton's bone count, the allocation size.
    pub(crate) bones: u32,
    pub(crate) ibp: Handle<SkinnedMeshInverseBindposes>,
    /// The [`SkinnedTwin`] part entities the promote and demote swap.
    pub(crate) parts: Vec<Entity>,
}

/// A part's static and skinned meshes, swapped by promote and demote and kept resident here.
#[derive(Component)]
pub(crate) struct SkinnedTwin {
    pub(crate) skinned: Handle<Mesh>,
    pub(crate) stat: Handle<Mesh>,
}

/// The part query both edges rewrite; a part despawned mid-frame is skipped, its host with it.
pub(super) type TwinParts<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Mesh3d,
        &'static mut MeshTag,
        &'static SkinnedTwin,
    ),
>;

/// The wake edge: allocate the slot, seed its rows from the pose buffer, then swap the parts, so
/// the first skinned frame shows the pose the static mesh did. Returns whether the host rigged.
pub(super) fn promote_lazy_rig(
    commands: &mut Commands,
    palettes: &mut RigPalettes,
    ibps: &Assets<SkinnedMeshInverseBindposes>,
    worlds: &Query<&GlobalTransform>,
    root: Entity,
    lazy: &LazyRig,
    pose: Option<&crate::rig_anim::RigPose>,
    parts: &mut TwinParts,
) -> bool {
    let Some(rig) = RigSkin::allocate_bones(palettes, lazy.bones, lazy.ibp.clone()) else {
        return false; // table full: the gate retries while drawn
    };
    let slot = rig.slot;
    if let (Some(ibp), Some(pose)) = (ibps.get(&lazy.ibp), pose) {
        // Rig-relative, from the root's propagated world, as the world pass composes.
        let root_g = worlds.get(root).copied().unwrap_or_default();
        crate::rig_anim::seed_rig_rows(pose, root_g, &rig, ibp, palettes);
    }
    // Liveness-checked at apply: only `RigSkin`'s `on_replace` hook frees the slot, so an insert
    // (or a failed `try_insert`) racing this frame's tile-unload despawn would leak it.
    commands.queue(
        move |world: &mut bevy::ecs::world::World| match world.get_entity_mut(root) {
            Ok(mut e) => {
                e.insert(rig);
            }
            Err(_) => {
                let slot = rig.slot;
                world.resource_mut::<RigPalettes>().free(slot);
            }
        },
    );
    for &part in &lazy.parts {
        let Ok((mut mesh, mut tag, twin)) = parts.get_mut(part) else {
            continue;
        };
        mesh.0 = twin.skinned.clone();
        tag.0 = crate::mesh_tag::with_rig(tag.0, slot);
    }
    true
}

/// The promote's inverse; removing the [`RigSkin`] frees the slot through its hook.
fn demote_lazy_rig(commands: &mut Commands, root: Entity, lazy: &LazyRig, parts: &mut TwinParts) {
    for &part in &lazy.parts {
        let Ok((mut mesh, mut tag, twin)) = parts.get_mut(part) else {
            continue;
        };
        mesh.0 = twin.stat.clone();
        tag.0 = crate::mesh_tag::with_rig(tag.0, 0);
    }
    // A host despawned this frame already freed its slot; `entity(root)` would panic.
    commands.queue(move |world: &mut bevy::ecs::world::World| {
        if let Ok(mut e) = world.get_entity_mut(root) {
            e.remove::<RigSkin>();
        }
    });
}

/// Under [`REAP_LOW_WATER`] headroom, demote up to [`REAP_PER_FRAME`] parked hosts, oldest first;
/// only hosts this lane promoted, never the eager lanes.
pub(super) fn reap_parked_rigs(
    time: Res<Time>,
    palettes: Res<RigPalettes>,
    hosts: Query<(Entity, &DoodadAnimHost, &LazyRig), With<RigSkin>>,
    mut parts: TwinParts,
    mut commands: Commands,
) {
    if palettes.slot_headroom() >= REAP_LOW_WATER {
        return;
    }
    let now = time.elapsed_secs();
    let mut parked: Vec<(f32, Entity)> = hosts
        .iter()
        .filter(|(_, host, _)| !host.active && now - host.parked_at >= REAP_MIN_PARKED_SECS)
        .map(|(root, host, _)| (host.parked_at, root))
        .collect();
    parked.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, root) in parked.iter().take(REAP_PER_FRAME) {
        let Ok((_, _, lazy)) = hosts.get(root) else {
            continue;
        };
        demote_lazy_rig(&mut commands, root, lazy, &mut parts);
        // Headroom moves only when the queued removals apply, so the batch size is the cap.
    }
}

#[cfg(test)]
mod tests;

/// Bevy's `calculate_bounds` recomputes a bound whenever `Mesh3d` changes, so an animated
/// placement's authored all-animation bound survives the lazy rig's twin swap only under
/// `NoAutoAabb`; these tests pin that Bevy contract.
#[cfg(test)]
mod bound_survives_the_twin_swap;
