//! A 1.12.1 WMO (World Map Object) reader, scoped to what the renderer consumes. A root file holds
//! `MOHD`, `MOTX` and `MOMT` among its tables; each group file holds a `MOGP` super-chunk wrapping
//! the geometry sub-chunks. A chunk is a 4-char magic stored reversed, a `u32` size, the payload.

use std::collections::HashMap;
use std::io::Cursor;

use wrathplus_bytes::{chunks, ByteExt};

/// A parsed WMO file: the root or one group.
pub enum ParsedWmo {
    Root(WmoRoot),
    Group(WmoGroup),
}

/// The WMO root: textures, materials, and the group count.
pub struct WmoRoot {
    pub textures: Vec<String>,
    /// MOTX byte offset → index into [`textures`](Self::textures): a material names its texture by
    /// the offset of the name in the blob.
    pub texture_offset_index_map: HashMap<u32, u32>,
    pub materials: Vec<Material>,
    /// Group count (`MOHD.nGroups`).
    pub n_groups: u32,
}

/// One MOMT material (the fields the renderer reads).
pub struct Material {
    pub flags: u32,
    pub blend_mode: u32,
    /// Byte offset of texture 1's name in the MOTX blob.
    pub texture_1: u32,
    /// The SIDN self-illumination colour (MOMT+0x10, BGRA on disk, RGB here): the client scales it
    /// by the night fraction and adds it as emission on batches with `flags & 0x10`, the windows'
    /// glow at night.
    pub sidn_rgb: [u8; 3],
    /// Build12340 unified SIDN RGB at MOMT+0x14, not the classic +0x10 field.
    pub unified_sidn_rgb: [u8; 3],
    /// MOMT+0x1C `diffColor` (BGRA on disk, RGB here), read only as an interior liquid's body
    /// colour: the reference's interior water kernel `0x6b6420` runs unlit with no pixel shader,
    /// its RGB this dword raw, `Cf = MOMT[MLIQ.materialId].diffColor`, combined with the animated
    /// sheet as `clamp(Cf + Ct)`. Its alpha at `+0x1f` is discarded; opacity comes per vertex.
    /// Stride 0x40, base `[CMapObj+0x1d8]` (`0x6c3ace`, `0x6c3ad7 shr edx,6`).
    pub diff_color: [u8; 3],
    /// MOMT+0x20: the surface's `TerrainType.dbc` id, what a footstep on it sounds like. The
    /// client's down-ray takes the nearer of the terrain and WMO probes, so a building's floor
    /// decides the step indoors. The 5875 data holds only `{0,1,2,3,4,5,7,8,10}`; 10 ("None", on
    /// 10 075 of 10 299 materials) resolves through `SoundID 0` to the generic `*Dirt` kit.
    pub ground_type: u32,
}

impl Material {
    /// Resolve texture 1 to its index in [`WmoRoot::textures`] via the offset map.
    pub fn get_texture1_index(&self, texture_offset_index_map: &HashMap<u32, u32>) -> u32 {
        texture_offset_index_map
            .get(&self.texture_1)
            .copied()
            .unwrap_or(u32::MAX)
    }
}

/// A 3-float vector (MOVT position / MONR normal).
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// A MOTV texture coordinate.
pub struct Uv {
    pub u: f32,
    pub v: f32,
}

/// A MOCV vertex colour (stored BGRA on disk).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

/// A MOBA render batch (the fields the renderer reads; 24 bytes on disk).
pub struct Batch {
    pub start_index: u32,
    pub count: u16,
    pub material_id: u8,
}

/// A MOPY entry, one triangle's flags and material (2 bytes); it drives collision filtering.
pub struct MopyEntry {
    pub flags: u8,
    pub material_id: u8,
}

