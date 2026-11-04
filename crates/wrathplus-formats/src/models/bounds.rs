//! An M2's authored bounds ([`M2Bounds`]): the header sphere and box, and the Stand-sequence box
//! reads that size the selection ring, the chat bubble and the camera pivot.

use std::io::Cursor;

use anyhow::{Context, Result};

use crate::Chain;

use super::model_path;

/// An M2's authored bounds and the reads sized from them, in model-local yards before the
/// placement scale.
#[derive(Clone, Copy, Debug)]
pub struct M2Bounds {
    /// The authored bounding-sphere radius; times the placement scale it is the reference's
    /// doodad radius `rec+0x68` (`0x6952a0`).
    pub sphere_radius: f32,
    /// The authored box; its centre is the sphere's.
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    /// The largest vertex distance from the model origin.
    pub vert_max_from_origin: f32,
    /// The selection ring's footprint: `sqrt(0.5 · sqrt(dx² + dy²))` over the Stand box's X and Y
    /// extents, the reference's living-unit ring input (`0x608e00`/`0x60aee0`); the ring's world
    /// radius is this × `OBJECT_FIELD_SCALE_X`. A model with no sequences uses the header box, and
    /// a box with zero X and Y extents takes [`DEGENERATE_RING_FOOTPRINT`].
    pub ring_footprint: f32,
    /// Model-space Z of attachment 17, the reference's follow-camera pivot height (`0x50ca90`:
    /// `feet + (attach17.z + 0.0972)·scale`); without one the camera uses a fraction of the box.
    pub pivot_z: Option<f32>,
    /// The Stand box's Z extent, the chat bubble's anchor above the feet (times the scale, plus
    /// 0.7 yd): `0x4b0e38` calls `0x711a20`, which reads the MD20 file image with no bone matrix
    /// and returns `out+0x20 − out+0x14`. The overhead name instead uses `0x608640`, the live posed
    /// attachment, which bobs. The header box's Z for a model with no sequences.
    pub stand_box_z: f32,
    /// How far the camera pivot drops while swimming: Stand's box `max.z` less Swim's, floored at
    /// zero, and `0.0` without a Swim sequence. The reference adds the neck height
    /// (`attach17.z + 0.0972222`, `0x808ab0`) to three presets, `cam+0x11c`/`+0x120`/`+0x124`
    /// (`0x50cc0c`–`0x50cc2e`, built by `0x50ca90`), then lowers only the swim one by
    /// `S · (box0.max.z − box42.max.z)` (`0x50ccf6`), and only when `0x711960` finds ids 0 and 42
    /// (`0x50cc67`–`0x50cd02`); `0x50f880` picks a preset per frame, then the `[5/6, 15.0]` clamp
    /// (`0x50d00d`–`0x50d092`). `HumanMale.m2` drops `0.3882572` at scale 1. The reference also
    /// picks zoomed-in `+0x11c` or zoomed-out `+0x120` on `cam+0x198 < 1.8315`; WrathPlus has one
    /// standing preset, and what sets those two apart is untraced.
    pub swim_pivot_drop: f32,
}

/// Read an M2's bounds from the chain, by model or doodad (`.mdx`/`.mdl`) path.
pub fn load_m2_bounds(chain: &mut Chain, raw_path: &str) -> Result<M2Bounds> {
    let path = model_path(raw_path);
    let bytes = chain
        .read_file(&path)
        .with_context(|| format!("reading M2 {path}"))?;
    parse_m2_bounds(&bytes).with_context(|| format!("parsing M2 bounds {path}"))
}

/// The ring footprint the reference stores for a box with zero X and Y extents: the writer
/// `0x60aee0` stores the literal 1.2 (`0x3f99999a`) into `[unit+0xcf0]` without running the
/// formula (`0x60af4f..0x60af67`). An `InvisibleStalker` body's sequence boxes are all zero, so
/// this is its ring; the ring also takes it for a unit with no model.
pub const DEGENERATE_RING_FOOTPRINT: f32 = 1.2;

/// Read an in-memory v264 M2's bounds.
pub fn parse_m2_bounds(bytes: &[u8]) -> Result<M2Bounds> {
    let wrath = wrathplus_m2::wrath::parse_wrath_m2(&mut Cursor::new(bytes))
        .map_err(|e| anyhow::anyhow!("parsing M2: {e}"))?;
    Ok(super::wrath::bounds(&wrath))
}
