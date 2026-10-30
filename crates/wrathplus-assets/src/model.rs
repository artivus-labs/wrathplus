//! What the M2 and WMO loaders share: the per-batch submesh and the WoW-to-Bevy bakes of its mesh,
//! skeleton, attachments and animation clips. Materials are built at the spawn site.

use bevy::animation::animatable::Animatable;
use bevy::animation::animation_curves::{AnimatableCurve, AnimatableKeyframeCurve};
use bevy::animation::{animated_field, AnimationClip, AnimationTargetId};
use bevy::asset::RenderAssetUsages;
use bevy::math::Mat3;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use wrathplus_formats::{
    BillboardKind, BoneScaleAnim, CharSkinSlot, M2Attachment, ModelAnimation, ModelBlend,
    RenderSubmesh, Skeleton,
};

use crate::coords::wow_to_bevy;

mod anims;
mod pose;
pub use anims::{AnimClip, ClipEvent, ModelAnimations};
pub use pose::{PoseBone, PoseClip, PoseNode, PoseSource, PoseTrack};

/// A billboarded batch's card in Bevy space; its mesh is centred on `pivot` (model-local).
#[derive(Clone)]
pub struct BillboardInfo {
    pub pivot: Vec3,
    /// The billboard bone, the joint the card rides on an animated host.
    pub bone: u16,
    pub kind: BillboardKind,
    /// The bone's global-sequence scale loop (a glow card's pulse), folded into the card's scale.
    pub scale_anim: Option<BoneScaleAnim>,
    /// The bone's per-sequence translation loops `(anim id, loop)`, keys in Bevy axes: the quest
    /// marker bobs low on anim 0, and higher on 190 while its unit shows an overhead name.
    pub seq_translations: Vec<(u16, BoneScaleAnim)>,
}