/// A WMO group's MLIQ liquid grid: `xverts × yverts` heights over `xtiles × ytiles` cells in WMO
/// model space (WoW axes, yards). A tile flag's low nibble is `0xf` for a hole, else the liquid
/// type indexing the reference's texture table (dispatch `0x6b62e0`, water `0x6b6420` and
/// `0x6b6630`, magma and slime `0x6b68f0`).
pub struct WmoLiquid {
    /// Vertex-grid dimensions (`xtiles = xverts - 1`, `ytiles = yverts - 1`).
    pub xverts: u32,
    pub yverts: u32,
    pub xtiles: u32,
    pub ytiles: u32,
    /// The `(0,0)` grid corner in WMO model space (WoW `[x, y, z]`, yards). Grid vertex `(i, j)`
    /// sits at `(base.x + i·MLIQ_CELL_STEP, base.y + j·MLIQ_CELL_STEP, heights[j·xverts + i])`.
    pub base: [f32; 3],
    /// MLIQ `materialId`, an index into the root MOMT, whose `diffColor` colours an interior pool.
    pub material_id: u16,
    /// Per-vertex liquid height (WMO-local Z), row-major `j·xverts + i`: the trailing `f32` of each
    /// 8-byte MLIQ vertex, whose leading 4 bytes depend on the liquid type.
    pub heights: Vec<f32>,
    /// Per-vertex opacity index, a water vertex's byte 0, row-major beside [`Self::heights`]. WMO
    /// liquid has no depth, so this byte is the authored opacity: the reference indexes a 256-entry
    /// alpha ramp with it (`[0xca7f10]`, rebuilt per frame from the zone's river shallow and deep
    /// alphas) and binds no depth-ramp texture (no reference to the ADT ramp globals `0xc7fbc0`,
    /// `0xc81768`, `0xc7fcd8` in `[0x6b0000, 0x6c4000)`). On magma and slime the four bytes are an
    /// `int16` `(s, t)` UV pair instead; the consumer picks by liquid type.
    pub opacity: Vec<u8>,
    /// Per-tile flags, row-major `j·xtiles + i`: the low nibble the liquid type (`0xf` a hole),
    /// `0x80` shared (the strip-builder gate). `0x40` (fishable) is unread: the reference's reader
    /// `0x6b9e50` is reachable only from the uncalled `0x69b5d0`, and the server decides fishing.
    pub tile_flags: Vec<u8>,
}

/// One WMO group's render geometry.
pub struct WmoGroup {
    /// MOGP group flags (bit test `& 0x48` selects exterior vs interior).
    pub flags: u32,
    /// MOGP `groupLiquid` at header `0x34`: other than `0xf` it overrides every tile's liquid type
    /// (`0x6b9f10`); `0xf`, also the value for a header too short to carry it, defers to the tiles.
    pub group_liquid: u32,
    pub vertex_positions: Vec<Vec3>,
    pub vertex_normals: Vec<Vec3>,
    pub texture_coords: Vec<Uv>,
    pub vertex_colors: Vec<Color>,
    pub vertex_indices: Vec<u16>,
    pub render_batches: Vec<Batch>,
    pub material_info: Vec<MopyEntry>,
    pub liquid: Option<WmoLiquid>,
}

/// WMO parse error.
#[derive(Debug)]
pub enum Error {
    NotWmo,
    /// A chunk's payload is shorter than its format requires (names the chunk).
    Truncated(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotWmo => write!(f, "not a WMO (no MOHD root or MOGP group chunk)"),
            Error::Truncated(chunk) => write!(f, "truncated WMO chunk: {chunk}"),
        }
    }
}
impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Parse a WMO root or group from a cursor over the whole file (position ignored).
pub fn parse_wmo(cursor: &mut Cursor<&[u8]>) -> Result<ParsedWmo> {
    let b: &[u8] = cursor.get_ref();
    // A group file carries a MOGP super-chunk; a root carries MOHD.
    for (magic, payload) in chunks(b) {
        if &magic == b"PGOM" {
            return Ok(ParsedWmo::Group(parse_group(payload)?));
        }
        if &magic == b"DHOM" {
            return Ok(ParsedWmo::Root(parse_root(b)?));
        }
    }
    Err(Error::NotWmo)
}

