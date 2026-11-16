//! Wrath v264 M2 + external SKIN adaptation into renderer-facing model semantics.
//!
//! This module consumes bytes already selected by a profile/asset loader. It deliberately does
//! not know about [`crate::Chain`]. Wrath tracks remain indexed by their native outer sequence
//! arrays: unlike pre-Wrath M2, their timestamps are local to each sequence and must not be cut
//! against one shared, flat timeline.

use std::io::Cursor;

use anyhow::{bail, Context, Result};
use wrathplus_m2::wrath::{
    parse_wrath_m2, parse_wrath_m2_with_animations, parse_wrath_skin, wrath_animation_dependencies,
    WrathAnimationDependency, WrathCamera, WrathM2, WrathSkin,
};
use wrathplus_m2::{M2Model, M2Texture, M2TextureType, M2Track};

use super::mat_anim;
use super::tex_anim;
use super::{
    remap_submesh, AlphaAnim, AlphaSeq, AnimEvent, BillboardKind, CharSkinSlot, CollisionMesh,
    EventMarker, FogPolicy, GlobalSeqBone, GlobalSeqChannel, M2Attachment, M2Bounds,
    ModelAnimation, ModelBlend, ParentArm, RenderSubmesh, Skeleton, SkeletonBone,
};
use super::{BoneKeys, SeqSlot, DEGENERATE_RING_FOOTPRINT};

/// All renderer-facing semantics currently representable from a v264 M2 and one external skin.
///
/// Native lights and ribbons are adapted separately from the particle byte reader.
/// Animation-event positions are retained in [`Self::event_markers`]; their per-sequence trigger
/// times are each [`ModelAnimation::events`].
pub struct WrathRenderModel {
    pub submeshes: Vec<RenderSubmesh>,
    pub skeleton: Skeleton,
    pub animations: Vec<ModelAnimation>,
    /// Per-animation interpolation modes parallel to [`Self::animations`].
    pub animation_interpolation: Vec<Vec<WrathBoneInterpolation>>,
    pub global_sequence_bones: Vec<GlobalSeqBone>,
    pub cameras: Vec<WrathCamera>,
    pub lights: Vec<super::M2Light>,
    pub ribbons: Vec<crate::RibbonEmitterDef>,
    pub camera_lookup: Vec<u16>,
    pub attachments: Vec<M2Attachment>,
    pub event_markers: Vec<EventMarker>,
    pub animation_lookup: Vec<u16>,
    pub bounds: M2Bounds,
    pub collision: CollisionMesh,
}

/// The native interpolation mode of one bone's channels in one Wrath sequence.
///
/// The existing [`BoneKeys`] value intentionally contains only sampled key values. A renderer
/// converting those keys to clips must use these modes: 0 is step and 1 is linear. Cubic bone
/// tracks fail in `wrathplus-m2` because its compact quaternion/vec3 semantic tracks do not retain
/// their tangents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrathBoneInterpolation {
    pub bone: u16,
    pub translation: u16,
    pub rotation: u16,
    pub scale: u16,
}

/// A sampled Wrath camera rig in raw WoW model space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrathCameraPose {
    pub fov: f32,
    pub far_clip: f32,
    pub near_clip: f32,
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub roll: f32,
}

/// Return the canonical external animation sequence dependencies declared by a v264 M2.
pub fn wrath_render_animation_dependencies(
    m2_bytes: &[u8],
) -> Result<Vec<WrathAnimationDependency>> {
    wrath_animation_dependencies(m2_bytes)
        .map_err(|error| anyhow::anyhow!("reading Wrath animation dependencies: {error}"))
}

/// Parse and adapt an `MD20` v264 model together with one external `SKIN` profile.
///
/// `dir` resolves hardcoded and creature-variation texture names and `skins` supplies the optional
/// `Monster1/2/3` basenames. Multi-texture
/// batches preserve both stages in one draw, including their native combiner and coordinate lookup.
pub fn parse_wrath_render_model(
    m2_bytes: &[u8],
    skin_bytes: &[u8],
    dir: &str,
    skins: &[Option<String>],
) -> Result<WrathRenderModel> {
    let wrath = parse_wrath_m2(&mut Cursor::new(m2_bytes))
        .map_err(|error| anyhow::anyhow!("parsing Wrath M2: {error}"))?;
    adapt_wrath_render_model(wrath, skin_bytes, dir, skins)
}

/// Parse and adapt a v264 model with sibling `.anim` bytes indexed by canonical sequence index.
/// Unlike [`parse_wrath_render_model`], required external sequences are an error when absent.
pub fn parse_wrath_render_model_with_animations(
    m2_bytes: &[u8],
    skin_bytes: &[u8],
    external_animations: &[Option<&[u8]>],
    dir: &str,
    skins: &[Option<String>],
) -> Result<WrathRenderModel> {
    let wrath = parse_wrath_m2_with_animations(m2_bytes, external_animations)
        .map_err(|error| anyhow::anyhow!("parsing Wrath M2 animations: {error}"))?;
    adapt_wrath_render_model(wrath, skin_bytes, dir, skins)
}

