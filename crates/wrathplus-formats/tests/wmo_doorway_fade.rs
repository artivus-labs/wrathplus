//! FixColorVertexAlpha, the portal fade: MOCV vertices at a portal whose far side is an exterior
//! group turn `(255, 255, 255, 255)`, interior-to-interior portals whiten nothing, and there is no
//! MOPY alpha pre-pass. A 1.12 capture of the reference's uploaded MOCV for the Northshire abbey
//! matched the file in every vertex of group 1 and all but the fade's 10 slots of group 3; the
//! 12340 abbey is re-authored, so the counts are not carried over. These fixtures explicitly
//! select the vanilla loader: Wrath preprocessing is tested in `wmo_unified`.

use wrathplus_formats::{wmo_group_fixed_colors, Chain, WmoRoot};

/// (slots the fade rewrites, slot count, mean luminance before, after) for one group.
fn fade_census(reader: &Chain, stem: &str, gi: u32) -> (usize, usize, f32, f32) {
    let root_bytes = reader.read(&format!("{stem}.wmo")).expect("read root");
    let root = WmoRoot::parse_vanilla(&root_bytes).expect("parse root");
    let gbytes = reader
        .read(&format!("{stem}_{gi:03}.wmo"))
        .expect("read group");
    let raw = wrathplus_formats::wmo_group_raw_colors(&gbytes).expect("group carries MOCV");
    let fixed = wmo_group_fixed_colors(&gbytes, &root).expect("group carries MOCV");
    let lum =
        |c: [u8; 4]| 0.299 * f32::from(c[2]) + 0.587 * f32::from(c[1]) + 0.114 * f32::from(c[0]);
    let n = raw.len().max(1) as f32;
    let changed = raw.iter().zip(&fixed).filter(|(a, b)| a != b).count();
    (
        changed,
        raw.len(),
        raw.iter().map(|&c| lum(c)).sum::<f32>() / n,
        fixed.iter().map(|&c| lum(c)).sum::<f32>() / n,
    )
}

#[test]
fn the_abbey_fade_touches_only_exterior_portals() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let reader = Chain::open(&data).expect("open patch chain");
    let stem = "World\\wmo\\Azeroth\\Buildings\\NSabbey\\NSabbey";

    let (changed, _, _, _) = fade_census(&reader, stem, 1);
    assert_eq!(
        changed, 0,
        "group 1's portals all lead to interior neighbours, so nothing whitens"
    );

    let (changed, total, before, after) = fade_census(&reader, stem, 3);
    assert!(
        changed * 20 < total,
        "the fade's footprint is a few portal slots, not a pre-pass ({changed}/{total})"
    );
    // The faded slots are near-white in the file already.
    assert!(
        (before - after).abs() < 0.5,
        "abbey g003 mean luminance moved {before} → {after}"
    );
}

/// Dire Maul's five entrance corridors author their walkway at MOCV `(10, 10, 40)`, alpha 0, which
/// the interior TRANS lighting renders near-black; its corners sit in the doorway portals, so the
/// fade takes them white.
#[test]
fn dire_maul_entrance_corridor_floors_are_lit() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let reader = Chain::open(&data).expect("open patch chain");
    let stem = "World\\wmo\\Dungeon\\KL_Diremaul\\KL_Diremaul";

    for gi in [12, 14, 17, 27, 31] {
        let root_bytes = reader.read(&format!("{stem}.wmo")).expect("read root");
        let root = WmoRoot::parse_vanilla(&root_bytes).expect("parse root");
        let gbytes = reader
            .read(&format!("{stem}_{gi:03}.wmo"))
            .expect("read group");
        let raw = wrathplus_formats::wmo_group_raw_colors(&gbytes).expect("MOCV");
        let fixed = wmo_group_fixed_colors(&gbytes, &root).expect("MOCV");

        let dark = raw.iter().filter(|c| **c == [40, 10, 10, 0]).count();
        assert!(
            dark >= 4,
            "g{gi:03} should author its walkway quad at BGRA (40,10,10,0); found {dark} such slots"
        );
        // A corner just off the portal plane takes the partial lerp at t ≈ 0.997 and lands on 254.
        for (i, (r, f)) in raw.iter().zip(&fixed).enumerate() {
            if *r == [40, 10, 10, 0] {
                assert!(
                    f.iter().all(|&c| c >= 250),
                    "g{gi:03} v{i}: the corridor floor must whiten, or it renders black — got {f:?}"
                );
            }
        }
    }
}
