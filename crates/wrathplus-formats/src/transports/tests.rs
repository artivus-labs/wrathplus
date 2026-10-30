//! Installed-route checks against independently executed AzerothCore GeneratePath timestamps.

use super::*;
use crate::taxi::load_taxi_path_nodes;

struct Golden {
    path_id: u32,
    move_speed: f32,
    accel_rate: f32,
}

/// Installed routes and AC gameobject_template speed/acceleration.
const GOLDENS: &[Golden] = &[
    Golden {
        path_id: 967,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 241,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 302,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 285,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 292,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 293,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 295,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 301,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 303,
        move_speed: 30.0,
        accel_rate: 1.0,
    },
    Golden {
        path_id: 436,
        move_speed: 1.0,
        accel_rate: 1.0,
    },
];

/// Also proves that at least one live transport crosses continents.
#[test]
fn touches_map_matches_the_paths_map_set() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_taxi_path_nodes(&mut chain).expect("load TaxiPathNode");

    let mut any_cross_map = false;
    for g in GOLDENS {
        let nodes = cat
            .path(g.path_id)
            .unwrap_or_else(|| panic!("path {} exists in TaxiPathNode.dbc", g.path_id));
        let tt = TransportTimetable::build(nodes, g.move_speed, g.accel_rate)
            .unwrap_or_else(|| panic!("path {} builds", g.path_id));
        let maps: std::collections::HashSet<u32> = nodes.iter().map(|n| n.map_id).collect();
        any_cross_map |= maps.len() > 1;
        for &m in &maps {
            assert!(tt.touches_map(m), "path {} should touch map {m}", g.path_id);
        }
        assert!(
            !tt.touches_map(9999),
            "path {} touches a map no node lives on",
            g.path_id
        );
    }
    assert!(
        any_cross_map,
        "no golden path crosses continents — the worldport spare predicate has nothing to spare"
    );
}

/// From any instant, the answer samples on the asked-for map, on every cross-continent path:
/// path 241 reaches map 1 only across the cycle wrap.
#[test]
fn first_cycle_on_map_lands_on_that_map() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_taxi_path_nodes(&mut chain).expect("load TaxiPathNode");

    let mut crossed = 0usize;
    for g in GOLDENS {
        let nodes = cat
            .path(g.path_id)
            .unwrap_or_else(|| panic!("path {} exists in TaxiPathNode.dbc", g.path_id));
        let tt = TransportTimetable::build(nodes, g.move_speed, g.accel_rate)
            .unwrap_or_else(|| panic!("path {} builds", g.path_id));
        let maps: std::collections::HashSet<u32> = nodes.iter().map(|n| n.map_id).collect();
        if maps.len() < 2 {
            continue;
        }
        crossed += 1;
        // 64 instants across the cycle, both sides of the wrap included.
        for k in 0..64u32 {
            let from = (u64::from(tt.period_ms) * u64::from(k) / 64) as u32;
            for &m in &maps {
                let at = tt.first_cycle_on_map(from, m).unwrap_or_else(|| {
                    panic!(
                        "path {} touches map {m} but found no cycle on it",
                        g.path_id
                    )
                });
                assert!(
                    at < tt.period_ms,
                    "path {} map {m}: {at} ms is outside the {} ms cycle",
                    g.path_id,
                    tt.period_ms
                );
                assert_eq!(
                    tt.sample(at).map,
                    m,
                    "path {} from {from} ms: first_cycle_on_map({m}) = {at} ms samples map {}",
                    g.path_id,
                    tt.sample(at).map
                );
            }
        }
        // A map the path never visits.
        assert_eq!(tt.first_cycle_on_map(0, 9999), None);
    }
    assert!(
        crossed > 0,
        "no golden path crosses continents — this test proves nothing"
    );
}

