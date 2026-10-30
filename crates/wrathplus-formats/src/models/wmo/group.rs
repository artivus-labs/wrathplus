//! WMO group-file parsing: the MOGP header and sub-chunks, and the render batches built into
//! [`RenderSubmesh`]es against a parsed root.

use std::io::Cursor;

use anyhow::Result;
use wrathplus_wmo::{parse_wmo, Color, ParsedWmo, WmoGroup, WmoLiquid};

use super::{find_wmo_chunk, WmoRoot};
use crate::liquid::{LiquidKind, LiquidMesh};
use crate::models::{remap_submesh, ModelBlend, RenderSubmesh, WmoBatchClass};

/// MLIQ grid spacing in yards, the reference's `[0x810cc0]` (`0x40855555`): the water
/// (`0x6b6420`/`0x6b6630`) and magma/slime (`0x6b68f0`) kernels both place vertex `(i, j)` at
/// `base + (i·STEP, j·STEP)`.
const MLIQ_CELL_STEP: f32 = f32::from_bits(0x4085_5555);

/// Yards per water-texture repeat: one per grid cell, as the reference's water kernel `0x6b6630`
/// writes `u = (float)i`, `v = (float)j`. Deviation: the UV comes from the model-space position,
/// not each surface's own `(i, j)`, because the two agree on cell-aligned surfaces and the position
/// never seams where they are not. Magma and slime use this period too; the reference reads their
/// UV from the vertex's authored `int16` pair × 1/256 (`0x6b6993`), which is not parsed.
const MLIQ_UV_PERIOD: f32 = MLIQ_CELL_STEP;

/// A WMO water vertex's opacity-ramp coordinate, its authored byte 0 normalised: the reference
/// indexes a linear LUT between the zone's river shallow and deep alphas with it (`0x6b64c6`,
/// built by `0x6b6b60`), not the ADT `1.6·(i/63)^8` curve, and the shader's lerp reproduces it.
fn wmo_water_alpha_v(opacity_byte: u8) -> f32 {
    f32::from(opacity_byte) / 255.0
}

/// Legacy (5875) MOGP `groupLiquid` no-liquid sentinel, used only by the legacy APIs.
/// Native Wrath must use [`wmo_group_wrath_liquid`] and MOHD.flags instead.
pub const NO_GROUP_LIQUID: u32 = 0xf;

/// A group's MOGP header fields. `flags` sits at `0x08` and the portal-ref slice at `0x24`/`0x26`,
/// which the reference copies to `grp+0x10` and `+0x2c`/`+0x30`.
#[derive(Debug, Clone, Copy)]
pub struct WmoGroupHeader {
    /// `0x8` is exterior, the bit the portal flood defers on; `& 0x48 == 0` is interior.
    pub flags: u32,
    /// First index of this group's portals in [`super::WmoPortals::refs`].
    pub portal_ref_start: u16,
    pub portal_ref_count: u16,
    /// MOGP `uniqueID` at `0x38`, the group's `WMOAreaTable.WMOGroupID` key; 0 for a short header.
    pub area_table_id: u32,
    /// MOGP fog indices at `0x30` into the root's [`super::WmoFog`] records, walked by the
    /// reference's interior-fog selector `0x69de20`; the offset is fixed empirically
    /// (`tests/wmo_fogs.rs`). Zero if the header is short.
    pub fog_indices: [u8; 4],
    /// Raw MOGP `groupLiquid` at `0x34`, not a resolved Wrath liquid ID. The legacy
    /// 5875 whole-group override uses [`NO_GROUP_LIQUID`]; native Wrath interpretation
    /// requires root flags and [`resolve_wrath_wmo_liquid_id`]. A short legacy header
    /// reads as [`NO_GROUP_LIQUID`]; the Wrath decoder rejects missing context instead.
    pub group_liquid: u32,
}

/// Read a group file's [`WmoGroupHeader`]; the MOGP payload opens with the 68-byte header.
pub fn wmo_group_header(group_bytes: &[u8]) -> Option<WmoGroupHeader> {
    let mogp = find_wmo_chunk(group_bytes, b"PGOM")?; // MOGP
    if mogp.len() < 0x28 {
        return None;
    }
    let u32_at = |i: usize| {
        (mogp.len() >= i + 4)
            .then(|| u32::from_le_bytes([mogp[i], mogp[i + 1], mogp[i + 2], mogp[i + 3]]))
    };
    Some(WmoGroupHeader {
        flags: u32::from_le_bytes([mogp[8], mogp[9], mogp[10], mogp[11]]),
        portal_ref_start: u16::from_le_bytes([mogp[0x24], mogp[0x25]]),
        portal_ref_count: u16::from_le_bytes([mogp[0x26], mogp[0x27]]),
        area_table_id: u32_at(0x38).unwrap_or(0),
        fog_indices: if mogp.len() >= 0x34 {
            [mogp[0x30], mogp[0x31], mogp[0x32], mogp[0x33]]
        } else {
            [0; 4]
        },
        group_liquid: u32_at(0x34).unwrap_or(NO_GROUP_LIQUID),
    })
}

