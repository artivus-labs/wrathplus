//! The retained pass's bake: the flush turns each dirty, quiet cell or region into one recentred
//! mesh and a per-item draw list ([`bake_cell`]).

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use wrathplus_assets::coords::wow_to_bevy;

use super::{
    render, GxItem, StaticGx, ATTRIBUTE_GX_ANCHOR, ATTRIBUTE_GX_WORD, IDLE_FRAMES,
    MAX_DIRTY_FRAMES, REBAKE_FRAMES,
};
use super::{
    WORD_CLASS_INT, WORD_CLASS_TRANS, WORD_FOG_OFF, WORD_HAS_VC, WORD_INTERIOR, WORD_MATTE,
    WORD_SHADE_LIT, WORD_TEXTURED, WORD_UNIFIED, WORD_UNLIT, WORD_WINDOW, WORD_WMO, WORD_WRAP_X,
    WORD_WRAP_Y,
};

/// Print the declined-batch census, beside the accepted count, once the counts have sat still for
/// [`CENSUS_SETTLE_FRAMES`], or after [`CENSUS_MAX_HOLD_FRAMES`] if they never do.
fn census_declines(gx: &mut StaticGx, frame: u32) {
    if gx.declined == gx.declined_logged {
        return;
    }
    if gx.declined != gx.declined_seen {
        gx.declined_seen = gx.declined;
        gx.declined_changed = frame;
    }
    let settled = frame.wrapping_sub(gx.declined_changed) >= CENSUS_SETTLE_FRAMES;
    let overdue = frame.wrapping_sub(gx.declined_printed) >= CENSUS_MAX_HOLD_FRAMES;
    if !settled && !overdue {
        return;
    }
    gx.declined_printed = frame;
    let d = gx.declined;
    info!(
        "static-gx: {} batch(es) accepted; declined — env-map {}, depth-flag {}, shade-family {}, \
         prop-fader {} (batches), prop-no-instance {} (props)",
        gx.accepted, d[0], d[1], d[2], d[3], d[4]
    );
    gx.declined_logged = d;
}

/// Frames the counts sit still before the census prints (~1 s), longer than a burst's gaps.
const CENSUS_SETTLE_FRAMES: u32 = 60;

/// The ceiling on that hold (~10 s), so counts that never settle, as on a flight path, still print.
const CENSUS_MAX_HOLD_FRAMES: u32 = 600;

/// Bake dirty-and-quiet cells into retained draw data; publish into [`render::GxWorld`].
pub(super) fn flush_static_gx(mut gx: ResMut<StaticGx>, mut meshes: ResMut<Assets<Mesh>>) {
    let _t = super::gx_perf_guard(0);
    gx.frame = gx.frame.wrapping_add(1);
    let frame = gx.frame;
    census_declines(&mut gx, frame);
    // A `StaticGx::flush_now` request bakes one flush's worth; the windows resume next frame.
    let now = std::mem::take(&mut gx.flush_now);
    let StaticGx {
        cells,
        wmos,
        props,
        world,
        ..
    } = &mut *gx;
    for (&cell, state) in cells.iter_mut() {
        if !bake_due(state, world.cells.contains_key(&cell), frame, now) {
            continue;
        }
        state.dirty = false;
        if state.items.is_empty() {
            world.cells.remove(&cell);
            continue;
        }
        // Sorted by (bucket, texture) so the node draws one range per run, not one per item.
        state
            .items
            .sort_by_key(|i| ((u8::from(i.cutout) << 1) | u8::from(i.two_sided), i.texture));
        remap_fader_items(state);
        let baked = bake_cell(&state.items, &mut meshes);
        debug!(
            "static-gx: cell ({},{}) baked — {} item(s), {} vert(s)",
            cell.0,
            cell.1,
            state.items.len(),
            baked.draws.last().map_or(0, |d| d.vertex_range.end),
        );
        world.cells.insert(cell, std::sync::Arc::new(baked));
    }
    for (&instance, state) in wmos.iter_mut() {
        if !bake_due(state, world.wmos.contains_key(&instance), frame, now) {
            continue;
        }
        state.dirty = false;
        if state.items.is_empty() {
            world.wmos.remove(&instance);
            continue;
        }
        // The group joins the key: the PVS selects per group, so a run must not cross one.
        state.items.sort_by_key(|i| {
            (
                (u8::from(i.cutout) << 1) | u8::from(i.two_sided),
                i.texture,
                i.wmo.as_ref().map_or(0, |w| w.group),
            )
        });
        let baked = bake_cell(&state.items, &mut meshes);
        debug!(
            "static-gx: wmo {instance} baked — {} item(s), {} group(s), {} vert(s)",
            state.items.len(),
            baked.groups.len(),
            baked.draws.last().map_or(0, |d| d.vertex_range.end),
        );
        world.wmos.insert(instance, std::sync::Arc::new(baked));
    }
    // Prop regions: the referrer set is the selection key, and the draw carries the set list.
    for (&instance, state) in props.iter_mut() {
        if !bake_due(state, world.props.contains_key(&instance), frame, now) {
            continue;
        }
        state.dirty = false;
        if state.items.is_empty() {
            world.props.remove(&instance);
            continue;
        }
        state.items.sort_by_key(|i| {
            (
                (u8::from(i.cutout) << 1) | u8::from(i.two_sided),
                i.texture,
                i.prop.as_ref().map_or(0, |p| p.set),
            )
        });
        let mut baked = bake_cell(&state.items, &mut meshes);
        baked.sets = state.sets.clone();
        debug!(
            "static-gx: props {instance} baked — {} item(s), {} set(s), {} vert(s)",
            state.items.len(),
            baked.sets.len(),
            baked.draws.last().map_or(0, |d| d.vertex_range.end),
        );
        world.props.insert(instance, std::sync::Arc::new(baked));
    }
}

