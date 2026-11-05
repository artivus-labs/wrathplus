//! Pathfinding over AzerothCore's **mmaps**: the Recast/Detour navmesh tiles the server's
//! `PathGenerator` and mod-playerbots walk on, read from the realm's extracted server data
//! (`<data>/mmaps`), queried with the same Detour code (`third_party/detour`).
//!
//! Files: `<map:03>.mmap` is a raw 28-byte `dtNavMeshParams`; `<map:03><gx:02><gy:02>.mmtile` is a
//! 56-byte `MmapTileHeader` (magic `0x4d4d4150`, mmap version 20) followed by a Detour tile.
//! `gx`/`gy` are the server's grid indices, `floor(32 - coord / 533.3333)` of WoW x and y. Detour
//! works in Recast order `{y, z, x}` of WoW coordinates; this crate takes and returns WoW `[x, y, z]`.

use std::collections::HashSet;
use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

#[repr(C)]
struct Shim {
    _opaque: [u8; 0],
}

extern "C" {
    fn nav_create(params: *const u8, len: c_int, max_nodes: c_int) -> *mut Shim;
    fn nav_destroy(shim: *mut Shim);
    fn nav_add_tile(shim: *mut Shim, data: *const u8, len: c_int, out_ref: *mut u64) -> u32;
    fn nav_set_filter(shim: *mut Shim, include: u16, exclude: u16);
    fn nav_set_area_cost(shim: *mut Shim, area: c_int, cost: f32);
    fn nav_set_avoid(shim: *mut Shim, zones: *const f32, count: c_int, mult: f32);
    fn nav_find_straight_path(
        shim: *mut Shim,
        start: *const f32,
        end: *const f32,
        extents: *const f32,
        out: *mut f32,
        max_points: c_int,
        out_count: *mut c_int,
        out_polys: *mut c_int,
    ) -> u32;
}

const DT_FAILURE: u32 = 1 << 31;
const DT_PARTIAL: u32 = 1 << 6;
const MMAP_MAGIC: u32 = 0x4d4d_4150;
const MMAP_VERSION: u32 = 20;
const MMAP_HEADER_LEN: usize = 56;
/// A server grid cell, in yards.
pub const GRID_SIZE: f32 = 533.333_3;
/// Detour's search-node pool. The server uses 1024, which runs out on routes of a few hundred
/// yards; a whole zone needs far more.
const MAX_NODES: c_int = 65_535;
const MAX_POINTS: usize = 512;
/// What a yard inside an avoided circle costs, in yards: a detour up to this many times the
/// crossing's length is preferred.
pub const AVOID_COST: f32 = 10.0;

/// Terrain flags (`NavTerrainFlag`), also the area ids.
pub const NAV_GROUND: u16 = 0x01;
pub const NAV_MAGMA: u16 = 0x02;
pub const NAV_SLIME: u16 = 0x04;
pub const NAV_WATER: u16 = 0x08;
pub const NAV_GROUND_STEEP: u16 = 0x10;

/// A route in WoW coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub points: Vec<[f32; 3]>,
    /// False when the end could not be reached and the route stops at the closest point.
    pub complete: bool,
    /// The route crosses steep ground (the extractor's 50-60 degree band): no gentler one reached.
    pub steep: bool,
}

/// The server grid cell a WoW coordinate falls in (`MMMXXYY.mmtile`'s XX for x, YY for y).
pub fn grid(coord: f32) -> i32 {
    (32.0 - coord / GRID_SIZE).floor() as i32
}

fn to_recast(p: [f32; 3]) -> [f32; 3] {
    [p[1], p[2], p[0]]
}

fn from_recast(p: &[f32]) -> [f32; 3] {
    [p[2], p[0], p[1]]
}

/// The Detour payload of an `.mmtile`, after checking its header.
pub fn tile_payload(bytes: &[u8]) -> Result<&[u8], String> {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if word(0) != Some(MMAP_MAGIC) {
        return Err("not an mmtile (bad magic)".into());
    }
    if word(8) != Some(MMAP_VERSION) {
        return Err(format!(
            "mmtile version {:?}, expected {MMAP_VERSION}: regenerate the server's mmaps",
            word(8)
        ));
    }
    let size = word(12).unwrap_or(0) as usize;
    bytes
        .get(MMAP_HEADER_LEN..MMAP_HEADER_LEN + size)
        .ok_or_else(|| "mmtile shorter than its header says".into())
}

/// One map's navmesh, tiles loaded on demand around each query.
pub struct NavMesh {
    shim: NonNull<Shim>,
    dir: PathBuf,
    map: u32,
    /// Grid cells already tried (loaded, or with no tile: open water, out of the world).
    tried: HashSet<(i32, i32)>,
}

// The Detour objects are only touched through `&mut self`.
unsafe impl Send for NavMesh {}

impl Drop for NavMesh {
    fn drop(&mut self) {
        unsafe { nav_destroy(self.shim.as_ptr()) }
    }
}