/// One render batch of an M2 or a WMO group: decoded geometry, texture and draw state, but no
/// `Mesh` asset, since a labeled mesh sub-asset uploads the whole model in one frame. The app
/// builds meshes paced, through [`submesh_to_static_mesh`] and [`submesh_to_skinned_mesh`].
#[derive(Clone)]
pub struct ModelSubmesh {
    /// The decoded geometry in WoW model space, the model's one resident CPU copy of its vertices.
    pub geometry: std::sync::Arc<RenderSubmesh>,
    /// The embedded albedo texture; `None` for a creature skin slot, filled at spawn.
    pub texture: Option<Handle<Image>>,
    pub texture2: Option<Handle<Image>>,
    /// The creature skin variation this batch draws from (`0/1/2` for `Monster1/2/3`).
    pub skin_slot: Option<u8>,
    /// The geoset id, `group * 100 + variant`, by which the character compositor picks parts.
    pub geoset_id: u16,
    /// The character texture slot the spawn site fills from the player's appearance.
    pub char_slot: Option<CharSkinSlot>,
    pub blend: ModelBlend,
    pub two_sided: bool,
    /// A WMO interior group's batch, lit by its baked MOCV with the sun off.
    pub interior: bool,
    /// Unlit, drawn fullbright: M2 render flag `UNLIT` (0x01), or WMO `UNLIT` on an exterior group.
    pub emissive: bool,
    /// The icon slot (M2 texture type 14), filled from `Model:ReplaceIconTexture`'s path.
    pub icon_slot: bool,
    /// The WMO MOMT SIDN night-glow colour, ramped by the night fraction.
    pub sidn: Option<[u8; 3]>,
    /// MOHD ambient RGB when the WMO uses unified render lighting.
    pub wmo_unified: Option<[u8; 3]>,
    /// WMO MOMT WINDOW: an interior batch lit by the Direct/Ambient midpoint.
    pub window: bool,
    /// Blends additively (M2 glow cards and coronae).
    pub additive: bool,
    /// Sphere-mapped: `wow_model.wgsl` derives the UV from the view-space reflection.
    pub env_map: bool,
    /// M2 render flag 0x10; unset, a transparent batch writes depth, as in the reference.
    pub no_depth_write: bool,
    /// M2 render flag 0x08: no depth test.
    pub no_depth_test: bool,
    /// The batch's fog colour policy; `Scene` for WMO.
    pub fog_policy: wrathplus_formats::FogPolicy,
    /// The receiver sphere the point-light gather sweeps (`0x81e400`): the M2's bounds centre
    /// (Bevy model space) and bounding radius; zero on WMO surfaces, which take no point light.
    pub light_sphere: bevy::math::Vec4,
    /// Set when the batch rides an M2 billboard bone; its mesh is built centred on the pivot.
    pub billboard: Option<BillboardInfo>,
    /// The animated material alpha, colour alpha times transparency weight (`0x707680`).
    pub alpha_anim: Option<std::sync::Arc<wrathplus_formats::AlphaAnim>>,
    /// The texture transform's translation loop (sampled at `0x714260`, applied at `0x70b740`).
    /// The `Arc` is also the material-dedup identity, shared by every instance.
    pub uv_anim: Option<std::sync::Arc<wrathplus_formats::UvAnim>>,
    /// The UV loop per file sequence slot, `Some` only where slots disagree; also a dedup identity.
    pub uv_seq: Option<std::sync::Arc<wrathplus_formats::SeqLoops<[f32; 2]>>>,
    /// The texture-transform rotation loop per file sequence slot, read only by the UI model tiles.
    pub uv_rot_seq: Option<std::sync::Arc<wrathplus_formats::SeqLoops<[f32; 4]>>>,
    /// The scale loop per file sequence slot, likewise.
    pub uv_scale_seq: Option<std::sync::Arc<wrathplus_formats::SeqLoops<[f32; 2]>>>,
    /// The time-varying M2Color tint; when `Some`, the material carries it, not the vertex colour.
    pub rgb_anim: Option<std::sync::Arc<wrathplus_formats::RgbAnim>>,
    /// The tint loop per file sequence slot, on [`Self::uv_seq`]'s rule.
    pub rgb_seq: Option<std::sync::Arc<wrathplus_formats::SeqLoops<[f32; 3]>>>,
    /// The WMO batch's MOBA section, which picks an interior batch's lighting (INT, TRANS, EXT).
    pub wmo_batch: Option<wrathplus_formats::WmoBatchClass>,
    /// A flat ground quad (WoW model space), which the ground-fx lane redraws as a projected decal.
    pub ground_quad: Option<wrathplus_formats::GroundQuad>,
}

/// Bake a [`RenderSubmesh`] to a Bevy-space [`Mesh`]: authored normals, else flat ones.
fn build_submesh_mesh(sub: &RenderSubmesh, usages: RenderAssetUsages) -> Mesh {
    // A billboard batch is centred on its bone pivot, so the spawn site turns it in place.
    let center = sub
        .billboard
        .as_ref()
        .map_or(Vec3::ZERO, |b| wow_to_bevy(b.pivot));
    let positions: Vec<[f32; 3]> = sub
        .positions
        .iter()
        .map(|p| (wow_to_bevy(*p) - center).to_array())
        .collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, usages);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, sub.uvs.clone());
    // Always a second UV set, UV0 again without a second stage: the shader reads it only under
    // `stage2`, and one vertex layout per family means the pipeline warm pass covers both.
    let uvs2 = if sub.uvs2.len() == sub.positions.len() {
        &sub.uvs2
    } else {
        &sub.uvs
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs2.clone());
    mesh.insert_indices(Indices::U32(sub.indices.clone()));
    if sub.vertex_colors.len() == sub.positions.len() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, sub.vertex_colors.clone());
    }
    if sub.normals.len() == sub.positions.len() {
        // Deviation: a card authored facing away from the viewer is lit on the face it presents,
        // because the reference shades it inverted and camera-dependent. Flip its normals, never
        // its winding, which would show the single-sided ones the reference culls at every angle.
        let flip = sub.billboard_card_faces_away();
        let normals: Vec<[f32; 3]> = sub
            .normals
            .iter()
            .map(|n| {
                let b = wow_to_bevy(*n);
                if flip { -b } else { b }.to_array()
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    } else {
        mesh.compute_normals();
    }
    mesh
}

/// The skinned mesh's per-vertex M2 bone indices, at shader location 10. Not Bevy's
/// `ATTRIBUTE_JOINT_INDEX`, whose presence in a layout forces Bevy's own skinning: this id makes
/// `WowModelExt::specialize` compile the `WOW_RIG_SKIN` path, which reads our own palette.
pub const ATTRIBUTE_WOW_JOINT_INDEX: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Wow_JointIndex",
        988_540_917,
        bevy::render::render_resource::VertexFormat::Uint16x4,
    );
