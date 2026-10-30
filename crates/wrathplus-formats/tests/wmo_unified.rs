//! Unified root lighting must not turn authored black residual light into a black multiplier.
use wrathplus_formats::{
    parse_wmo_root, wmo_group_fixed_colors, wmo_group_header, wmo_group_raw_colors,
    wmo_group_submeshes, Chain, WmoBatchClass,
};

fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    [tag.as_slice(), &(data.len() as u32).to_le_bytes(), data].concat()
}

fn root(flags: u32) -> Vec<u8> {
    let mut header = [0; 64];
    header[4..8].copy_from_slice(&1u32.to_le_bytes());
    header[0x1c..0x20].copy_from_slice(&[11, 22, 33, 255]);
    header[0x3c..].copy_from_slice(&flags.to_le_bytes());
    [
        chunk(b"DHOM", &header),
        chunk(b"XTOM", b"roof.blp\0"),
        chunk(b"TMOM", &[0; 64]),
    ]
    .concat()
}

fn group(alpha: u8, mocv: bool) -> Vec<u8> {
    let mut header = [0; 68];
    header[8..12].copy_from_slice(&0x80du32.to_le_bytes()); // exterior, with MOCV
    header[0x2a..0x2c].copy_from_slice(&1u16.to_le_bytes()); // INT on exterior
    let positions: Vec<u8> = [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let normals: Vec<u8> = [0f32, 0., 1.]
        .repeat(3)
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let mut batch = [0; 24];
    batch[16..18].copy_from_slice(&3u16.to_le_bytes());
    batch[20..22].copy_from_slice(&2u16.to_le_bytes());
    let mut data = [
        header.to_vec(),
        chunk(b"TVOM", &positions),
        chunk(b"RNOM", &normals),
        chunk(b"IVOM", &[0, 0, 1, 0, 2, 0]),
        chunk(b"ABOM", &batch),
    ]
    .concat();
    if mocv {
        data.extend(chunk(b"VCOM", &[0, 0, 0, alpha].repeat(3)));
    }
    chunk(b"PGOM", &data)
}

#[test]
fn unified_loader_preserves_black_residual_alpha_and_root_ambient() {
    for flags in 0..16 {
        let root = parse_wmo_root(&root(flags)).unwrap();
        assert_eq!(root.ambient(), [33, 22, 11]);
        for alpha in [0, 64, 255] {
            let bytes = group(alpha, true);
            let sub = wmo_group_submeshes(&bytes, &root).unwrap().remove(0);
            assert_eq!(sub.wmo_batch, Some(WmoBatchClass::Int));
            assert!(!sub.interior);
            assert_eq!(sub.wmo_unified, (flags & 2 != 0).then_some([33, 22, 11]));
            for color in &sub.vertex_colors {
                assert_eq!(&color[..3], &[0.; 3]);
                assert_eq!(
                    color[3],
                    if flags & 10 == 10 {
                        f32::from(alpha) / 255.
                    } else {
                        1.
                    }
                );
            }
            if flags & 8 != 0 {
                assert_eq!(
                    wmo_group_fixed_colors(&bytes, &root),
                    wmo_group_raw_colors(&bytes)
                );
            }
        }
        let no_colors = wmo_group_submeshes(&group(255, false), &root)
            .unwrap()
            .remove(0);
        assert!(no_colors.vertex_colors.is_empty());
        assert_eq!(
            no_colors.wmo_unified,
            (flags & 2 != 0).then_some([33, 22, 11])
        );
    }
}

#[test]
fn installed_stormwind_unified_roofs_and_colorless_controls() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let chain = Chain::open(&data).expect("patch chain");
    let stem = "World\\wmo\\Azeroth\\Buildings\\Stormwind\\Stormwind";
    let root = parse_wmo_root(&chain.read(&format!("{stem}.wmo")).unwrap()).unwrap();
    assert_eq!(root.flags().unwrap(), 15);
    assert_eq!(root.ambient(), [33; 3]);
    for (gi, black) in [(283, 25634), (285, 21092)] {
        let bytes = chain.read(&format!("{stem}_{gi:03}.wmo")).unwrap();
        assert_ne!(wmo_group_header(&bytes).unwrap().flags & 8, 0);
        let raw = wmo_group_raw_colors(&bytes).unwrap();
        assert_eq!(raw.iter().filter(|c| c[..3] == [0; 3]).count(), black);
        assert_eq!(wmo_group_fixed_colors(&bytes, &root).unwrap(), raw);
        let subs = wmo_group_submeshes(&bytes, &root).unwrap();
        assert!(subs.iter().all(|s| s.wmo_unified == Some([33; 3])));
        let roof = subs
            .iter()
            .filter(|s| {
                s.texture
                    .as_deref()
                    .is_some_and(|t| t.to_ascii_lowercase().contains("roof"))
            })
            .collect::<Vec<_>>();
        assert!(!roof.is_empty());
        assert!(roof
            .iter()
            .any(|s| s.vertex_colors.iter().all(|c| c[..3] == [0.; 3])));
        assert!(roof.iter().all(|s| !s.interior && !s.emissive));
    }
    for gi in [281, 282, 284, 265] {
        let bytes = chain.read(&format!("{stem}_{gi:03}.wmo")).unwrap();
        assert!(wmo_group_raw_colors(&bytes).is_none());
        let subs = wmo_group_submeshes(&bytes, &root).unwrap();
        assert!(!subs.is_empty());
        assert!(subs
            .iter()
            .all(|s| s.vertex_colors.is_empty() && s.wmo_unified == Some([33; 3]) && !s.interior));
    }
    let bytes = chain.read(&format!("{stem}_069.wmo")).unwrap();
    assert_eq!(wmo_group_header(&bytes).unwrap().flags & 0x48, 0);
    assert_eq!(
        wmo_group_fixed_colors(&bytes, &root),
        wmo_group_raw_colors(&bytes)
    );
    assert!(wmo_group_submeshes(&bytes, &root)
        .unwrap()
        .iter()
        .all(|s| s.interior
            && (s.wmo_unified == Some([33; 3]) || s.wmo_batch == Some(WmoBatchClass::Trans))));
}

