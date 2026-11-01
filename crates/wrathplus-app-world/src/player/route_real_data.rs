//! The session checklist's collision spots (`https://github.com/artivus-labs/wrathplus/wiki/WRATH-SESSION-CHECKLIST`, B2) walked headlessly
//! on the installed 3.3.5 data: the mover's own [`mover::step`] at a 60 fps run over the collision the
//! terrain streamer builds for the same tiles. That means the terrain trimesh, every WMO's walking
//! faces (all but MOPY `DETAIL`), the WMO doodad sets' hulls and the ADT doodads' hulls, each placed
//! as the spawn paths place it. Each route asserts that the body reaches every waypoint (no
//! run-in-place stall) and never drops a fall below the path (no fall-through).
//!
//! Labelled, not built: impassable-chunk fences, game objects (doors are server spawns), and the
//! camera-only faces, which the body never reads. The liquid half of the spots (no false swim hold)
//! is `wrathplus-world`'s `liquid::real_data::the_checklist_collision_spots_are_dry`. Skips without a Wrath install
//! (`WOW_CLIENT_PROFILE=wrath`); nothing is written or committed.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use wrathplus_assets::coords::{placement_rotation, wmo_doodad_local, wow_to_bevy};
use wrathplus_formats::{CollisionMesh, DataProfile};

use super::{mover, Player, AIR_NUDGE_SPEED, CAPSULE_HEIGHT, CAPSULE_RADIUS};

mod terrain_fall;

/// A run, yards per second.
const RUN: f32 = 7.0;
/// The frame step: 60 fps.
const DT: f32 = 1.0 / 60.0;

