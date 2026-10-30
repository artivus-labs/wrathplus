//! Bevy asset adaptation for Wrath's split v264 M2 + external SKIN representation.

use std::sync::Arc;

use bevy::animation::graph::AnimationGraph;
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::{M2Model, M2SequenceInfo, PaneCamera, PortraitCamera};
use crate::bone_target_id;
use crate::coords::wow_to_bevy;
use crate::model::{
    arm_subtree_roots, build_animation_clip, build_attachments, build_global_bones,
    build_grip_clip, build_skeleton, finger_subtree_roots, in_subtree, skeleton_pivots,
    upper_subtree_root, AnimClip, ModelAnimations, ModelSubmesh,
};

/// Bevy [`AssetLoader`] for v264 `*.m2` assets.
#[derive(Default, TypePath)]
pub struct WrathM2ModelLoader;

impl AssetLoader for WrathM2ModelLoader {
    type Asset = M2Model;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        ctx: &mut LoadContext<'_>,
    ) -> Result<M2Model, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let to_io = |error: anyhow::Error| std::io::Error::other(format!("{error:#}"));

        // Wrath profile zero is a sibling named `<model-stem>00.skin`. Reading through the load
        // context both avoids recursive loader dispatch and records the dependency for hot reload.
        let model_url = ctx
            .path()
            .to_string()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let model_url = model_url.split('#').next().unwrap_or(&model_url);
        let stem = model_url.strip_suffix(".m2").ok_or_else(|| {
            std::io::Error::other(format!("Wrath M2 asset has no .m2 suffix: {model_url}"))
        })?;
        let skin_url = format!("{stem}00.skin");
        let skin = ctx
            .read_asset_bytes(skin_url.clone())
            .await
            .map_err(|error| {
                std::io::Error::other(format!("reading Wrath skin dependency {skin_url}: {error}"))
            })?;
        // This renderer builds all clips at model admission, unlike the stock client's lazy
        // .anim loading. Keep strict missing-file errors rather than fabricating animation.
        // Installed HumanMale probe: 47 external dependencies / 156 clips; deferred loading
        // remains a separate architecture task.
        let dependencies =
            wrathplus_formats::wrath_render_animation_dependencies(&bytes).map_err(to_io)?;
        let mut external = vec![
            None;
            dependencies
                .iter()
                .map(|dependency| dependency.sequence_index + 1)
                .max()
                .unwrap_or(0)
        ];
        for dependency in dependencies {
            let anim_url = format!(
                "{stem}{:04}-{:02}.anim",
                dependency.animation_id, dependency.variation_index
            );
            let anim = ctx
                .read_asset_bytes(anim_url.clone())
                .await
                .map_err(|error| {
                    std::io::Error::other(format!(
                        "reading required Wrath animation dependency {anim_url}: {error}"
                    ))
                })?;
            external[dependency.sequence_index] = Some(anim);
        }
        let external_refs = external
            .iter()
            .map(|bytes| bytes.as_deref())
            .collect::<Vec<_>>();
        let semantic = wrathplus_formats::parse_wrath_render_model_with_animations(
            &bytes,
            &skin,
            &external_refs,
            "",
            &[],
        )
        .map_err(to_io)?;

        reject_unrepresentable_interpolation(&semantic)?;
        let cameras = build_cameras(&semantic)?;

        let bounds = Some(semantic.bounds);
        let light_sphere = {
            let b = &semantic.bounds;
            let mid = |i: usize| (b.bbox_min[i] + b.bbox_max[i]) * 0.5;
            wow_to_bevy([mid(0), mid(1), mid(2)]).extend(b.sphere_radius)
        };
        let collision = (!semantic.collision.is_empty()).then_some(semantic.collision.clone());
        let submeshes = semantic
            .submeshes
            .iter()
            .cloned()
            .map(|sub| {
                let texture = sub.texture.as_deref().map(|texture| {
                    ctx.load::<Image>(crate::texture_url(texture, (sub.wrap_x, sub.wrap_y)))
                });
                ModelSubmesh {
                    texture,
                    texture2: sub.stage2.as_ref().and_then(|stage| {
                        stage.texture.as_deref().map(|t| {
                            ctx.load::<Image>(crate::texture_url(t, (stage.wrap_x, stage.wrap_y)))
                        })
                    }),
                    skin_slot: sub.skin_slot,
                    geoset_id: sub.geoset_id,
                    char_slot: sub.char_slot,
                    icon_slot: sub.icon_slot,
                    blend: sub.blend,
                    two_sided: sub.two_sided,
                    interior: false,
                    emissive: sub.emissive,
                    sidn: None,
                    wmo_unified: None,
                    window: false,
                    additive: sub.additive,
                    env_map: sub.env_map,
                    no_depth_write: sub.no_depth_write,
                    no_depth_test: sub.no_depth_test,
                    fog_policy: sub.fog_policy,
                    light_sphere,
                    // Wrath's adapter deliberately keeps billboard geometry on the joint palette.
                    billboard: None,
                    alpha_anim: sub.alpha_anim.clone().map(Arc::new),
                    uv_anim: sub.uv_anim.clone().map(Arc::new),
                    uv_seq: sub.uv_seq.clone().map(Arc::new),
                    uv_rot_seq: sub.uv_rot_seq.clone().map(Arc::new),
                    uv_scale_seq: sub.uv_scale_seq.clone().map(Arc::new),
                    rgb_anim: sub.rgb_anim.clone().map(Arc::new),
                    rgb_seq: sub.rgb_seq.clone().map(Arc::new),
                    wmo_batch: None,
                    ground_quad: sub.ground_quad(),
                    geometry: Arc::new(sub),
                }
            })
            .collect::<Vec<_>>();

        let (skeleton, inverse_bindposes) = build_skeleton(&semantic.skeleton);
        let inverse_bindposes = ctx.add_labeled_asset(
            "inverse_bindposes".to_string(),
            SkinnedMeshInverseBindposes::from(inverse_bindposes),
        );
        let pivots = skeleton_pivots(&semantic.skeleton);
        let attachments = build_attachments(&semantic.attachments, &pivots);
        let markers = crate::model::build_markers(&semantic.event_markers, &pivots);
        // Parsed ahead of the clips: an emitter samples the sequence clock, which earns a model its
        // clips even with no posed bone (the rifle's muzzle blast, gated on BowRelease).
        let emitter_defs =
            wrathplus_formats::parse_m2_particle_emitters_with_animations(&bytes, &external_refs)
                .unwrap_or_default();
        let animations = build_animations(
            ctx,
            &semantic,
            &skeleton,
            &attachments,
            &pivots,
            !emitter_defs.is_empty(),
        );
        let sequence_infos = semantic
            .animations
            .iter()
            .map(|sequence| M2SequenceInfo {
                anim_id: sequence.anim_id,
                seq_index: sequence.seq_index,
                duration_ms: sequence.end_ms.saturating_sub(sequence.start_ms),
                start_ms: sequence.start_ms,
                looping: sequence.looping,
            })
            .collect::<Vec<_>>();
        let first_seq_span = semantic
            .animations
            .first()
            .map(|sequence| sequence.duration)
            .filter(|duration| *duration > 0.0);

        let emitters = build_emitters(ctx, emitter_defs, &semantic, animations.as_ref());
        let owner_reach = wrathplus_formats::m2_owner_reach(&semantic.submeshes);
        let b = &semantic.bounds;
        let water_bound = (
            wow_to_bevy([
                (b.bbox_min[0] + b.bbox_max[0]) * 0.5,
                (b.bbox_min[1] + b.bbox_max[1]) * 0.5,
                (b.bbox_min[2] + b.bbox_max[2]) * 0.5,
            ]),
            b.sphere_radius,
        );
        let ribbons = semantic
            .ribbons
            .iter()
            .cloned()
            .map(|def| {
                let texture = def.texture.as_deref().map(|t| {
                    ctx.load::<Image>(format!(
                        "mpq://{}",
                        t.replace('\\', "/").to_ascii_lowercase()
                    ))
                });
                let bone_pivot = semantic
                    .skeleton
                    .bones
                    .get(def.bone as usize)
                    .map_or([0.0; 3], |b| b.pivot);
                super::ModelRibbon {
                    def,
                    texture,
                    bone_pivot,
                    owner_reach,
                    water_bound,
                }
            })
            .collect();
        let lights = semantic
            .lights
            .iter()
            .cloned()
            .map(|def| {
                let bone_pivot = usize::try_from(def.bone)
                    .ok()
                    .and_then(|bone| semantic.skeleton.bones.get(bone))
                    .map_or([0.0; 3], |b| b.pivot);
                super::ModelLight { def, bone_pivot }
            })
            .collect();

        Ok(M2Model {
            submeshes,
            bounds,
            collision,
            emitters,
            ribbons,
            lights,
            skeleton,
            inverse_bindposes,
            animations,
            first_seq_span,
            sequences: sequence_infos,
            attachments,
            markers,
            portrait_camera: cameras.portrait,
            camera0: cameras.raw.first().map(|camera| camera.still),
            pane_camera: cameras.raw.get(1).map(|camera| camera.still),
            cameras: cameras.raw,
            string_anchors: marker_pair(&semantic.event_markers, *b"$WTT", *b"$WTB"),
            cch_marker: semantic
                .event_markers
                .iter()
                .find(|marker| marker.ident == *b"$CCH")
                .map(|marker| wow_to_bevy(marker.position)),
            global_flags: u32::from_le_bytes(bytes[0x10..0x14].try_into().unwrap()),
        })
    }

    fn extensions(&self) -> &[&str] {
        &["m2"]
    }
}