/// The skinned mesh's per-vertex joint weights, normalized, at shader location 11.
pub const ATTRIBUTE_WOW_JOINT_WEIGHT: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Wow_JointWeight",
        988_540_918,
        bevy::render::render_resource::VertexFormat::Float32x4,
    );

/// A merged blob's per-vertex fade sphere (world centre, fade radius), the same on every vertex
/// of one placement, from which `wow_model.wgsl` fades each placement. Shader location 12.
pub const ATTRIBUTE_WOW_FADE_SPHERE: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Wow_FadeSphere",
        988_540_919,
        bevy::render::render_resource::VertexFormat::Float32x4,
    );

/// A merged interior-prop blob's per-vertex SH-probe slot, in place of the `MeshTag` payload.
/// Shader location 13.
pub const ATTRIBUTE_WOW_MERGED_SLOT: bevy::mesh::MeshVertexAttribute =
    bevy::mesh::MeshVertexAttribute::new(
        "Wow_MergedSlot",
        988_540_920,
        bevy::render::render_resource::VertexFormat::Uint32,
    );

/// The static mesh, render-world only. Pair it with an explicit `Aabb`: a render-world mesh races
/// Bevy's `calculate_bounds`, and the exterior cull draws a mesh that has no bound.
pub fn submesh_to_static_mesh(sub: &RenderSubmesh) -> Mesh {
    build_submesh_mesh(sub, RenderAssetUsages::RENDER_WORLD)
}

/// The skinned mesh: the static one plus the joint attributes. It keeps its main-world copy,
/// which the mouseover picker (`target::hover`) skins on the CPU.
pub fn submesh_to_skinned_mesh(sub: &RenderSubmesh) -> Mesh {
    let mut mesh = build_submesh_mesh(sub, RenderAssetUsages::default());
    if sub.joints.len() == sub.positions.len() && !sub.joints.is_empty() {
        mesh.insert_attribute(
            ATTRIBUTE_WOW_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(sub.joints.clone()),
        );
        mesh.insert_attribute(
            ATTRIBUTE_WOW_JOINT_WEIGHT,
            VertexAttributeValues::Float32x4(sub.weights.clone()),
        );
    }
    mesh
}

/// One rest-skeleton joint in Bevy space: its parent (`-1` for the root) and local translation.
#[derive(Clone, Copy)]
pub struct ModelJoint {
    pub parent: i16,
    pub local_translation: Vec3,
    /// The bone's billboard arm (bone flags 0x08/0x10/0x20/0x40). The reference replaces the
    /// bone's rotation in the palette, so its children inherit the facing, and arms every flagged
    /// bone: the animate kernel `0x714260` reads no vertex data, so a welded seam is no gate. The
    /// seam holds because the replacement is built about the bone's own pivot.
    pub billboard: Option<wrathplus_formats::BillboardKind>,
    /// Bone flags 0x1/0x2/0x4: how the parent matrix is rebuilt from the model's root frame,
    /// applied before the billboard arm.
    pub parent_arm: Option<wrathplus_formats::ParentArm>,
}

/// A model's rest skeleton in Bevy space, shared by every instance.
#[derive(Clone, Default)]
pub struct ModelSkeleton {
    pub joints: Vec<ModelJoint>,
    /// The KeyBoneID 4 (SpineLow) bone, which counter-twists toward the aim while the body's yaw
    /// is off it, as in a strafe (`0x711f10(4, …)` in `0x607ed0`).
    pub spine_bone: Option<u16>,
    /// The KeyBoneID 6 (Head) bone, which takes the rest of that twist (`0x711f10(6, …)`).
    pub head_bone: Option<u16>,
}

