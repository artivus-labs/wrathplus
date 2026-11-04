//! The build-12340 layouts against the installed client: known rows read their known values.

use crate::Chain;

fn chain() -> Option<Chain> {
    Chain::open_wrath(&crate::wow_data()?, "enUS").ok()
}

#[test]
fn installed_wrath_spells_read_through_the_mapped_layout() {
    let Some(mut chain) = chain() else { return };
    let spells = crate::load_spell_catalog(&mut chain).expect("Spell.dbc");
    let fireball = spells.get(133).expect("Fireball rank 1");
    eprintln!(
        "133: {:?} {:?} icon={:?} school={} family={} mana={} level={} range={}",
        fireball.name,
        fireball.rank,
        fireball.icon,
        fireball.school,
        fireball.spell_family,
        fireball.mana_cost,
        fireball.spell_level,
        fireball.range_index
    );
    assert_eq!(fireball.name, "Fireball");
    assert_eq!(fireball.rank.as_deref(), Some("Rank 1"));
    assert_eq!(fireball.school, 2, "fire");
    assert_eq!(fireball.spell_family, 3, "mage");
    assert_eq!(
        fireball.icon.as_deref(),
        Some("Interface\\Icons\\Spell_Fire_FlameBolt")
    );
    assert_eq!(fireball.effect_base_points[0], 13);
    assert_eq!(fireball.effect_die_sides[0], 9);
    assert_eq!(fireball.effect_base_dice[0], 1);
    let durations = crate::load_spell_durations(&mut chain).unwrap();
    let radii = crate::load_spell_radii(&mut chain).unwrap();
    let context = crate::TokenContext {
        durations: &durations,
        radii: &radii,
        ranges: None,
        skill: &|_| 0,
        lookup: &|id| spells.get(id),
        mods: None,
        unmodified_points: false,
        home_area: &|| None,
        player: None,
        global: &|key| (key == "INT_SPELL_POINTS_SPREAD_TEMPLATE").then(|| "%d to %d".into()),
        printf: &|template, args| {
            let mut text = template.to_owned();
            for arg in args {
                if let crate::TokenNumber::Int(n) = arg {
                    text = text.replacen("%d", &n.to_string(), 1);
                }
            }
            text
        },
    };
    assert_eq!(
        crate::substitute("$s1 damage", fireball, &context),
        "14 to 22 damage"
    );
    let pick_lock = spells.get(1804).expect("Pick Lock");
    assert_eq!(pick_lock.effect_base_points, [4, 0, 0]);
    assert_eq!(pick_lock.effect_die_sides, [1, 0, 0]);
    assert_eq!(pick_lock.effect_base_dice, [1, 0, 0]);
    for skill in [150, 225, 300] {
        assert_eq!(pick_lock.open_lock_skill(skill), Some(skill as i32));
    }
    let heroic = spells.get(78).expect("Heroic Strike rank 1");
    eprintln!(
        "78: {:?} power={} cost={}",
        heroic.name, heroic.power_type, heroic.mana_cost
    );
    assert_eq!(heroic.name, "Heroic Strike");
}

// AzerothCore DBCStructure.h: SkillLine/SkillLineAbility, TalentTab/Talent,
// CreatureFamily and SpellShapeshiftForm; verified against the effective installed rows.
#[test]
fn installed_wrath_skill_and_talent_known_rows() {
    let Some(mut chain) = chain() else { return };
    let skills = crate::load_skill_line_catalog(&mut chain).expect("skill lines");
    let smithing = skills.line(164).expect("Blacksmithing");
    // `ChrClasses` 35 slots: the name blocks and file name `FillLocalizedClassList` reads.
    let classes = crate::load_chr_classes(&mut chain)
        .expect("classes")
        .localized_names();
    assert!(
        classes.contains(&(
            "DEATHKNIGHT".into(),
            "Death Knight".into(),
            "Death Knight".into()
        )),
        "{classes:?}"
    );
    // `alternateVerb_lang`, file column 38: Enchanting's recipes create no item.
    assert_eq!(
        skills.line(333).map(|l| l.alternate_verb.as_str()),
        Some("Enchant")
    );
    assert_eq!(smithing.alternate_verb, "Modify");
    assert_eq!(
        (smithing.name.as_str(), smithing.category_id),
        ("Blacksmithing", 11)
    );
    assert_eq!(
        smithing.icon.as_deref(),
        Some(r"Interface\Icons\Trade_BlackSmithing")
    );
    assert!(smithing
        .description
        .starts_with("Higher smithing skill allows you"));
    let frost = skills.line(6).expect("Frost skill line");
    assert_eq!((frost.name.as_str(), frost.category_id), ("Frost", 7));
    assert_eq!(
        frost.icon.as_deref(),
        Some(r"Interface\Icons\Spell_Frost_FrostBolt02")
    );
    assert!(frost.description.is_empty());
    let apprentice = skills
        .ability(2018)
        .expect("Apprentice Blacksmithing ability");
    assert_eq!(
        (
            apprentice.skill_id,
            apprentice.req_skill_value,
            apprentice.forward_spell_id
        ),
        (164, 1, 3100)
    );
    assert_eq!((apprentice.trivial_low, apprentice.trivial_high), (0, 0));

    let talents = crate::load_talent_catalog(&mut chain).expect("talents");
    let tabs = talents.tabs_for_class(1, 8);
    assert_eq!(tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [81, 41, 61]);
    let fire = tabs[1];
    assert_eq!(
        (fire.name.as_str(), fire.background.as_str()),
        ("Fire", "MageFire")
    );
    assert_eq!((fire.race_mask, fire.class_mask), (2047, 128));
    let impact = talents.talent(41, 27).expect("Impact in Fire tab");
    assert_eq!((impact.row, impact.col), (0, 0));
    assert_eq!(impact.ranks, [11078, 11080, 0, 0, 0]);
    assert_eq!(impact.max_rank(), 2);
    assert_eq!(
        (
            impact.prereq_talent,
            impact.prereq_rank,
            impact.required_spell
        ),
        (0, 0, 0)
    );
    assert!(!impact.exceptional);
}