fn reject_unrepresentable_interpolation(
    semantic: &wrathplus_formats::WrathRenderModel,
) -> Result<(), std::io::Error> {
    for (sequence, modes) in semantic
        .animations
        .iter()
        .zip(&semantic.animation_interpolation)
    {
        for mode in modes {
            let keys = sequence.bones.iter().find(|keys| keys.bone == mode.bone);
            let Some(keys) = keys else { continue };
            for (name, interpolation, key_count) in [
                ("translation", mode.translation, keys.translation.len()),
                ("rotation", mode.rotation, keys.rotation.len()),
                ("scale", mode.scale, keys.scale.len()),
            ] {
                if interpolation > 1 || (interpolation == 0 && key_count > 1) {
                    return Err(std::io::Error::other(format!(
                        "unsupported Wrath bone {name} interpolation {interpolation} on sequence {} bone {}",
                        sequence.seq_index, mode.bone
                    )));
                }
            }
        }
    }
    Ok(())
}

struct BuiltCameras {
    portrait: Option<PortraitCamera>,
    raw: Vec<PaneCamera>,
}

fn build_cameras(
    semantic: &wrathplus_formats::WrathRenderModel,
) -> Result<BuiltCameras, std::io::Error> {
    let mut raw = Vec::with_capacity(semantic.cameras.len());
    for (index, camera) in semantic.cameras.iter().enumerate() {
        let moving = [&camera.positions, &camera.target]
            .into_iter()
            .any(|track| track.sequences.iter().any(|keys| keys.len() > 1))
            || camera.roll.sequences.iter().any(|keys| keys.len() > 1);
        let pose = wrathplus_formats::sample_wrath_camera(camera, 0, 0, 0).map_err(|error| {
            std::io::Error::other(format!("sampling Wrath camera {index}: {error:#}"))
        })?;
        raw.push(PaneCamera {
            camera_type: camera.camera_type,
            still: PortraitCamera {
                eye: wow_to_bevy(pose.position),
                target: wow_to_bevy(pose.target),
                roll: pose.roll,
                fov: pose.fov,
                near: pose.near_clip,
                far: pose.far_clip,
            },
            tracks: moving.then(|| std::sync::Arc::new(camera.clone())),
        });
    }
    let portrait = semantic
        .camera_lookup
        .first()
        .filter(|&&index| index != u16::MAX)
        .and_then(|&index| raw.get(usize::from(index)))
        .map(|camera| camera.still);
    Ok(BuiltCameras { portrait, raw })
}