impl NavMesh {
    /// Open map `map` from an mmaps folder; tiles load as routes need them.
    pub fn open(dir: &Path, map: u32) -> Result<NavMesh, String> {
        let params_path = dir.join(format!("{map:03}.mmap"));
        let params =
            std::fs::read(&params_path).map_err(|e| format!("{}: {e}", params_path.display()))?;
        let shim =
            NonNull::new(unsafe { nav_create(params.as_ptr(), params.len() as c_int, MAX_NODES) })
                .ok_or_else(|| format!("{}: not a navmesh header", params_path.display()))?;
        // Water at mod-playerbots' bot cost (`PathGenerator::CreateFilter`): swim only when needed.
        unsafe { nav_set_area_cost(shim.as_ptr(), NAV_WATER as c_int, 20.0) };
        Ok(NavMesh {
            shim,
            dir: dir.to_path_buf(),
            map,
            tried: HashSet::new(),
        })
    }

    /// Circles to route around, as WoW `[x, y, radius]` (an agent's hostile mobs): crossing one
    /// costs [`AVOID_COST`] times the distance, so a route goes round when a fair detour exists and
    /// through when none does. Kept for every later route until set again; empty clears them.
    pub fn set_avoid(&mut self, zones: &[[f32; 3]]) {
        let flat: Vec<f32> = zones.iter().flat_map(|z| [z[1], z[0], z[2]]).collect();
        let count = zones.len() as c_int;
        unsafe { nav_set_avoid(self.shim.as_ptr(), flat.as_ptr(), count, AVOID_COST) };
    }

    pub fn map(&self) -> u32 {
        self.map
    }

