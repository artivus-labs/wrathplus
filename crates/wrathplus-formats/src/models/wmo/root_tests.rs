use super::super::test_bytes::{chunk, f3};
use super::*;

#[test]
fn wrath_root_flags_require_complete_context() {
    assert!(wmo_root_flags(&[])
        .unwrap_err()
        .to_string()
        .contains("missing MOHD"));
    assert!(wmo_root_flags(&chunk(b"DHOM", &[0; 0x3f]))
        .unwrap_err()
        .to_string()
        .contains("truncated MOHD.flags"));
    let short_root = parse_wmo_root(&chunk(b"DHOM", &[0; 8])).unwrap();
    assert!(short_root
        .flags()
        .unwrap_err()
        .to_string()
        .contains("truncated"));
    assert!(super::super::wmo_group_wrath_liquid(&[], &short_root)
        .unwrap_err()
        .to_string()
        .contains("MOHD.flags"));
    let mut mohd = vec![0; 0x40];
    for flags in [0u32, 4, 5, u32::MAX] {
        mohd[0x3c..0x40].copy_from_slice(&flags.to_le_bytes());
        assert_eq!(wmo_root_flags(&chunk(b"DHOM", &mohd)).unwrap(), flags);
        let root = parse_wmo_root(&chunk(b"DHOM", &mohd)).unwrap();
        assert_eq!(root.flags().unwrap(), flags);
    }
}

#[test]
fn parses_portal_vertices_infos_and_refs() {
    let mut mopv = Vec::new();
    for v in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
        mopv.extend(f3(v));
    }
    let mut mopt = Vec::new();
    mopt.extend_from_slice(&0u16.to_le_bytes());
    mopt.extend_from_slice(&3u16.to_le_bytes());
    mopt.extend(f3([0.0, 0.0, 1.0]));
    mopt.extend_from_slice(&(-2.5f32).to_le_bytes());
    let mut mopr = Vec::new();
    for (p, g, s) in [(0u16, 1u16, 1i16), (0, 7, -1)] {
        mopr.extend_from_slice(&p.to_le_bytes());
        mopr.extend_from_slice(&g.to_le_bytes());
        mopr.extend_from_slice(&s.to_le_bytes());
        mopr.extend_from_slice(&0u16.to_le_bytes()); // filler
    }
    let mut bytes = Vec::new();
    bytes.extend(chunk(b"VPOM", &mopv));
    bytes.extend(chunk(b"TPOM", &mopt));
    bytes.extend(chunk(b"RPOM", &mopr));

    let p = parse_wmo_portals(&bytes);
    assert_eq!(p.vertices.len(), 3);
    assert_eq!(p.vertices[2], [0.0, 0.0, 1.0]);
    assert_eq!(p.infos.len(), 1);
    assert_eq!(p.infos[0].start_vertex, 0);
    assert_eq!(p.infos[0].count, 3);
    assert_eq!(p.infos[0].plane, [0.0, 0.0, 1.0, -2.5]);
    assert_eq!(p.refs.len(), 2);
    assert_eq!(
        (p.refs[0].portal, p.refs[0].group, p.refs[0].side),
        (0, 1, 1)
    );
    assert_eq!(
        (p.refs[1].portal, p.refs[1].group, p.refs[1].side),
        (0, 7, -1)
    );
}

#[test]
fn absent_portal_chunks_yield_empty_graph() {
    let p = parse_wmo_portals(&[]);
    assert!(p.vertices.is_empty() && p.infos.is_empty() && p.refs.is_empty());
}

#[test]
fn parses_mfog_records() {
    // A null fog (huge end) and a room fog.
    let mut mfog = Vec::new();
    for (pos, r_in, r_out, end, start_scalar, color) in [
        (
            [0.0f32, 0.0, 0.0],
            0.0f32,
            0.0f32,
            444.4445f32,
            0.25f32,
            0xff000000u32,
        ),
        ([10.0, -4.0, 2.5], 5.0, 20.0, 80.0, 0.1, 0xff102030),
    ] {
        mfog.extend_from_slice(&0u32.to_le_bytes()); // flags
        mfog.extend(f3(pos));
        for v in [r_in, r_out, end, start_scalar] {
            mfog.extend_from_slice(&v.to_le_bytes());
        }
        mfog.extend_from_slice(&color.to_le_bytes());
        // Underwater triple: end, start scalar, colour.
        for v in [222.2f32, 0.5] {
            mfog.extend_from_slice(&v.to_le_bytes());
        }
        mfog.extend_from_slice(&0xff445566u32.to_le_bytes());
    }
    let bytes = chunk(b"GOFM", &mfog);

    let fogs = parse_wmo_fogs(&bytes);
    assert_eq!(fogs.len(), 2);
    assert_eq!(fogs[0].fog_end, 444.4445);
    assert_eq!(fogs[0].color, 0xff000000);
    assert_eq!(fogs[1].pos, [10.0, -4.0, 2.5]);
    assert_eq!(fogs[1].radius_inner, 5.0);
    assert_eq!(fogs[1].radius_outer, 20.0);
    assert_eq!(fogs[1].fog_start_scalar, 0.1);
    assert_eq!(fogs[1].uw_fog_end, 222.2);
    assert_eq!(fogs[1].uw_color, 0xff445566);
    assert!(parse_wmo_fogs(&[]).is_empty());
}