#[test]
fn installed_wrath_family_and_form_known_rows() {
    let Some(mut chain) = chain() else { return };
    let families = crate::load_creature_families(&mut chain).expect("families");
    assert_eq!(families.len(), 40);
    let wolf = families.get(1).expect("Wolf family");
    assert_eq!(
        (wolf.name.as_str(), wolf.icon.as_str(), wolf.pet_food_mask),
        ("Wolf", r"Interface\Icons\Ability_Hunter_Pet_Wolf", 1)
    );
    assert!((wolf.min_scale - 0.7).abs() < 1e-6);
    assert_eq!((wolf.min_scale_level, wolf.max_scale_level), (1, 60));
    assert_eq!(wolf.max_scale, 1.0);
    let imp = families.get(23).expect("Imp family");
    assert_eq!(
        (imp.name.as_str(), imp.icon.as_str(), imp.pet_food_mask),
        ("Imp", "", 0)
    );
    assert_eq!((imp.min_scale, imp.max_scale), (0.5, 0.5));
    let forms = crate::load_shapeshift_forms(&mut chain).expect("forms");
    assert_eq!(forms.len(), 32);
    let cat = forms.get(&1).expect("Cat Form");
    assert_eq!(
        (
            cat.name.as_str(),
            cat.bonus_bar,
            cat.flags,
            cat.creature_type
        ),
        ("Cat Form", 1, 760, 1)
    );
    assert_eq!(
        cat.attack_icon.as_deref(),
        Some(r"Interface\Icons\Ability_Druid_CatFormAttack")
    );
}

#[test]
fn installed_wrath_human_reaction_to_a_diseased_young_wolf() {
    let Some(mut chain) = chain() else { return };
    let factions = crate::load_faction_catalog(&mut chain).expect("factions");
    let human = factions.race_template(1).expect("human template");
    let wolf = factions.template(32).expect("template 32");
    assert_eq!(
        (
            human.faction,
            human.group_mask,
            human.friend_group_mask,
            human.enemy_group_mask
        ),
        (1, 3, 2, 12)
    );
    assert_eq!(
        (wolf.faction, wolf.group_mask, wolf.enemies),
        (29, 0, [28, 0, 0, 0])
    );
    assert_eq!(wolf.reaction_toward(human), crate::Reaction::Neutral);
    assert_eq!(human.reaction_toward(wolf), crate::Reaction::Neutral);
    // Faction's wider name/description blocks must not read the spillover columns.
    assert_eq!(factions.faction_name(72), Some("Stormwind"));
    assert_eq!(factions.faction_description(72), Some("One of the last bastions of human power, this Alliance capital is ruled by the prodigal king, Varian Wrynn."));
    let stormwind = factions
        .reputation_faction(72)
        .expect("Stormwind reputation");
    assert_eq!((stormwind.rep_index, stormwind.team), (19, 469));
    assert_eq!(stormwind.base_for(1, 1), 4000);
    assert_eq!(stormwind.base_for(2, 1), -42000);
    assert_eq!(stormwind.default_flags_for(1, 1), 17);
}

#[test]
fn installed_wrath_spell_visuals_load() {
    let Some(mut chain) = chain() else { return };
    let cat = crate::load_spell_visual_catalog(&mut chain).expect("SpellVisual/SpellVisualKit");
    eprintln!("spell visual kits: {}", cat.kit_len());
    assert!(cat.kit_len() > 8000);
}

