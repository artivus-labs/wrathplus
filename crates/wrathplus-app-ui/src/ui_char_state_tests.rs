use super::*;

#[test]
fn outdoors_reads_the_wmo_row_then_the_exterior_bit_then_the_area() {
    // In a WMO group: its row's outdoor flag, its indoor flag, else indoors (an interior claim is
    // a group without the exterior bit).
    assert!(outdoors(
        Some(Some(WMO_AREA_OUTDOORS | WMO_AREA_INDOORS)),
        None
    ));
    assert!(!outdoors(
        Some(Some(WMO_AREA_INDOORS)),
        Some(AREA_FLAG_OUTSIDE)
    ));
    assert!(!outdoors(Some(Some(0)), Some(AREA_FLAG_OUTSIDE)));
    assert!(!outdoors(Some(None), None));
    // On terrain: the area's outside flag, its inside flag, else outdoors.
    assert!(outdoors(None, Some(AREA_FLAG_OUTSIDE | AREA_FLAG_INSIDE)));
    assert!(!outdoors(None, Some(AREA_FLAG_INSIDE)));
    assert!(outdoors(None, Some(0)));
    assert!(outdoors(None, None));
}

#[test]
fn flyable_needs_the_area_flag_and_cold_weather_flying_in_northrend() {
    assert!(flyable(Some(AREA_FLAG_FLYABLE), None));
    assert!(!flyable(
        Some(AREA_FLAG_FLYABLE | AREA_FLAG_NO_FLY_ZONE),
        None
    ));
    assert!(!flyable(Some(0), None));
    assert!(!flyable(None, None));
    assert!(flyable(Some(AREA_FLAG_FLYABLE), Some((530, false))));
    assert!(!flyable(
        Some(AREA_FLAG_FLYABLE),
        Some((MAP_NORTHREND, false))
    ));
    assert!(flyable(
        Some(AREA_FLAG_FLYABLE),
        Some((MAP_NORTHREND, true))
    ));
}

#[test]
fn in_water_is_the_swim_flag_or_deep_liquid_for_a_swimmer_off_transports() {
    const PLAYER: u32 = 0x8;
    let h = 2.0;
    assert!(in_water(move_flags::SWIMMING, None, h, 0, true));
    assert!(in_water(0, Some(1.6), h, PLAYER, false));
    assert!(
        !in_water(0, Some(1.5), h, PLAYER, false),
        "0.75 h is not past it"
    );
    assert!(
        !in_water(0, Some(3.0), h, PLAYER, true),
        "riding a transport"
    );
    assert!(
        !in_water(0, Some(3.0), h, PLAYER | 0x4000, false),
        "cannot swim"
    );
    assert!(!in_water(0, Some(3.0), h, 0, false), "no swim permit");
    assert!(!in_water(0, None, h, PLAYER, false));
}