/// The model's particle emitters ([`wrathplus_formats::parse_m2_particle_emitters`] reads v264), as
/// the Vanilla loader places them: texture, bone pivot, billboard host, child models, the owner's
/// draw-order rung and water sphere, and its idle slot. Tracks keyed inside external sequences
/// read the `.anim` files resolved for the bones.
fn build_emitters(
    ctx: &mut LoadContext<'_>,
    defs: Vec<wrathplus_formats::ParticleEmitterDef>,
    semantic: &wrathplus_formats::WrathRenderModel,
    animations: Option<&ModelAnimations>,
) -> Vec<super::ModelEmitter> {
    let owner_reach = wrathplus_formats::m2_owner_reach(&semantic.submeshes);
    let b = &semantic.bounds;
    let water_bound = (
        wow_to_bevy([
            (b.bbox_min[0] + b.bbox_max[0]) * 0.5,
            (b.bbox_min[1] + b.bbox_max[1]) * 0.5,
            (b.bbox_min[2] + b.bbox_max[2]) * 0.5,
        ]),
        b.sphere_radius,
    );
    let idle_seq = animations.and_then(ModelAnimations::idle_seq).unwrap_or(0);
    let skeleton = &semantic.skeleton;
    defs.into_iter()
        .map(|def| {
            let texture = def.texture.as_deref().map(|t| {
                ctx.load::<Image>(format!(
                    "mpq://{}",
                    t.replace('\\', "/").to_ascii_lowercase()
                ))
            });
            let bone_pivot = skeleton
                .bones
                .get(def.bone as usize)
                .map_or([0.0; 3], |b| b.pivot);
            let recursion = def
                .recursion_model
                .as_deref()
                .map(|p| ctx.load::<M2Model>(super::m2_dep_url(p)));
            let geometry = def
                .geometry_model
                .as_deref()
                .map(|p| ctx.load::<M2Model>(super::m2_dep_url(p)));
            let billboard = skeleton.billboard_host(def.bone).and_then(|h| {
                let b = skeleton.bones.get(h)?;
                Some(super::EmitterBillboard {
                    kind: b.billboard?,
                    pivot: b.pivot,
                    bone: u16::try_from(h).ok()?,
                })
            });
            super::ModelEmitter {
                def,
                texture,
                bone_pivot,
                billboard,
                recursion,
                geometry,
                owner_reach,
                water_bound,
                idle_seq,
            }
        })
        .collect()
}

