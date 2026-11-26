//! The world-map bindings: catalog and feed pushes, the engine-owned selection, and the deferred
//! `WORLD_MAP_UPDATE` queue.

use super::common::script;
use crate::script::*;

/// A two-continent catalog shaped like the real one: Kalimdor first, since the push order defines
/// every Lua-visible index, with the real disjoint sheet rects. The sparse EK grid marks the real
/// Goldshire cell (12735 in `Azeroth.zmp`) as zone 1.
fn push_catalog(s: &mut UiScript) {
    let mut ek_grid = vec![0u16; 128 * 128];
    ek_grid[12735] = 1;
    // Cell 8801 is where (0.5, 0.5) lands through Elwynn's own loc rect: the Stormwind City peer,
    // which a click on the Elwynn map drills into (`0x4a67a0`).
    ek_grid[8801] = 2;
    // Cell 9060, where (0.82, 0.82) lands: Elwynn itself, which the zone-level hover never names.
    ek_grid[9060] = 1;
    s.set_world_map_catalog(vec![
        WorldMapContinentView {
            map_id: 0,
            wma_id: 0,
            texture_height: 0,
            name: "Kalimdor".into(),
            map_file: "Kalimdor".into(),
            world_rect: (0.08882, 0.07910, 0.40020, 0.86952),
            loc_rect: (17066.6, -19733.2, 12799.9, -11733.3),
            zone_grid: Vec::new(),
            zones: vec![
                WorldMapZoneView {
                    wma_id: 0,
                    name: "Durotar".into(),
                    area_id: 14,
                    map_file: "Durotar".into(),
                    loc_rect: (-1800.0, -6800.0, -3800.0, -8500.0),
                    overlays: Vec::new(),
                },
                WorldMapZoneView {
                    wma_id: 0,
                    name: "The Barrens".into(),
                    area_id: 17,
                    map_file: "Barrens".into(),
                    loc_rect: (-1000.0, -11000.0, -5500.0, -17000.0),
                    overlays: Vec::new(),
                },
            ],
        },
        WorldMapContinentView {
            map_id: 0,
            wma_id: 0,
            texture_height: 0,
            name: "Eastern Kingdoms".into(),
            map_file: "Azeroth".into(),
            world_rect: (0.62375, 0.02695, 0.92315, 0.87126),
            loc_rect: (16000.0, -19199.9, 7466.6, -16000.0),
            zone_grid: ek_grid,
            zones: vec![
                WorldMapZoneView {
                    wma_id: 0,
                    name: "Elwynn Forest".into(),
                    area_id: 12,
                    map_file: "Elwynn".into(),
                    // Synthetic, inside EK's rect: w=2000, h=1500, for round highlight coords.
                    loc_rect: (-8000.0, -10000.0, -400.0, -1900.0),
                    // Northshire (bit 125) and Goldshire (124) carry the real exploreFlags; hit
                    // rects are (top, left, bottom, right) in px of 1002×668. The first row's area
                    // is unresolvable (bit 126) and overlaps Northshire: the hover walks past it.
                    overlays: vec![
                        WorldMapOverlayView {
                            always: false,
                            texture: "Interface\\WorldMap\\Elwynn\\STONECAIRNLAKE".into(),
                            width: 128,
                            height: 128,
                            offset_x: 450,
                            offset_y: 300,
                            explore_bits: vec![126],
                            hit_rect: (300, 450, 400, 560),
                            area_name: None,
                        },
                        WorldMapOverlayView {
                            always: false,
                            texture: "Interface\\WorldMap\\Elwynn\\NORTHSHIREVALLEY".into(),
                            width: 200,
                            height: 176,
                            offset_x: 490,
                            offset_y: 200,
                            explore_bits: vec![125],
                            hit_rect: (200, 490, 376, 690),
                            area_name: Some("Northshire Valley".into()),
                        },
                        WorldMapOverlayView {
                            always: false,
                            texture: "Interface\\WorldMap\\Elwynn\\GOLDSHIRE".into(),
                            width: 172,
                            height: 128,
                            offset_x: 380,
                            offset_y: 320,
                            explore_bits: vec![124],
                            hit_rect: (400, 300, 520, 460),
                            area_name: Some("Goldshire".into()),
                        },
                    ],
                },
                // A city peer of Elwynn, which the reference treats as a zone (`0x4a67a0`).
                WorldMapZoneView {
                    wma_id: 0,
                    name: "Stormwind City".into(),
                    area_id: 1519,
                    map_file: "StormwindCity".into(),
                    loc_rect: (-8200.0, -8900.0, -400.0, -1100.0),
                    overlays: Vec::new(),
                },
            ],
        },
    ]);
}