#[test]
fn installed_wrath_particle_emitters_read() {
    let Some(mut chain) = chain() else { return };
    let bytes = chain
        .read_file("World\\Azeroth\\Elwynn\\PassiveDoodads\\Campfire\\ElwynnCampfire.m2")
        .expect("campfire");
    let emitters = crate::parse_m2_particle_emitters(&bytes).expect("emitters");
    for e in &emitters {
        eprintln!(
            "campfire emitter: blend={:?} tex={:?} rate={:?} color0={:?} scale={:?}",
            e.blend,
            e.texture,
            e.timing.constant_rate(),
            e.over_life.color[0],
            e.over_life.scale
        );
    }
    assert_eq!(emitters.len(), 2);
    assert!(emitters
        .iter()
        .all(|e| e.blend == crate::ParticleBlend::Add && e.texture.is_some()));
    assert!(emitters.iter().all(|e| e.timing.peak_rate() > 0.0));
    let kobold =
        crate::parse_m2_particle_emitters(&chain.read_file("Creature\\Kobold\\Kobold.m2").unwrap())
            .expect("kobold");
    assert_eq!(kobold.len(), 1, "the candle");
}

// Wrath UnitHasRelicSlot (0x6113a9) tests Flags bit 0x8, not nonzero Flags.
#[test]
fn installed_wrath_class_flags_family_and_pet_token() {
    let Some(mut chain) = chain() else { return };
    let classes = crate::load_chr_classes(&mut chain).expect("ChrClasses");
    for (id, family, relic) in [
        (1, 4, false),
        (2, 10, true),
        (3, 9, false),
        (4, 8, false),
        (5, 6, false),
        (6, 15, true),
        (7, 11, true),
        (8, 3, false),
        (9, 5, false),
        (11, 7, true),
    ] {
        assert_eq!(classes.spell_family(id), family, "class {id}");
        assert_eq!(classes.has_relic_slot(id), relic, "class {id}");
        assert_eq!(
            classes.pet_name_token(id),
            if id == 9 { "DEMON" } else { "PET" }
        );
    }
    assert_eq!(classes.damage_bonus_stat(3), Some(1));
    assert_eq!(classes.damage_bonus_stat(1), Some(0));
    // Preserve the native enum value, not a guessed Strength fallback.
    assert_eq!(classes.damage_bonus_stat(6), Some(9));
    assert!(!classes.has_relic_slot(0));
}

#[test]
fn installed_wrath_dispel_tokens_follow_native_aura_gate() {
    let Some(mut chain) = chain() else { return };
    let types = crate::load_spell_dispel_types(&mut chain).expect("SpellDispelType");
    for (id, name) in [(1, "Magic"), (2, "Curse"), (3, "Disease"), (4, "Poison")] {
        assert_eq!(types.name(id), Some(name));
    }
    for id in [0, 5, 6, 7, 8, 9, 10, 11] {
        assert_eq!(types.name(id), None, "dispel {id}: no native aura token");
    }
    assert_eq!(types.len(), 4);
}

#[test]
fn installed_wrath_taxi_flights_and_cross_map_transport_nodes() {
    let Some(mut chain) = chain() else { return };
    // Build-12340 TaxiPathNode: ID, path, index, map, XYZ, flags, delay,
    // arrival/departure events. The latter two are not exposed by this reader.
    let nodes = crate::load_taxi_path_nodes(&mut chain).expect("TaxiPathNode");
    let close = |actual: [f32; 3], expected: [f32; 3]| {
        for (a, b) in actual.into_iter().zip(expected) {
            assert!((a - b).abs() < 0.002, "{a} != {b}");
        }
    };
    // TaxiPath 6 joins node 2 (Stormwind) to node 4 (Sentinel Hill),
    // while 7 is the reverse route. IDs are not contiguous along either route.
    for (id, len, first_id, last_id, first, last) in [
        (
            6,
            28,
            35,
            2931,
            [-8851.658, 498.55176, 111.25602],
            [-10629.065, 1039.4456, 37.954334],
        ),
        (
            7,
            27,
            68,
            2935,
            [-10628.539, 1040.069, 35.094147],
            [-8833.164, 479.76254, 112.09562],
        ),
    ] {
        let path = nodes.path(id).expect("ordinary flight");
        assert_eq!(path.len(), len);
        assert_eq!((path[0].id, path[len - 1].id), (first_id, last_id));
        for (i, node) in path.iter().enumerate() {
            assert_eq!(
                (
                    node.path_id,
                    node.node_index,
                    node.map_id,
                    node.flags,
                    node.delay
                ),
                (id, i as u32, 0, 0, 0)
            );
        }
        close(path[0].pos, first);
        close(path[len - 1].pos, last);
    }
    for (id, len, stops) in [
        (302, 22, [(6744, 4, 1), (6758, 18, 0)]),
        (285, 18, [(6266, 3, 0), (6277, 14, 1)]),
    ] {
        let path = nodes.path(id).expect("cross-map transport");
        assert_eq!(path.len(), len);
        for (i, node) in path.iter().enumerate() {
            assert_eq!((node.path_id, node.node_index), (id, i as u32));
        }
        assert_eq!(
            path.iter()
                .filter(|n| n.flags & 2 != 0)
                .map(|n| (n.id, n.node_index, n.map_id, n.flags, n.delay))
                .collect::<Vec<_>>(),
            stops.map(|(row, index, map)| (row, index, map, 2, 60))
        );
    }
    close(
        nodes.path(302).unwrap()[4].pos,
        [1318.1068, -4658.047, 71.86043],
    );
}

