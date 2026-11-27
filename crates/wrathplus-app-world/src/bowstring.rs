//! The engine-drawn bowstring: bow M2s have no string geometry. The reference's per-frame callback
//! (`0x611ff0`) draws a two-segment line between the `$WTT`/`$WTB` limb-tip markers, its middle
//! vertex at the HandArrow attach while the nock latch (`[+0xd58] & 0x4000`) holds, else the tip
//! midpoint. Drawn here as gizmo lines; the color and width are inferred, since the reference's
//! packed vertex color is not decoded.
//!
//! The tips are posed: `$BWP` plays BowPull (160) on the prop and `$BWR` returns it to Stand (0),
//! so the markers ride limb bones that bend through the draw.

use bevy::prelude::*;

use crate::app_core::anim::NockLatch;
use crate::entities::BoneAttach;

/// The HandArrow attach id (35, `0x6121b8`): the string's middle point while nocked.
const HAND_ARROW: u16 = 0x23;

/// Marks a bow prop root whose model authors the `$WTT`/`$WTB` anchors; despawned with the prop.
#[derive(Component)]
pub(crate) struct Bowstring {
    /// The unit wearing the bow, which carries the HandArrow attach and the nock latch.
    pub(crate) owner: Entity,
    /// The `$WTT` top and `$WTB` bottom anchors as `(bone, model-local offset)`.
    pub(crate) top: (u16, Vec3),
    pub(crate) bottom: (u16, Vec3),
    /// The same anchors as offsets from their bones' bind pivots, `[top, bottom]`, for the flexed
    /// pose: a `RigPose` bone frame sits at the bone's pivot (as for every `BoneAttach` marker),
    /// so posing the model-local point adds the pivot twice and flings the tips yards away.
    pub(crate) from_pivot: [Vec3; 2],
}

impl Bowstring {
    /// The string of `owner`'s bow, from the model's `$WTT`/`$WTB` anchors and its baked event
    /// markers, which hold the same points relative to their bone pivots. Both come from the same
    /// event records, so a missing marker cannot happen; the model-local point stands in if it did.
    pub(crate) fn new(
        owner: Entity,
        top: (u16, Vec3),
        bottom: (u16, Vec3),
        markers: &[wrathplus_assets::ModelMarker],
    ) -> Self {
        let from_pivot = |ident: [u8; 4], (_, point): (u16, Vec3)| {
            markers
                .iter()
                .find(|m| m.ident == ident)
                .map_or(point, |m| m.offset)
        };
        Self {
            owner,
            top,
            bottom,
            from_pivot: [from_pivot(*b"$WTT", top), from_pivot(*b"$WTB", bottom)],
        }
    }
}

/// Draws every visible bow's string, tip to middle to tip, from this frame's joint frames.
fn draw_bowstrings(
    bows: Query<(
        &Bowstring,
        &GlobalTransform,
        &InheritedVisibility,
        Option<&wrathplus_world::rig_anim::RigPose>,
    )>,
    owners: Query<(
        &BoneAttach,
        &wrathplus_world::rig_anim::RigPose,
        Has<NockLatch>,
    )>,
    joints: Query<&GlobalTransform>,
    mut gizmos: Gizmos,
) {
    // Inferred: the reference's packed vertex color is not decoded.
    const STRING_COLOR: Color = Color::srgb(0.12, 0.10, 0.08);
    for (bs, prop, vis, flex) in &bows {
        if !vis.get() {
            continue;
        }
        // The prop's own pose when it flexes (`joints_root` is this entity); its rigid frame
        // otherwise.
        let tip = |(bone, offset): (u16, Vec3), from_pivot: Vec3| {
            flex.and_then(|p| p.posed_point(prop, bone, from_pivot))
                .unwrap_or_else(|| prop.transform_point(offset))
        };
        let top = tip(bs.top, bs.from_pivot[0]);
        let bottom = tip(bs.bottom, bs.from_pivot[1]);
        let middle = owners
            .get(bs.owner)
            .ok()
            .filter(|(_, _, latched)| *latched)
            .and_then(|(bones, pose, _)| {
                let &(bone, offset) = bones.points.get(&HAND_ARROW)?;
                pose.posed_point(joints.get(pose.joints_root).ok()?, bone, offset)
            })
            .unwrap_or_else(|| (top + bottom) / 2.0);
        gizmos.line(top, middle, STRING_COLOR);
        gizmos.line(middle, bottom, STRING_COLOR);
    }
}

/// Registers the string drawer after the palette pass, beside the other joint-frame readers.
pub struct BowstringPlugin;

impl Plugin for BowstringPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            draw_bowstrings.in_set(wrathplus_world::billboard::BillboardPlace),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_assets::ModelMarker;

    /// The flexed pose takes the tips from the baked markers (pivot-relative), not the model-local
    /// anchors, which a bone frame at its pivot would offset by the pivot a second time.
    #[test]
    fn the_flexed_tips_come_from_the_pivot_relative_markers() {
        let marker = |ident: [u8; 4], bone: u16, offset: Vec3| ModelMarker {
            ident,
            bone,
            offset,
        };
        let markers = [
            marker(*b"$CSL", 1, Vec3::X),
            marker(*b"$WTT", 7, Vec3::new(0.0, 0.1, 0.0)),
            marker(*b"$WTB", 9, Vec3::new(0.0, -0.1, 0.0)),
        ];
        let top = (7, Vec3::new(0.2, 0.9, 0.0));
        let bottom = (9, Vec3::new(0.2, -0.9, 0.0));
        let string = Bowstring::new(Entity::PLACEHOLDER, top, bottom, &markers);
        assert_eq!(
            string.from_pivot,
            [Vec3::new(0.0, 0.1, 0.0), Vec3::new(0.0, -0.1, 0.0)]
        );
        // The rigid frame keeps the model-local points.
        assert_eq!((string.top, string.bottom), (top, bottom));
        // A model without the markers falls back to the model-local points.
        let bare = Bowstring::new(Entity::PLACEHOLDER, top, bottom, &[]);
        assert_eq!(bare.from_pivot, [top.1, bottom.1]);
    }
}