/// A headless physics world holding `map`'s collision around `points` (WoW coordinates); `None`
/// without a Wrath install.
fn scene(map: &str, points: &[[f32; 3]]) -> Option<App> {
    let data = wrathplus_formats::wow_data_or_skip!(None);
    let profile = DataProfile::from_env().expect("valid WOW_CLIENT_PROFILE/WOW_LOCALE");
    if !matches!(profile, DataProfile::Wrath { .. }) {
        eprintln!("skipped: WOW_CLIENT_PROFILE is not wrath");
        return None;
    }
    let mut chain =
        wrathplus_formats::Chain::open_profile(&data, &profile).expect("open the Wrath chain");

    // Each point's tile and its eight neighbours: a building is placed in every tile it straddles.
    let mut tiles: Vec<(u32, u32)> = Vec::new();
    for p in points {
        let (tx, ty) = wrathplus_formats::world_to_tile(p[0], p[1]);
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                let t = ((tx as i32 + dx) as u32, (ty as i32 + dy) as u32);
                if !tiles.contains(&t) {
                    tiles.push(t);
                }
            }
        }
    }

    let mut app = App::new();
    app.init_resource::<wrathplus_world::collision::MoverTraceExclusions>();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::scene::ScenePlugin,
        PhysicsPlugins::new(bevy::app::PostUpdate),
    ));
    app.init_asset::<Mesh>();
    // Finish Avian's diagnostics setup before advancing the headless physics schedule.
    app.finish();
    app.cleanup();

    let mut hulls: HashMap<String, Option<CollisionMesh>> = HashMap::new();
    let mut hull = |chain: &mut wrathplus_formats::Chain, path: &str| {
        hulls
            .entry(path.to_ascii_lowercase())
            .or_insert_with(|| {
                wrathplus_formats::load_m2_collision_hull(chain, path)
                    .ok()
                    .filter(|m| !m.is_empty())
            })
            .clone()
    };
    let mut bodies: Vec<(String, Vec<Vec3>, Vec<[u32; 3]>)> = Vec::new();
    let placed =
        |bodies: &mut Vec<_>, source: String, mesh: &CollisionMesh, transform: &Transform| {
            if let Some((verts, tris)) =
                wrathplus_world::terrain_stream::placement_collider_data(Some(mesh), transform)
            {
                bodies.push((source, verts, tris));
            }
        };
    let mut seen_wmos: Vec<u32> = Vec::new();
    let mut seen_doodads: Vec<u32> = Vec::new();
    for (tx, ty) in tiles {
        let Ok(tile) = wrathplus_formats::load_tile_mesh(&mut chain, map, tx, ty) else {
            continue;
        };
        // The terrain: one trimesh welded from the tile's chunks, holes already dropped.
        let (mut verts, mut tris) = (Vec::new(), Vec::new());
        for chunk in &tile.chunks {
            let base = verts.len() as u32;
            verts.extend(chunk.positions.iter().map(|p| wow_to_bevy(*p)));
            tris.extend(
                chunk
                    .indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| [base + c[0], base + c[1], base + c[2]]),
            );
        }
        if !tris.is_empty() {
            bodies.push((format!("terrain {tx}_{ty}"), verts, tris));
        }
        for d in &tile.doodads {
            if seen_doodads.contains(&d.unique_id) {
                continue;
            }
            seen_doodads.push(d.unique_id);
            if let Some(mesh) = hull(&mut chain, &d.model) {
                placed(
                    &mut bodies,
                    format!("doodad {}", d.model),
                    &mesh,
                    &Transform {
                        translation: wow_to_bevy(d.position),
                        rotation: placement_rotation(d.rotation),
                        scale: Vec3::splat(d.scale),
                    },
                );
            }
        }
        for w in &tile.wmos {
            // Once per `unique_id`, as the streamer dedups a straddling building.
            if seen_wmos.contains(&w.unique_id) {
                continue;
            }
            seen_wmos.push(w.unique_id);
            let transform = Transform {
                translation: wow_to_bevy(w.position),
                rotation: placement_rotation(w.rotation),
                scale: Vec3::ONE,
            };
            if let Ok(mesh) = wrathplus_formats::load_wmo_collision_tris(&mut chain, &w.model) {
                placed(&mut bodies, format!("wmo {}", w.model), &mesh, &transform);
            }
            // The placement's doodads: the global set 0 and its own set.
            let Ok(bytes) = chain.read_file(&w.model.to_ascii_lowercase()) else {
                continue;
            };
            let Ok(root) = wrathplus_formats::parse_wmo_root(&bytes) else {
                continue;
            };
            for (si, set) in root.doodad_sets().iter().enumerate() {
                if si != 0 && si != usize::from(w.doodad_set) {
                    continue;
                }
                let range = set.start as usize..(set.start + set.count) as usize;
                for doodad in root.doodads().get(range).unwrap_or_default() {
                    if doodad.model.is_empty() {
                        continue;
                    }
                    if let Some(mesh) = hull(&mut chain, &doodad.model) {
                        let local =
                            wmo_doodad_local(doodad.position, doodad.orientation, doodad.scale);
                        placed(
                            &mut bodies,
                            format!("wmo doodad {}", doodad.model),
                            &mesh,
                            &transform.mul_transform(local),
                        );
                    }
                }
            }
        }
    }
    for (source, verts, tris) in bodies {
        app.world_mut().spawn((
            Name::new(source),
            RigidBody::Static,
            Collider::trimesh(verts, tris),
            Transform::default(),
        ));
    }
    // The first update seats transforms; physics first steps on the second update.
    app.update();
    app.update();
    Some(app)
}

/// The walking floors under a WoW `(x, y)`, top down, from a down-ray at `top`.
fn floors(app: &mut App, x: f32, y: f32, top: f32, depth: f32) -> Vec<f32> {
    app.world_mut()
        .run_system_once(move |query: SpatialQuery| {
            // One hit per cast, stepping past it: `ray_hits` reports one hit per collider, and a
            // WMO's floors are one trimesh.
            let mut zs = Vec::new();
            let mut z = top;
            while z > top - depth {
                let Some(hit) = query.cast_ray(
                    wow_to_bevy([x, y, z]),
                    Dir3::NEG_Y,
                    z - (top - depth),
                    true,
                    &SpatialQueryFilter::default(),
                ) else {
                    break;
                };
                zs.push(z - hit.distance);
                z -= hit.distance + 0.05;
            }
            zs
        })
        .unwrap()
}