#[test]
fn installed_wrath_world_map_area_known_rects_and_continent_order() {
    let Some(mut chain) = chain() else { return };
    // Raw build-12340 rows: first eight columns retain ID/map/area/art/Y1/Y2/X1/X2.
    // DisplayMapID, DefaultDungeonFloor and ParentWorldMapID are not exposed here.
    let areas = crate::load_world_map_area_catalog(&mut chain).expect("WorldMapArea");
    assert_eq!(
        areas
            .iter()
            .filter(|(_, r)| r.area_id == 0)
            .map(|(id, r)| (id, r.map_id, r.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (13, 1, "Kalimdor"),
            (14, 0, "Azeroth"),
            (466, 530, "Expansion01"),
            (485, 571, "Northrend")
        ]
    );
    for (id, map, area, name, rect) in [
        (
            30,
            0,
            12,
            "Elwynn",
            [1535.4166, -1935.4166, -7939.583, -10254.166],
        ),
        (
            301,
            0,
            1519,
            "Stormwind",
            [1722.9166, -14.583333, -7995.833, -9154.166],
        ),
        (
            465,
            530,
            3483,
            "Hellfire",
            [5539.583, 375.0, 1481.25, -1962.4999],
        ),
        (
            486,
            571,
            3537,
            "BoreanTundra",
            [8570.833, 2806.25, 4897.9165, 1054.1666],
        ),
    ] {
        let row = areas.get(id).expect("zone map");
        assert_eq!(
            (row.map_id, row.area_id, row.name.as_str()),
            (map, area, name)
        );
        for (actual, expected) in [row.loc_left, row.loc_right, row.loc_top, row.loc_bottom]
            .into_iter()
            .zip(rect)
        {
            assert!(
                (actual - expected).abs() < 0.002,
                "{name}: {actual} != {expected}"
            );
        }
    }
    for (map, id) in [(0, 14), (1, 13), (530, 466), (571, 485)] {
        assert_eq!(areas.continent(map).unwrap().0, id);
    }
}

#[test]
fn installed_wrath_world_map_continent_sheet_and_taxi_bounds() {
    let Some(mut chain) = chain() else { return };
    // All four native rows: first thirteen columns retain their layout; column 13
    // WorldMapID is not exposed. Do not reuse Vanilla's two-row/0.75-scale expectations.
    let continents =
        crate::load_world_map_continent_catalog(&mut chain).expect("WorldMapContinent");
    assert_eq!(continents.len(), 4);
    for (map, bounds, offsets, scale, taxi_min, taxi_max) in [
        (
            0,
            [26, 44, 8, 60],
            [16.875, -1.5],
            0.7,
            (-16530.0, -16530.0),
            (12270.0, 12270.0),
        ),
        (
            1,
            [16, 46, 9, 52],
            [-20.1875, 3.3125],
            0.7,
            (-11870.0, -13370.0),
            (12470.0, 10970.0),
        ),
        (
            530,
            [23, 47, 15, 61],
            [14.5, -7.0],
            0.75,
            (-5867.0, -1600.0),
            (6400.0, 10670.0),
        ),
        (
            571,
            [16, 45, 12, 33],
            [1.8125, -6.375],
            0.7,
            (-3733.0, -6933.0),
            (11730.0, 8533.0),
        ),
    ] {
        let row = continents.get(map).expect("continent");
        assert_eq!(row.map_id, map);
        assert_eq!(
            [
                row.left_boundary,
                row.right_boundary,
                row.top_boundary,
                row.bottom_boundary
            ],
            bounds
        );
        assert_eq!([row.offset_x, row.offset_y], offsets);
        assert!((row.scale - scale).abs() < 0.000001);
        assert_eq!((row.taxi_min, row.taxi_max), (taxi_min, taxi_max));
    }
}