/// Oracle: local AC Maps/TransportMgr.cpp GeneratePath executed independently in C++
/// over the server's extracted build-12340 TaxiPathNode.dbc, with Spline.h's 3 chords,
/// G3D float vector lengths and TransportSpline<double> knots. Values are timings only;
/// no installed positions/assets are embedded. All frames, not just a derived period, are pinned.
#[test]
fn installed_routes_match_ac_keyframe_timestamps() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_taxi_path_nodes(&mut chain).expect("taxi nodes");
    // (path, period, [(map, arrival, departure, next arrival)])
    type FrameTimes = (u32, u32, u32, u32);
    let expected: &[(u32, u32, &[FrameTimes])] = &[
        (
            241,
            231110,
            &[
                (1, 0, 0, 4670),
                (1, 4670, 4670, 11066),
                (1, 11066, 11066, 15821),
                (1, 15821, 15821, 19670),
                (1, 19670, 19670, 30422),
                (1, 30422, 90422, 101450),
                (1, 101450, 101450, 105087),
                (1, 105087, 105087, 109073),
                (1, 109073, 109073, 115558),
                (1, 115558, 115558, 115558),
                (0, 115558, 115558, 121607),
                (0, 121607, 121607, 127749),
                (0, 127749, 127749, 140818),
                (0, 140818, 200818, 215434),
                (0, 215434, 215434, 221062),
                (0, 221062, 221062, 226072),
                (0, 226072, 226072, 231110),
                (0, 231110, 231110, 231110),
            ],
        ),
        (
            302,
            239262,
            &[
                (1, 0, 0, 8367),
                (1, 8367, 8367, 13634),
                (1, 13634, 13634, 23675),
                (1, 23675, 83675, 98160),
                (1, 98160, 98160, 100748),
                (1, 100748, 100748, 103449),
                (1, 103449, 103449, 106351),
                (1, 106351, 106351, 109559),
                (1, 109559, 109559, 114240),
                (1, 114240, 114240, 118617),
                (1, 118617, 118617, 118617),
                (0, 118617, 118617, 124062),
                (0, 124062, 124062, 129849),
                (0, 129849, 129849, 137970),
                (0, 137970, 137970, 151661),
                (0, 151661, 211661, 228372),
                (0, 228372, 228372, 239262),
                (0, 239262, 239262, 239262),
            ],
        ),
        (
            285,
            248842,
            &[
                (0, 0, 0, 11835),
                (0, 11835, 11835, 30331),
                (0, 30331, 90331, 106966),
                (0, 106966, 106966, 113236),
                (0, 113236, 113236, 118389),
                (0, 118389, 118389, 118389),
                (1, 118389, 118389, 124707),
                (1, 124707, 124707, 132435),
                (1, 132435, 132435, 137462),
                (1, 137462, 137462, 142261),
                (1, 142261, 142261, 158898),
                (1, 158898, 218898, 237256),
                (1, 237256, 237256, 248842),
                (1, 248842, 248842, 248842),
            ],
        ),
        (
            292,
            230246,
            &[
                (0, 0, 0, 6342),
                (0, 6342, 6342, 10669),
                (0, 10669, 10669, 16338),
                (0, 16338, 16338, 30138),
                (0, 30138, 90138, 104193),
                (0, 104193, 104193, 112137),
                (0, 112137, 112137, 118809),
                (0, 118809, 118809, 118809),
                (1, 118809, 118809, 124155),
                (1, 124155, 124155, 130338),
                (1, 130338, 130338, 144365),
                (1, 144365, 204365, 218324),
                (1, 218324, 218324, 224027),
                (1, 224027, 224027, 230246),
                (1, 230246, 230246, 230246),
            ],
        ),
        (
            293,
            312177,
            &[
                (1, 0, 0, 6941),
                (1, 6941, 6941, 12279),
                (1, 12279, 12279, 17788),
                (1, 17788, 17788, 34233),
                (1, 34233, 94233, 110549),
                (1, 110549, 110549, 115154),
                (1, 115154, 115154, 119776),
                (1, 119776, 119776, 127229),
                (1, 127229, 127229, 134584),
                (1, 134584, 134584, 144782),
                (1, 144782, 144782, 155544),
                (1, 155544, 155544, 167005),
                (1, 167005, 167005, 172124),
                (1, 172124, 172124, 177371),
                (1, 177371, 177371, 183851),
                (1, 183851, 183851, 200433),
                (1, 200433, 260433, 274457),
                (1, 274457, 274457, 280521),
                (1, 280521, 280521, 286368),
                (1, 286368, 286368, 292879),
                (1, 292879, 292879, 312177),
                (1, 312177, 312177, 312177),
            ],
        ),
        (
            295,
            241715,
            &[
                (0, 0, 0, 6092),
                (0, 6092, 6092, 10869),
                (0, 10869, 10869, 15127),
                (0, 15127, 15127, 19809),
                (0, 19809, 19809, 31527),
                (0, 31527, 91527, 104682),
                (0, 104682, 104682, 111290),
                (0, 111290, 111290, 117052),
                (0, 117052, 117052, 123052),
                (0, 123052, 123052, 123052),
                (1, 123052, 123052, 127703),
                (1, 127703, 127703, 132946),
                (1, 132946, 132946, 139637),
                (1, 139637, 139637, 151389),
                (1, 151389, 211389, 223372),
                (1, 223372, 223372, 230618),
                (1, 230618, 230618, 236730),
                (1, 236730, 236730, 241715),
                (1, 241715, 241715, 241715),
            ],
        ),
        (
            301,
            314929,
            &[
                (0, 0, 0, 14778),
                (0, 14778, 14778, 35834),
                (0, 35834, 95834, 115242),
                (0, 115242, 115242, 121642),
                (0, 121642, 121642, 128901),
                (0, 128901, 128901, 136385),
                (0, 136385, 136385, 146550),
                (0, 146550, 146550, 155409),
                (0, 155409, 155409, 155409),
                (0, 155409, 155409, 159548),
                (0, 159548, 159548, 169935),
                (0, 169935, 169935, 180176),
                (0, 180176, 180176, 186224),
                (0, 186224, 186224, 192927),
                (0, 192927, 192927, 200414),
                (0, 200414, 200414, 212813),
                (0, 212813, 272813, 284292),
                (0, 284292, 284292, 290112),
                (0, 290112, 290112, 293958),
                (0, 293958, 293958, 298961),
                (0, 298961, 298961, 305556),
                (0, 305556, 305556, 314929),
                (0, 314929, 314929, 314929),
            ],
        ),
        (
            303,
            316916,
            &[
                (1, 0, 30000, 50974),
                (1, 50974, 50974, 57811),
                (1, 57811, 57811, 70003),
                (1, 70003, 70003, 76528),
                (1, 76528, 76528, 90092),
                (1, 90092, 150092, 164673),
                (1, 164673, 164673, 175005),
                (1, 175005, 175005, 186938),
                (1, 186938, 186938, 195064),
                (1, 195064, 195064, 202624),
                (1, 202624, 202624, 212159),
                (1, 212159, 212159, 220112),
                (1, 220112, 220112, 225734),
                (1, 225734, 225734, 230981),
                (1, 230981, 230981, 237304),
                (1, 237304, 237304, 246717),
                (1, 246717, 246717, 255293),
                (1, 255293, 255293, 267940),
                (1, 267940, 267940, 286916),
                (1, 286916, 316916, 316916),
            ],
        ),
        (
            436,
            1204730,
            &[
                (0, 0, 60000, 179168),
                (0, 179168, 179168, 257565),
                (0, 257565, 257565, 379019),
                (0, 379019, 379019, 485952),
                (0, 485952, 485952, 606161),
                (0, 606161, 606161, 726485),
                (0, 726485, 726485, 846146),
                (0, 846146, 846146, 966681),
                (0, 966681, 966681, 1065800),
                (0, 1065800, 1065800, 1204730),
                (0, 1204730, 1204730, 1204730),
            ],
        ),
        (
            967,
            234433,
            &[
                (0, 0, 0, 3500),
                (0, 3500, 3500, 8105),
                (0, 8105, 8105, 14823),
                (0, 14823, 14823, 29114),
                (0, 29114, 89114, 100991),
                (0, 100991, 100991, 105164),
                (0, 105164, 105164, 109656),
                (0, 109656, 109656, 115681),
                (0, 115681, 115681, 115681),
                (1, 115681, 115681, 120367),
                (1, 120367, 120367, 125574),
                (1, 125574, 125574, 132068),
                (1, 132068, 132068, 144081),
                (1, 144081, 204081, 216150),
                (1, 216150, 216150, 223488),
                (1, 223488, 223488, 229517),
                (1, 229517, 229517, 234433),
                (1, 234433, 234433, 234433),
            ],
        ),
    ];
    for &(path, period, frames) in expected {
        let nodes = cat.path(path).expect("installed path");
        let tt = TransportTimetable::build(nodes, if path == 436 { 1.0 } else { 30.0 }, 1.0)
            .expect("timetable");
        assert_eq!(tt.period_ms, period, "AC pathTime for path {path}");
        assert_eq!(tt.frames.len(), frames.len(), "kept frames path {path}");
        for (i, (actual, &(map, arrive, depart, next))) in tt.frames.iter().zip(frames).enumerate()
        {
            let actual_arrive = if i == 0 {
                0
            } else {
                tt.frames[i - 1].next_arrive_time
            };
            assert_eq!(actual.map_id, map);
            // Allow one millisecond at intermediate float-to-integer boundaries across
            // the standalone C++ oracle and Rust; periods and logged seam remain exact.
            for (got, want) in [
                (actual_arrive, arrive),
                (actual.departure_time, depart),
                (actual.next_arrive_time, next),
            ] {
                assert!(
                    got.abs_diff(want) <= 1,
                    "AC frame {i} path {path}: {got} != {want}"
                );
            }
        }
    }
}