fn adapt_wrath_render_model(
    wrath: WrathM2,
    skin_bytes: &[u8],
    dir: &str,
    skins: &[Option<String>],
) -> Result<WrathRenderModel> {
    let skin = parse_wrath_skin(skin_bytes)
        .map_err(|error| anyhow::anyhow!("parsing external Wrath skin: {error}"))?;
    if wrath.skin_count() == 0 {
        bail!("Wrath M2 declares no external skin profiles");
    }
    validate_track_sequence_counts(&wrath)?;

    let mut submeshes = render_submeshes(&wrath, &skin, dir, skins)?;
    let skeleton = skeleton(&wrath);
    let (animations, animation_interpolation) = animations(&wrath)?;
    let global_sequence_bones = global_sequence_bones(&wrath)?;
    let bounds = bounds(&wrath);
    if is_white1_placeholder(bounds.bbox_min, bounds.bbox_max, &submeshes) {
        submeshes.clear();
    }
    let collision = collision(wrath.model())?;
    let model = wrath.model();
    let attachments = model
        .attach_lookup
        .iter()
        .enumerate()
        .filter_map(|(id, &index)| {
            let attachment = model.attachments.get(usize::from(index))?;
            Some(M2Attachment {
                id: u16::try_from(id).ok()?,
                bone: attachment.bone,
                position: attachment.position,
            })
        })
        .collect();
    let event_markers = model
        .event_markers
        .iter()
        .map(|marker| EventMarker {
            ident: marker.ident,
            bone: marker.bone,
            position: marker.position,
        })
        .collect();

    Ok(WrathRenderModel {
        submeshes,
        skeleton,
        animations,
        animation_interpolation,
        global_sequence_bones,
        cameras: wrath.cameras().to_vec(),
        lights: super::wrath_effects::lights(&wrath),
        ribbons: super::wrath_effects::ribbons(&wrath, dir, skins)?,
        camera_lookup: wrath.camera_lookup().to_vec(),
        attachments,
        event_markers,
        animation_lookup: model.animation_lookup.clone(),
        bounds,
        collision,
    })
}

/// Sample a camera's native nested track for `sequence_index` at sequence-local `time_ms`.
/// `global_time_ms` is used only by tracks tied to a global sequence. Step, linear, Hermite and
/// Bézier interpolation are all retained; missing channels contribute zero to their base.
pub fn sample_wrath_camera(
    camera: &WrathCamera,
    sequence_index: usize,
    time_ms: u32,
    global_time_ms: u32,
) -> Result<WrathCameraPose> {
    let position = sample_vec3_track(&camera.positions, sequence_index, time_ms, global_time_ms)?
        .unwrap_or([0.0; 3]);
    let target = sample_vec3_track(&camera.target, sequence_index, time_ms, global_time_ms)?
        .unwrap_or([0.0; 3]);
    let roll =
        sample_scalar_track(&camera.roll, sequence_index, time_ms, global_time_ms)?.unwrap_or(0.0);
    Ok(WrathCameraPose {
        fov: camera.fov,
        far_clip: camera.far_clip,
        near_clip: camera.near_clip,
        position: add3(camera.position_base, position),
        target: add3(camera.target_base, target),
        roll,
    })
}

fn validate_track_sequence_counts(wrath: &WrathM2) -> Result<()> {
    let sequence_count = wrath.animations().len();
    let validate = |bone_index: usize,
                    channel: &str,
                    gseq: u16,
                    range_count: usize|
     -> Result<()> {
        if gseq == u16::MAX && range_count != 0 && range_count != sequence_count {
            bail!(
                "bone {bone_index} {channel} track has {range_count} nested sequence arrays, expected {sequence_count}"
            );
        }
        Ok(())
    };
    for (bone_index, tracks) in wrath.bone_tracks().iter().enumerate() {
        validate(
            bone_index,
            "translation",
            tracks.translation.gseq,
            tracks.translation.ranges.len(),
        )?;
        validate(
            bone_index,
            "rotation",
            tracks.rotation.gseq,
            tracks.rotation.ranges.len(),
        )?;
        validate(
            bone_index,
            "scale",
            tracks.scaling.gseq,
            tracks.scaling.ranges.len(),
        )?;
    }
    Ok(())
}

fn skeleton(wrath: &WrathM2) -> Skeleton {
    Skeleton {
        bones: wrath
            .model()
            .bones
            .iter()
            .map(|bone| SkeletonBone {
                parent: bone.parent,
                pivot: [bone.pivot.x, bone.pivot.y, bone.pivot.z],
                key_bone: bone.key_bone,
                billboard: BillboardKind::from_bone_flags(bone.flags.bits()),
                parent_arm: ParentArm::from_bone_flags(bone.flags.bits()),
            })
            .collect(),
    }
}