#[test]
fn root_alpha_suppression_preserves_authored_portal_vertices() {
    let mut portal_root = root(8);
    let vertices: Vec<u8> = [[-1f32, -1., 0.], [2., -1., 0.], [2., 2., 0.], [-1., 2., 0.]]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let mut info = [0; 20];
    info[2..4].copy_from_slice(&4u16.to_le_bytes());
    info[12..16].copy_from_slice(&1f32.to_le_bytes());
    let mut neighbor = [0; 32];
    neighbor[..4].copy_from_slice(&8u32.to_le_bytes());
    portal_root.extend(chunk(b"IGOM", &neighbor));
    portal_root.extend(chunk(b"VPOM", &vertices));
    portal_root.extend(chunk(b"TPOM", &info));
    portal_root.extend(chunk(b"RPOM", &[0, 0, 0, 0, 1, 0, 0, 0]));
    let mut bytes = group(0, true);
    // MOGP payload starts after the 8-byte chunk header.
    bytes[8 + 0x26..8 + 0x28].copy_from_slice(&1u16.to_le_bytes());
    // TRANS vertices at the exterior portal exercise both real dialects' distinct fades.
    let mut trans = bytes.clone();
    trans[8 + 0x28..8 + 0x2a].copy_from_slice(&1u16.to_le_bytes());
    for flags in 0u32..16 {
        portal_root[8 + 0x3c..8 + 0x40].copy_from_slice(&flags.to_le_bytes());
        let wrath = parse_wmo_root(&portal_root).unwrap();
        let vanilla = wrathplus_formats::WmoRoot::parse_vanilla(&portal_root).unwrap();
        assert_eq!(
            wmo_group_fixed_colors(&trans, &wrath).unwrap(),
            vec![
                if flags & 1 == 0 {
                    [127, 127, 127, 255]
                } else {
                    [0; 4]
                };
                3
            ],
        );
        assert_eq!(
            wmo_group_fixed_colors(&trans, &vanilla).unwrap(),
            vec![[255; 4]; 3]
        );
    }
    for unified in [0, 2] {
        for attenuate in [0, 1] {
            let flags = 8u32 | unified | attenuate;
            portal_root[8 + 0x3c..8 + 0x40].copy_from_slice(&flags.to_le_bytes());
            let root = parse_wmo_root(&portal_root).unwrap();
            assert_eq!(
                wmo_group_fixed_colors(&bytes, &root),
                wmo_group_raw_colors(&bytes)
            );
        }
    }
}