/// Native Wrath resolver (0x007D7310), including the loader's MOHD bit-4 pretransform.
/// Even direct mode maps inputs 1..=20; 0 and IDs >=21 pass through unchanged.
pub fn resolve_wrath_wmo_liquid_id(root_flags: u32, group_flags: u32, raw: u32) -> u32 {
    let n = if root_flags & 4 != 0 {
        raw
    } else if raw == 15 {
        0
    } else {
        raw.wrapping_add(1)
    };
    resolve_wrath_liquid_class(group_flags, n)
}

fn resolve_wrath_liquid_class(group_flags: u32, n: u32) -> u32 {
    if n == 0 || n >= 21 {
        return n;
    }
    match (n - 1) & 3 {
        0 if group_flags & 0x80000 != 0 => 14,
        0 => 13,
        1 => 14,
        2 => 19,
        _ => 20,
    }
}

/// Native grid presence, independent of whether the legacy mesh builder emits triangles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrathWmoLiquidGrid {
    /// No flagged MLIQ path: native dimensions remain zero (even if stray MLIQ bytes exist).
    Absent,
    /// A valid flagged MLIQ with at least one zero tile dimension.
    ZeroSized,
    /// A valid nonzero grid. All holes still means bounded queries, never a whole-group flood.
    Tiles { all_holes: bool },
}

/// Native query mode (0x007C8360), not a rendering decision or liquid-category mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrathWmoLiquidMode {
    Dry,
    /// Native query succeeds with height f32::MAX, without sampling tiles.
    WholeGroupFlood,
    /// Query must sample bounded tiles/heights and reject low-nibble-15 holes.
    Grid,
}

/// Decoded Wrath loader/query state; malformed data returns an error rather than a false flood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrathWmoLiquidState {
    pub resolved_id: u32,
    /// Native xTiles/yTiles, not vertex dimensions or the presence of a LiquidMesh.
    pub tile_dimensions: [u32; 2],
    pub grid: WrathWmoLiquidGrid,
}

impl WrathWmoLiquidState {
    pub fn mode(&self) -> WrathWmoLiquidMode {
        if self.resolved_id == 0 {
            WrathWmoLiquidMode::Dry
        } else if self.tile_dimensions.contains(&0) {
            WrathWmoLiquidMode::WholeGroupFlood
        } else {
            WrathWmoLiquidMode::Grid
        }
    }

    pub fn is_whole_group_flood(&self) -> bool {
        self.mode() == WrathWmoLiquidMode::WholeGroupFlood
    }
}

/// Decode the native Wrath liquid loader using retained MOHD context. Legacy mesh APIs are
/// deliberately unchanged. Flagged missing/truncated/overflowing MLIQ is an error, including
/// when the header ID is nonzero; unflagged MLIQ is ignored as in 0x007D7C30.
pub fn wmo_group_wrath_liquid(group_bytes: &[u8], root: &WmoRoot) -> Result<WrathWmoLiquidState> {
    decode_wrath_wmo_liquid(group_bytes, root.flags()?)
}

fn decode_wrath_wmo_liquid(group_bytes: &[u8], root_flags: u32) -> Result<WrathWmoLiquidState> {
    let mogp = find_wmo_chunk(group_bytes, b"PGOM")
        .ok_or_else(|| anyhow::anyhow!("Wrath WMO liquid decoder: missing MOGP"))?;
    anyhow::ensure!(
        mogp.len() >= 0x44,
        "Wrath WMO liquid decoder: truncated MOGP header"
    );
    let u = |b: &[u8], i| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let flags = u(mogp, 8);
    let mut state = WrathWmoLiquidState {
        resolved_id: resolve_wrath_wmo_liquid_id(root_flags, flags, u(mogp, 0x34)),
        tile_dimensions: [0, 0],
        grid: WrathWmoLiquidGrid::Absent,
    };
    if flags & 0x1000 == 0 {
        return Ok(state);
    }
    // Unlike the legacy clamping finder, reject a declared payload extending beyond EOF.
    let mut off = 0x44usize;
    let mliq = loop {
        anyhow::ensure!(
            mogp.len().saturating_sub(off) >= 8,
            "Wrath WMO liquid decoder: flagged MLIQ missing or truncated chunk header"
        );
        let size = u(mogp, off + 4) as usize;
        let start = off + 8;
        let end = start
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("Wrath WMO liquid decoder: MLIQ chunk size overflow"))?;
        anyhow::ensure!(
            end <= mogp.len(),
            "Wrath WMO liquid decoder: truncated flagged MLIQ/subchunk payload"
        );
        if &mogp[off..off + 4] == b"QILM" {
            break &mogp[start..end];
        }
        off = end;
    };
    anyhow::ensure!(
        mliq.len() >= 30,
        "Wrath WMO liquid decoder: truncated MLIQ header"
    );
    let [xv, yv, xt, yt] = [u(mliq, 0), u(mliq, 4), u(mliq, 8), u(mliq, 12)];
    let tile_start = (xv as usize)
        .checked_mul(yv as usize)
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_add(30))
        .ok_or_else(|| anyhow::anyhow!("Wrath WMO liquid decoder: MLIQ vertex size overflow"))?;
    let count = (xt as usize)
        .checked_mul(yt as usize)
        .ok_or_else(|| anyhow::anyhow!("Wrath WMO liquid decoder: MLIQ tile count overflow"))?;
    let end = tile_start
        .checked_add(count)
        .ok_or_else(|| anyhow::anyhow!("Wrath WMO liquid decoder: MLIQ tile size overflow"))?;
    anyhow::ensure!(
        end <= mliq.len(),
        "Wrath WMO liquid decoder: truncated MLIQ vertices/tiles"
    );
    // Bounded by xTiles*yTiles, never trailing bytes in the chunk.
    let first_wet = mliq[tile_start..end]
        .iter()
        .map(|b| b & 15)
        .find(|&n| n != 15);
    if state.resolved_id == 0 {
        state.resolved_id =
            resolve_wrath_liquid_class(flags, first_wet.map_or(0, |n| u32::from(n) + 1));
    }
    state.tile_dimensions = [xt, yt];
    state.grid = if xt == 0 || yt == 0 {
        WrathWmoLiquidGrid::ZeroSized
    } else {
        WrathWmoLiquidGrid::Tiles {
            all_holes: first_wet.is_none(),
        }
    };
    Ok(state)
}