fn parse_root(b: &[u8]) -> Result<WmoRoot> {
    let mut root = WmoRoot {
        textures: Vec::new(),
        texture_offset_index_map: HashMap::new(),
        materials: Vec::new(),
        n_groups: 0,
    };
    for (magic, p) in chunks(b) {
        match &magic {
            // MOHD: nTextures@0, nGroups@4
            b"DHOM" => root.n_groups = p.u32_at(4).ok_or(Error::Truncated("MOHD"))?,
            b"XTOM" => {
                let (t, m) = parse_motx(p);
                root.textures = t;
                root.texture_offset_index_map = m;
            }
            // A 64-byte MOMT entry: flags@0, shader@4, blend_mode@8, texture_1@12, sidnColor@16.
            b"TMOM" => {
                for m in p.as_chunks::<64>().0 {
                    // CImVector: BGRA on disk, so the LE u32 reads B | G<<8 | R<<16 | A<<24.
                    let sidn = m.u32_at(0x10).ok_or(Error::Truncated("MOMT"))?;
                    let unified_sidn = m.u32_at(0x14).ok_or(Error::Truncated("MOMT"))?;
                    let diff = m.u32_at(0x1c).ok_or(Error::Truncated("MOMT"))?;
                    root.materials.push(Material {
                        flags: m.u32_at(0).ok_or(Error::Truncated("MOMT"))?,
                        blend_mode: m.u32_at(8).ok_or(Error::Truncated("MOMT"))?,
                        texture_1: m.u32_at(12).ok_or(Error::Truncated("MOMT"))?,
                        sidn_rgb: [(sidn >> 16) as u8, (sidn >> 8) as u8, sidn as u8],
                        unified_sidn_rgb: [
                            (unified_sidn >> 16) as u8,
                            (unified_sidn >> 8) as u8,
                            unified_sidn as u8,
                        ],
                        diff_color: [(diff >> 16) as u8, (diff >> 8) as u8, diff as u8],
                        ground_type: m.u32_at(0x20).ok_or(Error::Truncated("MOMT"))?,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(root)
}

/// MOTX, a NUL-separated name blob: the texture list, and each name's first-byte offset → index.
fn parse_motx(blob: &[u8]) -> (Vec<String>, HashMap<u32, u32>) {
    let mut textures = Vec::new();
    let mut map = HashMap::new();
    let mut current = String::new();
    for (i, &byte) in blob.iter().enumerate() {
        if byte == 0 {
            if !current.is_empty() {
                textures.push(std::mem::take(&mut current));
            }
        } else {
            if current.is_empty() {
                map.insert(i as u32, textures.len() as u32);
            }
            current.push(byte as char);
        }
    }
    (textures, map)
}

/// Parse a MOGP payload: a 68-byte header (flags at 8), then the geometry sub-chunks. A payload
/// shorter than the header is an empty group.
fn parse_group(mogp: &[u8]) -> Result<WmoGroup> {
    let flags = mogp.u32_at(8).unwrap_or(0);
    let group_liquid = mogp.u32_at(0x34).unwrap_or(0xf);
    let mut g = WmoGroup {
        flags,
        group_liquid,
        vertex_positions: Vec::new(),
        vertex_normals: Vec::new(),
        texture_coords: Vec::new(),
        vertex_colors: Vec::new(),
        vertex_indices: Vec::new(),
        render_batches: Vec::new(),
        material_info: Vec::new(),
        liquid: None,
    };
    if mogp.len() < 68 {
        return Ok(g);
    }
    // Push, never assign: a repeated sub-chunk concatenates (some WMOs carry a second MOTV, e.g.
    // KL_PvPBarracks).
    for (magic, s) in chunks(&mogp[68..]) {
        match &magic {
            b"TVOM" => {
                for c in s.as_chunks::<12>().0 {
                    g.vertex_positions.push(Vec3 {
                        x: c.f32_at(0).ok_or(Error::Truncated("MOVT"))?,
                        y: c.f32_at(4).ok_or(Error::Truncated("MOVT"))?,
                        z: c.f32_at(8).ok_or(Error::Truncated("MOVT"))?,
                    });
                }
            }
            b"RNOM" => {
                for c in s.as_chunks::<12>().0 {
                    g.vertex_normals.push(Vec3 {
                        x: c.f32_at(0).ok_or(Error::Truncated("MONR"))?,
                        y: c.f32_at(4).ok_or(Error::Truncated("MONR"))?,
                        z: c.f32_at(8).ok_or(Error::Truncated("MONR"))?,
                    });
                }
            }
            b"VTOM" => {
                for c in s.as_chunks::<8>().0 {
                    g.texture_coords.push(Uv {
                        u: c.f32_at(0).ok_or(Error::Truncated("MOTV"))?,
                        v: c.f32_at(4).ok_or(Error::Truncated("MOTV"))?,
                    });
                }
            }
            b"IVOM" => {
                for c in s.as_chunks::<2>().0 {
                    g.vertex_indices
                        .push(c.u16_at(0).ok_or(Error::Truncated("MOVI"))?);
                }
            }
            // MOBA: bbox_min[i16;3] bbox_max[i16;3] start_index(u32@12) count(u16@16)
            // min/max(u16) flags(u8@22) material_id(u8@23)
            b"ABOM" => {
                for c in s.as_chunks::<24>().0 {
                    g.render_batches.push(Batch {
                        start_index: c.u32_at(12).ok_or(Error::Truncated("MOBA"))?,
                        count: c.u16_at(16).ok_or(Error::Truncated("MOBA"))?,
                        material_id: c.u8_at(23).ok_or(Error::Truncated("MOBA"))?,
                    });
                }
            }
            b"VCOM" => {
                for c in s.as_chunks::<4>().0 {
                    g.vertex_colors.push(Color {
                        b: c[0],
                        g: c[1],
                        r: c[2],
                        a: c[3],
                    });
                }
            }
            b"YPOM" => {
                for c in s.as_chunks::<2>().0 {
                    g.material_info.push(MopyEntry {
                        flags: c[0],
                        material_id: c[1],
                    });
                }
            }
            b"QILM" => g.liquid = parse_mliq(s)?,
            _ => {}
        }
    }
    Ok(g)
}

/// Parse an `MLIQ` payload into a [`WmoLiquid`]; no liquid when the grid is empty, oversized or
/// longer than the payload, so the group still renders.
fn parse_mliq(s: &[u8]) -> Result<Option<WmoLiquid>> {
    // SMOLiquidHeader, 30 B: xverts@0 yverts@4 xtiles@8 ytiles@12 base[f32;3]@16
    // materialId(u16)@28; then 8-byte vertices (height at +4), then 1-byte tile flags.
    let (Some(xverts), Some(yverts), Some(xtiles), Some(ytiles)) =
        (s.u32_at(0), s.u32_at(4), s.u32_at(8), s.u32_at(12))
    else {
        return Ok(None);
    };
    let nverts = (xverts as usize).saturating_mul(yverts as usize);
    let ntiles = (xtiles as usize).saturating_mul(ytiles as usize);
    // A corrupt header must not size gigabytes; real grids are about 32² at most.
    if nverts == 0 || ntiles == 0 || nverts > 1 << 20 || ntiles > 1 << 20 {
        return Ok(None);
    }
    let base = [
        s.f32_at(16).ok_or(Error::Truncated("MLIQ"))?,
        s.f32_at(20).ok_or(Error::Truncated("MLIQ"))?,
        s.f32_at(24).ok_or(Error::Truncated("MLIQ"))?,
    ];
    let material_id = s.u16_at(28).ok_or(Error::Truncated("MLIQ"))?;
    let vbase = 30usize;
    let tbase = vbase + nverts * 8;
    if s.len() < tbase + ntiles {
        return Ok(None); // the declared grid runs past the payload: no liquid
    }
    let heights: Vec<f32> = (0..nverts)
        .map(|k| s.f32_at(vbase + k * 8 + 4).unwrap_or(0.0))
        .collect();
    // Byte 0 of each vertex, in bounds by the `tbase` check above.
    let opacity: Vec<u8> = (0..nverts).map(|k| s[vbase + k * 8]).collect();
    let tile_flags = s[tbase..tbase + ntiles].to_vec();
    Ok(Some(WmoLiquid {
        xverts,
        yverts,
        xtiles,
        ytiles,
        base,
        material_id,
        heights,
        opacity,
        tile_flags,
    }))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