    /// Load the tile for grid cell `(gx, gy)` once. A missing file is not an error.
    fn ensure(&mut self, gx: i32, gy: i32) -> Result<(), String> {
        if !self.tried.insert((gx, gy)) {
            return Ok(());
        }
        let path = self
            .dir
            .join(format!("{:03}{gx:02}{gy:02}.mmtile", self.map));
        let Ok(bytes) = std::fs::read(&path) else {
            return Ok(());
        };
        let payload = tile_payload(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let status = unsafe {
            nav_add_tile(
                self.shim.as_ptr(),
                payload.as_ptr(),
                payload.len() as c_int,
                std::ptr::null_mut(),
            )
        };
        if status & DT_FAILURE != 0 {
            return Err(format!(
                "{}: Detour refused the tile ({status:#x})",
                path.display()
            ));
        }
        Ok(())
    }

    /// A walkable route from `from` to `to` (WoW coordinates), as Detour's straight path: the
    /// corners to walk through. Loads the tiles spanning both ends, plus a cell of margin.
    ///
    /// Like mod-playerbots' bots, it keeps off steep ground, which a walking player slides back
    /// down; the server's filter for real players allows it. Only when no gentle route reaches the
    /// end (or the walk starts on a steep face) does it take one over steep ground.
    pub fn route(&mut self, from: [f32; 3], to: [f32; 3]) -> Result<Route, String> {
        let (ax, ay, bx, by) = (grid(from[0]), grid(from[1]), grid(to[0]), grid(to[1]));
        for gx in ax.min(bx) - 1..=ax.max(bx) + 1 {
            for gy in ay.min(by) - 1..=ay.max(by) + 1 {
                self.ensure(gx, gy)?;
            }
        }
        let gentle = self.search(from, to, false);
        if gentle.as_ref().is_ok_and(|r| r.complete) {
            return gentle;
        }
        match self.search(from, to, true) {
            Ok(steep) if steep.complete => Ok(steep),
            steep => gentle.or(steep),
        }
    }

    fn search(&mut self, from: [f32; 3], to: [f32; 3], steep: bool) -> Result<Route, String> {
        let (include, exclude) = if steep {
            (
                NAV_GROUND | NAV_GROUND_STEEP | NAV_WATER,
                NAV_MAGMA | NAV_SLIME,
            )
        } else {
            (
                NAV_GROUND | NAV_WATER,
                NAV_MAGMA | NAV_SLIME | NAV_GROUND_STEEP,
            )
        };
        unsafe { nav_set_filter(self.shim.as_ptr(), include, exclude) };
        let (start, end) = (to_recast(from), to_recast(to));
        // The server's first search box; a point more than a few yards off the mesh finds nothing.
        let extents = [3.0f32, 5.0, 3.0];
        let mut out = vec![0f32; MAX_POINTS * 3];
        let (mut count, mut polys) = (0 as c_int, 0 as c_int);
        let mut status = unsafe {
            nav_find_straight_path(
                self.shim.as_ptr(),
                start.as_ptr(),
                end.as_ptr(),
                extents.as_ptr(),
                out.as_mut_ptr(),
                MAX_POINTS as c_int,
                &mut count,
                &mut polys,
            )
        };
        // The server's second try, a taller box for points above or below the mesh; then a much
        // taller one, for a height that is simply wrong (an agent's guess).
        for tall in [[3.0f32, 50.0, 3.0], [3.0f32, 400.0, 3.0]] {
            if status & DT_FAILURE == 0 && count > 0 {
                break;
            }
            status = unsafe {
                nav_find_straight_path(
                    self.shim.as_ptr(),
                    start.as_ptr(),
                    end.as_ptr(),
                    tall.as_ptr(),
                    out.as_mut_ptr(),
                    MAX_POINTS as c_int,
                    &mut count,
                    &mut polys,
                )
            };
        }
        if status & DT_FAILURE != 0 || count == 0 {
            return Err(format!(
                "no route: start or end is not on walkable ground ({status:#x})"
            ));
        }
        let points: Vec<[f32; 3]> = out[..count as usize * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| from_recast(p))
            .collect();
        let reached = points
            .last()
            .is_some_and(|p| (p[0] - to[0]).hypot(p[1] - to[1]) < 2.0);
        Ok(Route {
            complete: status & DT_PARTIAL == 0 && reached,
            points,
            steep,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_cells_match_the_server_naming() {
        // Northshire Abbey is in 0004832.mmtile.
        assert_eq!((grid(-8913.0), grid(-137.0)), (48, 32));
        assert_eq!(grid(0.0), 32);
    }

    #[test]
    fn coordinates_round_trip_through_recast_order() {
        let p = [-8907.2, -113.6, 81.9];
        assert_eq!(from_recast(&to_recast(p)), p);
    }

    #[test]
    fn tile_headers_are_checked() {
        let mut tile = vec![0u8; MMAP_HEADER_LEN + 4];
        tile[0..4].copy_from_slice(&MMAP_MAGIC.to_le_bytes());
        tile[8..12].copy_from_slice(&MMAP_VERSION.to_le_bytes());
        tile[12..16].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(tile_payload(&tile).unwrap().len(), 4);
        tile[8] = 19;
        assert!(tile_payload(&tile).unwrap_err().contains("version"));
        assert!(tile_payload(b"nope").is_err());
    }

    /// Needs real mmaps: `WRATHPLUS_MMAPS=<server data>/mmaps cargo test -p wrathplus-nav -- --ignored`.
    #[test]
    #[ignore = "needs the server's extracted mmaps"]
    fn a_route_into_northshire_abbey_goes_through_the_door() {
        let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
        let mut mesh = NavMesh::open(Path::new(&dir), 0).unwrap();
        let route = mesh
            .route([-8907.2, -113.6, 81.9], [-8902.6, -162.6, 82.0])
            .unwrap();
        assert!(route.complete, "{route:?}");
        // Through the abbey door near (-8910, -140), not straight through the wall.
        assert!(
            route
                .points
                .iter()
                .any(|p| (p[0] + 8910.4).abs() < 2.0 && (p[1] + 140.0).abs() < 3.0),
            "{route:?}"
        );
    }

    /// Goldshire's road to Stormwind with a circle on it, as a wolf pack would be: the route goes
    /// round, never through the middle, and still arrives.
    #[test]
    #[ignore = "needs the server's extracted mmaps"]
    fn a_route_goes_round_an_avoided_circle() {
        let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
        let mut mesh = NavMesh::open(Path::new(&dir), 0).unwrap();
        let (from, to) = ([-9460.0, 64.0, 56.0], [-9300.0, 140.0, 64.0]);
        let plain = mesh.route(from, to).unwrap();
        // A circle around the plain route's middle corner.
        let mid = plain.points[plain.points.len() / 2];
        mesh.set_avoid(&[[mid[0], mid[1], 15.0]]);
        let round = mesh.route(from, to).unwrap();
        mesh.set_avoid(&[]);
        assert!(round.complete, "{round:?}");
        let near = |r: &Route| {
            r.points
                .windows(2)
                .flat_map(|w| {
                    (0..=10).map(move |i| {
                        let t = i as f32 / 10.0;
                        [
                            w[0][0] + (w[1][0] - w[0][0]) * t,
                            w[0][1] + (w[1][1] - w[0][1]) * t,
                        ]
                    })
                })
                .map(|p| (p[0] - mid[0]).hypot(p[1] - mid[1]))
                .fold(f32::INFINITY, f32::min)
        };
        assert!(near(&plain) < 1.0);
        assert!(near(&round) > 7.5, "{round:?}");
    }

    /// The walk an agent got stuck on (#82): out of Northshire toward Goldshire. With steep ground
    /// allowed, the route's next corner was 12 yards up the valley wall, 14 yards away.
    #[test]
    #[ignore = "needs the server's extracted mmaps"]
    fn the_road_out_of_northshire_keeps_off_the_valley_wall() {
        let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
        let mut mesh = NavMesh::open(Path::new(&dir), 0).unwrap();
        let route = mesh
            .route([-9045.7, -129.3, 96.0], [-9464.0, 64.0, 56.0])
            .unwrap();
        assert!(route.complete && !route.steep, "{route:?}");
        // The corner on the wall the stuck walk kept falling short of.
        assert!(
            !route
                .points
                .iter()
                .any(|p| (p[0] + 9058.1).hypot(p[1] + 121.1) < 3.0),
            "{route:?}"
        );
    }
}
