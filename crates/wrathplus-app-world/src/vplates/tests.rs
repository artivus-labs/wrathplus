use super::*;

/// `0x98e6a0` at level 30 (grayband[6] = 7: green down to 23, gray at 22), the low edge and the
/// band's cap of 8 (`0xaa34b8`).
#[test]
fn con_color_matches_the_byte_table() {
    assert_eq!(con_color(30, 35), CON_RED);
    assert_eq!(con_color(30, 33), CON_ORANGE);
    assert_eq!(con_color(30, 28), CON_YELLOW);
    assert_eq!(
        con_color(30, 23),
        CON_GREEN,
        "gap 7 = grayband[6], still green"
    );
    assert_eq!(con_color(30, 22), CON_GRAY, "gap 8 crosses the band");
    assert_eq!(con_color(60, 51), CON_GRAY, "gap 9 past the band's 8");
    assert_eq!(con_color(1, 1), CON_YELLOW);
    assert_ne!(
        con_color(5, 1),
        CON_GRAY,
        "grayband[1] = 4 ≥ the max gap at 5"
    );
    // 0xFFFF1919, 0xFFFF7F3F, 0xFF3FB23F.
    assert_eq!(CON_RED[1], 25.0 / 255.0);
    assert_eq!(CON_ORANGE[2], 63.0 / 255.0);
    assert_eq!(CON_GREEN[1], 178.0 / 255.0);
}

#[test]
fn the_plate_snaps_on_the_device_grid_not_the_logical_one() {
    // At 2x every snapped value is a whole number of physical pixels.
    for (v, want) in [(10.0, 10.0), (10.2, 10.0), (10.3, 10.5), (10.6, 10.5)] {
        let got = device_snap(v, 2.0);
        assert_eq!(got, want, "{v} at 2×");
        assert_eq!((got * 2.0).fract(), 0.0, "{got} is a whole physical pixel");
    }
    assert_eq!(device_snap(10.4, 1.0), 10.0);
    assert_eq!(device_snap(10.6, 1.0), 11.0);
    assert_eq!((device_snap(10.4, 1.5) * 1.5).fract(), 0.0);
    assert_eq!((device_snap(10.9, 1.5) * 1.5).fract(), 0.0);
}

/// Name em 13 at 1152x648 as measured in the reference; at 2560x1440 the reference's 29 and
/// the native 13 damp to 21.
#[test]
fn plate_text_sizes_take_the_damped_diagonal_basis() {
    let ref43 = plate_basis(Vec2::new(1024.0, 768.0));
    let refwin = plate_basis(Vec2::new(1152.0, 648.0));
    assert_eq!(text_px(NAME_H, ref43), 13.0);
    assert_eq!(text_px(LEVEL_H, ref43), 11.0);
    assert_eq!(text_px(NAME_H, refwin), 13.0);
    assert_eq!(text_px(LEVEL_H, refwin), 11.0);
    let big = plate_basis(Vec2::new(2560.0, 1440.0));
    assert_eq!(text_px(NAME_H, big), 21.0, "midway between 29 and 13");
    assert_eq!(text_px(NAME_H, 10_000.0), 32.0, "atlas-cell cap, raw law");
}

/// The flash shows for a threat situation of 1 to 3 in `GetThreatStatusColor`'s colours,
/// the `0xad2d70` table one slot up; situation 0 has none.
#[test]
fn the_threat_flash_takes_the_threat_tables_colours() {
    assert_eq!(threat_flash(0), None);
    assert_eq!(threat_flash(1), Some([1.0, 1.0, 119.0 / 255.0]));
    assert_eq!(threat_flash(2), Some([1.0, 153.0 / 255.0, 0.0]));
    assert_eq!(threat_flash(3), Some([1.0, 0.0, 0.0]));
}

/// The registered defaults show nothing until a category is on, and a category's sub-gates
/// pass its player-owned units through.
#[test]
fn the_plate_cvars_gate_by_category_then_sub_gate() {
    let mut mode = VPlateMode::default();
    for sort in [UnitSort::Player, UnitSort::Creature, UnitSort::Pet] {
        assert!(
            !mode.shows(false, sort) && !mode.shows(true, sort),
            "{sort:?}"
        );
    }
    mode.enemies = true;
    assert!(mode.shows(false, UnitSort::Creature));
    assert!(
        mode.shows(false, UnitSort::Totem),
        "the sub-gates default on"
    );
    assert!(!mode.shows(true, UnitSort::Player), "friends stay off");
    mode.enemy_totems = false;
    assert!(!mode.shows(false, UnitSort::Totem));
    assert!(mode.shows(false, UnitSort::Pet), "each sub-gate its own");
    mode.friends = true;
    mode.friendly_guardians = false;
    assert!(mode.shows(true, UnitSort::Pet));
    assert!(!mode.shows(true, UnitSort::Guardian));
}