fn animations(wrath: &WrathM2) -> Result<(Vec<ModelAnimation>, Vec<Vec<WrathBoneInterpolation>>)> {
    let mut animations = Vec::with_capacity(wrath.animations().len());
    let mut all_interpolation = Vec::with_capacity(wrath.animations().len());
    for (sequence_index, sequence) in wrath.animations().iter().enumerate() {
        let mut bones = Vec::new();
        let mut interpolation = Vec::new();
        for (bone_index, tracks) in wrath.bone_tracks().iter().enumerate() {
            let translation = sequence_keys(&tracks.translation, sequence_index)?;
            let rotation = sequence_keys(&tracks.rotation, sequence_index)?;
            let scale = sequence_keys(&tracks.scaling, sequence_index)?;
            if translation.is_empty() && rotation.is_empty() && scale.is_empty() {
                continue;
            }
            let bone = u16::try_from(bone_index).context("Wrath bone index exceeds u16")?;
            bones.push(BoneKeys {
                bone,
                translation,
                rotation,
                scale,
            });
            interpolation.push(WrathBoneInterpolation {
                bone,
                translation: tracks.translation.interp,
                rotation: tracks.rotation.interp,
                scale: tracks.scaling.interp,
            });
        }
        let bounds_center = std::array::from_fn(|axis| {
            (sequence.bounds_min[axis] + sequence.bounds_max[axis]) * 0.5
        });
        animations.push(ModelAnimation {
            anim_id: sequence.id,
            seq_index: sequence_index,
            start_ms: 0,
            end_ms: sequence.duration_ms,
            duration: sequence.duration_ms as f32 / 1000.0,
            // Bit 0x1 clamps: the sequence clock (`0x82e140`) wraps `elapsed % duration` only when
            // it is clear, and `0x826c40` marks a set one finished at its end. 0x20 is residency.
            looping: sequence.flags & 0x1 == 0,
            move_speed: sequence.move_speed,
            blend_time: sequence.blend_time_ms as f32 / 1000.0,
            bounds_center,
            bounds_radius: sequence.bounds_radius,
            bounds_min: sequence.bounds_min,
            bounds_max: sequence.bounds_max,
            frequency: sequence.frequency.max(0) as u16,
            min_replay: sequence.replay_min,
            max_replay: sequence.replay_max,
            bones,
            events: sequence_events(wrath, sequence_index, sequence.duration_ms),
        });
        all_interpolation.push(interpolation);
    }
    Ok((animations, all_interpolation))
}

/// The event keys one sequence triggers, seconds from its start, in time order. v264 times are
/// already sequence-relative; a key past the band never fires, and a global-sequence event has no
/// sequence timeline.
fn sequence_events(wrath: &WrathM2, sequence: usize, duration_ms: u32) -> Vec<AnimEvent> {
    let mut events = wrath
        .events()
        .iter()
        .filter(|event| event.global_sequence == u16::MAX)
        .flat_map(|event| {
            event
                .times
                .get(sequence)
                .into_iter()
                .flatten()
                .filter(move |&&time| time <= duration_ms)
                .map(move |&time| AnimEvent {
                    time: time as f32 / 1000.0,
                    ident: event.ident,
                    data: event.data,
                    bone: event.bone,
                    position: event.position,
                })
        })
        .collect::<Vec<_>>();
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    events
}

fn sequence_keys<T: Copy>(track: &M2Track<T>, sequence: usize) -> Result<Vec<(f32, T)>> {
    if track.gseq != u16::MAX {
        return Ok(match track.keys.as_slice() {
            [(_, value)] => vec![(0.0, *value)],
            _ => Vec::new(),
        });
    }
    if track.ranges.is_empty() {
        return Ok(track
            .keys
            .iter()
            .map(|&(time, value)| (time as f32 / 1000.0, value))
            .collect());
    }
    let &(start, end) = track
        .ranges
        .get(sequence)
        .with_context(|| format!("missing nested track sequence {sequence}"))?;
    // The v264 parser encodes an empty inner array as `(start, start)`. A following array at
    // that same start owns the key; the empty array must not borrow it through inclusive slicing.
    if track
        .ranges
        .get(sequence + 1)
        .is_some_and(|&(next_start, _)| next_start == start)
    {
        return Ok(Vec::new());
    }
    if end < start || track.keys.is_empty() || start as usize >= track.keys.len() {
        return Ok(Vec::new());
    }
    let end = (end as usize).min(track.keys.len() - 1);
    Ok(track.keys[start as usize..=end]
        .iter()
        .map(|&(time, value)| (time as f32 / 1000.0, value))
        .collect())
}