/// How far under the path's height the feet may go before the walk counts a fall-through.
const FALL_LIMIT: f32 = 3.0;

/// Run the mover from `route[0]` through each later waypoint in turn, steering straight at it.
fn walk(app: &mut App, route: &[[f32; 3]]) -> Result<(), String> {
    let route = route.to_vec();
    app.world_mut()
        .run_system_once(move |world: wrathplus_world::collision::WorldCollision| {
            let capsule = Collider::capsule(CAPSULE_RADIUS, CAPSULE_HEIGHT - 2.0 * CAPSULE_RADIUS);
            let mut player = Player {
                pos: wow_to_bevy(route[0]) + Vec3::Y * 0.05,
                ..Default::default()
            };
            let mut time = Time::<()>::default();
            let wow = |p: Vec3| wrathplus_assets::coords::bevy_to_wow(p);
            for (i, target) in route.iter().enumerate().skip(1) {
                let from = route[i - 1];
                let goal = wow_to_bevy(*target);
                let flat = |p: Vec3| Vec2::new(p.x, p.z);
                let mut best = f32::INFINITY;
                let mut since_best = 0;
                loop {
                    let left = flat(goal) - flat(player.pos);
                    if left.length() < 0.6 {
                        break;
                    }
                    let dir = Vec3::new(left.x, 0.0, left.y).normalize();
                    time.advance_by(std::time::Duration::from_secs_f32(DT));
                    mover::step(
                        &mut player,
                        &time,
                        &world,
                        &capsule,
                        true,
                        dir,
                        RUN,
                        false,
                        false,
                        None,
                        None,
                        AIR_NUDGE_SPEED,
                    );
                    let here = wow(player.pos);
                    // The path's height under the body, interpolated along the leg.
                    let span = Vec2::new(target[0] - from[0], target[1] - from[1]);
                    let t = (Vec2::new(here[0] - from[0], here[1] - from[1]).dot(span)
                        / span.length_squared().max(1e-6))
                    .clamp(0.0, 1.0);
                    let path_z = from[2] + (target[2] - from[2]) * t;
                    if here[2] < path_z - FALL_LIMIT {
                        return Err(format!("fell short of waypoint {i} at {here:?}"));
                    }
                    let d = (flat(goal) - flat(player.pos)).length();
                    if d < best - 0.05 {
                        best = d;
                        since_best = 0;
                    } else {
                        since_best += 1;
                        if since_best > 60 {
                            return Err(format!("stalled short of waypoint {i} at {here:?}"));
                        }
                    }
                }
            }
            Ok(())
        })
        .unwrap()
}

/// Print the floor heights on a grid, to pick a route's waypoints from the real geometry:
/// `WOW_ROUTE_PROBE="map x0 y0 x1 y1 top"` (WoW coordinates).
#[test]
#[ignore = "a probe: WOW_ROUTE_PROBE and an installed Wrath client"]
fn probe_floors() {
    let Ok(spec) = std::env::var("WOW_ROUTE_PROBE") else {
        eprintln!("skipped: WOW_ROUTE_PROBE is not set");
        return;
    };
    let parts: Vec<&str> = spec.split_whitespace().collect();
    let map = parts[0];
    let n: Vec<f32> = parts[1..].iter().map(|s| s.parse().unwrap()).collect();
    let (x0, y0, x1, y1, top) = (n[0], n[1], n[2], n[3], n[4]);
    let Some(mut app) = scene(map, &[[x0, y0, top], [x1, y1, top]]) else {
        return;
    };
    let mut x = x0;
    while x <= x1 {
        let mut row = format!("x={x:8.1}:");
        let mut y = y0;
        while y <= y1 {
            let zs = floors(&mut app, x, y, top, 40.0);
            let cell = zs
                .iter()
                .map(|z| format!("{z:.1}"))
                .collect::<Vec<_>>()
                .join("/");
            row += &format!(" {}", if cell.is_empty() { "-" } else { &cell });
            y += 2.0;
        }
        println!("{row}");
        x += 2.0;
    }
}