/// Bake a [`Skeleton`] to Bevy space, with its inverse bind poses. A vanilla M2 has no
/// inverse-bind array and an identity rest pose, and a bone's pivot is its bind position, so the
/// inverse bind pose is `translate(−pivot)` and a joint's rest translation `pivot − parent pivot`:
/// at rest every joint matrix is the entity transform.
pub(crate) fn build_skeleton(skel: &Skeleton) -> (ModelSkeleton, Vec<Mat4>) {
    let pivots = skeleton_pivots(skel);
    let joints = skel
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let parent_pivot = usize::try_from(b.parent)
                .ok()
                .and_then(|p| pivots.get(p).copied())
                .unwrap_or(Vec3::ZERO);
            ModelJoint {
                parent: b.parent,
                local_translation: pivots[i] - parent_pivot,
                billboard: b.billboard,
                parent_arm: b.parent_arm,
            }
        })
        .collect();
    let inverse_bindposes = pivots.iter().map(|p| Mat4::from_translation(-*p)).collect();
    // A bone carries KeyBoneID `k` iff `keyBoneLookup[k]` points at it.
    let key_bone = |k: i16| {
        skel.bones
            .iter()
            .position(|b| b.key_bone == k)
            .map(|i| i as u16)
    };
    (
        ModelSkeleton {
            joints,
            spine_bone: key_bone(4),
            head_bone: key_bone(6),
        },
        inverse_bindposes,
    )
}

/// Each bone's Bevy-space pivot, the one source for the joints and the attachment offsets.
pub(crate) fn skeleton_pivots(skel: &Skeleton) -> Vec<Vec3> {
    skel.bones.iter().map(|b| wow_to_bevy(b.pivot)).collect()
}

/// One M2 attachment point as an offset from its bone's pivot, Bevy space; about zero on
/// character models, whose attach bones sit on the point.
#[derive(Clone, Copy)]
pub struct ModelAttachment {
    pub id: u16,
    pub bone: u16,
    pub offset: Vec3,
}

/// The right and left arm subtree roots: each hand attachment's bone (1 right, 2 left) walked up
/// to its shoulder key-bone (2 or 3).
pub(crate) fn arm_subtree_roots(
    skel: &Skeleton,
    attachments: &[ModelAttachment],
) -> Option<(usize, usize)> {
    let shoulder_of = |hand_attach_id: u16| -> Option<usize> {
        let mut bone = attachments
            .iter()
            .find(|a| a.id == hand_attach_id)
            .map(|a| a.bone as usize)?;
        loop {
            let b = skel.bones.get(bone)?;
            if matches!(b.key_bone, 2 | 3) {
                return Some(bone);
            }
            bone = usize::try_from(b.parent).ok()?;
        }
    };
    Some((shoulder_of(1)?, shoulder_of(2)?))
}

/// The upper-body subtree root, the reference's `CGUnit+0xd5c` from the probe `0x60ce70`:
/// key-bone 4 (SpineLow), else 6 (Head). SpineLow's subtree leaves out the legs, which hang off
/// the pelvis. `None` means no split, and a one-shot plays full-body.
pub(crate) fn upper_subtree_root(skel: &Skeleton) -> Option<usize> {
    let bone_with = |k: i16| skel.bones.iter().position(|b| b.key_bone == k);
    bone_with(4).or_else(|| bone_with(6))
}

/// The finger key-bones per hand `(right, left)`, 8-12 and 13-17: the reference's `CloseHand`
/// targets, posed with `HandsClosed` (AnimationData 15) while that hand holds a weapon
/// (`0x479660`/`0x60b590`). Empty for a model without them.
pub(crate) fn finger_subtree_roots(skel: &Skeleton) -> [Vec<usize>; 2] {
    let roots = |lo: i16, hi: i16| -> Vec<usize> {
        skel.bones
            .iter()
            .enumerate()
            .filter(|(_, b)| (lo..=hi).contains(&b.key_bone))
            .map(|(i, _)| i)
            .collect()
    };
    [roots(8, 12), roots(13, 17)]
}