fn global_sequence_channel<T: Clone>(
    track: &M2Track<T>,
    periods: &[u32],
) -> Result<Option<GlobalSeqChannel<T>>> {
    if track.gseq == u16::MAX || track.keys.len() <= 1 {
        return Ok(None);
    }
    if track.interp != 1 {
        bail!(
            "unsupported Wrath global bone interpolation {} on global sequence {}",
            track.interp,
            track.gseq
        );
    }
    let period_ms = periods
        .get(usize::from(track.gseq))
        .copied()
        .filter(|&period| period > 0)
        .with_context(|| format!("global sequence {} has no positive duration", track.gseq))?;
    Ok(Some(GlobalSeqChannel {
        period_ms,
        keys: track.keys.clone(),
    }))
}

fn global_sequence_bones(wrath: &WrathM2) -> Result<Vec<GlobalSeqBone>> {
    let periods = &wrath.model().global_sequences;
    let mut out = Vec::new();
    for (index, tracks) in wrath.bone_tracks().iter().enumerate() {
        let translation = global_sequence_channel(&tracks.translation, periods)?;
        let rotation = global_sequence_channel(&tracks.rotation, periods)?;
        let scale = global_sequence_channel(&tracks.scaling, periods)?;
        if translation.is_some() || rotation.is_some() || scale.is_some() {
            out.push(GlobalSeqBone {
                bone: u16::try_from(index).context("Wrath bone index exceeds u16")?,
                translation,
                rotation,
                scale,
            });
        }
    }
    Ok(out)
}

/// A flat render box whose every batch is opaque `WHITE1.BLP`, the invisible-trap placeholder
/// (GameObject display 1287) the reference draws at per-instance alpha ~0: it renders nothing.
/// Textured flat decals stay.
fn is_white1_placeholder(bbox_min: [f32; 3], bbox_max: [f32; 3], subs: &[RenderSubmesh]) -> bool {
    if subs.is_empty() {
        return false;
    }
    let flat = (0..3).any(|k| (bbox_max[k] - bbox_min[k]).abs() < 1e-3);
    flat && subs.iter().all(|s| {
        matches!(s.blend, ModelBlend::Opaque)
            && !s.additive
            && s.texture
                .as_deref()
                .is_some_and(|t| t.to_ascii_lowercase().ends_with("white1.blp"))
    })
}

pub(crate) fn bounds(wrath: &WrathM2) -> M2Bounds {
    let model = wrath.model();
    let header = &model.header;
    let vert_max_from_origin = model
        .vertices
        .iter()
        .map(|vertex| {
            let p = vertex.position;
            (p.x * p.x + p.y * p.y + p.z * p.z).sqrt()
        })
        .fold(0.0_f32, f32::max);
    let stand = wrath
        .animations()
        .iter()
        .find(|sequence| sequence.id == 0)
        .or_else(|| wrath.animations().first());
    let (dx, dy, stand_box_z) = stand.map_or_else(
        || {
            (
                header.bounding_box_max[0] - header.bounding_box_min[0],
                header.bounding_box_max[1] - header.bounding_box_min[1],
                (header.bounding_box_max[2] - header.bounding_box_min[2]).max(0.0),
            )
        },
        |sequence| {
            (
                sequence.bounds_max[0] - sequence.bounds_min[0],
                sequence.bounds_max[1] - sequence.bounds_min[1],
                (sequence.bounds_max[2] - sequence.bounds_min[2]).max(0.0),
            )
        },
    );
    let ring_footprint = if dx == 0.0 && dy == 0.0 {
        DEGENERATE_RING_FOOTPRINT
    } else {
        (0.5 * (dx * dx + dy * dy).sqrt()).sqrt()
    };
    let swim_pivot_drop = match (
        wrath.animations().iter().find(|sequence| sequence.id == 0),
        wrath.animations().iter().find(|sequence| sequence.id == 42),
    ) {
        (Some(stand), Some(swim)) => (stand.bounds_max[2] - swim.bounds_max[2]).max(0.0),
        _ => 0.0,
    };
    M2Bounds {
        sphere_radius: header.bounding_sphere_radius,
        bbox_min: header.bounding_box_min,
        bbox_max: header.bounding_box_max,
        vert_max_from_origin,
        ring_footprint,
        pivot_z: model.pivot_attach_z,
        stand_box_z,
        swim_pivot_drop,
    }
}

pub(crate) fn collision(model: &M2Model) -> Result<CollisionMesh> {
    let positions = model
        .raw_data
        .bounding_vertices
        .as_chunks::<12>()
        .0
        .iter()
        .map(|bytes| {
            [
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            ]
        })
        .collect::<Vec<_>>();
    let indices = model
        .raw_data
        .bounding_triangles
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u32::from(u16::from_le_bytes(*bytes)))
        .collect::<Vec<_>>();
    if !indices.len().is_multiple_of(3) {
        bail!("Wrath collision index count is not a triangle list");
    }
    if indices
        .iter()
        .any(|&index| index as usize >= positions.len())
    {
        bail!("Wrath collision triangle references a missing vertex");
    }
    Ok(CollisionMesh { positions, indices })
}

