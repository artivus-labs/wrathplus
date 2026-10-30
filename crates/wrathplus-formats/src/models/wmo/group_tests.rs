use super::super::test_bytes::chunk;
use super::*;

fn wrath_group(raw: u32, flags: u32, mliq: Option<&[u8]>) -> Vec<u8> {
    let mut mogp = vec![0; 0x44];
    mogp[8..12].copy_from_slice(&flags.to_le_bytes());
    mogp[0x34..0x38].copy_from_slice(&raw.to_le_bytes());
    if let Some(liquid) = mliq {
        mogp.extend(chunk(b"QILM", liquid));
    }
    chunk(b"PGOM", &mogp)
}

fn wrath_mliq(vertices: [u32; 2], tiles: [u32; 2], flags: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for n in vertices.into_iter().chain(tiles) {
        bytes.extend(n.to_le_bytes());
    }
    bytes.resize(30 + vertices[0] as usize * vertices[1] as usize * 8, 0);
    bytes.extend(flags);
    bytes
}

#[test]
fn wrath_resolver_audit_classes_and_preprocessing() {
    for group_flags in [0, 0x80000, 0x81008] {
        for n in 0u32..=21 {
            let expected = if n == 0 || n == 21 {
                n
            } else {
                match (n - 1) & 3 {
                    0 if group_flags & 0x80000 != 0 => 14,
                    0 => 13,
                    1 => 14,
                    2 => 19,
                    _ => 20,
                }
            };
            for root_flags in [4, 5, 0xffff_ffff] {
                assert_eq!(
                    resolve_wrath_wmo_liquid_id(root_flags, group_flags, n),
                    expected
                );
            }
            // Legacy mode applies +1 first, except raw 15.
            let legacy = if n == 15 {
                0
            } else {
                resolve_wrath_wmo_liquid_id(4, group_flags, n + 1)
            };
            assert_eq!(resolve_wrath_wmo_liquid_id(0, group_flags, n), legacy);
            assert_eq!(resolve_wrath_wmo_liquid_id(1, group_flags, n), legacy);
        }
    }
    assert_eq!(resolve_wrath_wmo_liquid_id(0, 0, 15), 0);
    assert_eq!(resolve_wrath_wmo_liquid_id(4, 0, 15), 19);
    for n in [22, 0x1234, u32::MAX] {
        assert_eq!(resolve_wrath_wmo_liquid_id(4, 0, n), n);
    }
    assert_eq!(resolve_wrath_wmo_liquid_id(0, 0, u32::MAX), 0);
    assert_eq!(resolve_wrath_wmo_liquid_id(0, 0, u32::MAX - 1), u32::MAX);
}

#[test]
fn wrath_absence_flag_gates_and_native_flood() {
    let abbey = decode_wrath_wmo_liquid(&wrath_group(0, 8, None), 5).unwrap();
    assert_eq!(abbey.resolved_id, 0);
    assert_eq!(abbey.mode(), WrathWmoLiquidMode::Dry);
    assert_eq!(abbey.grid, WrathWmoLiquidGrid::Absent);
    let legacy = decode_wrath_wmo_liquid(&wrath_group(0, 8, None), 0).unwrap();
    assert_eq!(legacy.resolved_id, 13);
    assert!(legacy.is_whole_group_flood());
    // Stray/unflagged invalid MLIQ is ignored, not decoded or used as fallback.
    let stray = decode_wrath_wmo_liquid(&wrath_group(0, 0, Some(&[0])), 4).unwrap();
    assert_eq!(
        stray,
        decode_wrath_wmo_liquid(&wrath_group(0, 0, None), 4).unwrap()
    );
    let high = decode_wrath_wmo_liquid(&wrath_group(1234, 0, None), 4).unwrap();
    assert_eq!(high.resolved_id, 1234);
    assert!(high.is_whole_group_flood());
}

#[test]
fn wrath_tile_fallback_is_bounded_and_skips_holes() {
    let mliq = wrath_mliq([3, 2], [2, 1], &[0xff, 0xa2, 0]);
    let group = wrath_group(0, 0x1000, Some(&mliq));
    let state = decode_wrath_wmo_liquid(&group, 4).unwrap();
    assert_eq!(state.resolved_id, 19); // first nonhole low nibble 2 -> n3
    assert_eq!(state.tile_dimensions, [2, 1]);
    assert_eq!(state.mode(), WrathWmoLiquidMode::Grid);
    assert_eq!(state.grid, WrathWmoLiquidGrid::Tiles { all_holes: false });
    // Existing nonzero header wins over wet tile fallback.
    let state = decode_wrath_wmo_liquid(&wrath_group(21, 0x1000, Some(&mliq)), 4).unwrap();
    assert_eq!(state.resolved_id, 21);
    let water = wrath_mliq([2, 2], [1, 1], &[0x80]);
    let state = decode_wrath_wmo_liquid(&wrath_group(0, 0x81000, Some(&water)), 4).unwrap();
    assert_eq!(state.resolved_id, 14);
    // A wet byte beyond declared tile count cannot supply fallback.
    let holes = wrath_mliq([2, 2], [1, 1], &[0xff, 0]);
    for raw in [0, 21] {
        let state = decode_wrath_wmo_liquid(&wrath_group(raw, 0x1000, Some(&holes)), 4).unwrap();
        assert_eq!(state.resolved_id, raw);
        assert_eq!(state.grid, WrathWmoLiquidGrid::Tiles { all_holes: true });
        assert!(!state.is_whole_group_flood());
        assert_eq!(
            state.mode(),
            if raw == 0 {
                WrathWmoLiquidMode::Dry
            } else {
                WrathWmoLiquidMode::Grid
            }
        );
    }
}