/// An interior group's render triangles, per-vertex MOCV and per-triangle MOPY: the faces a game
/// object's light attach (`0x6717d0` → `0x69e4c0`) down-rays, taking the hit's barycentric MOCV
/// (floor 168, cap 96) as its light.
#[derive(Clone)]
pub struct FootprintTris {
    /// Vertex positions, WMO model space.
    pub positions: Vec<[f32; 3]>,
    /// Triangle list in MOVI order; the MOPY vectors run parallel, one entry per triangle.
    pub indices: Vec<u16>,
    /// Per-vertex MOCV RGB, kept as bytes because the floor/cap arithmetic is exact u8 fixed-point.
    pub mocv: Vec<[u8; 3]>,
    /// Per-triangle MOPY flags; `0x1` on the hit face selects the day/night exterior colours
    /// instead of the MOCV sample (`0x6b9a50`).
    pub mopy_flags: Vec<u8>,
    /// Per-triangle MOPY material id, an index into the root's MOMT; the reference's footstep ray
    /// reads it off the hit face (`0x6a26fc`). The on-disk `0xFF` non-index occurs only on faces
    /// [`FOOTPRINT_REJECT`] drops; consumers still bounds-check.
    pub mopy_material: Vec<u8>,
}

/// The footprint ray's MOPY reject mask (the reference's BSP walk `0x6b92b0`/`0x6bc370`): a
/// collision (`0x08`) or visited (`0x80`) face is never a candidate, so of a floor's coplanar
/// render and collision sheets only the render one is sampled.
const FOOTPRINT_REJECT: u8 = 0x88;

/// A group's [`FootprintTris`], `None` unless it is an interior group with MOCV parallel to its
/// positions. Rejected faces drop at build: the set is static, so this equals the per-ray reject.
pub fn wmo_group_footprint_tris(group_bytes: &[u8]) -> Option<FootprintTris> {
    let Ok(ParsedWmo::Group(group)) = parse_wmo(&mut Cursor::new(group_bytes)) else {
        return None;
    };
    if group.flags & 0x48 != 0 || group.vertex_colors.len() != group.vertex_positions.len() {
        return None;
    }
    let mut indices = Vec::new();
    let mut mopy_flags = Vec::new();
    let mut mopy_material = Vec::new();
    for (ti, tri) in group.vertex_indices.as_chunks::<3>().0.iter().enumerate() {
        let flags = group.material_info.get(ti).map_or(0, |m| m.flags);
        if flags & FOOTPRINT_REJECT != 0 {
            continue;
        }
        indices.extend_from_slice(tri);
        mopy_flags.push(flags);
        mopy_material.push(group.material_info.get(ti).map_or(0xFF, |m| m.material_id));
    }
    Some(FootprintTris {
        positions: group
            .vertex_positions
            .iter()
            .map(|v| [v.x, v.y, v.z])
            .collect(),
        indices,
        mocv: group
            .vertex_colors
            .iter()
            .map(|c| [c.r, c.g, c.b])
            .collect(),
        mopy_flags,
        mopy_material,
    })
}

/// Find a MOGP sub-chunk by its reversed magic, past the `0x44`-byte header. The last one clamps to
/// the payload end as in [`find_wmo_chunk`], since a clamped MOGP leaves its last sub-chunk short.
fn find_mogp_subchunk<'a>(group_bytes: &'a [u8], magic: &[u8; 4]) -> Option<&'a [u8]> {
    let mogp = find_wmo_chunk(group_bytes, b"PGOM")?;
    let mut off = 0x44usize; // past the fixed group header
    while off + 8 <= mogp.len() {
        let size = u32::from_le_bytes([mogp[off + 4], mogp[off + 5], mogp[off + 6], mogp[off + 7]])
            as usize;
        let data_start = off + 8;
        let data_end = data_start.saturating_add(size).min(mogp.len());
        if &mogp[off..off + 4] == magic {
            return Some(&mogp[data_start..data_end]);
        }
        off = data_end;
    }
    None
}

/// A group's MODR list, the MODD indices it owns: the reference creates one doodad per (index,
/// group) from it (`0x695aa0`), lit by that group's interior class and MOLR lights.
pub fn wmo_group_doodad_refs(group_bytes: &[u8]) -> Vec<u16> {
    find_mogp_subchunk(group_bytes, b"RDOM")
        .map(|c| {
            c.as_chunks::<2>()
                .0
                .iter()
                .map(|r| u16::from_le_bytes([r[0], r[1]]))
                .collect()
        })
        .unwrap_or_default()
}