fn render_submeshes(
    wrath: &WrathM2,
    skin: &WrathSkin,
    dir: &str,
    skins: &[Option<String>],
) -> Result<Vec<RenderSubmesh>> {
    let model = wrath.model();
    let seq_slots = wrath
        .animations()
        .iter()
        .enumerate()
        .map(|(index, sequence)| SeqSlot {
            index,
            band: (0, sequence.duration_ms),
            // The authored wrap, as ModelAnimation above (`0x82e140`).
            looping: sequence.flags & 0x1 == 0,
        })
        .collect::<Vec<_>>();
    let seq0 = seq_slots.first().copied();
    let vertex = |global: u32| {
        let vertex = &model.vertices[global as usize];
        (
            [vertex.position.x, vertex.position.y, vertex.position.z],
            [vertex.normal.x, vertex.normal.y, vertex.normal.z],
            [vertex.tex_coords.x, vertex.tex_coords.y],
            [1.0; 4],
        )
    };
    let global_indices = |section: &wrathplus_m2::SkinSection| -> Result<Vec<u32>> {
        let start = usize::from(section.triangle_start);
        let end = start
            .checked_add(usize::from(section.triangle_count))
            .context("Wrath skin section triangle range overflow")?;
        skin.triangles()[start..end]
            .iter()
            .map(|&triangle_index| {
                let global = *skin
                    .indices()
                    .get(usize::from(triangle_index))
                    .context("Wrath skin triangle references a missing lookup index")?;
                if usize::from(global) >= model.vertices.len() {
                    bail!("Wrath skin references missing model vertex {global}");
                }
                Ok(u32::from(global))
            })
            .collect()
    };

    let mut out = Vec::new();
    for batch in skin.batches() {
        // Combined stages remain one batch; a separate blended pass is not equivalent.
        if batch.texture_count > 2 {
            bail!(
                "unsupported Wrath batch with {} texture stages",
                batch.texture_count
            );
        }
        let section = skin
            .submeshes()
            .get(usize::from(batch.skin_section_index))
            .context("Wrath skin batch references a missing section")?;
        let globals = global_indices(section)?;
        if globals.is_empty() {
            continue;
        }
        let texture_record = model
            .raw_data
            .texture_lookup_table
            .get(usize::from(batch.texture_combo_index))
            .and_then(|&index| model.textures.get(usize::from(index)));
        if batch.texture_count != 0 && texture_record.is_none() {
            bail!("Wrath batch texture lookup is missing");
        }
        let (texture, skin_slot, char_slot, icon_slot) = texture_record
            .map(|texture| resolve_texture(texture, dir, skins))
            .unwrap_or((None, None, None, false));
        let (wrap_x, wrap_y) = texture_record.map_or((true, true), |t| (t.wrap_x, t.wrap_y));
        let material = model.materials.get(usize::from(batch.material_index));
        if material.is_none() {
            bail!("Wrath batch material {} is missing", batch.material_index);
        }
        let material = material.unwrap();
        let blend = blend(material.blend_mode.bits())?;
        let additive = matches!(material.blend_mode.bits(), 3 | 4);
        let fog_policy = if material.flags.bits() & 0x02 != 0 {
            FogPolicy::Off
        } else {
            match material.blend_mode.bits() {
                3 | 4 => FogPolicy::Black,
                5 => FogPolicy::White,
                6 => FogPolicy::Grey,
                _ => FogPolicy::Scene,
            }
        };
        let color_track = model.color_alpha_tracks.get(usize::from(batch.color_index));
        let weight_track = (batch.texture_count != 0)
            .then(|| {
                model
                    .transparency_lookup
                    .get(usize::from(batch.weight_combo_index))
                    .and_then(|&index| model.transparency_tracks.get(usize::from(index)))
            })
            .flatten();
        if color_track.and_then(track_constant).unwrap_or(1.0)
            * weight_track.and_then(track_constant).unwrap_or(1.0)
            <= 0.0
        {
            continue;
        }
        let alpha_anim = AlphaAnim::new(
            seq_slots
                .iter()
                .map(|&slot| AlphaSeq {
                    color: color_track.and_then(|track| {
                        mat_anim::bake_scalar_anim(track, &model.global_sequences, Some(slot))
                    }),
                    weight: weight_track.and_then(|track| {
                        mat_anim::bake_scalar_anim(track, &model.global_sequences, Some(slot))
                    }),
                })
                .collect(),
        );
        let uv_anim = tex_anim::bake_uv_anim(model, batch.texture_transform_combo_index, seq0);
        let uv_seq = tex_anim::bake_uv_seqs(model, batch.texture_transform_combo_index, &seq_slots)
            .filter(|loops| loops.uniform().is_none());
        let uv_rot_seq =
            tex_anim::bake_uv_rot_seqs(model, batch.texture_transform_combo_index, &seq_slots);
        let uv_scale_seq =
            tex_anim::bake_uv_scale_seqs(model, batch.texture_transform_combo_index, &seq_slots);
        let rgb_track = model.color_rgb_tracks.get(usize::from(batch.color_index));
        let rgb_anim = rgb_track
            .and_then(|track| mat_anim::bake_rgb_anim(track, &model.global_sequences, seq0));
        let rgb_seq = rgb_track
            .and_then(|track| mat_anim::bake_rgb_seqs(track, &model.global_sequences, &seq_slots))
            .filter(|loops| loops.uniform().is_none());
        let color_tint = match (&rgb_anim, blend) {
            (Some(_), _) | (_, ModelBlend::Mod | ModelBlend::Mod2x) => None,
            _ => rgb_track
                .and_then(|track| track.keys.first())
                .map(|&(_, rgb)| [rgb[0], rgb[1], rgb[2], 1.0]),
        };
        let env_map = model.stage_is_env_mapped(batch, 0);
        let (mut submesh, remapped_globals) = remap_submesh(
            globals.into_iter(),
            vertex,
            texture,
            blend,
            material.flags.bits() & 0x04 != 0,
            false,
            material.flags.bits() & 0x01 != 0,
        );
        if batch.texture_count == 2 {
            let combos = wrath.texture_combiner_combos();
            let mut op = if combos.is_empty() {
                0
            } else {
                *combos
                    .get(usize::from(batch.shader_id) + 1)
                    .context("Wrath second-stage combiner override missing")?
            };
            if op > 7 {
                bail!("unsupported Wrath stage combiner {op}");
            }
            if !combos.is_empty() {
                let first_op = if material.blend_mode.bits() == 0 {
                    0
                } else {
                    *combos
                        .get(usize::from(batch.shader_id))
                        .context("Wrath first-stage combiner override missing")?
                };
                // A Mod2x first stage selects Combiners_Mod_Mod2x (over Mod) or Mod2x_Mod2x
                // (`0x836600`), one program: `t0·t1·primary·2`, alpha `t0.a·primary.a·t1.a·2`,
                // which is a Mod first stage under op 4. An Add first stage over Mod is
                // Combiners_Add_Mod: `t1·(primary + t0)`, alpha `t1.a·(primary.a + t0.a)`, op 8.
                match (first_op, op) {
                    (0 | 1, _) => {}
                    (4, 1 | 4) => op = 4,
                    (3, 1) => op = 8,
                    _ => bail!("unsupported Wrath first-stage combiner {first_op}"),
                }
            }
            // The vertex programs read the stages' coordinates off the environment bits alone
            // (`Shaders\Vertex\arbvp1\Diffuse_{T1_T2,Env_T2,T1_Env}`): stage 2 takes UV1, or UV0
            // after an environment-mapped stage 1, whatever its lookup value.
            let coord = if model.stage_is_env_mapped(batch, 1) {
                *model
                    .texture_unit_lookup
                    .get(usize::from(batch.texture_coord_combo_index) + 1)
                    .context("Wrath second-stage coordinate lookup missing")?
            } else {
                u16::from(!model.stage_is_env_mapped(batch, 0))
            };
            let texture2 = model
                .raw_data
                .texture_lookup_table
                .get(usize::from(batch.texture_combo_index) + 1)
                .and_then(|&i| model.textures.get(usize::from(i)))
                .context("Wrath second-stage texture lookup missing")?;
            let (path, skin, char2, icon2) = resolve_texture(texture2, dir, skins);
            if char2.is_some_and(|slot| slot != CharSkinSlot::Object) || icon2 {
                bail!("unsupported Wrath second-stage character/icon texture");
            }
            submesh.stage2 = Some(super::ModelTextureStage {
                texture: path,
                skin_slot: skin,
                char_slot: char2,
                wrap_x: texture2.wrap_x,
                wrap_y: texture2.wrap_y,
                coord,
                op,
                first_opaque: material.blend_mode.bits() == 0,
                uv_anim: tex_anim::bake_uv_anim(
                    model,
                    batch.texture_transform_combo_index.saturating_add(1),
                    seq0,
                ),
                uv_seq: tex_anim::bake_uv_seqs(
                    model,
                    batch.texture_transform_combo_index.saturating_add(1),
                    &seq_slots,
                ),
                uv_rot_seq: tex_anim::bake_uv_rot_seqs(
                    model,
                    batch.texture_transform_combo_index.saturating_add(1),
                    &seq_slots,
                ),
                uv_scale_seq: tex_anim::bake_uv_scale_seqs(
                    model,
                    batch.texture_transform_combo_index.saturating_add(1),
                    &seq_slots,
                ),
            });
        }
        let second_coord = submesh.stage2.as_ref().map_or(1, |s| s.coord);
        submesh.uvs2 = remapped_globals
            .iter()
            .map(|&i| {
                let v = &model.vertices[i as usize];
                let uv = if second_coord == 1 {
                    v.tex_coords2
                } else {
                    v.tex_coords
                };
                [uv.x, uv.y]
            })
            .collect();
        // A single stage reads UV1 for lookup 1 (`Diffuse_T2`); a two-stage batch's first stage
        // always reads UV0 (`Diffuse_T1_T2`, `Diffuse_T1_Env`).
        if batch.texture_count != 2
            && model
                .texture_unit_lookup
                .get(usize::from(batch.texture_coord_combo_index))
                == Some(&1)
        {
            submesh.uvs = remapped_globals
                .iter()
                .map(|&i| {
                    let uv = model.vertices[i as usize].tex_coords2;
                    [uv.x, uv.y]
                })
                .collect();
        }
        submesh.skin_slot = skin_slot;
        submesh.char_slot = char_slot;
        submesh.icon_slot = icon_slot;
        submesh.geoset_id = section.id;
        submesh.section = Some(batch.skin_section_index);
        submesh.wrap_x = wrap_x;
        submesh.wrap_y = wrap_y;
        submesh.additive = additive;
        submesh.no_depth_write = material.flags.bits() & 0x10 != 0;
        submesh.no_depth_test = material.flags.bits() & 0x08 != 0;
        submesh.fog_policy = fog_policy;
        submesh.env_map = env_map;
        submesh.joints = remapped_globals
            .iter()
            .map(|&global| model.vertices[global as usize].bone_indices.map(u16::from))
            .collect();
        submesh.weights = remapped_globals
            .iter()
            .map(|&global| normalize_weights(model.vertices[global as usize].bone_weights))
            .collect();
        // Wrath billboards stay on the ordinary joint-palette path. Splitting them without also
        // carrying their nested scale/translation tracks would be an inaccurate optimization.
        submesh.welded_billboard = remapped_globals.iter().any(|&global| {
            let vertex = &model.vertices[global as usize];
            vertex
                .bone_indices
                .iter()
                .zip(vertex.bone_weights)
                .any(|(&bone, weight)| {
                    weight != 0
                        && model
                            .bones
                            .get(usize::from(bone))
                            .is_some_and(|bone| bone.is_billboard())
                })
        });
        if let Some(color) = color_tint {
            submesh.vertex_colors = vec![color; submesh.positions.len()];
        } else {
            submesh.vertex_colors.clear();
        }
        submesh.alpha_anim = alpha_anim;
        submesh.uv_anim = uv_anim;
        submesh.uv_seq = uv_seq;
        submesh.uv_rot_seq = uv_rot_seq;
        submesh.uv_scale_seq = uv_scale_seq;
        submesh.rgb_anim = rgb_anim;
        submesh.rgb_seq = rgb_seq;
        out.push(submesh);
    }
    Ok(out)
}