#[test]
fn stormwind_seam_and_teldrassil_docks_match_ac_boundaries() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_taxi_path_nodes(&mut chain).expect("taxi nodes");
    let stormwind = TransportTimetable::build(cat.path(967).unwrap(), 30.0, 1.0).unwrap();
    assert_eq!(stormwind.sample(115_680).map, 0);
    assert_eq!(stormwind.sample(115_681).map, 1);
    assert_eq!(stormwind.sample(234_432).map, 1);
    assert_eq!(stormwind.sample(234_433).map, 0);
    // Actual ObjectCreate at 05:57:58.910818Z in azb-live/155325/client.log.
    // Descriptor period=234433, progress=1451256056; the old 234510 modulo gives map 0.
    let at = stormwind.sample(1_451_256_056);
    assert_eq!(1_451_256_056 % stormwind.period_ms, 115_786);
    assert_eq!(at.map, 1);
    for (got, want) in at.pos.into_iter().zip([6078.8516, 746.40015, 0.0]) {
        assert!((got - want).abs() < 0.01, "packet position {got} != {want}");
    }
    assert!((at.heading - 2.7461863).abs() < 0.0001);
    // Same packet batch: Moonspray entry 176244 / path 293, same progress.
    let teldrassil = TransportTimetable::build(cat.path(293).unwrap(), 30.0, 1.0).unwrap();
    let at = teldrassil.sample(1_451_256_056);
    for (got, want) in at.pos.into_iter().zip([6594.592, 769.79724, 0.0]) {
        assert!(
            (got - want).abs() < 0.01,
            "Moonspray packet {got} != {want}"
        );
    }
    assert!((at.heading - 4.4532466).abs() < 0.0001);
    for (arrival, departure) in [(34_233, 94_233), (200_433, 260_433)] {
        assert!(teldrassil.sample(arrival - 1).moving);
        assert!(!teldrassil.sample(arrival).moving);
        assert!(!teldrassil.sample(departure - 1).moving);
        assert!(teldrassil.sample(departure).moving);
    }
}