/// A group's MOLR list, the MOLT lights its doodads receive (`0x695c00`); a group without one
/// gives its doodads no point light at all.
pub fn wmo_group_light_refs(group_bytes: &[u8]) -> Vec<u16> {
    find_mogp_subchunk(group_bytes, b"RLOM")
        .map(|c| {
            c.as_chunks::<2>()
                .0
                .iter()
                .map(|r| u16::from_le_bytes([r[0], r[1]]))
                .collect()
        })
        .unwrap_or_default()
}

/// A group's MLIQ surface as a [`LiquidMesh`] in WMO model space, as the reference's dispatch
/// `0x6b62e0` draws it: vertex `(i, j)` at `base + (i·STEP, j·STEP, heights[j·xverts + i])`, a
/// tile drawn unless its low nibble is `0xf`, and one kind for the whole surface (`0x6ba970`).
pub fn wmo_group_liquid_mesh(group_bytes: &[u8]) -> Option<LiquidMesh> {
    let Ok(ParsedWmo::Group(group)) = parse_wmo(&mut Cursor::new(group_bytes)) else {
        return None;
    };
    let lq = group.liquid?;
    build_wmo_liquid_mesh(&lq, group.group_liquid)
}

/// A Wrath group's MLIQ grid as a [`LiquidMesh`] of `kind`, the native resolver's category
/// ([`wmo_group_wrath_liquid`] through `LiquidType.dbc`). Under MOHD flag 4 `groupLiquid` is the
/// resolver's input itself, not a nibble: Stormwind's canals and Ironforge's two pools carry 5, an
/// empty nibble slot, so [`wmo_group_liquid_mesh`] drops all seven.
///
/// [`LiquidMesh::sound_nibble`] keeps the native speed bits (`n − 1`, where `n` is the resolver's
/// input) over `kind`'s class: over the shipped cities that reproduces the 1.12 nibbles exactly
/// (Ironforge 2 and 6, Undercity 3, Blackrock 6, Stratholme 7). Labelled: that the 12340 liquid
/// renderer gates its scroll on those bits is unread.
pub fn wmo_group_liquid_mesh_of_kind(
    group_bytes: &[u8],
    root_flags: u32,
    kind: LiquidKind,
) -> Option<LiquidMesh> {
    let Ok(ParsedWmo::Group(group)) = parse_wmo(&mut Cursor::new(group_bytes)) else {
        return None;
    };
    let lq = group.liquid?;
    let raw = group.group_liquid;
    let n = if root_flags & 4 != 0 {
        raw
    } else if raw == NO_GROUP_LIQUID {
        0
    } else {
        raw.wrapping_add(1)
    };
    // `n` 0 takes the first wet tile's nibble, as the legacy surface does; `n` ≥ 21 is a direct
    // LiquidType id with no speed bits.
    let speed = match n {
        0 => lq.tile_flags.iter().map(|&f| f & 0xf).find(|&f| f != 0xf)? & 0xc,
        1..=20 => ((n - 1) & 0xc) as u8,
        _ => 0,
    };
    let class = match kind {
        LiquidKind::Ocean => 1,
        LiquidKind::Magma => 2,
        LiquidKind::Slime => 3,
        LiquidKind::Still | LiquidKind::Rapids => 0,
    };
    build_wmo_liquid_grid(&lq, kind, speed | class)
}

/// The [`LiquidMesh`] for a parsed [`WmoLiquid`] grid and its group's `groupLiquid`.
fn build_wmo_liquid_mesh(lq: &WmoLiquid, group_liquid: u32) -> Option<LiquidMesh> {
    // One kind per surface: the `groupLiquid` override, else the first wet nibble (`0x6ba970`).
    let type_nibble = if group_liquid != NO_GROUP_LIQUID {
        (group_liquid & 0xf) as u8
    } else {
        let first_wet = lq.tile_flags.iter().map(|&f| f & 0xf).find(|&n| n != 0xf)?;
        first_wet
    };
    let kind = LiquidKind::from_nibble(type_nibble)?;
    build_wmo_liquid_grid(lq, kind, type_nibble)
}

