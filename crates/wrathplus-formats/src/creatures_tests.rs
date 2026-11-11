use super::*;

/// HumanMale's Base boot print (id 1) is 12×10 inches, 1/3 × 5/18 yd; a `−1` model has none.
#[test]
fn footprints_resolve_on_real_data() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_creature_catalog(&mut chain).expect("load creature catalog");
    let human = cat
        .display
        .iter()
        .find(|(_, r)| {
            cat.models.get(&r.model_id).is_some_and(|m| {
                m.path
                    .eq_ignore_ascii_case(r"Character\Human\Male\HumanMale.mdx")
            })
        })
        .map(|(&d, _)| d)
        .expect("some display wears the HumanMale body");
    let p = cat.footprint(human).expect("HumanMale leaves prints");
    assert_eq!(p.texture_id, 1, "the Base boot print");
    assert!((p.length - 12.0 / 36.0).abs() < 1e-6, "length {}", p.length);
    assert!((p.width - 10.0 / 36.0).abs() < 1e-6, "width {}", p.width);
    let printless = cat
        .display
        .iter()
        .find(|(_, r)| {
            cat.models
                .get(&r.model_id)
                .is_some_and(|m| m.footprint_texture == -1)
        })
        .map(|(&d, _)| d)
        .expect("some display over a printless model");
    assert_eq!(cat.footprint(printless), None);
}

#[test]
fn character_model_npcs_resolve_a_shipped_baked_atlas() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_creature_catalog(&mut chain).expect("load creature catalog");
    assert!(
        cat.extra_len() > 1000,
        "CreatureDisplayInfoExtra loaded ({} rows)",
        cat.extra_len()
    );

    let mut verified = 0;
    for (&disp, row) in &cat.display {
        if row.extended_id == 0 {
            continue;
        }
        let Some(m) = cat.model(disp) else { continue };
        let Some(bake) = m.npc_appearance.as_ref().and_then(|a| a.bake_name.as_ref()) else {
            continue;
        };
        assert!(
            m.model_path.to_ascii_lowercase().starts_with("character\\"),
            "an extended-display NPC wears a Character\\ body, got {}",
            m.model_path
        );
        let path = format!("Textures\\BakedNpcTextures\\{bake}");
        assert!(
            chain.read_file(&path).is_ok(),
            "the baked body atlas ships: {path}"
        );
        verified += 1;
        if verified >= 20 {
            break;
        }
    }
    assert!(
        verified >= 5,
        "found + verified several baked character-model NPCs (got {verified})"
    );
}

#[test]
fn stormwind_guard_equipment_columns_decode_in_bodyslot_order() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_creature_catalog(&mut chain).expect("load creature catalog");
    let guard = cat
        .model(3167)
        .expect("Stormwind City Guard display 3167 resolves");
    let npc = guard
        .npc_appearance
        .expect("display 3167 is a character-model NPC with an appearance row");
    // Body-slot order: head, shoulder, shirt, chest, belt, pants, boots, wrist, gloves, tabard.
    assert_eq!(
        npc.equipment,
        [14964, 7541, 7223, 0, 7224, 7225, 7255, 0, 7698, 6255],
        "SW Guard worn-equipment ids in bodyslot order"
    );
}

/// `0x71c110` over both scale columns and, for an NPC, its race display's.
#[test]
fn unit_model_scales_compose_the_race_display_and_both_columns() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_creature_catalog(&mut chain).expect("load creature catalog");
    for (display, want, what) in [
        (4449, 0.5, "Imp, creatureModelScale"),
        (9535, 0.2, "Mangy Duskbat, creatureModelScale"),
        (59, 1.35, "tauren male body, creatureModelScale"),
        (60, 1.25, "tauren female body, modelScale"),
        (1624, 1.35, "tauren male NPC, the race display's scale"),
        (1832, 1.15, "gnome male NPC, the race display's scale"),
    ] {
        let k = cat.unit_model_scale(display).expect("display resolves");
        assert!((k - want).abs() < 5e-4, "{display} ({what}) is {k}");
    }
    assert_eq!(cat.unit_model_scale(u32::MAX), None);
}

/// vmangos folds both scales into `SCALE_X`, so the prism's `max(SCALE_X, display scale)` floor
/// bites only where `modelScale < 1`, and no shipped row is below 1.
#[test]
fn no_shipped_model_scales_below_one_so_the_prism_floor_is_inert_at_rest() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_creature_catalog(&mut chain).expect("load creature catalog");

    let under: Vec<_> = cat
        .models
        .iter()
        .filter(|(_, m)| m.scale < 1.0)
        .map(|(id, m)| (*id, m.scale))
        .collect();
    assert!(
        under.is_empty(),
        "a sub-1 modelScale would make the floor bite at rest: {under:?}"
    );
    // Not vacuous: the column is read, and varies.
    assert!(
        cat.models.values().any(|m| m.scale > 1.0),
        "some row scales above 1.0, or this is asserting on a zeroed column"
    );
}

/// The Shore Strider (display 4945, model 35): column 2.083 × display scale 1.75 = 3.645 yd
/// with or without the floor, since its `modelScale` is 1.0.
#[test]
fn the_shore_strider_prism_is_the_same_under_both_readings() {
    let data = crate::wow_data_or_skip!();
    let mut chain = crate::open_chain(&data).expect("open chain");
    let cat = load_creature_catalog(&mut chain).expect("load creature catalog");

    let m = cat.model(4945).expect("display 4945 resolves");
    assert_eq!(m.model_path, "Creature\\SeaGiant\\SeaGiant.mdx");
    let column = cat.collision_height(4945).expect("collision height");
    let display_scale = cat.display_scale(4945).expect("display scale");
    assert!((column - 2.083).abs() < 5e-4, "column is {column}");
    assert!(
        (display_scale - 1.75).abs() < 5e-4,
        "CreatureDisplayInfo.scale is {display_scale}"
    );
    // vmangos ships `creature_template.display_scale = 0` for entry 5359, so SCALE_X is the
    // folded `modelScale × displayScale` = 1.0 × 1.75.
    let scale_x = m.scale;
    assert!((scale_x - 1.75).abs() < 5e-4, "folded SCALE_X is {scale_x}");
    assert_eq!(
        column * scale_x,
        column * scale_x.max(display_scale),
        "the floor is inert on this row — the height was not the bug"
    );
}