/// The orphan list, `0x4a5d00`'s third array: in 1.12 exactly the three battlegrounds, with their
/// real row, map and area ids and art folders, in `WorldMapArea.dbc` file order (no sort). Alterac
/// Valley carries one of its real `WorldMapOverlay` rows (AreaBit 954); WSG and AB have none.
fn push_direct_areas(s: &mut UiScript) {
    let row = |name: &str, area_id: u32, folder: &str, overlays: Vec<WorldMapOverlayView>| {
        WorldMapZoneView {
            wma_id: 0,
            name: name.into(),
            area_id,
            map_file: folder.into(),
            // A synthetic rect: the app owns every projection, so nothing here reads it.
            loc_rect: (1500.0, 500.0, 1600.0, 600.0),
            overlays,
        }
    };
    s.set_world_map_direct_areas(vec![
        (
            401,
            row(
                "Alterac Valley",
                2597,
                "AlteracValley",
                vec![WorldMapOverlayView {
                    always: false,
                    texture: "Interface\\WorldMap\\AlteracValley\\DUNBALDAR".into(),
                    width: 128,
                    height: 128,
                    offset_x: 300,
                    offset_y: 200,
                    explore_bits: vec![954],
                    hit_rect: (200, 300, 328, 428),
                    area_name: Some("Dun Baldar".into()),
                }],
            ),
        ),
        (443, row("Warsong Gulch", 3277, "WarsongGulch", Vec::new())),
        (461, row("Arathi Basin", 3358, "ArathiBasin", Vec::new())),
    ]);
}

/// Build 12340 lists overlays for a zone only: an instance map shows none, explored or not
/// (`0x546310`'s overlay walk runs only for a continent index, never `-2`).
#[test]
fn an_instance_map_shows_no_overlays() {
    let mut s = script();
    push_catalog(&mut s);
    push_direct_areas(&mut s);

    s.set_world_map_player_direct_area(Some(401), None);
    s.run("SetMapToCurrentZone()").unwrap();
    s.set_world_map_explored(vec![u32::MAX; 64]);
    assert_eq!(s.eval::<i64>("return GetNumMapOverlays()").unwrap(), 0);
    assert!(s
        .eval::<bool>("return GetMapOverlayInfo(1) == nil")
        .unwrap());
}