/// Whether bone `i` is `root` or one of its descendants.
pub(crate) fn in_subtree(skel: &Skeleton, i: usize, root: usize) -> bool {
    let mut cur = i;
    loop {
        if cur == root {
            return true;
        }
        match skel
            .bones
            .get(cur)
            .and_then(|b| usize::try_from(b.parent).ok())
        {
            Some(p) => cur = p,
            None => return false,
        }
    }
}

/// Bake the [`M2Attachment`] table to bone-local offsets, dropping a record with a bad bone.
pub(crate) fn build_attachments(
    attachments: &[M2Attachment],
    pivots: &[Vec3],
) -> Vec<ModelAttachment> {
    attachments
        .iter()
        .filter_map(|a| {
            let pivot = pivots.get(a.bone as usize)?;
            Some(ModelAttachment {
                id: a.id,
                bone: a.bone,
                offset: wow_to_bevy(a.position) - *pivot,
            })
        })
        .collect()
}

/// One animation-event position, baked like [`ModelAttachment`]. The reference's by-4CC queries
/// (`0x7130e0`/`0x7131b0`) take the first match: `$CSL`/`$CSR`/`$CST` hands, `$BWR` ranged release.
#[derive(Clone, Copy)]
pub struct ModelMarker {
    /// The identifier 4CC, stored forward (`*b"$CSL"`).
    pub ident: [u8; 4],
    pub bone: u16,
    pub offset: Vec3,
}

/// Bake the event markers like [`build_attachments`], in file order: queries take the first match.
pub(crate) fn build_markers(
    markers: &[wrathplus_formats::EventMarker],
    pivots: &[Vec3],
) -> Vec<ModelMarker> {
    markers
        .iter()
        .filter_map(|m| {
            let pivot = pivots.get(m.bone as usize)?;
            Some(ModelMarker {
                ident: m.ident,
                bone: m.bone,
                offset: wow_to_bevy(m.position) - *pivot,
            })
        })
        .collect()
}

/// A bone's stable [`AnimationTargetId`], shared by the clips' curves and the joint entities.
pub fn bone_target_id(bone: u16) -> AnimationTargetId {
    AnimationTargetId::from_name(&Name::new(format!("benilla_bone_{bone}")))
}

/// The WoW-to-Bevy basis change as a quaternion `r`; a rotation key converts as `r·q·r⁻¹`.
fn wow_to_bevy_quat() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(
        wow_to_bevy([1.0, 0.0, 0.0]),
        wow_to_bevy([0.0, 1.0, 0.0]),
        wow_to_bevy([0.0, 0.0, 1.0]),
    ))
}

/// A keyframe curve; one key becomes a flat curve over `[0, duration]`, as Bevy needs two.
fn keyframe_curve<T: Animatable + Clone>(
    duration: f32,
    keys: Vec<(f32, T)>,
) -> Option<AnimatableKeyframeCurve<T>> {
    match keys.len() {
        0 => None,
        1 => AnimatableKeyframeCurve::new([
            (0.0, keys[0].1.clone()),
            (duration.max(1e-3), keys[0].1.clone()),
        ])
        .ok(),
        _ => AnimatableKeyframeCurve::new(keys).ok(),
    }
}

/// The bone channels any sequence of a model keys: `[translation, rotation, scale]` by bone.
#[derive(Default)]
pub(crate) struct KeyedChannels(std::collections::BTreeMap<u16, [bool; 3]>);

impl KeyedChannels {
    pub(crate) fn of(animations: &[ModelAnimation]) -> Self {
        let mut keyed = std::collections::BTreeMap::<u16, [bool; 3]>::new();
        for bk in animations.iter().flat_map(|a| &a.bones) {
            let c = keyed.entry(bk.bone).or_default();
            c[0] |= !bk.translation.is_empty();
            c[1] |= !bk.rotation.is_empty();
            c[2] |= !bk.scale.is_empty();
        }
        Self(keyed)
    }
}

