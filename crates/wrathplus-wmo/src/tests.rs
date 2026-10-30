use super::*;

fn chunk(magic: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(magic);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

fn parse(bytes: &[u8]) -> Result<ParsedWmo> {
    parse_wmo(&mut Cursor::new(bytes))
}

#[test]
fn root_parses_textures_materials_and_group_count() {
    let mut mohd = vec![0u8; 64];
    mohd[4..8].copy_from_slice(&7u32.to_le_bytes()); // nGroups
    let motx = b"a.blp\0\0b.blp\0";
    let mut momt = vec![0u8; 64];
    momt[8..12].copy_from_slice(&1u32.to_le_bytes()); // blend_mode
    momt[12..16].copy_from_slice(&7u32.to_le_bytes()); // texture_1 offset → "b.blp"
    let mut b = chunk(b"DHOM", &mohd);
    b.extend(chunk(b"XTOM", motx));
    b.extend(chunk(b"TMOM", &momt));
    let ParsedWmo::Root(root) = parse(&b).expect("parses") else {
        panic!("expected a root");
    };
    assert_eq!(root.n_groups, 7);
    assert_eq!(
        root.textures,
        vec!["a.blp".to_string(), "b.blp".to_string()]
    );
    assert_eq!(root.materials.len(), 1);
    assert_eq!(root.materials[0].blend_mode, 1);
    assert_eq!(
        root.materials[0].get_texture1_index(&root.texture_offset_index_map),
        1
    );
}

#[test]
fn group_parses_geometry() {
    let mut mogp = vec![0u8; 68];
    mogp[8..12].copy_from_slice(&0x48u32.to_le_bytes()); // flags
    let vert = [1.0f32, 2.0, 3.0].map(f32::to_le_bytes).concat();
    mogp.extend(chunk(b"TVOM", &vert));
    mogp.extend(chunk(b"IVOM", &[0u8, 0, 1, 0, 2, 0]));
    let b = chunk(b"PGOM", &mogp);
    let ParsedWmo::Group(g) = parse(&b).expect("parses") else {
        panic!("expected a group");
    };
    assert_eq!(g.flags, 0x48);
    assert_eq!(g.vertex_positions.len(), 1);
    assert_eq!(g.vertex_positions[0].y, 2.0);
    assert_eq!(g.vertex_indices, vec![0, 1, 2]);
}

#[test]
fn group_parses_mliq_liquid_grid() {
    // A 2×2-vertex (1×1-tile) MLIQ: header, 4 verts (8 B each, height at +4), 1 tile byte.
    let mut mliq = Vec::new();
    mliq.extend(2u32.to_le_bytes()); // xverts
    mliq.extend(2u32.to_le_bytes()); // yverts
    mliq.extend(1u32.to_le_bytes()); // xtiles
    mliq.extend(1u32.to_le_bytes()); // ytiles
    mliq.extend([10.0f32, 20.0, -5.0].map(f32::to_le_bytes).concat()); // base xyz
    mliq.extend(115u16.to_le_bytes()); // materialId
    for h in [-5.0f32, -5.0, -5.0, -5.0] {
        mliq.extend([0u8, 0, 0, 0]); // leading bytes, opacity first
        mliq.extend(h.to_le_bytes()); // height
    }
    mliq.push(0x44); // tile flag: type 4 (lake_a), fishable (0x40)

    let mut mogp = vec![0u8; 68];
    mogp[8..12].copy_from_slice(&0x2000u32.to_le_bytes()); // flags
    mogp[0x34..0x38].copy_from_slice(&0xfu32.to_le_bytes()); // groupLiquid = 0xf (per-tile)
    mogp.extend(chunk(b"QILM", &mliq));
    let b = chunk(b"PGOM", &mogp);
    let ParsedWmo::Group(g) = parse(&b).expect("parses") else {
        panic!("expected a group");
    };
    assert_eq!(g.group_liquid, 0xf);
    let lq = g.liquid.expect("has MLIQ liquid");
    assert_eq!((lq.xverts, lq.yverts, lq.xtiles, lq.ytiles), (2, 2, 1, 1));
    assert_eq!(lq.base, [10.0, 20.0, -5.0]);
    assert_eq!(lq.material_id, 115);
    assert_eq!(lq.heights, vec![-5.0, -5.0, -5.0, -5.0]);
    assert_eq!(lq.tile_flags, vec![0x44]);
    assert_eq!(lq.tile_flags[0] & 0xf, 4); // wet tile, type 4 (lake_a)
}

#[test]
fn truncated_mliq_yields_no_liquid_not_a_panic() {
    // A header declaring a grid larger than the payload: no liquid, and the group still parses.
    let mut mliq = Vec::new();
    mliq.extend(9u32.to_le_bytes()); // xverts
    mliq.extend(9u32.to_le_bytes()); // yverts
    mliq.extend(8u32.to_le_bytes()); // xtiles
    mliq.extend(8u32.to_le_bytes()); // ytiles
    mliq.extend([0.0f32, 0.0, 0.0].map(f32::to_le_bytes).concat());
    mliq.extend(0u16.to_le_bytes());
    // and no vertex or tile bytes follow.
    let mut mogp = vec![0u8; 68];
    mogp.extend(chunk(b"QILM", &mliq));
    let b = chunk(b"PGOM", &mogp);
    let ParsedWmo::Group(g) = parse(&b).expect("parses") else {
        panic!("expected a group");
    };
    assert!(g.liquid.is_none());
}

#[test]
fn truncated_mohd_is_an_error_not_a_panic() {
    // A 3-byte MOHD payload cannot hold nGroups at 4.
    let b = chunk(b"DHOM", &[0u8; 3]);
    assert!(matches!(parse(&b), Err(Error::Truncated("MOHD"))));
}

#[test]
fn hostile_shapes_do_not_panic() {
    assert!(matches!(parse(&[]), Err(Error::NotWmo)));
    assert!(matches!(parse(&[0u8; 7]), Err(Error::NotWmo)));
    // An over-declared chunk size clamps, leniently.
    let mut b = chunk(b"DHOM", &[0u8; 64]);
    b[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(parse(&b), Err(Error::Truncated("MOHD")) | Ok(_)));
    // A MOGP shorter than its 68-byte header is an empty group.
    let b = chunk(b"PGOM", &[0u8; 10]);
    let Ok(ParsedWmo::Group(g)) = parse(&b) else {
        panic!("short MOGP should parse to an empty group");
    };
    assert!(g.vertex_positions.is_empty());
}