/// The grid of one `kind`; `sound_nibble` keys `SoundWaterType.dbc`.
fn build_wmo_liquid_grid(lq: &WmoLiquid, kind: LiquidKind, sound_nibble: u8) -> Option<LiquidMesh> {
    let (xv, yv, xt, yt) = (
        lq.xverts as usize,
        lq.yverts as usize,
        lq.xtiles as usize,
        lq.ytiles as usize,
    );
    if xv < 2 || yv < 2 || xt + 1 != xv || yt + 1 != yv {
        return None;
    }
    if lq.heights.len() < xv * yv || lq.tile_flags.len() < xt * yt {
        return None;
    }
    // Fullbright kinds read no ramp and their vertex bytes are a UV pair: the channel stays 0.
    let water = !kind.is_fullbright();

    // Two triangles per wet tile, drawn two-sided; built first, as `drawn` decides the fill below.
    let mut indices = Vec::with_capacity(xt * yt * 6);
    let mut wet = vec![false; xt * yt];
    // `0x80`: a neighbour also claims the cell (gated model-wide in `LiquidMesh::shared`).
    let mut shared = vec![false; xt * yt];
    let mut drawn = vec![false; xv * yv];
    for ty in 0..yt {
        for tx in 0..xt {
            if lq.tile_flags[ty * xt + tx] & 0xf == 0xf {
                continue; // a hole
            }
            wet[ty * xt + tx] = true;
            shared[ty * xt + tx] = lq.tile_flags[ty * xt + tx] & 0x80 != 0;
            let tl = (ty * xv + tx) as u32;
            let tr = tl + 1;
            let bl = ((ty + 1) * xv + tx) as u32;
            let br = bl + 1;
            indices.extend_from_slice(&[tl, bl, br, tl, br, tr]);
            for v in [tl, tr, bl, br] {
                drawn[v as usize] = true;
            }
        }
    }
    if indices.is_empty() {
        return None; // grid present but every tile a hole
    }

    // An undrawn vertex takes the lowest drawn height, so it never stretches the AABB.
    let fill_z = (0..xv * yv)
        .filter(|&n| drawn[n] && lq.heights[n].is_finite())
        .map(|n| lq.heights[n])
        .fold(f32::INFINITY, f32::min);
    let fill_z = if fill_z.is_finite() {
        fill_z
    } else {
        lq.base[2]
    };

    // Every grid vertex is emitted, row-major, so the tile indices line up. An undrawn vertex's
    // authored height is not a height: shipped files leave hole interiors at 0.0 (Blackfathom g007
    // has 186 of 868, its water at z −58.28), which would stretch the AABB to z 0.
    let mut positions = Vec::with_capacity(xv * yv);
    let mut uvs = Vec::with_capacity(xv * yv);
    let mut depths = Vec::with_capacity(xv * yv);
    for j in 0..yv {
        for i in 0..xv {
            let n = j * xv + i;
            let mx = lq.base[0] + i as f32 * MLIQ_CELL_STEP;
            let my = lq.base[1] + j as f32 * MLIQ_CELL_STEP;
            let h = lq.heights[n];
            let z = if drawn[n] && h.is_finite() { h } else { fill_z };
            positions.push([mx, my, z]);
            depths.push(if water {
                wmo_water_alpha_v(lq.opacity.get(n).copied().unwrap_or(0))
            } else {
                0.0
            });
            // UV from the model-space position, one field for all surfaces (`MLIQ_UV_PERIOD`).
            uvs.push([mx / MLIQ_UV_PERIOD, my / MLIQ_UV_PERIOD]);
        }
    }
    Some(LiquidMesh {
        grid: [xv as u32, yv as u32],
        wet,
        shared,
        positions,
        uvs,
        depths,
        indices,
        // The resolved nibble is also the surface's ambient-sound key.
        sound_nibble,
        // The body colour's MOMT index; MOMT is in the root, so the spawner resolves it.
        material_id: Some(lq.material_id),
        kind,
    })
}

/// The bright-doorway fade's reach in yards (`[0x811110]`): a vertex farther from every exterior
/// portal keeps its bake.
const DOORWAY_FADE_REACH: f32 = 6.666_666_5;

/// The fade's per-yard slope, `t = 1 − 0.15·dist` (`[0x7ffd6c]`).
const DOORWAY_FADE_SLOPE: f64 = 0.15;

/// The distance kernel's plane epsilon in yards (`0x6c4240`): a vertex this close to a portal's
/// plane, inside its outline, is at distance 0 and whitens outright.
const PORTAL_PLANE_EPS: f32 = 1.0 / 6.0;

/// The reference's float-to-fixed bias (`[0x8029cc]`): the f32 sum's bit pattern `>> 14` is the
/// byte, never a numeric conversion.
const FIXED_BIAS: f64 = 512.0;