fn build_animations(
    ctx: &mut LoadContext<'_>,
    semantic: &wrathplus_formats::WrathRenderModel,
    skeleton: &crate::ModelSkeleton,
    attachments: &[crate::ModelAttachment],
    pivots: &[Vec3],
    has_emitters: bool,
) -> Option<ModelAnimations> {
    let mut graph = AnimationGraph::new();
    let root = graph.root;
    let mut pose = crate::model::PoseSource {
        bone_masks: vec![0; semantic.skeleton.bones.len()],
        ..Default::default()
    };
    let arm_roots = arm_subtree_roots(&semantic.skeleton, attachments);
    if let Some((right, left)) = arm_roots {
        for bone in 0..semantic.skeleton.bones.len() {
            let target = bone_target_id(bone as u16);
            if !in_subtree(&semantic.skeleton, bone, right) {
                graph.add_target_to_mask_group(target, 0);
                pose.bone_masks[bone] |= 1 << 0;
            }
            if !in_subtree(&semantic.skeleton, bone, left) {
                graph.add_target_to_mask_group(target, 1);
                pose.bone_masks[bone] |= 1 << 1;
            }
        }
    }
    let upper_root = upper_subtree_root(&semantic.skeleton);
    if let Some(upper) = upper_root {
        for bone in 0..semantic.skeleton.bones.len() {
            if !in_subtree(&semantic.skeleton, bone, upper) {
                graph.add_target_to_mask_group(bone_target_id(bone as u16), 2);
                pose.bone_masks[bone] |= 1 << 2;
            }
        }
    }
    // Groups 3 and 4 hold every joint outside the right and the left fingers, for the weapon grip
    // (`CloseHand`, `0x479660`).
    let finger_roots = finger_subtree_roots(&semantic.skeleton);
    let in_fingers = |bone: usize, hand: usize| {
        finger_roots[hand]
            .iter()
            .any(|&root| in_subtree(&semantic.skeleton, bone, root))
    };
    for bone in 0..semantic.skeleton.bones.len() {
        for (hand, roots) in finger_roots.iter().enumerate() {
            if !roots.is_empty() && !in_fingers(bone, hand) {
                graph.add_target_to_mask_group(bone_target_id(bone as u16), 3 + hand as u32);
                pose.bone_masks[bone] |= 1 << (3 + hand);
            }
        }
    }
    let keyed = crate::model::KeyedChannels::of(&semantic.animations);
    let mut clips = Vec::with_capacity(semantic.animations.len());
    let mut first_seq = None;
    for (index, animation) in semantic.animations.iter().enumerate() {
        let (clip, pose_clip, poses_bones) = build_animation_clip(animation, skeleton, &keyed);
        if first_seq.is_none() && animation.anim_id == 0 && !animation.is_rest_pose() {
            first_seq = Some(clips.len());
        }
        let pose_index = pose.clips.len() as u32;
        pose.clips.push(pose_clip);
        let clip = ctx.add_labeled_asset(format!("clip{index}"), clip);
        let node = graph.add_clip(clip.clone(), 1.0, root);
        pose.set_node(node, pose_index, 0);
        let arm_nodes = (matches!(animation.anim_id, 89 | 90) && arm_roots.is_some()).then(|| {
            let nodes = (
                graph.add_clip_with_mask(clip.clone(), 1 << 0, 1.0, root),
                graph.add_clip_with_mask(clip.clone(), 1 << 1, 1.0, root),
            );
            pose.set_node(nodes.0, pose_index, 1 << 0);
            pose.set_node(nodes.1, pose_index, 1 << 1);
            nodes
        });
        let upper_node = upper_root.map(|_| {
            let node = graph.add_clip_with_mask(clip, 1 << 2, 1.0, root);
            pose.set_node(node, pose_index, 1 << 2);
            node
        });
        clips.push(AnimClip {
            anim_id: animation.anim_id,
            seq_index: animation.seq_index,
            node,
            looping: animation.looping,
            duration: animation.duration,
            move_speed: animation.move_speed,
            blend_time: animation.blend_time,
            bounds_center: wow_to_bevy(animation.bounds_center),
            bounds_radius: animation.bounds_radius,
            bounds_min: wow_to_bevy(animation.bounds_min).min(wow_to_bevy(animation.bounds_max)),
            bounds_max: wow_to_bevy(animation.bounds_min).max(wow_to_bevy(animation.bounds_max)),
            // `offset` and `point` both use the record's own bone, never a by-4CC lookup: a model
            // may author a tag more than once.
            events: animation
                .events
                .iter()
                .map(|e| crate::ClipEvent {
                    time: e.time,
                    ident: e.ident,
                    data: e.data,
                    bone: e.bone,
                    offset: wow_to_bevy(e.position)
                        - pivots.get(e.bone as usize).copied().unwrap_or(Vec3::ZERO),
                    point: wow_to_bevy(e.position),
                })
                .collect::<Vec<_>>()
                .into(),
            arm_nodes,
            upper_node,
            frequency: animation.frequency,
            replay: (animation.min_replay, animation.max_replay),
            poses_bones,
        });
    }
    // The grip is its own clip of the `HandsClosed` (AnimationData 15) finger poses, played masked
    // to the hand that holds a weapon.
    let mut hand_close = [None; 2];
    let grip_poses =
        hand_grip_finger_poses(semantic, |bone| in_fingers(bone, 0) || in_fingers(bone, 1));
    if !grip_poses.is_empty() {
        let (grip_clip, grip_pose) = build_grip_clip(&grip_poses);
        let grip_index = pose.clips.len() as u32;
        pose.clips.push(grip_pose);
        let grip = ctx.add_labeled_asset("grip_clip".to_string(), grip_clip);
        for (hand, slot) in hand_close.iter_mut().enumerate() {
            if !finger_roots[hand].is_empty() {
                let mask = 1 << (3 + hand);
                let node = graph.add_clip_with_mask(grip.clone(), mask, 1.0, root);
                pose.set_node(node, grip_index, mask);
                *slot = Some(node);
            }
        }
    }
    let global_bones = build_global_bones(&semantic.global_sequence_bones, skeleton);
    // A model animates if a bone poses, a global sequence runs, or something samples the sequence
    // clock: emitters, ribbons, or a material's alpha, colour or UV loop.
    let samples_sequence = has_emitters
        || !semantic.ribbons.is_empty()
        || semantic.submeshes.iter().any(|s| {
            s.alpha_anim.is_some()
                || s.uv_anim.is_some()
                || s.uv_seq.is_some()
                || s.uv_rot_seq.is_some()
                || s.uv_scale_seq.is_some()
                || s.rgb_anim.is_some()
                || s.rgb_seq.is_some()
        });
    if clips.iter().all(|clip| !clip.poses_bones)
        && global_bones.is_empty()
        && !(samples_sequence && !clips.is_empty())
    {
        return None;
    }
    let graph = ctx.add_labeled_asset("anim_graph".to_string(), graph);
    Some(ModelAnimations {
        graph,
        clips,
        playable_animation_lookup: Vec::new(),
        animation_lookup: direct_animation_lookup(&semantic.animations),
        hand_close,
        global_bones,
        first_seq,
        pose: Arc::new(pose),
    })
}