/// Whether a dirty region bakes now: after the short quiet window first, the long one once
/// published, at the age cap, or on `now` ([`StaticGx::flush_now`]).
fn bake_due(state: &super::GxCell, published: bool, frame: u32, now: bool) -> bool {
    if !state.dirty {
        return false;
    }
    if now {
        return true;
    }
    let window = if published {
        REBAKE_FRAMES
    } else {
        IDLE_FRAMES
    };
    frame.wrapping_sub(state.last_change) >= window
        || frame.wrapping_sub(state.dirty_since) >= MAX_DIRTY_FRAMES
}

/// Remap each fader's kill targets to the post-sort item order and mark the bitmap stale. The
/// scan runs next in the same chain, so no frame draws with a previous bake's indices.
fn remap_fader_items(state: &mut super::GxCell) {
    if state.faders.is_empty() {
        return;
    }
    for f in state.faders.values_mut() {
        f.items.clear();
    }
    for (idx, item) in state.items.iter().enumerate() {
        if let Some(uid) = item.fader {
            if let Some(f) = state.faders.get_mut(&uid) {
                f.items
                    .push(u16::try_from(idx).expect("gx cell under u16 items"));
            }
        }
    }
    state.bits_stale = true;
}

/// Bake one cell's or region's recentred mesh, per-item draws and per-selection-key bounds.
fn bake_cell(items: &[GxItem], meshes: &mut Assets<Mesh>) -> render::GxCellDraw {
    use bevy::math::DVec3;
    // World positions stay f64 until recentred: an f32 coordinate near 9,000 yd rounds by up to
    // ~0.0005 yd, half an indoor pixel, which would shift every interpolant.
    let mut positions64: Vec<DVec3> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut words: Vec<u32> = Vec::new();
    let mut anchors: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut draws: Vec<render::GxItemDraw> = Vec::new();
    let (mut mn, mut mx) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
    let mut group_bounds: Vec<(u16, DVec3, DVec3)> = Vec::new();
    for (item_idx, item) in items.iter().enumerate() {
        let sub = &item.geometry;
        let base = u32::try_from(positions64.len()).expect("gx cell under u32 vertices");
        // Low 16 bits: the item index, which the render side's record table maps to a layer.
        let has_vc = sub.vertex_colors.len() == sub.positions.len();
        let word_flags = u32::try_from(item_idx).expect("gx cell under u16 items")
            | (u32::from(item.wrap_x) * WORD_WRAP_X)
            | (u32::from(item.wrap_y) * WORD_WRAP_Y)
            | (u32::from(item.unlit) * WORD_UNLIT)
            | (u32::from(item.fog_off) * WORD_FOG_OFF)
            | (u32::from(item.shade_lit) * WORD_SHADE_LIT)
            | (u32::from(item.matte) * WORD_MATTE)
            | (u32::from(item.texture.is_some()) * WORD_TEXTURED)
            | (u32::from(has_vc) * WORD_HAS_VC)
            // An interior prop is WORD_INTERIOR without WORD_WMO; a slot-less prop keeps the
            // exterior law, as on the entity path.
            | item.prop.as_ref().map_or(0, |p| {
                u32::from(p.slot.is_some()) * WORD_INTERIOR
            })
            | item.wmo.as_ref().map_or(0, |w| {
                WORD_WMO
                    | (u32::from(w.interior) * WORD_INTERIOR)
                    | (u32::from(w.class_lane == 1) * WORD_CLASS_INT)
                    | (u32::from(w.class_lane == 2) * WORD_CLASS_TRANS)
                    | (u32::from(w.window) * WORD_WINDOW)
                    | (u32::from(w.wmo_unified.is_some()) * WORD_UNIFIED)
            });
        let anchor = item.transform.translation;
        let rot = item.transform.rotation;
        // Scale, rotate, translate, as the placement transform does. Normals take the rotation
        // alone (placement scale is uniform); a zero normal stays zero for `wow_normalize`.
        let rot64 = rot.as_dquat();
        let scale64 = item.transform.scale.as_dvec3();
        let t64 = item.transform.translation.as_dvec3();
        let has_normals = sub.normals.len() == sub.positions.len();
        let (mut gmn, mut gmx) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
        for (vi, p) in sub.positions.iter().enumerate() {
            let w = rot64 * (wow_to_bevy(*p).as_dvec3() * scale64) + t64;
            mn = mn.min(w);
            mx = mx.max(w);
            gmn = gmn.min(w);
            gmx = gmx.max(w);
            positions64.push(w);
            let n = if has_normals {
                (rot * wow_to_bevy(sub.normals[vi])).normalize_or_zero()
            } else {
                Vec3::Y
            };
            normals.push(n.to_array());
            uvs.push(*sub.uvs.get(vi).unwrap_or(&[0.0, 0.0]));
            // MOCV or the baked constant tint, raw as on the entity mesh; white where none.
            colors.push(if has_vc {
                sub.vertex_colors[vi]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            });
            words.push(word_flags);
            anchors.push(anchor.to_array());
        }
        // The selection key: a WMO item's group or a prop item's referrer set, selected alike.
        let sel_group = item
            .wmo
            .as_ref()
            .map(|w| w.group)
            .or(item.prop.as_ref().map(|p| p.set));
        if let Some(g) = sel_group {
            match group_bounds.iter_mut().find(|(gb, _, _)| *gb == g) {
                Some((_, bmn, bmx)) => {
                    *bmn = bmn.min(gmn);
                    *bmx = bmx.max(gmx);
                }
                None => group_bounds.push((g, gmn, gmx)),
            }
        }
        let start = u32::try_from(indices.len()).expect("gx cell under u32 indices");
        indices.extend(sub.indices.iter().map(|i| base + i));
        draws.push(render::GxItemDraw {
            index_range: start..u32::try_from(indices.len()).unwrap(),
            texture: item.texture,
            cutout: item.cutout,
            two_sided: item.two_sided,
            vertex_range: base..u32::try_from(positions64.len()).unwrap(),
            group: sel_group,
            order: item.wmo.as_ref().map_or(0, |w| w.order),
            sidn: item.wmo.as_ref().map_or([0; 3], |w| w.sidn),
            wmo_unified: item.wmo.as_ref().and_then(|w| w.wmo_unified),
            slot: item.prop.as_ref().and_then(|p| p.slot).unwrap_or(0),
        });
    }
    // The shader rebuilds world = v + origin. The origin is the f32-rounded centre and the
    // subtraction runs in f64 against it, so only the small recentred value is quantized.
    let center = (mn + mx) * 0.5;
    let origin = center.as_vec3();
    let origin64 = origin.as_dvec3();
    let positions: Vec<[f32; 3]> = positions64
        .iter()
        .map(|w| (*w - origin64).as_vec3().to_array())
        .collect();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_attribute(ATTRIBUTE_GX_WORD, words);
    mesh.insert_attribute(ATTRIBUTE_GX_ANCHOR, anchors);
    mesh.insert_indices(Indices::U32(indices));
    // All-zero; the flush marks a fader cell `bits_stale`, so the scan fills it before publish.
    let killed = vec![0u64; draws.len().div_ceil(64)];
    render::GxCellDraw {
        mesh: meshes.add(mesh),
        origin,
        aabb: Aabb::from_min_max((mn - origin64).as_vec3(), (mx - origin64).as_vec3()),
        draws,
        groups: group_bounds
            .into_iter()
            .map(|(g, bmn, bmx)| {
                (
                    g,
                    Aabb::from_min_max((bmn - origin64).as_vec3(), (bmx - origin64).as_vec3()),
                )
            })
            .collect(),
        sets: Vec::new(), // filled by the flush for a prop region
        killed,
        killed_rev: 0,
    }
}

#[cfg(test)]
#[path = "bake_tests.rs"]
mod tests;
