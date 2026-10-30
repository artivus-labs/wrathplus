use super::*;

fn item(index_range: Range<u32>, cutout: bool) -> GxItemDraw {
    GxItemDraw {
        index_range,
        texture: None,
        cutout,
        two_sided: false,
        vertex_range: 0..0,
        group: None,
        order: 0,
        sidn: [0; 3],
        wmo_unified: None,
        slot: 0,
    }
}

#[test]
fn unified_record_rgb_preserves_sidn_probe_kill_and_fog() {
    let mut draw = item(0..3, false);
    draw.sidn = [0x12, 0x34, 0x56];
    draw.order = 73;
    draw.slot = 8191;
    for ambient in [None, Some([0; 3]), Some([0x87, 0xa9, 0xcb]), Some([255; 3])] {
        draw.wmo_unified = ambient;
        let rec = item_record(&draw, 19, 1);
        let rgb = ambient.unwrap_or([0; 3]);
        assert_eq!(rec[0], 19);
        assert_eq!(rec[1], 73);
        assert_eq!(rec[2] & 0x00ff_ffff, 0x0056_3412);
        assert_eq!(rec[2] >> 24, u32::from(rgb[0]));
        assert_eq!((rec[3] >> 16) & 255, u32::from(rgb[1]));
        assert_eq!(rec[3] >> 24, u32::from(rgb[2]));
        assert_eq!(rec[3] & 0xffff, 0x3fff);
        // Production revision sync only changes these two bits, preserving upper RGB.
        let mut flags = rec[3] | RECORD_FOG_BIT;
        flags = (flags & !1) | kill_bit(&[], 0);
        assert_eq!(flags & 0xffff, 0x7ffe);
        assert_eq!(flags >> 16, rec[3] >> 16);
        flags &= !RECORD_FOG_BIT;
        assert_eq!(flags & 0xffff, 0x3ffe);
        assert_eq!(flags >> 16, rec[3] >> 16);
    }
}

#[test]
fn runs_fuse_live_items_and_split_at_kills() {
    let draws = vec![
        item(0..3, false),
        item(3..6, false),
        item(6..9, false),
        item(9..12, true), // bucket change
        item(12..15, true),
    ];
    let slots = vec![0, 0, 0, 0, 1]; // the last item binds another pool class
    let runs = build_runs(&draws, &slots, &[]);
    assert_eq!(runs.len(), 3, "opaque span fused; cutout split by slot");
    assert_eq!(runs[0].index_range, 0..9);
    assert_eq!(runs[1].index_range, 9..12);
    assert!(runs[1].cutout);
    assert_eq!(runs[2].slot, 1);
    // Kill the middle opaque item: the fused run splits around it.
    let runs = build_runs(&draws, &slots, &[0b010u64]);
    assert_eq!(runs.len(), 4);
    assert_eq!(runs[0].index_range, 0..3);
    assert_eq!(runs[1].index_range, 6..9);
    // Kill everything: nothing is submitted.
    assert!(build_runs(&draws, &slots, &[0b11111u64]).is_empty());
}