/// The id-indexed sequence table [`ModelAnimations::owns`] reads, each id's first file slot. v264
/// hashes its own (`id % len`: `Rotating-MinimapArrow.m2` keeps id 165 at slot 13 of 19) or ships
/// it empty for a linear search (every weapon model), so it is rebuilt from the sequences.
fn direct_animation_lookup(animations: &[wrathplus_formats::ModelAnimation]) -> Vec<u16> {
    let top = animations
        .iter()
        .map(|a| usize::from(a.anim_id) + 1)
        .max()
        .unwrap_or(0);
    let mut lookup = vec![u16::MAX; top];
    for a in animations {
        let slot = &mut lookup[usize::from(a.anim_id)];
        if *slot == u16::MAX {
            *slot = u16::try_from(a.seq_index).unwrap_or(u16::MAX);
        }
    }
    lookup
}

/// Each finger bone's rotation, `[x, y, z, w]`, at the head of `HandsClosed` (AnimationData 15),
/// the pose `CloseHand` (`0x479660`) holds while the body animates. A v264 sequence keeps its own
/// key arrays from t = 0, so the head key is the frame.
fn hand_grip_finger_poses(
    semantic: &wrathplus_formats::WrathRenderModel,
    is_finger: impl Fn(usize) -> bool,
) -> Vec<(u16, [f32; 4])> {
    let Some(hands_closed) = semantic.animations.iter().find(|a| a.anim_id == 15) else {
        return Vec::new();
    };
    hands_closed
        .bones
        .iter()
        .filter(|keys| is_finger(usize::from(keys.bone)))
        .filter_map(|keys| Some((keys.bone, keys.rotation.first()?.1)))
        .collect()
}

fn marker_pair(
    markers: &[wrathplus_formats::EventMarker],
    top: [u8; 4],
    bottom: [u8; 4],
) -> Option<[(u16, Vec3); 2]> {
    let find = |ident| {
        markers
            .iter()
            .find(|marker| marker.ident == ident)
            .map(|marker| (marker.bone, wow_to_bevy(marker.position)))
    };
    Some([find(top)?, find(bottom)?])
}

#[cfg(test)]
#[path = "wrath_m2_installed_tests.rs"]
mod installed_tests;