/// `anim`'s keys plus a bind-pose key on every channel another sequence keys and this one does
/// not. An M2 track with no keys in the playing sequence reads its default (no offset, identity,
/// unit scale); a Bevy channel with no curve keeps the last sequence's value instead, which left
/// the Mottled Boar's Run on Stand's root pitch and an orc's Walk on Swim's (2026-10-24).
fn with_rest_keys(
    anim: &ModelAnimation,
    keyed: &KeyedChannels,
) -> Vec<wrathplus_formats::BoneKeys> {
    let mut bones = anim.bones.clone();
    for (&bone, &[t, r, s]) in &keyed.0 {
        let i = bones
            .iter()
            .position(|b| b.bone == bone)
            .unwrap_or_else(|| {
                bones.push(wrathplus_formats::BoneKeys {
                    bone,
                    translation: Vec::new(),
                    rotation: Vec::new(),
                    scale: Vec::new(),
                });
                bones.len() - 1
            });
        let b = &mut bones[i];
        if t && b.translation.is_empty() {
            b.translation.push((0.0, [0.0; 3]));
        }
        if r && b.rotation.is_empty() {
            b.rotation.push((0.0, [0.0, 0.0, 0.0, 1.0]));
        }
        if s && b.scale.is_empty() {
            b.scale.push((0.0, [1.0; 3]));
        }
    }
    bones
}

/// Build one sequence's [`AnimationClip`] and its [`PoseClip`] in one walk. The M2 translation
/// track is a delta on the rest offset, a rotation converts as `r·q·r⁻¹`, and scale permutes axes.
/// A sequence that poses no bone still gets a clip of its own duration, since the clip is the
/// instance's sequence clock; the flag says whether any channel made a curve for a rig to pose.
/// `keyed` is [`KeyedChannels::of`] the model's sequences ([`with_rest_keys`]).
pub(crate) fn build_animation_clip(
    anim: &ModelAnimation,
    skeleton: &ModelSkeleton,
    keyed: &KeyedChannels,
) -> (AnimationClip, PoseClip, bool) {
    let r = wow_to_bevy_quat();
    let mut clip = AnimationClip::default();
    let mut pose = PoseClip::default();
    let mut any = false;
    for bk in &with_rest_keys(anim, keyed) {
        let target = bone_target_id(bk.bone);
        let rest = skeleton
            .joints
            .get(bk.bone as usize)
            .map_or(Vec3::ZERO, |j| j.local_translation);
        let trans: Vec<_> = bk
            .translation
            .iter()
            .map(|(t, v)| (*t, rest + wow_to_bevy(*v)))
            .collect();
        let pose_trans = PoseTrack::new(&trans);
        if let Some(c) = keyframe_curve(anim.duration, trans) {
            clip.add_curve_to_target(
                target,
                AnimatableCurve::new(animated_field!(Transform::translation), c),
            );
            any = true;
        }
        let rot: Vec<_> = bk
            .rotation
            .iter()
            .map(|(t, q)| {
                (
                    *t,
                    r * Quat::from_xyzw(q[0], q[1], q[2], q[3]) * r.inverse(),
                )
            })
            .collect();
        let pose_rot = PoseTrack::new(&rot);
        if let Some(c) = keyframe_curve(anim.duration, rot) {
            clip.add_curve_to_target(
                target,
                AnimatableCurve::new(animated_field!(Transform::rotation), c),
            );
            any = true;
        }
        let scale: Vec<_> = bk
            .scale
            .iter()
            .map(|(t, s)| (*t, Vec3::new(s[1], s[2], s[0])))
            .collect();
        let pose_scale = PoseTrack::new(&scale);
        if let Some(c) = keyframe_curve(anim.duration, scale) {
            clip.add_curve_to_target(
                target,
                AnimatableCurve::new(animated_field!(Transform::scale), c),
            );
            any = true;
        }
        pose.push(PoseBone {
            bone: bk.bone,
            translation: pose_trans,
            rotation: pose_rot,
            scale: pose_scale,
        });
    }
    if !any {
        // Bevy takes a clip's duration from its curves; with none, the clock would never leave 0.
        clip.set_duration(anim.duration.max(1e-3));
    }
    (clip, pose, any)
}