/// Northshire Abbey, out through the front door: the walk session 144407 sent to AzerothCore
/// (`target/azb-live/144407/movement.log`), from the hall past the 10-15 stuck anchor, down the
/// front steps and into the yard.
const ABBEY_OUT: &[[f32; 3]] = &[
    [-8904.2, -157.8, 82.0],
    [-8907.6, -150.1, 82.0],
    [-8910.6, -143.6, 82.0],
    [-8912.0, -140.4, 82.2],
    [-8913.5, -137.2, 80.7],
    [-8916.4, -133.1, 80.6],
    [-8916.8, -124.0, 81.6],
    [-8918.4, -122.1, 81.9],
    [-8907.5, -117.7, 82.0],
    [-8895.6, -121.7, 81.8],
    [-8879.4, -125.9, 81.4],
];

/// The Lion's Pride Inn, Goldshire: from the road (`game_tele` "Goldshire") in to the hall, through
/// the one gap in the hall's south wall (x −9467.5: capsule sweeps clear from both sides; tables,
/// chairs and the WMO close the rest of the line), up both stair flights (probed: z 57 → landing
/// 60.2 → 63.8) and to the upper floor's north room (an AzerothCore creature spawn).
const GOLDSHIRE_INN: &[[f32; 3]] = &[
    [-9448.5, 68.2, 56.3],
    [-9454.7, 49.0, 56.7],
    [-9460.0, 36.0, 57.0],
    [-9467.5, 38.0, 57.0],
    [-9467.5, 28.0, 57.0],
    [-9476.0, 24.0, 57.0],
    [-9477.0, 20.0, 57.9],
    [-9477.0, 14.0, 60.2],
    [-9472.0, 13.0, 60.2],
    [-9472.0, 18.0, 62.4],
    [-9472.0, 24.0, 63.8],
    [-9471.7, 34.5, 63.9],
];

/// Stormwind's main gate: the guard posts on the upper deck (z 96.4–96.9) and the ones below the
/// ramp (z 93.9), AzerothCore creature spawns.
const STORMWIND_GATE: &[[f32; 3]] = &[
    [-8995.8, 496.2, 96.9],
    [-8958.4, 509.0, 96.7],
    [-8938.2, 518.2, 96.5],
    [-8918.7, 495.7, 93.9],
    [-8927.8, 481.3, 93.9],
];

/// The Deeprun Tram's Stormwind entrance: from where AzerothCore lands a player leaving the tram
/// (`areatrigger_teleport` 2171, z 91.8) up the entrance corridor to the Dwarven District street,
/// over the stacked floors the probe shows there (91.8 over a 90.6 slab, the tunnel floor at 87.5).
const TRAM_ENTRANCE: &[[f32; 3]] = &[
    [-8364.6, 536.0, 91.8],
    [-8372.0, 543.0, 91.8],
    [-8379.0, 551.0, 91.8],
    [-8384.0, 560.0, 91.8],
    [-8387.0, 568.0, 91.8],
    [-8389.0, 576.0, 91.4],
];