/// Distance from a point to a portal polygon, the reference's kernel `0x6c4240`: 0 inside the
/// outline within [`PORTAL_PLANE_EPS`] of the plane, else the nearest edge distance. The outline
/// test matters: some portals are whole ceilings (Dire Maul's courtyard, 283×154 yd).
fn portal_distance(p: [f32; 3], poly: &[[f32; 3]], plane: [f32; 4]) -> f32 {
    let n = [plane[0], plane[1], plane[2]];
    let pd = n[0] * p[0] + n[1] * p[1] + n[2] * p[2] + plane[3];
    let mut best = f32::MAX;
    let mut inside = pd * pd < PORTAL_PLANE_EPS * PORTAL_PLANE_EPS;
    let proj = [p[0] - n[0] * pd, p[1] - n[1] * pd, p[2] - n[2] * pd];
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let e = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        // Outward edge test about the plane normal: a projection left of every edge is inside.
        let out = [
            e[1] * n[2] - e[2] * n[1],
            e[2] * n[0] - e[0] * n[2],
            e[0] * n[1] - e[1] * n[0],
        ];
        let w = [proj[0] - a[0], proj[1] - a[1], proj[2] - a[2]];
        if out[0] * w[0] + out[1] * w[1] + out[2] * w[2] > 0.0 {
            inside = false;
        }
        let wp = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let len2 = e[0] * e[0] + e[1] * e[1] + e[2] * e[2];
        let t = if len2 > 0.0 {
            ((wp[0] * e[0] + wp[1] * e[1] + wp[2] * e[2]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = [wp[0] - e[0] * t, wp[1] - e[1] * t, wp[2] - e[2] * t];
        best = best.min((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
    }
    if inside {
        0.0
    } else {
        best
    }
}

/// One byte of the white-lerp (`0x6c43d0`): the bits of `(255 − ch)·t + ch + 512` as f32, `>> 14`.
fn white_lerp_byte(ch: u8, t: f64) -> u8 {
    let chf = f64::from(ch);
    let s = ((255.0 - chf) * t + chf + FIXED_BIAS) as f32;
    ((s.to_bits() >> 14) & 0xff) as u8
}

/// The reference's bright-doorway fade, FixColorVertexAlpha (`0x6c43d0`), run once per group in
/// place over its MOCV before any draw; transition corridors are authored near-black and rely on
/// it. Per vertex, `dist` is the nearest [`portal_distance`] over portals to exterior neighbours
/// (`MOGI.flags & 0x48 != 0`): at 0 the vertex goes white-opaque, under [`DOORWAY_FADE_REACH`] with
/// alpha 0 it is white-lerped by `t = 1 − 0.15·dist` with alpha `t·255`, else it is untouched. The
/// MOPY pre-pass `0x6c41e0` is gated (`DAT_00ca8064`) and a reference capture shows it unrun, so it
/// is not run here.
fn fix_color_vertex_alpha(
    colors: &mut [Color],
    group: &WmoGroup,
    root: &WmoRoot,
    header_refs: (u16, u16),
) {
    let portals = root.portals();
    let infos = root.group_infos();
    let (start, count) = (header_refs.0 as usize, header_refs.1 as usize);
    // This group's portals whose far side is an exterior group.
    let doors: Vec<(Vec<[f32; 3]>, [f32; 4])> = portals
        .refs
        .get(start..start + count)
        .unwrap_or(&[])
        .iter()
        .filter(|r| infos.get(r.group as usize).is_some_and(|g| !g.interior))
        .filter_map(|r| portals.infos.get(r.portal as usize))
        .map(|info| {
            let v = (0..info.count as usize)
                .filter_map(|k| {
                    portals
                        .vertices
                        .get(info.start_vertex as usize + k)
                        .copied()
                })
                .collect();
            (v, info.plane)
        })
        .filter(|(v, _): &(Vec<[f32; 3]>, _)| v.len() >= 3)
        .collect();
    if doors.is_empty() {
        return;
    }
    for (i, c) in colors.iter_mut().enumerate() {
        let Some(p) = group.vertex_positions.get(i) else {
            continue;
        };
        let p = [p.x, p.y, p.z];
        let dist = doors
            .iter()
            .map(|(poly, plane)| portal_distance(p, poly, *plane))
            .fold(f32::MAX, f32::min);
        if dist == 0.0 {
            *c = Color {
                b: 0xff,
                g: 0xff,
                r: 0xff,
                a: 0xff,
            };
        } else if dist < DOORWAY_FADE_REACH && c.a == 0 {
            let t = 1.0 - f64::from(dist) * DOORWAY_FADE_SLOPE;
            c.b = white_lerp_byte(c.b, t);
            c.g = white_lerp_byte(c.g, t);
            c.r = white_lerp_byte(c.r, t);
            c.a = ((((t * 255.0) + FIXED_BIAS) as f32).to_bits() >> 14 & 0xff) as u8;
        }
    }
}

/// The group's MOCV as the renderer uploads it, BGRA, with the doorway fade applied.
pub fn wmo_group_fixed_colors(group_bytes: &[u8], root: &WmoRoot) -> Option<Vec<[u8; 4]>> {
    let Ok(ParsedWmo::Group(group)) = parse_wmo(&mut Cursor::new(group_bytes)) else {
        return None;
    };
    let colors = fixed_colors(&group, group_bytes, root)?;
    Some(colors.iter().map(|c| [c.b, c.g, c.r, c.a]).collect())
}

/// The group's MOCV as authored, BGRA.
pub fn wmo_group_raw_colors(group_bytes: &[u8]) -> Option<Vec<[u8; 4]>> {
    let Ok(ParsedWmo::Group(group)) = parse_wmo(&mut Cursor::new(group_bytes)) else {
        return None;
    };
    let colors = parallel_colors(&group)?;
    Some(colors.iter().map(|c| [c.b, c.g, c.r, c.a]).collect())
}

/// The group's MOCV parallel to its positions. A MOCV clamped at EOF comes back one colour short
/// (`Undercity_144.wmo` holds 1159 of 1160 bytes) and is padded from the last colour; a larger
/// shortfall is a broken bake and reads `None`.
fn parallel_colors(group: &WmoGroup) -> Option<Vec<Color>> {
    let (have, want) = (group.vertex_colors.len(), group.vertex_positions.len());
    if have == want {
        return Some(group.vertex_colors.clone());
    }
    let last = (have + 1 == want).then(|| group.vertex_colors.last().copied())??;
    let mut colors = group.vertex_colors.clone();
    colors.push(last);
    Some(colors)
}

/// The authored MOCV with the doorway fade run over it.
fn fixed_colors(group: &WmoGroup, group_bytes: &[u8], root: &WmoRoot) -> Option<Vec<Color>> {
    let mut colors = parallel_colors(group)?;
    let flags = root.flags().unwrap_or(0);
    if root.wrath_loader {
        // Drawer selection (root0x2) does not gate the Wrath loader.
        // 12340 0x7d7c30/0x7d7380: root0x8 skips the RGB AND alpha fixup.
        if flags & 8 == 0 {
            wrath_color_fixup(&mut colors, group_bytes);
        }
        // Root0x1 independently marks the portal attenuation complete (0x7d82e0).
        if flags & 1 == 0 {
            wrath_portal_attenuation(&mut colors, group, group_bytes, root);
        }
    } else {
        let refs = wmo_group_header(group_bytes)
            .map_or((0, 0), |h| (h.portal_ref_start, h.portal_ref_count));
        fix_color_vertex_alpha(&mut colors, group, root, refs);
    }
    Some(colors)
}

/// Native last TRANS vertex is MOBA.lastVertex, not the index count (0x7d7380).
fn wrath_trans_vertex_end(bytes: &[u8]) -> usize {
    let Some(mogp) = find_wmo_chunk(bytes, b"PGOM").filter(|m| m.len() >= 68) else {
        return 0;
    };
    let count = u16::from_le_bytes([mogp[0x28], mogp[0x29]]) as usize;
    let batches = find_wmo_chunk(&mogp[68..], b"ABOM").unwrap_or_default();
    count
        .checked_sub(1)
        .and_then(|i| batches.get(i * 24 + 20..i * 24 + 22))
        .map_or(0, |v| usize::from(u16::from_le_bytes([v[0], v[1]])) + 1)
}

fn wrath_color_fixup(colors: &mut [Color], bytes: &[u8]) {
    let split = wrath_trans_vertex_end(bytes);
    for (i, c) in colors.iter_mut().enumerate() {
        let a = u32::from(c.a);
        for channel in [&mut c.r, &mut c.g, &mut c.b] {
            let v = u32::from(*channel);
            *channel = if i < split {
                (v >> 1) as u8
            } else {
                ((v + ((v * a) >> 6)) >> 1).min(255) as u8
            };
        }
        if i >= split {
            c.a = 255;
        }
    }
}

/// 12340 0x7d78c0: TRANS vertices fade toward half-range white127, not white255.
fn wrath_portal_attenuation(colors: &mut [Color], group: &WmoGroup, bytes: &[u8], root: &WmoRoot) {
    let Some(header) = wmo_group_header(bytes) else {
        return;
    };
    let portals = root.portals();
    let refs = portals
        .refs
        .get(
            usize::from(header.portal_ref_start)
                ..usize::from(header.portal_ref_start) + usize::from(header.portal_ref_count),
        )
        .unwrap_or_default();
    for (color, p) in colors
        .iter_mut()
        .take(wrath_trans_vertex_end(bytes))
        .zip(&group.vertex_positions)
    {
        let p = [p.x, p.y, p.z];
        let mut fade = 0.0f32;
        for r in refs {
            let Some(info) = portals.infos.get(usize::from(r.portal)) else {
                continue;
            };
            let Some(poly) = portals.vertices.get(
                usize::from(info.start_vertex)
                    ..usize::from(info.start_vertex) + usize::from(info.count),
            ) else {
                continue;
            };
            if poly.len() < 3 {
                continue;
            }
            let plane = info.plane;
            let distance = plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3];
            let projected = [
                p[0] - distance * plane[0],
                p[1] - distance * plane[1],
                p[2] - distance * plane[2],
            ];
            let inside = (0..poly.len()).all(|i| {
                let a = poly[i];
                let b = poly[(i + 1) % poly.len()];
                let e = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let out = [
                    e[1] * plane[2] - e[2] * plane[1],
                    e[2] * plane[0] - e[0] * plane[2],
                    e[0] * plane[1] - e[1] * plane[0],
                ];
                out[0] * (projected[0] - a[0])
                    + out[1] * (projected[1] - a[1])
                    + out[2] * (projected[2] - a[2])
                    <= 0.0
            });
            let d = if inside {
                distance * f32::from(r.side)
            } else {
                portal_distance(p, poly, plane)
            };
            if root
                .group_infos()
                .get(usize::from(r.group))
                .is_some_and(|g| !g.interior)
            {
                let contribution = 1.0 - d.max(0.0) * 0.15;
                if contribution > 0.001 {
                    fade += contribution;
                }
            } else if d > -1.0 && d < 1.0 {
                fade = 0.0;
                break;
            }
        }
        let fade = if fade > 0.001 { fade.min(1.0) } else { 0.0 };
        // Native x87 conversion rounds to nearest/even, not Rust's ties-away round().
        for c in [&mut color.r, &mut color.g, &mut color.b] {
            *c = (f32::from(*c) + (127.0 - f32::from(*c)) * fade).round_ties_even() as u8;
        }
        color.a = (255.0 * fade).round_ties_even() as u8;
    }
}

/// Build one group's render submeshes against `root`; empty if the bytes are not a group.
pub fn wmo_group_submeshes(group_bytes: &[u8], root: &WmoRoot) -> Result<Vec<RenderSubmesh>> {
    let Ok(ParsedWmo::Group(group)) = parse_wmo(&mut Cursor::new(group_bytes)) else {
        return Ok(Vec::new());
    };
    // The reference fades once per group, gated on its `+0xbc` done-byte (`0x6b3f90`); a group
    // loads once, so the load is the gate.
    let colors = fixed_colors(&group, group_bytes, root).unwrap_or_default();
    let unified = root.unified_ambient();
    let ParsedWmo::Root(root) = &root.parsed else {
        anyhow::bail!("WmoRoot is not a root");
    };
    // MOGP `+0x28`/`+0x2a`: MOBA batches run TRANS, then INT, then EXT, the class deciding an
    // interior group's lighting; a short header reads all EXT.
    let (trans_n, int_n) = find_wmo_chunk(group_bytes, b"PGOM")
        .filter(|m| m.len() >= 0x2c)
        .map_or((0usize, 0usize), |m| {
            (
                u16::from_le_bytes([m[0x28], m[0x29]]) as usize,
                u16::from_le_bytes([m[0x2a], m[0x2b]]) as usize,
            )
        });
    let mut out = Vec::new();
    {
        let has_normals = group.vertex_normals.len() == group.vertex_positions.len();
        let has_colors = colors.len() == group.vertex_positions.len();
        // Neither exterior (`0x8`) nor exterior-lit (`0x40`): the reference's interior test.
        let interior = (group.flags & 0x48) == 0;
        let vertex = |g: u32| {
            let p = &group.vertex_positions[g as usize];
            let n = group
                .vertex_normals
                .get(g as usize)
                .map_or([0.0, 1.0, 0.0], |n| [n.x, n.y, n.z]);
            let uv = group
                .texture_coords
                .get(g as usize)
                .map_or([0.0, 0.0], |t| [t.u, t.v]);
            // MOCV RGB is the shade multiplier; the alpha rides along and is forced opaque below
            // where it is not lighting data. No MOCV is white.
            let c = colors.get(g as usize).map_or([1.0, 1.0, 1.0, 1.0], |c| {
                [
                    c.r as f32 / 255.0,
                    c.g as f32 / 255.0,
                    c.b as f32 / 255.0,
                    c.a as f32 / 255.0,
                ]
            });
            ([p.x, p.y, p.z], n, uv, c)
        };
        for (bi, batch) in group.render_batches.iter().enumerate() {
            let class = if bi < trans_n {
                WmoBatchClass::Trans
            } else if bi < trans_n + int_n {
                WmoBatchClass::Int
            } else {
                WmoBatchClass::Ext
            };
            let start = batch.start_index as usize;
            let global_indices: Vec<u32> = group
                .vertex_indices
                .get(start..start + batch.count as usize)
                .unwrap_or(&[])
                .iter()
                .map(|&i| u32::from(i))
                .filter(|&g| (g as usize) < group.vertex_positions.len())
                .collect();
            if global_indices.is_empty() {
                continue;
            }
            let material = root.materials.get(batch.material_id as usize);
            let texture = material
                .map(|m| m.get_texture1_index(&root.texture_offset_index_map))
                .and_then(|i| root.textures.get(i as usize))
                .cloned()
                .filter(|s| !s.is_empty());
            let blend = match material.map(|m| m.blend_mode) {
                Some(0) | None => ModelBlend::Opaque,
                Some(1) => ModelBlend::AlphaTest,
                // `MOMT.blendMode` indexes EGxBlend directly, no M2 remap (`0x6b500f`): 4 is Mod
                // (DST_COLOR/ZERO), 5 Mod2x (DST_COLOR/SRC_COLOR).
                Some(4) => ModelBlend::Mod,
                Some(5) => ModelBlend::Mod2x,
                Some(_) => ModelBlend::Blend,
            };
            // MOMT flags (drawers `0x6b4f10`/`0x6b5190`, SIDN updater `0x6b4090`): UNLIT `0x01`
            // draws an exterior batch fullbright and is ignored by the interior drawer; SIDN `0x10`
            // adds the night-glow colour inside the lit sum; WINDOW `0x20` gives an interior batch
            // the brighter midpoint light.
            let flags = material.map_or(0, |m| m.flags);
            let emissive = !interior && flags & 0x01 != 0;
            // UNCULLED `0x04`: without it the reference back-face culls the batch (`0x6b4fd7`,
            // `0x6b52bf`). Two-sided cloth is authored as the same faces twice with reversed
            // winding, so drawing both sides z-fights. A WMO has no skeleton: the skin is unused.
            let (mut submesh, _globals) = remap_submesh(
                global_indices.into_iter(),
                vertex,
                texture,
                blend,
                flags & 0x04 != 0,
                interior,
                emissive,
            );
            submesh.wmo_batch = Some(class);
            // Native TRANS is multipass; keep its previous route until the blend law is certified.
            submesh.wmo_unified = unified.filter(|_| class != WmoBatchClass::Trans);
            submesh.sidn = (flags & 0x10 != 0)
                .then(|| {
                    material.map(|m| {
                        if submesh.wmo_unified.is_some() {
                            m.unified_sidn_rgb
                        } else {
                            m.sidn_rgb
                        }
                    })
                })
                .flatten();
            submesh.window = flags & 0x20 != 0;
            if !has_normals {
                submesh.normals.clear(); // no MONR: the renderer computes flat normals
            }
            if !has_colors {
                submesh.vertex_colors.clear(); // no MOCV: untinted
            }
            // MOCV alpha is lighting data on interior batches: TRANS reads it as the lit-to-bake
            // lerp factor, INT as the self-illumination mask (the reference's
            // `tex·MOCV·(1 + 4·a)`). Elsewhere it is blend state, forced opaque.
            if submesh.wmo_unified.is_none()
                && !(interior && matches!(class, WmoBatchClass::Trans | WmoBatchClass::Int))
            {
                for c in &mut submesh.vertex_colors {
                    c[3] = 1.0;
                }
            }
            out.push(submesh);
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "group_tests.rs"]
mod tests;