/// The weapon-grip overlay: one constant rotation per finger bone, played masked to one hand.
pub(crate) fn build_grip_clip(poses: &[(u16, [f32; 4])]) -> (AnimationClip, PoseClip) {
    let r = wow_to_bevy_quat();
    let mut clip = AnimationClip::default();
    let mut pose = PoseClip::default();
    for &(bone, q) in poses {
        let rot = r * Quat::from_xyzw(q[0], q[1], q[2], q[3]) * r.inverse();
        if let Some(c) = keyframe_curve(0.033, vec![(0.0, rot)]) {
            clip.add_curve_to_target(
                bone_target_id(bone),
                AnimatableCurve::new(animated_field!(Transform::rotation), c),
            );
            pose.push(PoseBone {
                bone,
                translation: PoseTrack::default(),
                rotation: PoseTrack::new(&[(0.0, rot)]),
                scale: PoseTrack::default(),
            });
        }
    }
    (clip, pose)
}

/// A global-sequence bone channel in Bevy space, a free-running loop; times in seconds.
#[derive(Clone)]
pub struct GlobalSeqChannel<T> {
    pub period: f32,
    pub keys: Vec<(f32, T)>,
}

impl<T: Copy> GlobalSeqChannel<T> {
    /// The keys around `t` mod the period, clamped at the end keys, which sit on the loop's ends.
    fn bracket(&self, t: f32) -> (T, T, f32) {
        let period = self.period.max(1e-3);
        let t = t.rem_euclid(period);
        let keys = &self.keys;
        if t <= keys[0].0 {
            return (keys[0].1, keys[0].1, 0.0);
        }
        for w in keys.windows(2) {
            if t <= w[1].0 {
                let span = (w[1].0 - w[0].0).max(1e-6);
                return (w[0].1, w[1].1, (t - w[0].0) / span);
            }
        }
        let last = keys[keys.len() - 1].1;
        (last, last, 0.0)
    }
}

impl GlobalSeqChannel<Vec3> {
    pub fn sample(&self, t: f32) -> Vec3 {
        let (a, b, f) = self.bracket(t);
        a.lerp(b, f)
    }
}

impl GlobalSeqChannel<Quat> {
    pub fn sample(&self, t: f32) -> Quat {
        let (a, b, f) = self.bracket(t);
        a.slerp(b, f)
    }
}

/// A bone's global-sequence channels; only the driven ones override the playing animation.
#[derive(Clone)]
pub struct GlobalBone {
    pub bone: u16,
    pub translation: Option<GlobalSeqChannel<Vec3>>,
    pub rotation: Option<GlobalSeqChannel<Quat>>,
    pub scale: Option<GlobalSeqChannel<Vec3>>,
}

/// Bake the global-sequence channels with [`build_animation_clip`]'s transforms, ms to seconds.
pub(crate) fn build_global_bones(
    gseq: &[wrathplus_formats::GlobalSeqBone],
    skeleton: &ModelSkeleton,
) -> Vec<GlobalBone> {
    let r = wow_to_bevy_quat();
    gseq.iter()
        .map(|g| {
            let rest = skeleton
                .joints
                .get(g.bone as usize)
                .map_or(Vec3::ZERO, |j| j.local_translation);
            let ms = |t: u32| t as f32 / 1000.0;
            GlobalBone {
                bone: g.bone,
                translation: g.translation.as_ref().map(|c| GlobalSeqChannel {
                    period: ms(c.period_ms),
                    keys: c
                        .keys
                        .iter()
                        .map(|(t, v)| (ms(*t), rest + wow_to_bevy(*v)))
                        .collect(),
                }),
                rotation: g.rotation.as_ref().map(|c| GlobalSeqChannel {
                    period: ms(c.period_ms),
                    keys: c
                        .keys
                        .iter()
                        .map(|(t, q)| {
                            (
                                ms(*t),
                                r * Quat::from_xyzw(q[0], q[1], q[2], q[3]) * r.inverse(),
                            )
                        })
                        .collect(),
                }),
                scale: g.scale.as_ref().map(|c| GlobalSeqChannel {
                    period: ms(c.period_ms),
                    keys: c
                        .keys
                        .iter()
                        .map(|(t, s)| (ms(*t), Vec3::new(s[1], s[2], s[0])))
                        .collect(),
                }),
            }
        })
        .collect()
}