pub(super) fn resolve_texture(
    texture: &M2Texture,
    dir: &str,
    skins: &[Option<String>],
) -> (Option<String>, Option<u8>, Option<CharSkinSlot>, bool) {
    let embedded = {
        let filename = texture.filename.string.to_string_lossy();
        (!filename.is_empty()).then(|| filename.into_owned())
    };
    let variation = |index: usize| {
        skins
            .get(index)
            .and_then(Clone::clone)
            .map(|name| format!("{dir}\\{name}.blp"))
    };
    let char_slot = match texture.texture_type {
        M2TextureType::Other(1) => Some(CharSkinSlot::Body),
        M2TextureType::Other(2) => Some(CharSkinSlot::Object),
        M2TextureType::Other(6) => Some(CharSkinSlot::Hair),
        M2TextureType::Other(8) => Some(CharSkinSlot::SkinExtra),
        _ => None,
    };
    let icon_slot = matches!(texture.texture_type, M2TextureType::Other(14));
    match texture.texture_type {
        M2TextureType::Monster1 => (variation(0).or(embedded), Some(0), None, false),
        M2TextureType::Monster2 => (variation(1).or(embedded), Some(1), None, false),
        M2TextureType::Monster3 => (variation(2).or(embedded), Some(2), None, false),
        _ => (embedded, None, char_slot, icon_slot),
    }
}