/// `SetMapToCurrentZone` lands on the app-fed player zone, and the positions project at each call
/// (`0x545880`): any token naming the player or a party or raid member (`0x60c1c0`), the projection
/// reading the selection as it stands, so a `SetMapZoom` in the same handler moves the answer.
#[test]
fn worldmap_current_zone_and_player_feed() {
    let mut s = script();
    push_catalog(&mut s);
    let pos = |x: f32, y: f32| crate::script::MapPos {
        map: 0,
        x,
        y,
        floor: None,
    };
    // On a zone the position is its own UV; anywhere else it is off the map.
    s.set_world_map_projection(|sel, p| (sel.zone != 0).then_some((p.x, p.y)));
    s.set_unit_guids(&crate::script::UnitGuids {
        player: 1,
        party: [2, 0, 0, 0],
        target: 2,
        raid: vec![1, 2],
        ..Default::default()
    });
    s.set_world_map_feed(
        Some((1, 2)),
        Some(pos(0.25, 0.75)),
        1.5,
        None,
        vec![(2, pos(0.1, 0.2))],
    );
    s.run("SetMapToCurrentZone()").unwrap();
    assert_eq!(
        s.eval::<(i64, i64)>("return GetCurrentMapContinent(), GetCurrentMapZone()")
            .unwrap(),
        (1, 2)
    );
    let near = |token: &str, want: (f64, f64)| {
        let (x, y) = s
            .eval::<(f64, f64)>(&format!(r#"return GetPlayerMapPosition("{token}")"#))
            .unwrap();
        assert!(
            (x - want.0).abs() < 1e-6 && (y - want.1).abs() < 1e-6,
            "{token}: {x}, {y}"
        );
    };
    for token in ["player", "PLAYER", "raid1"] {
        near(token, (0.25, 0.75));
    }
    for token in ["party1", "target", "raid2"] {
        near(token, (0.1, 0.2));
    }
    for token in ["party2", "party5", "raid3", "focus", "nonsense"] {
        near(token, (0.0, 0.0));
    }
    assert!(s.run("GetPlayerMapPosition()").is_err());
    // The same handler, after the selection moved: the continent map, where nothing projects.
    let both: (f64, f64, f64, f64) = s
        .eval(
            r#"local a, b = GetPlayerMapPosition("player") SetMapZoom(1)
                 local c, d = GetPlayerMapPosition("player") return a, b, c, d"#,
        )
        .unwrap();
    assert_eq!(both, (0.25, 0.75, 0.0, 0.0));

    // No feed → the world sheet (never an error).
    s.set_world_map_feed(None, None, 0.0, None, Vec::new());
    s.run("SetMapToCurrentZone()").unwrap();
    assert_eq!(s.eval::<i64>("return GetCurrentMapContinent()").unwrap(), 0);
}

/// A world-level `ProcessMapClick` picks the continent whose sheet rect contains it (`0x4a7100`,
/// the real rects being disjoint); continent-level clicks and hovers resolve through the zone grid
/// (`0x4a6ec0`).
#[test]
fn worldmap_click_containment_and_zone_grid() {
    let mut s = script();
    push_catalog(&mut s);

    // Mid-Kalimdor on the world sheet.
    s.run("ProcessMapClick(0.25, 0.45)").unwrap();
    assert_eq!(s.eval::<i64>("return GetCurrentMapContinent()").unwrap(), 1);

    // Kalimdor ships no grid in this fixture: continent-level clicks are inert.
    s.run("ProcessMapClick(0.5, 0.5)").unwrap();
    assert_eq!(
        s.eval::<(i64, i64)>("return GetCurrentMapContinent(), GetCurrentMapZone()")
            .unwrap(),
        (1, 0)
    );

    // Back at the world level, EK land picks continent 2.
    s.run("SetMapZoom(0)").unwrap();
    s.run("ProcessMapClick(0.72, 0.63)").unwrap();
    assert_eq!(s.eval::<i64>("return GetCurrentMapContinent()").unwrap(), 2);

    // Continent level: Goldshire's uv lands on cell 12735, zone 1, which lights up with its name,
    // file name and six geometry values (`0x4a8494`) against its loc rect in the continent:
    // cont_w=35199.9, cont_h=23466.6; w=2000, h=1500.
    let (name, file, tpx, tpy, tx, ty, sx, sy) = s
        .eval::<(String, String, f64, f64, f64, f64, f64, f64)>(
            "return UpdateMapHighlight(0.452843, 0.720880)",
        )
        .unwrap();
    assert_eq!(name, "Elwynn Forest");
    assert_eq!(
        file, "Elwynn",
        "fileName = the zone art folder → <file>Highlight"
    );
    assert_eq!(
        tpx, 1.0,
        "continent-branch texPercentageX is the constant 1.0"
    );
    assert!(
        (tpy - 0.75).abs() < 1e-6,
        "texPercentageY = dim/potdim = 96/128 = 0.75, got {tpy}"
    );
    assert!(
        (tx - 2000.0 / 35199.9).abs() < 1e-4,
        "textureX = w/contW, got {tx}"
    );
    assert!(
        (ty - 1500.0 / 23466.6).abs() < 1e-4,
        "textureY = h/contH, got {ty}"
    );
    assert!(
        (sx - 24000.0 / 35199.9).abs() < 1e-4,
        "scrollChildX = (contX0-zoneX0)/contW, got {sx}"
    );
    assert!(
        (sy - 7866.6 / 23466.6).abs() < 1e-4,
        "scrollChildY = (contY0-zoneY0)/contH, got {sy}"
    );
    // Ocean at continent level: off the grid → the nil/zero tail (frame hides the quad).
    assert!(s
        .eval::<Option<String>>("return UpdateMapHighlight(0.99, 0.01)")
        .unwrap()
        .is_none());
    s.run("ProcessMapClick(0.452843, 0.720880)").unwrap();
    assert_eq!(
        s.eval::<(i64, i64)>("return GetCurrentMapContinent(), GetCurrentMapZone()")
            .unwrap(),
        (2, 1),
        "the grid click drills into the zone"
    );

    // At zone level a click through Elwynn's own rect lands on cell 8801, the Stormwind City peer,
    // and drills in: `0x4a75c9` runs continent- and zone-level clicks alike, windowed differently.
    s.run("ProcessMapClick(0.5, 0.5)").unwrap();
    assert_eq!(
        s.eval::<(i64, i64)>("return GetCurrentMapContinent(), GetCurrentMapZone()")
            .unwrap(),
        (2, 2),
        "a zone-level click on a city footprint drills into the city"
    );

    // Open ocean at the world level: outside both rects, no selection change.
    s.run("SetMapZoom(0)").unwrap();
    s.run("ProcessMapClick(0.01, 0.99)").unwrap();
    assert_eq!(s.eval::<i64>("return GetCurrentMapContinent()").unwrap(), 0);
}

/// At zone level `UpdateMapHighlight` answers a name only (`0x4a812e`): a revealed overlay's
/// sub-area under the cursor, else the neighbouring zone or city whose cell the cursor is in
/// through the displayed zone's rect, never that zone itself. The file name is nil and the
/// geometry zeros, so the stock frame hides the quad.
#[test]
fn worldmap_zone_level_hover_names_without_highlight() {
    let mut s = script();
    push_catalog(&mut s);
    s.run("SetMapZoom(2, 1)").unwrap(); // the Elwynn zone map

    type Answer = (Option<String>, Option<String>, f64, f64, f64, f64, f64, f64);
    let hover = |s: &mut UiScript, x: f32, y: f32| {
        s.eval::<Answer>(&format!("return UpdateMapHighlight({x}, {y})"))
            .unwrap()
    };
    let name_only =
        |name: &str| -> Answer { (Some(name.into()), None, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0) };
    let miss: Answer = (None, None, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);

    // Nothing explored: (0.5, 0.5) is cell 8801, the Stormwind City peer, named and nothing else.
    assert_eq!(hover(&mut s, 0.5, 0.5), name_only("Stormwind City"));
    // Cell 9060 = Elwynn itself: the displayed zone never names itself.
    assert_eq!(hover(&mut s, 0.82, 0.82), miss);
    // An empty cell, no overlay: the tail.
    assert_eq!(hover(&mut s, 0.2, 0.2), miss);
    // Inside Goldshire's hit rect, fogged (bit 124 clear): the pre-search walks only revealed
    // overlays, and cell 8929 under it is empty.
    assert_eq!(hover(&mut s, 0.42, 0.7), miss);

    // Reveal Northshire (bit 125 = word 3, bit 29): the overlay pre-search runs before the grid,
    // so its sub-area wins over the city under (0.5, 0.5).
    let mut explored = vec![0u32; 64];
    explored[3] = 1 << 29;
    s.set_world_map_explored(explored.clone());
    assert_eq!(hover(&mut s, 0.5, 0.5), name_only("Northshire Valley"));
    assert_eq!(hover(&mut s, 0.42, 0.7), miss, "Goldshire is still fogged");
    // Reveal Goldshire too (bit 124).
    explored[3] |= 1 << 28;
    s.set_world_map_explored(explored.clone());
    assert_eq!(hover(&mut s, 0.42, 0.7), name_only("Goldshire"));

    // Only the nameless overlay revealed (bit 126 = word 3, bit 30): its rect contains (0.5, 0.5)
    // but its first area resolves to nothing, so the walk passes it and the grid answers.
    let mut explored = vec![0u32; 64];
    explored[3] = 1 << 30;
    s.set_world_map_explored(explored.clone());
    assert_eq!(hover(&mut s, 0.5, 0.5), name_only("Stormwind City"));
    // Nameless first, Northshire second, both revealed: the walk continues to Northshire.
    explored[3] |= 1 << 29;
    s.set_world_map_explored(explored);
    assert_eq!(hover(&mut s, 0.5, 0.5), name_only("Northshire Valley"));

    // Control: at continent level the hovered zone still lights up, with its art folder.
    s.run("SetMapZoom(2)").unwrap();
    let (name, file, tpx, _, tx, ty, _, _) = hover(&mut s, 0.452843, 0.720880);
    assert_eq!(
        (name.as_deref(), file.as_deref(), tpx),
        (Some("Elwynn Forest"), Some("Elwynn"), 1.0)
    );
    assert!(tx > 0.0 && ty > 0.0);
}

/// `SetMapZoom` queues `WORLD_MAP_UPDATE` rather than firing it, since a binding cannot re-enter
/// dispatch; the registered frame hears it on the next tick.
#[test]
fn worldmap_update_event_fires_on_next_tick() {
    let mut s = script();
    push_catalog(&mut s);
    // Drain the catalog push's own queued event first.
    s.tick(0.01);
    s.run(
        r#"
        heard = 0
        f = CreateFrame("Frame", "MapListener")
        f:RegisterEvent("WORLD_MAP_UPDATE")
        f:SetScript("OnEvent", function(this, event, arg1, arg2, arg3, arg4) heard = heard + 1 end)
        SetMapZoom(1)
    "#,
    )
    .unwrap();
    assert_eq!(
        s.eval::<i64>("return heard").unwrap(),
        0,
        "nothing fires inside the SetMapZoom call itself"
    );
    s.tick(0.01);
    assert_eq!(s.eval::<i64>("return heard").unwrap(), 1);
    s.tick(0.01);
    assert_eq!(
        s.eval::<i64>("return heard").unwrap(),
        1,
        "the queue drains — no re-fire"
    );
}

/// Overlays reveal per the pushed explored bitset: none before a push, the matching subset after,
/// each with its full info tuple.
#[test]
fn worldmap_overlays_reveal_by_explored_bits() {
    let mut s = script();
    push_catalog(&mut s);
    s.run("SetMapZoom(2, 1)").unwrap(); // the Elwynn zone map

    assert_eq!(s.eval::<i64>("return GetNumMapOverlays()").unwrap(), 0);

    // Bit 125 set (word 3, bit 29): Northshire reveals, Goldshire stays fogged.
    let mut explored = vec![0u32; 64];
    explored[3] = 1 << 29;
    s.set_world_map_explored(explored);
    assert_eq!(s.eval::<i64>("return GetNumMapOverlays()").unwrap(), 1);
    let (tex, w, h, ox, oy) = s
        .eval::<(String, i64, i64, i64, i64)>("return GetMapOverlayInfo(1)")
        .unwrap();
    assert_eq!(tex, "Interface\\WorldMap\\Elwynn\\NORTHSHIREVALLEY");
    assert_eq!((w, h, ox, oy), (200, 176, 490, 200));

    // Both bits: both overlays, catalog order; out-of-range asks read nil.
    let mut explored = vec![0u32; 64];
    explored[3] = (1 << 29) | (1 << 28);
    s.set_world_map_explored(explored);
    assert_eq!(s.eval::<i64>("return GetNumMapOverlays()").unwrap(), 2);
    assert!(s
        .eval::<bool>("return GetMapOverlayInfo(3) == nil")
        .unwrap());

    // At continent level the overlay family reads empty: fog is a zone-map thing.
    s.run("SetMapZoom(2)").unwrap();
    assert_eq!(s.eval::<i64>("return GetNumMapOverlays()").unwrap(), 0);
}

/// `GetWorldLocMapPosition` (`0x4a88f0`) hands the displayed selection, the truncated map id and
/// the f32 position to the host's projection, answers `(0, 0)` without one, and raises on any
/// non-number argument.
#[test]
fn world_loc_map_position_projects_through_the_host() {
    let mut s = script();
    push_catalog(&mut s);
    assert_eq!(
        s.eval::<(f64, f64)>("return GetWorldLocMapPosition(0, 100, 200)")
            .unwrap(),
        (0.0, 0.0),
        "no projection: the zeroed outputs"
    );
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = seen.clone();
    s.set_world_loc_projector(Box::new(move |sel, map, x, y| {
        log.borrow_mut().push((sel, map, x, y));
        (map == 0).then_some((0.25, 0.75))
    }));
    s.run("SetMapZoom(2, 1)").unwrap();
    assert_eq!(
        s.eval::<(f64, f64)>("return GetWorldLocMapPosition(0.9, '-8913.2', 554.6)")
            .unwrap(),
        (0.25, 0.75)
    );
    assert_eq!(
        s.eval::<(f64, f64)>("return GetWorldLocMapPosition(-1, 0, 0)")
            .unwrap(),
        (0.0, 0.0)
    );
    let seen = seen.borrow();
    assert_eq!(
        seen[0],
        (
            crate::script::MapSelection {
                continent: 2,
                zone: 1,
                ..Default::default()
            },
            0,
            -8913.2f32,
            554.6f32
        )
    );
    assert_eq!(seen[1].1, u32::MAX, "a negative map id matches nothing");
    for call in [
        "GetWorldLocMapPosition(0, 1)",
        "GetWorldLocMapPosition('x', 1, 1)",
        "GetWorldLocMapPosition(0, nil, 1)",
    ] {
        let err = s.run(call).unwrap_err().to_string();
        assert!(
            err.contains("Usage: GetWorldLocMapPosition(continent, x, y)"),
            "{call}: {err}"
        );
    }
}

#[test]
fn the_death_release_position_projects_like_a_world_location() {
    let mut s = crate::script::UiScript::new().unwrap();
    assert_eq!(
        s.eval::<(f64, f64)>("return GetDeathReleasePosition()")
            .unwrap(),
        (0.0, 0.0)
    );
    s.set_death_release_location(Some((1, 10.0, 20.0)));
    assert_eq!(
        s.eval::<(f64, f64)>("return GetDeathReleasePosition()")
            .unwrap(),
        (0.0, 0.0),
        "no projector"
    );
    s.set_world_loc_projector(Box::new(|_, map, x, y| {
        (map == 1 && x == 10.0 && y == 20.0).then_some((0.5, 0.25))
    }));
    assert_eq!(
        s.eval::<(f64, f64)>("return GetDeathReleasePosition()")
            .unwrap(),
        (0.5, 0.25)
    );
    s.set_death_release_location(None);
    assert_eq!(
        s.eval::<(f64, f64)>("return GetDeathReleasePosition()")
            .unwrap(),
        (0.0, 0.0)
    );
}