/// Every checklist collision route on the installed data: each reaches its last waypoint on the
/// ground, with no stall and no fall-through.
#[test]
fn the_checklist_collision_spots_walk_clean() {
    let reversed = |r: &[[f32; 3]]| r.iter().rev().copied().collect::<Vec<_>>();
    let routes: Vec<(&str, Vec<[f32; 3]>)> = vec![
        ("abbey out", ABBEY_OUT.to_vec()),
        ("abbey in", reversed(ABBEY_OUT)),
        ("goldshire inn up", GOLDSHIRE_INN.to_vec()),
        ("goldshire inn down", reversed(GOLDSHIRE_INN)),
        ("stormwind gate in", STORMWIND_GATE.to_vec()),
        ("stormwind gate out", reversed(STORMWIND_GATE)),
        ("tram entrance out", TRAM_ENTRANCE.to_vec()),
        ("tram entrance in", reversed(TRAM_ENTRANCE)),
    ];
    let points: Vec<[f32; 3]> = routes.iter().flat_map(|(_, r)| r.iter().copied()).collect();
    let Some(mut app) = scene("Azeroth", &points) else {
        return;
    };
    let failures: Vec<String> = routes
        .iter()
        .filter_map(|(name, route)| walk(&mut app, route).err().map(|f| format!("{name}: {f}")))
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Walk candidate routes and print each verdict, to pick waypoints the geometry allows:
/// `WOW_ROUTE_WALK="map|x,y,z;x,y,z;...|x,y,z;..."` (WoW coordinates, one route per `|`).
#[test]
#[ignore = "a probe: WOW_ROUTE_WALK and an installed Wrath client"]
fn probe_walks() {
    let Ok(spec) = std::env::var("WOW_ROUTE_WALK") else {
        eprintln!("skipped: WOW_ROUTE_WALK is not set");
        return;
    };
    let mut parts = spec.split('|');
    let map = parts.next().expect("map");
    let routes: Vec<Vec<[f32; 3]>> = parts
        .map(|r| {
            r.split(';')
                .map(|p| {
                    let v: Vec<f32> = p.split(',').map(|n| n.trim().parse().unwrap()).collect();
                    [v[0], v[1], v[2]]
                })
                .collect()
        })
        .collect();
    let points: Vec<[f32; 3]> = routes.iter().flatten().copied().collect();
    let Some(mut app) = scene(map, &points) else {
        return;
    };
    for route in &routes {
        println!("{route:?}: {:?}", walk(&mut app, route));
    }
}

/// Sweep the capsule from each start along its direction and print what stops it:
/// `WOW_ROUTE_HIT="map|x,y,z,dx,dy;..."` (WoW coordinates, feet, a horizontal direction).
#[test]
#[ignore = "a probe: WOW_ROUTE_HIT and an installed Wrath client"]
fn probe_hits() {
    let Ok(spec) = std::env::var("WOW_ROUTE_HIT") else {
        eprintln!("skipped: WOW_ROUTE_HIT is not set");
        return;
    };
    let (map, rest) = spec.split_once('|').expect("map|casts");
    let casts: Vec<[f32; 5]> = rest
        .split(';')
        .map(|c| {
            let v: Vec<f32> = c.split(',').map(|n| n.trim().parse().unwrap()).collect();
            [v[0], v[1], v[2], v[3], v[4]]
        })
        .collect();
    let points: Vec<[f32; 3]> = casts.iter().map(|c| [c[0], c[1], c[2]]).collect();
    let Some(mut app) = scene(map, &points) else {
        return;
    };
    app.world_mut()
        .run_system_once(
            move |world: wrathplus_world::collision::WorldCollision, names: Query<&Name>| {
                let capsule =
                    Collider::capsule(CAPSULE_RADIUS, CAPSULE_HEIGHT - 2.0 * CAPSULE_RADIUS);
                for c in &casts {
                    let center =
                        wow_to_bevy([c[0], c[1], c[2]]) + Vec3::Y * (CAPSULE_HEIGHT * 0.5 + 0.05);
                    let dir = (wow_to_bevy([c[0] + c[3], c[1] + c[4], c[2]])
                        - wow_to_bevy([c[0], c[1], c[2]]))
                    .normalize();
                    match world.cast_mover(&capsule, center, dir * 6.0, 0.02) {
                        Some(h) => println!(
                            "{c:?}: hit {:?} at {:.2} yd, normal {:?}",
                            names.get(h.entity).map(|n| n.as_str()).unwrap_or("?"),
                            h.distance,
                            wrathplus_assets::coords::bevy_to_wow(h.normal1)
                        ),
                        None => println!("{c:?}: clear for 6 yd"),
                    }
                }
            },
        )
        .unwrap();
}

mod terrain_support;