/// One mesh from many placed static parts, transforms baked in (normals take only the rotation,
/// as placements scale uniformly), each vertex carrying its part's fade sphere and, with `slots`,
/// its SH-probe slot; missing normals and colours pad (up, white). Returns the mesh, its local
/// bounds and its world centre, where the caller must place the entity: Bevy sorts transparent
/// draws by entity origin, and a mesh left at the world origin draws first and depth-hides the
/// transparent draws behind it. The fade spheres stay world-space.
pub fn merged_static_mesh_faded(
    parts: &[(std::sync::Arc<RenderSubmesh>, Transform)],
    spheres: &[Vec4],
    slots: Option<&[u32]>,
) -> (Mesh, Vec3, Vec3, Vec3) {
    merged_static_mesh_impl(parts, Some((spheres, slots)))
}

fn merged_static_mesh_impl(
    parts: &[(std::sync::Arc<RenderSubmesh>, Transform)],
    extras: Option<(&[Vec4], Option<&[u32]>)>,
) -> (Mesh, Vec3, Vec3, Vec3) {
    let any_colors = parts
        .iter()
        .any(|(s, _)| s.vertex_colors.len() == s.positions.len());
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut uvs2: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut fade: Vec<[f32; 4]> = Vec::new();
    let mut slot_verts: Vec<u32> = Vec::new();
    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (i, (sub, transform)) in parts.iter().enumerate() {
        let base = u32::try_from(positions.len()).expect("merged mesh under u32 vertices");
        for p in &sub.positions {
            let w = transform.transform_point(wow_to_bevy(*p));
            mn = mn.min(w);
            mx = mx.max(w);
            positions.push(w.to_array());
        }
        if sub.normals.len() == sub.positions.len() {
            let flip = sub.billboard_card_faces_away();
            for n in &sub.normals {
                let b = transform.rotation * wow_to_bevy(*n);
                normals.push(if flip { -b } else { b }.normalize_or_zero().to_array());
            }
        } else {
            normals.extend(std::iter::repeat_n([0.0, 1.0, 0.0], sub.positions.len()));
        }
        uvs.extend(sub.uvs.iter().copied());
        uvs2.extend(
            if sub.uvs2.len() == sub.positions.len() {
                &sub.uvs2
            } else {
                &sub.uvs
            }
            .iter()
            .copied(),
        );
        if any_colors {
            if sub.vertex_colors.len() == sub.positions.len() {
                colors.extend(sub.vertex_colors.iter().copied());
            } else {
                colors.extend(std::iter::repeat_n(
                    [1.0, 1.0, 1.0, 1.0],
                    sub.positions.len(),
                ));
            }
        }
        if let Some((spheres, slots)) = extras {
            fade.extend(std::iter::repeat_n(
                spheres[i].to_array(),
                sub.positions.len(),
            ));
            if let Some(slots) = slots {
                slot_verts.extend(std::iter::repeat_n(slots[i], sub.positions.len()));
            }
        }
        indices.extend(sub.indices.iter().map(|i| base + i));
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    // Always, as `build_submesh_mesh` does: one layout per family.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs2);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    if any_colors {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if let Some((_, slots)) = extras {
        mesh.insert_attribute(ATTRIBUTE_WOW_FADE_SPHERE, fade);
        if slots.is_some() {
            mesh.insert_attribute(
                ATTRIBUTE_WOW_MERGED_SLOT,
                bevy::mesh::VertexAttributeValues::Uint32(slot_verts),
            );
        }
    }
    mesh.insert_indices(Indices::U32(indices));
    // Recentre on the union-AABB centre; with no vertices (`mn > mx`) the centre is the origin.
    let center = if mn.x > mx.x {
        Vec3::ZERO
    } else {
        (mn + mx) * 0.5
    };
    if center != Vec3::ZERO {
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        else {
            unreachable!("positions were just inserted as Float32x3");
        };
        for p in pos.iter_mut() {
            p[0] -= center.x;
            p[1] -= center.y;
            p[2] -= center.z;
        }
    }
    (mesh, mn - center, mx - center, center)
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
