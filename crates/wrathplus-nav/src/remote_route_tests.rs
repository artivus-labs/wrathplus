//! The realm module's route search (`mod-agent-nav`'s `AgentNavRoute.h`, through
//! `csrc/realm_module_shim.cpp`) against this crate's own.

use crate::Route;
use std::ffi::{c_char, c_int, c_void, CString};

extern "C" {
    fn realm_cache_new(dir: *const c_char, nodes: c_int, tiles: c_int) -> *mut c_void;
    fn realm_cache_free(cache: *mut c_void);
    fn realm_cache_tiles(cache: *mut c_void) -> c_int;
    fn realm_cache_route(
        cache: *mut c_void,
        map: u32,
        from: *const f32,
        to: *const f32,
        avoid: *const f32,
        avoid_count: c_int,
        end_snap: f32,
        max_points: c_int,
        any_height: c_int,
        points: *mut f32,
        flags: *mut c_int,
    ) -> c_int;
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[test]
fn a_realm_without_mmaps_for_the_map_says_so() {
    let dir = std::env::temp_dir().join(format!("wrathplus-nav-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = c(dir.to_str().unwrap());
    let cache = unsafe { realm_cache_new(path.as_ptr(), 65_535, 16) };
    let mut points = vec![0f32; 256 * 3];
    let mut flags = 0;
    let (from, to) = ([0f32, 0.0, 0.0], [10f32, 0.0, 0.0]);
    for _ in 0..2 {
        let count = unsafe {
            realm_cache_route(
                cache,
                0,
                from.as_ptr(),
                to.as_ptr(),
                std::ptr::null(),
                0,
                0.0,
                256,
                0,
                points.as_mut_ptr(),
                &mut flags,
            )
        };
        assert_eq!(count, -1);
    }
    unsafe { realm_cache_free(cache) };
    let _ = std::fs::remove_dir(&dir);
}

/// The module's route for a request, as the realm would compute it.
fn module_route(
    cache: *mut c_void,
    from: [f32; 3],
    to: [f32; 3],
    avoid: &[[f32; 3]],
    max_points: c_int,
    any_height: bool,
) -> Result<Route, c_int> {
    let mut points = vec![0f32; 512 * 3];
    let mut flags = 0;
    let flat: Vec<f32> = avoid.iter().flatten().copied().collect();
    let count = unsafe {
        realm_cache_route(
            cache,
            0,
            from.as_ptr(),
            to.as_ptr(),
            flat.as_ptr(),
            avoid.len() as c_int,
            0.0,
            max_points,
            c_int::from(any_height),
            points.as_mut_ptr(),
            &mut flags,
        )
    };
    if count < 0 {
        return Err(count);
    }
    Ok(Route {
        points: points[..count as usize * 3].as_chunks::<3>().0.to_vec(),
        complete: flags & 1 != 0 && flags & 4 == 0,
        steep: flags & 2 != 0,
    })
}

/// A route request: from, to, and the circles to go round.
type Case<'a> = ([f32; 3], [f32; 3], &'a [[f32; 3]]);

/// Needs real mmaps: `WRATHPLUS_MMAPS=<server data>/mmaps cargo test -p wrathplus-nav -- --ignored`.
/// The realm's module and the client's own search give the same route, corner for corner: into
/// the abbey, out of Northshire (the steep valley wall), round an avoided circle, and to a place
/// off the mesh; and the module keeps doing so while it drops tiles to stay within its limit.
#[test]
#[ignore = "needs the server's extracted mmaps"]
fn the_module_routes_as_the_client_does() {
    let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
    let mut mesh = crate::NavMesh::open(std::path::Path::new(&dir), 0).unwrap();
    let path = c(&dir);
    let roomy = unsafe { realm_cache_new(path.as_ptr(), 65_535, 128) };
    // Nine tiles is one short route's worth: every longer one makes it drop some.
    let tight = unsafe { realm_cache_new(path.as_ptr(), 65_535, 9) };
    let circle = [[-9380.0f32, 100.0, 15.0]];
    let cases: [Case; 6] = [
        ([-8907.2, -113.6, 81.9], [-8902.6, -162.6, 82.0], &[]),
        ([-9045.7, -129.3, 96.0], [-9464.0, 64.0, 56.0], &[]),
        ([-9460.0, 64.0, 56.0], [-9300.0, 140.0, 64.0], &[]),
        ([-9460.0, 64.0, 56.0], [-9300.0, 140.0, 64.0], &circle),
        ([-8907.2, -113.6, 81.9], [-9464.0, 64.0, 300.0], &[]),
        ([-8907.2, -113.6, 81.9], [-10500.0, -1160.0, 28.0], &[]),
    ];
    for (from, to, avoid) in cases {
        mesh.set_avoid(avoid);
        let want = mesh.route(from, to);
        for cache in [roomy, tight] {
            let got = module_route(cache, from, to, avoid, 512, false);
            match (&want, &got) {
                (Ok(want), Ok(got)) => {
                    assert_eq!(
                        (got.complete, got.steep),
                        (want.complete, want.steep),
                        "{from:?} {to:?}"
                    );
                    assert_eq!(got.points.len(), want.points.len(), "{from:?} {to:?}");
                    for (g, w) in got.points.iter().zip(&want.points) {
                        assert!((0..3).all(|i| (g[i] - w[i]).abs() < 0.01), "{g:?} {w:?}");
                    }
                }
                (Err(_), Err(-2)) => {}
                other => panic!("{from:?} {to:?}: {other:?}"),
            }
        }
    }
    mesh.set_avoid(&[]);
    assert!(unsafe { realm_cache_tiles(tight) } <= 45);
    assert!(unsafe { realm_cache_tiles(roomy) } > unsafe { realm_cache_tiles(tight) });
    // The corner limit cuts a route and says so.
    let cut = module_route(
        roomy,
        [-9045.7, -129.3, 96.0],
        [-9464.0, 64.0, 56.0],
        &[],
        3,
        false,
    )
    .unwrap();
    assert!(cut.points.len() == 3 && !cut.complete);
    // A point in the sea is not on walkable ground.
    assert_eq!(
        module_route(
            roomy,
            [-8907.2, -113.6, 81.9],
            [-8300.0, 3000.0, 0.0],
            &[],
            512,
            false
        )
        .map(|_| ()),
        mesh.route([-8907.2, -113.6, 81.9], [-8300.0, 3000.0, 0.0])
            .map(|_| ())
            .map_err(|_| -2)
    );
    unsafe {
        realm_cache_free(roomy);
        realm_cache_free(tight);
    }
}

/// Needs real mmaps. A target given without its height (`h`): Elmore's Task is handed in at the
/// quest area (-8387, 685), on a floor in Stormwind's Dwarven District at z 95. From Goldshire
/// (z 58) the flat point at the player's height gives a route ending on another level; the module
/// looks on other levels as the client does (`wrathplus-agent` `nav::heights`) and finds the floor,
/// with the same route the client's search gives to that height.
#[test]
#[ignore = "needs the server's extracted mmaps"]
fn the_module_finds_the_floor_of_a_target_given_without_a_height() {
    let dir = std::env::var("WRATHPLUS_MMAPS").expect("set WRATHPLUS_MMAPS");
    let mut mesh = crate::NavMesh::open(std::path::Path::new(&dir), 0).unwrap();
    let path = c(&dir);
    let cache = unsafe { realm_cache_new(path.as_ptr(), 65_535, 128) };
    let (from, flat) = ([-9462.0, 88.0, 58.0], [-8387.0, 685.0, 58.0]);
    let guessed = module_route(cache, from, flat, &[], 512, false).unwrap();
    assert!(!guessed.complete, "{:?}", guessed.points.last());
    let found = module_route(cache, from, flat, &[], 512, true).unwrap();
    assert!(found.complete, "{:?}", found.points.last());
    let end = *found.points.last().unwrap();
    assert!(
        (end[0] + 8387.0).abs() < 2.0
            && (end[1] - 685.0).abs() < 2.0
            && (end[2] - 95.0).abs() < 3.0,
        "{end:?}"
    );
    // The heights tried are the client's: 15 yards apart, nearest first, up then down; the one
    // found is among them, and the client's own route to it is the same.
    let z = (1..=10)
        .flat_map(|i| [58.0 + 15.0 * i as f32, 58.0 - 15.0 * i as f32])
        .find(|z| {
            mesh.route(from, [flat[0], flat[1], *z])
                .is_ok_and(|r| r.complete)
        })
        .expect("a height the client finds");
    let client = mesh.route(from, [flat[0], flat[1], z]).unwrap();
    assert_eq!(client.points.len(), found.points.len());
    for (g, w) in found.points.iter().zip(&client.points) {
        assert!((0..3).all(|i| (g[i] - w[i]).abs() < 0.01), "{g:?} {w:?}");
    }
    unsafe { realm_cache_free(cache) };
}