/// `0x71f300`'s creature type and the owner links: critters and non-combat pets first, then a
/// player owner's totem, guardian (creator only) or pet; an NPC's own summons are creatures.
#[test]
fn units_sort_by_type_then_player_owner() {
    use wrathplus_protocol::types::Field;
    const PLAYER: u64 = 0x0000_0000_0000_0042;
    const NPC: u64 = 0xF130_0000_1234_0001;
    let at = |f: Field| f.index();
    let fields = |links: &[(Field, u64)]| {
        let pairs: Vec<(u16, u32)> = links
            .iter()
            .flat_map(|&(f, g)| [(at(f), g as u32), (at(f) + 1, (g >> 32) as u32)])
            .collect();
        wrathplus_protocol::ObjectFields::from_pairs(&pairs)
    };
    let none = fields(&[]);
    assert_eq!(unit_sort(true, &none, None), UnitSort::Player);
    assert_eq!(unit_sort(false, &none, Some(8)), UnitSort::NonCombat);
    assert_eq!(unit_sort(false, &none, Some(12)), UnitSort::NonCombat);
    assert_eq!(unit_sort(false, &none, Some(7)), UnitSort::Creature);
    let totem = fields(&[
        (Field::UnitCreatedby, PLAYER),
        (Field::UnitSummonedby, PLAYER),
    ]);
    assert_eq!(unit_sort(false, &totem, Some(11)), UnitSort::Totem);
    let guardian = fields(&[(Field::UnitCreatedby, PLAYER)]);
    assert_eq!(unit_sort(false, &guardian, Some(7)), UnitSort::Guardian);
    let pet = fields(&[
        (Field::UnitCreatedby, PLAYER),
        (Field::UnitSummonedby, PLAYER),
    ]);
    assert_eq!(unit_sort(false, &pet, Some(1)), UnitSort::Pet);
    let charmed = fields(&[(Field::UnitCharmedby, PLAYER)]);
    assert_eq!(unit_sort(false, &charmed, Some(7)), UnitSort::Pet);
    let npc_totem = fields(&[(Field::UnitCreatedby, NPC)]);
    assert_eq!(
        unit_sort(false, &npc_totem, Some(11)),
        UnitSort::Creature,
        "an NPC's totem takes only the category gate"
    );
}

#[test]
fn plate_tint_matches_the_byte_order() {
    assert_eq!(plate_tint(1, false), PLATE_HOSTILE);
    assert_eq!(plate_tint(1, true), PLATE_HOSTILE, "hostile beats player");
    assert_eq!(plate_tint(2, false), PLATE_NEUTRAL, "unfriendly is yellow");
    assert_eq!(plate_tint(3, false), PLATE_NEUTRAL);
    assert_eq!(plate_tint(3, true), PLATE_PLAYER, "player beats neutral");
    assert_eq!(plate_tint(4, false), PLATE_FRIENDLY);
    assert_eq!(plate_tint(6, true), PLATE_PLAYER, "player beats friendly");
}

/// At 1024x768 (diagonal 1280) the frame is the border's native 128x32 px; at 1080p the
/// reference's 220 and the native 128 damp to 174.
#[test]
fn plate_geometry_damps_past_the_native_knee() {
    let ref43 = plate_basis(Vec2::new(1024.0, 768.0));
    assert_eq!(gx_px(PLATE_W, ref43), 128.0);
    assert_eq!(gx_px(PLATE_H, ref43), 32.0);
    assert_eq!(gx_px(BAR_W, ref43), 103.0);
    assert_eq!(gx_px(BAR_H, ref43), 9.0);
    assert_eq!(
        gx_px(PLATE_W, plate_basis(Vec2::new(1920.0, 1080.0))),
        174.0,
        "midway between 220 and 128"
    );
    assert_eq!(gx_px(PLATE_W, plate_basis(Vec2::new(800.0, 600.0))), 100.0);
}