fn blend(mode: u16) -> Result<ModelBlend> {
    Ok(match mode {
        0 => ModelBlend::Opaque,
        1 => ModelBlend::AlphaTest,
        2..=4 => ModelBlend::Blend,
        5 => ModelBlend::Mod,
        6 => ModelBlend::Mod2x,
        other => bail!("unsupported Wrath M2 blend mode {other}"),
    })
}

fn track_constant(track: &wrathplus_m2::M2ScalarTrack) -> Option<f32> {
    if track.keys.is_empty() {
        return Some(1.0);
    }
    // A sequence that keys nothing plays the default 1.0, so a constant elsewhere is not static.
    let constant = track.constant()?;
    (!super::key_anim::has_unkeyed_sequence(track) || constant == 1.0).then_some(constant)
}

fn normalize_weights(weights: [u8; 4]) -> [f32; 4] {
    let sum = weights.iter().map(|&weight| f32::from(weight)).sum::<f32>();
    if sum == 0.0 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    weights.map(|weight| f32::from(weight) / sum)
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

trait CameraValue: Copy {
    fn linear(a: Self, b: Self, t: f32) -> Self;
    fn hermite(a: Self, out_a: Self, b: Self, in_b: Self, t: f32) -> Self;
    fn bezier(a: Self, out_a: Self, b: Self, in_b: Self, t: f32) -> Self;
}

impl CameraValue for f32 {
    fn linear(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }
    fn hermite(a: Self, out_a: Self, b: Self, in_b: Self, t: f32) -> Self {
        let t2 = t * t;
        let t3 = t2 * t;
        a * (2.0 * t3 - 3.0 * t2 + 1.0)
            + out_a * (t3 - 2.0 * t2 + t)
            + b * (-2.0 * t3 + 3.0 * t2)
            + in_b * (t3 - t2)
    }
    fn bezier(a: Self, out_a: Self, b: Self, in_b: Self, t: f32) -> Self {
        let u = 1.0 - t;
        u * u * u * a + 3.0 * u * u * t * out_a + 3.0 * u * t * t * in_b + t * t * t * b
    }
}

impl CameraValue for [f32; 3] {
    fn linear(a: Self, b: Self, t: f32) -> Self {
        std::array::from_fn(|axis| f32::linear(a[axis], b[axis], t))
    }
    fn hermite(a: Self, out_a: Self, b: Self, in_b: Self, t: f32) -> Self {
        std::array::from_fn(|axis| f32::hermite(a[axis], out_a[axis], b[axis], in_b[axis], t))
    }
    fn bezier(a: Self, out_a: Self, b: Self, in_b: Self, t: f32) -> Self {
        std::array::from_fn(|axis| f32::bezier(a[axis], out_a[axis], b[axis], in_b[axis], t))
    }
}

fn sample_track<V: CameraValue>(
    track: &wrathplus_m2::wrath::WrathTrack<V>,
    sequence_index: usize,
    time_ms: u32,
    global_time_ms: u32,
) -> Result<Option<V>> {
    let keys = if track.global_sequence == u16::MAX {
        track
            .sequences
            .get(sequence_index)
            .with_context(|| format!("camera track has no nested sequence {sequence_index}"))?
    } else {
        track
            .sequences
            .first()
            .context("global camera track has no outer array")?
    };
    let Some(first) = keys.first() else {
        return Ok(None);
    };
    if keys.len() == 1 {
        return Ok(Some(first.value));
    }
    let time = if track.global_sequence == u16::MAX {
        time_ms
    } else {
        global_time_ms
    };
    if time <= first.timestamp_ms {
        return Ok(Some(first.value));
    }
    let index = keys
        .partition_point(|key| key.timestamp_ms <= time)
        .saturating_sub(1);
    let Some(next) = keys.get(index + 1) else {
        return Ok(Some(keys[index].value));
    };
    let current = &keys[index];
    let span = next.timestamp_ms.saturating_sub(current.timestamp_ms);
    let t = if span == 0 {
        0.0
    } else {
        (time - current.timestamp_ms) as f32 / span as f32
    };
    Ok(Some(match track.interpolation {
        0 => current.value,
        1 => V::linear(current.value, next.value, t),
        // The reference's cubic dispatch (`0x716b51`): 2 is Bézier over {value, outTan, inTan,
        // value} (`0x716c41`), what every `Cameras\*.m2` fly-by authors; 3 is Hermite (`0x716b9e`).
        // These were swapped, so fly-bys read their absolute control points as velocities and
        // looped hundreds of yards between keys (the Night Elf intro through trees and buildings).
        2 => V::bezier(
            current.value,
            current
                .out_tangent
                .context("Bézier camera key has no outgoing tangent")?,
            next.value,
            next.in_tangent
                .context("Bézier camera key has no incoming tangent")?,
            t,
        ),
        3 => V::hermite(
            current.value,
            current
                .out_tangent
                .context("Hermite camera key has no outgoing tangent")?,
            next.value,
            next.in_tangent
                .context("Hermite camera key has no incoming tangent")?,
            t,
        ),
        mode => bail!("unsupported Wrath camera interpolation mode {mode}"),
    }))
}

fn sample_vec3_track(
    track: &wrathplus_m2::wrath::WrathTrack<[f32; 3]>,
    sequence_index: usize,
    time_ms: u32,
    global_time_ms: u32,
) -> Result<Option<[f32; 3]>> {
    sample_track(track, sequence_index, time_ms, global_time_ms)
}

fn sample_scalar_track(
    track: &wrathplus_m2::wrath::WrathTrack<f32>,
    sequence_index: usize,
    time_ms: u32,
    global_time_ms: u32,
) -> Result<Option<f32>> {
    sample_track(track, sequence_index, time_ms, global_time_ms)
}

#[cfg(test)]
#[path = "wrath_tests.rs"]
mod tests;