#[test]
fn wrath_zero_dimensions_are_valid_but_invalid_payload_is_not_absence() {
    for dims in [[0, 0], [0, 7], [8, 0]] {
        let mliq = wrath_mliq([0, 0], dims, &[0]); // trailing wet byte is outside empty grid
        for raw in [0, 21] {
            let state = decode_wrath_wmo_liquid(&wrath_group(raw, 0x1000, Some(&mliq)), 4).unwrap();
            assert_eq!(state.resolved_id, raw);
            assert_eq!(state.tile_dimensions, dims);
            assert_eq!(state.grid, WrathWmoLiquidGrid::ZeroSized);
            assert_eq!(state.is_whole_group_flood(), raw != 0);
        }
    }
    for raw in [0, 21] {
        assert!(decode_wrath_wmo_liquid(&wrath_group(raw, 0x1000, None), 4).is_err());
        assert!(decode_wrath_wmo_liquid(&wrath_group(raw, 0x1000, Some(&[0; 29])), 4).is_err());
        let missing_tile = wrath_mliq([2, 2], [1, 1], &[]);
        assert!(
            decode_wrath_wmo_liquid(&wrath_group(raw, 0x1000, Some(&missing_tile)), 4).is_err()
        );
        let mut missing_vertices = wrath_mliq([0, 0], [0, 0], &[]);
        missing_vertices[0..4].copy_from_slice(&2u32.to_le_bytes());
        missing_vertices[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(
            decode_wrath_wmo_liquid(&wrath_group(raw, 0x1000, Some(&missing_vertices)), 4).is_err()
        );
    }
    let mut overflow = vec![0xff; 30];
    overflow[8..16].fill(0);
    assert!(decode_wrath_wmo_liquid(&wrath_group(21, 0x1000, Some(&overflow)), 4).is_err());
    let mut huge_tiles = vec![0; 30];
    huge_tiles[8..16].fill(0xff);
    assert!(decode_wrath_wmo_liquid(&wrath_group(21, 0x1000, Some(&huge_tiles)), 4).is_err());
    let mut mogp = vec![0; 0x44];
    mogp[8..12].copy_from_slice(&0x1000u32.to_le_bytes());
    mogp.extend(b"QILM");
    mogp.extend(100u32.to_le_bytes()); // declared payload longer than available bytes
    mogp.extend([0; 30]);
    assert!(decode_wrath_wmo_liquid(&chunk(b"PGOM", &mogp), 4).is_err());
    assert!(decode_wrath_wmo_liquid(&[], 4).is_err());
    assert!(decode_wrath_wmo_liquid(&chunk(b"PGOM", &[0; 0x37]), 4).is_err());
}

#[test]
fn builds_wmo_water_mesh_skipping_holes() {
    // A 3×2-vertex grid (2×1 tiles): left tile wet (lake_a, nibble 4), right tile a hole (0xf).
    let lq = WmoLiquid {
        xverts: 3,
        yverts: 2,
        xtiles: 2,
        ytiles: 1,
        base: [100.0, 200.0, -3.0],
        material_id: 0,
        heights: vec![-3.0; 6],
        // Byte 0 per vertex, the authored opacity index.
        opacity: vec![0, 51, 102, 153, 204, 255],
        tile_flags: vec![0x44, 0x0f], // wet (type 4 + fishable), hole
    };
    let m = build_wmo_liquid_mesh(&lq, 0xf).expect("a wet mesh");
    assert_eq!(m.kind, LiquidKind::Still);
    assert_eq!(m.positions.len(), 6); // full 3×2 grid emitted
    assert_eq!(m.indices.len(), 6); // one wet tile → 2 tris → 6 indices
    assert!((m.positions[1][0] - (100.0 + MLIQ_CELL_STEP)).abs() < 1e-3);
    assert_eq!(m.positions[0], [100.0, 200.0, -3.0]);
    // Opacity is the vertex's own authored byte, normalised.
    for (n, &v) in m.depths.iter().enumerate() {
        assert!(
            (v - f32::from(lq.opacity[n]) / 255.0).abs() < 1e-6,
            "vertex {n}: depth channel {v} should carry byte {}/255",
            lq.opacity[n]
        );
    }
    // The wet tile is the left one, corners 0, 1, 3 and 4.
    assert!(m.indices.iter().all(|&i| [0u32, 1, 3, 4].contains(&i)));
}

/// Surface B starts at A's last column (2·STEP east), so their shared vertex must get one UV.
#[test]
fn adjacent_surfaces_share_a_continuous_uv_field() {
    let base = [100.0, 200.0, -3.0];
    let a = WmoLiquid {
        xverts: 3,
        yverts: 2,
        xtiles: 2,
        ytiles: 1,
        base,
        material_id: 0,
        heights: vec![-3.0; 6],
        opacity: Vec::new(),
        tile_flags: vec![0x44, 0x44], // both tiles wet
    };
    let b = WmoLiquid {
        xverts: 3,
        yverts: 2,
        xtiles: 2,
        ytiles: 1,
        base: [base[0] + 2.0 * MLIQ_CELL_STEP, base[1], base[2]],
        material_id: 0,
        heights: vec![-3.0; 6],
        opacity: Vec::new(),
        tile_flags: vec![0x44, 0x44],
    };
    let ma = build_wmo_liquid_mesh(&a, 0xf).unwrap();
    let mb = build_wmo_liquid_mesh(&b, 0xf).unwrap();
    // A's vertex (2, 0) and B's (0, 0) are the same point.
    let (ua, ub) = (ma.uvs[2], mb.uvs[0]);
    assert!(
        (ua[0] - ub[0]).abs() < 1e-4 && (ua[1] - ub[1]).abs() < 1e-4,
        "seam: shared-vertex UVs differ A={ua:?} B={ub:?}"
    );
    // One repeat per cell, the reference's `u = (float)i` (`0x6b6630`).
    assert!((ma.uvs[1][0] - ma.uvs[0][0] - 1.0).abs() < 1e-4);
}

#[test]
fn wmo_liquid_type_override_and_all_holes() {
    // groupLiquid override = 2 (magma) forces the kind regardless of the wet tile's nibble.
    let lq = WmoLiquid {
        xverts: 2,
        yverts: 2,
        xtiles: 1,
        ytiles: 1,
        base: [0.0, 0.0, 0.0],
        material_id: 0,
        heights: vec![0.0; 4],
        opacity: Vec::new(),
        tile_flags: vec![0x40], // nibble 0 (would be Still), but the override wins
    };
    let m = build_wmo_liquid_mesh(&lq, 2).expect("magma mesh");
    assert_eq!(m.kind, LiquidKind::Magma);
    assert!(m.depths.iter().all(|&v| v == 0.0)); // fullbright ⇒ depth V unused

    let all_holes = WmoLiquid {
        tile_flags: vec![0x0f],
        ..lq
    };
    assert!(build_wmo_liquid_mesh(&all_holes, 0xf).is_none());
}

#[test]
fn reads_group_header_flags_and_portal_ref_span() {
    let mut hdr = vec![0u8; 68];
    hdr[8..12].copy_from_slice(&0x8u32.to_le_bytes()); // EXTERIOR
    hdr[0x24..0x26].copy_from_slice(&5u16.to_le_bytes());
    hdr[0x26..0x28].copy_from_slice(&3u16.to_le_bytes());
    hdr[0x30..0x34].copy_from_slice(&[2, 0, 0, 0]); // fog indices
    let mut group = Vec::new();
    group.extend(chunk(b"REVM", &17u32.to_le_bytes()));
    group.extend(chunk(b"PGOM", &hdr));

    let h = wmo_group_header(&group).expect("group header");
    assert_eq!(h.flags, 0x8);
    assert_eq!(h.portal_ref_start, 5);
    assert_eq!(h.portal_ref_count, 3);
    assert_eq!(h.fog_indices, [2, 0, 0, 0]);
    assert_eq!(
        h.group_liquid, 0,
        "0x34 is read raw: a zeroed header says liquid type 0, NOT the 0xf sentinel — the \
         reader must never conflate 'no liquid' with 'this whole group is water'"
    );
}

/// All 13 shipped overrides are `0`, water, so the no-liquid sentinel cannot be zero.
#[test]
fn reads_the_whole_group_liquid_override() {
    let group_with = |liquid: u32| {
        let mut hdr = vec![0u8; 68];
        hdr[0x34..0x38].copy_from_slice(&liquid.to_le_bytes());
        let mut group = Vec::new();
        group.extend(chunk(b"REVM", &17u32.to_le_bytes()));
        group.extend(chunk(b"PGOM", &hdr));
        group
    };

    // The shipped case: the whole group is water, with no MLIQ.
    let flooded = wmo_group_header(&group_with(0)).expect("group header");
    assert_eq!(flooded.group_liquid, 0);
    assert_ne!(flooded.group_liquid, NO_GROUP_LIQUID);

    // The other 5207 groups.
    assert_eq!(
        wmo_group_header(&group_with(NO_GROUP_LIQUID))
            .expect("group header")
            .group_liquid,
        NO_GROUP_LIQUID
    );

    // A header short of 0x34 reads as no liquid.
    let mut short = Vec::new();
    short.extend(chunk(b"REVM", &17u32.to_le_bytes()));
    short.extend(chunk(b"PGOM", &[0u8; 0x30]));
    assert_eq!(
        wmo_group_header(&short)
            .expect("a short header still parses")
            .group_liquid,
        NO_GROUP_LIQUID
    );
}