#[test]
fn unified_rgb_fixup_and_portal_gates_are_independent() {
    for flags in 0u32..16 {
        let root = parse_wmo_root(&root(flags)).unwrap();
        let mut bytes = group(128, true);
        let i = bytes.windows(4).position(|s| s == b"VCOM").unwrap() + 8;
        bytes[i..i + 12].copy_from_slice(&[200, 100, 40, 128].repeat(3));
        let fixed = wmo_group_fixed_colors(&bytes, &root).unwrap();
        let expected = if flags & 8 != 0 {
            [200, 100, 40, 128]
        } else {
            [255, 150, 60, 255]
        };
        assert_eq!(fixed, vec![expected; 3]);
        // Last TRANS vertex=2: integer half RGB, preserve alpha, or skip both on root0x8.
        bytes[8 + 0x28..8 + 0x2a].copy_from_slice(&1u16.to_le_bytes());
        let fixed = wmo_group_fixed_colors(&bytes, &root).unwrap();
        let expected = if flags & 1 == 0 {
            if flags & 8 == 0 {
                [100, 50, 20, 0]
            } else {
                [200, 100, 40, 0]
            }
        } else if flags & 8 == 0 {
            [100, 50, 20, 128]
        } else {
            [200, 100, 40, 128]
        };
        assert_eq!(fixed, vec![expected; 3]);
        assert!(
            wmo_group_submeshes(&bytes, &root)
                .unwrap()
                .iter()
                .all(|s| s.wmo_unified.is_none()),
            "TRANS is explicitly excluded from the ordinary single-pass law"
        );
    }
}

#[test]
fn unified_window_and_sidn_read_native_context() {
    let mut rb = root(15);
    let material = rb.windows(4).position(|s| s == b"TMOM").unwrap() + 8;
    rb[material..material + 4].copy_from_slice(&0x10u32.to_le_bytes());
    rb[material + 0x10..material + 0x14].copy_from_slice(&[1, 2, 3, 255]);
    rb[material + 0x14..material + 0x18].copy_from_slice(&[11, 22, 33, 255]);
    let parsed = parse_wmo_root(&rb).unwrap();
    let subs = wmo_group_submeshes(&group(255, true), &parsed).unwrap();
    assert_eq!(subs[0].sidn, Some([33, 22, 11]));
    rb[material..material + 4].copy_from_slice(&0x30u32.to_le_bytes());
    let parsed = parse_wmo_root(&rb).unwrap();
    let subs = wmo_group_submeshes(&group(255, true), &parsed).unwrap();
    assert!(subs[0].window);
    assert_eq!(subs[0].wmo_unified, Some([33, 22, 11]));
    assert_eq!(subs[0].sidn, Some([33, 22, 11]));
}

#[test]
fn vanilla_loader_dialect_is_not_inferred_from_drawer_flags() {
    for flags in 0..16 {
        let root = wrathplus_formats::WmoRoot::parse_vanilla(&root(flags)).unwrap();
        let mut bytes = group(128, true);
        let i = bytes.windows(4).position(|s| s == b"VCOM").unwrap() + 8;
        bytes[i..i + 12].copy_from_slice(&[200, 100, 40, 128].repeat(3));
        assert_eq!(
            wmo_group_fixed_colors(&bytes, &root),
            wmo_group_raw_colors(&bytes)
        );
        assert!(root.unified_ambient().is_none());
        // Even TRANS vertices keep authored RGB/alpha with no vanilla exterior portal.
        bytes[8 + 0x28..8 + 0x2a].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            wmo_group_fixed_colors(&bytes, &root),
            wmo_group_raw_colors(&bytes)
        );
    }
}
