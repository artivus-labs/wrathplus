//! Build-12340 layouts for the DBCs whose shape changed beyond wider locale blocks, as maps from
//! the 1.12 schema each loader declares to the 3.3.5 file's columns. A loader keeps its 1.12
//! column numbers; [`crate::dbc::parse`] applies the map when the file is the 3.3.5 shape.
//!
//! Source of the 3.3.5 layouts: AzerothCore's `DBCStructure.h`/`DBCfmt.h`. A 1.12 column the
//! 3.3.5 file no longer carries maps to `None` and reads as 0.

/// The map for `table` (its name, `.dbc` optional) from a `schema_len`-slot 1.12 schema to a
/// `file_fields`-column 3.3.5 file, when one is known.
pub(crate) fn columns(
    table: &str,
    schema_len: usize,
    file_fields: usize,
) -> Option<Vec<Option<usize>>> {
    let table = table.strip_suffix(".dbc").unwrap_or(table);
    let (vanilla, wrath, build): (usize, usize, fn(&mut Map)) = match table {
        // Build-12340 WoWDBDefs: nine icon states precede faction/position; locale blocks widen.
        "AreaPOI" => (29, 54, |m| {
            m.same(0, 3)
                .run(3, 11, 7)
                .loc(10, 18)
                .loc(19, 35)
                .run(28, 52, 1);
        }),
        "CreatureModelData" => (16, 28, |m| {
            m.same(0, 16);
        }),
        // Loyalty was removed in Wrath: absent loyalty-rate slots remain zero, never damage.
        "PetPersonality" => (19, 24, |m| {
            m.same(0, 1).loc(1, 1).run(10, 18, 6);
        }),
        "Spell" => (197, 234, spell),
        "SpellDispelType" => (12, 21, spell_dispel_type),
        "SkillLine" => (32, 56, skill_line),
        "TalentTab" => (16, 24, talent_tab),
        "SpellShapeshiftForm" => (18, 35, spell_shapeshift_form),
        "Faction" => (41, 57, faction),
        "ChrRaces" => (29, 69, chr_races),
        "ChrClasses" => (35, 60, chr_classes),
        "CreatureDisplayInfo" => (12, 16, creature_display_info),
        "CreatureFamily" => (19, 28, creature_family),
        "SpellItemEnchantment" => (24, 38, spell_item_enchantment),
        "AreaTable" => (25, 36, area_table),
        "WorldMapArea" => (8, 11, |m| {
            m.same(0, 8);
        }),
        "WorldMapContinent" => (13, 14, |m| {
            m.same(0, 13);
        }),
        "TaxiPathNode" => (9, 11, |m| {
            m.same(0, 9);
        }),
        // Two pet-talent masks appended.
        "Talent" => (21, 23, |m| {
            m.same(0, 21);
        }),
        // 1.12's trailing skill-up count is gone.
        "SkillLineAbility" => (15, 14, |m| {
            m.same(0, 14);
        }),
        "SpellRange" => (24, 40, spell_range),
        // Native 0x8a93e0 retains raw word order; fallback walker 0x717814 reads +0x14,
        // not +0x18 (the separate animation/variation lookup). Slot 5 is unused by our reader.
        "AnimationData" => (7, 8, |m| {
            m.same(0, 5).run(6, 5, 1);
        }),
        "SoundEntries" => (29, 30, |m| {
            m.same(0, 29);
        }),
        "Material" => (3, 5, |m| {
            m.same(0, 3);
        }),
        "SpellVisual" => (16, 32, |m| {
            // StateDoneKit inserted before ChannelKit; native area kits are NOT the
            // removed Vanilla area predicate/effect-name fields. Leave those absent.
            m.same(0, 5).run(5, 6, 6).run(14, 12, 1);
        }),
        // StartAnim replaces kit type; SpecialEffect expands from one to three,
        // shifting WorldEffect, Sound, Shake and the four parallel CharProc arrays.
        "SpellVisualKit" => (35, 38, |m| {
            m.same(0, 1).same(2, 10).run(12, 14, 23);
        }),
        // Four doodad weights between the doodads and the density.
        "GroundEffectTexture" => (7, 11, |m| {
            m.same(0, 5).run(5, 9, 2);
        }),
        // Build-12340 WoWDBDefs: `PhaseShift` follows `AreaID`, locale blocks widen, and 1.12's
        // unread column after the tooltip is gone.
        "WorldStateUI" => (39, 63, |m| {
            m.same(0, 3)
                .run(3, 4, 1)
                .loc(4, 5)
                .loc(13, 22)
                .run(23, 39, 3)
                .loc(26, 42)
                .run(35, 59, 4);
        }),
        _ => return None,
    };
    if schema_len != vanilla || file_fields != wrath {
        return None;
    }
    let mut map = Map(vec![None; vanilla]);
    build(&mut map);
    Some(map.0)
}

/// 1.12 slot → 3.3.5 column.
struct Map(Vec<Option<usize>>);

impl Map {
    /// `len` consecutive slots from `from` read the file from `to`.
    fn run(&mut self, from: usize, to: usize, len: usize) -> &mut Self {
        for k in 0..len {
            self.0[from + k] = Some(to + k);
        }
        self
    }

    /// Slots `from..from+len` at the same columns.
    fn same(&mut self, from: usize, len: usize) -> &mut Self {
        self.run(from, from, len)
    }

    /// A 1.12 localized string (8 locale slots + flags, at `from`) reads the 3.3.5 block at `to`
    /// (16 slots + flags): its first eight slots and its flags.
    fn loc(&mut self, from: usize, to: usize) -> &mut Self {
        self.run(from, to, 8);
        self.0[from + 8] = Some(to + 16);
        self
    }
}

/// `Spell.dbc` 173 → 234. 1.12's `School` (1) is an index; 3.3.5 keeps a `SchoolMask` (225),
/// which the spell loader converts. `CastUI` (3) and the per-effect dice (67-72) are gone. Slots
/// 173-194 are the loader's 3.3.5-only columns: the family mask's third word, the per-effect
/// class masks, `EffectMiscValueB`, `AttributesEx5`, `AttributesEx7`, `RuneCostID`,
/// `PowerDisplayId`, `AttributesEx6`, `EffectBonusMultiplier`, `SpellDescriptionVariableID` and
/// the two `RequiredTotemCategoryID`s.
fn spell(m: &mut Map) {
    m.run(0, 0, 1)
        .run(1, 225, 1)
        .run(2, 1, 1)
        .run(4, 2, 1)
        .run(5, 3, 1)
        .run(6, 4, 5) // Attributes..AttributesEx4
        .run(11, 12, 1) // Stances
        .run(12, 14, 1) // StancesNot
        .run(13, 16, 3) // Targets, TargetCreatureType, RequiresSpellFocus
        .run(16, 20, 2) // CasterAuraState, TargetAuraState
        .run(18, 28, 43) // CastingTimeIndex..EquippedItemInventoryTypeMask
        .run(61, 71, 6) // Effect[3], EffectDieSides[3]
        .run(73, 77, 36) // EffectRealPointsPerLevel..EffectMiscValue
        .run(109, 116, 6) // EffectTriggerSpell[3], EffectPointsPerComboPoint[3]
        .run(115, 131, 5) // SpellVisual[2], SpellIconID, ActiveIconID, SpellPriority
        .loc(120, 136) // SpellName
        .loc(129, 153) // Rank
        .loc(138, 170) // Description
        .loc(147, 187) // ToolTip
        .run(156, 204, 5) // ManaCostPercentage..SpellFamilyName
        .run(161, 209, 2) // SpellFamilyFlags (low, high)
        .run(163, 212, 10) // MaxAffectedTargets..RequiredAuraVision
        .run(173, 211, 1) // SpellFamilyFlags[2]
        .run(174, 122, 9) // EffectSpellClassMask[3], flag96 each
        .run(183, 113, 3) // EffectMiscValueB[3]
        .run(186, 9, 1) // AttributesEx5
        .run(187, 11, 1) // AttributesEx7
        .run(188, 226, 1) // RuneCostID
        .run(189, 228, 1) // PowerDisplayId
        .run(190, 10, 1) // AttributesEx6
        .run(191, 229, 4) // EffectBonusMultiplier[3], SpellDescriptionVariableID
        .run(195, 222, 2); // RequiredTotemCategoryID[2]
}

/// `SpellDispelType.dbc` 12 → 21: id, name, then ImmunityPossible (the aura name gate), not DispelMask,
/// and the internal name. Native UnitDebuff tests row+0xc at 0x6148b9;
/// reader 0x8b7a90 stores file columns 18/19 at row+8/+0xc.
fn spell_dispel_type(m: &mut Map) {
    m.run(0, 0, 1).loc(1, 1).run(10, 19, 1).run(11, 20, 1);
}

/// `SkillLine.dbc` 32 → 56: id, category, costs, name, description, icon, and the loader's
/// 3.3.5-only `canLink` and `alternateVerb` block (slots 23-31, file 38-54).
fn skill_line(m: &mut Map) {
    m.same(0, 3)
        .loc(3, 3)
        .loc(12, 20)
        .run(21, 37, 1)
        .run(22, 55, 1)
        .loc(23, 38);
}

/// `TalentTab.dbc` 15 → 24: id, name, icon, race mask, class mask, order, background; slot 15,
/// past the 1.12 width, is build 12340's pet talent mask.
fn talent_tab(m: &mut Map) {
    m.run(0, 0, 1)
        .loc(1, 1)
        .run(10, 18, 3)
        .run(13, 22, 2)
        .run(15, 21, 1);
}

/// `SpellShapeshiftForm.dbc` 18 → 35: id, bonus action bar, name, flags, creature type,
/// attack icon; slots 14-17 are the loader's 3.3.5-only `CreatureDisplayID[4]` (23-26).
fn spell_shapeshift_form(m: &mut Map) {
    m.same(0, 2).loc(2, 2).run(11, 19, 3).run(14, 23, 4);
}

/// `Faction.dbc` 41 → 57: the reputation columns are unchanged; the name and description blocks
/// move past the 3.3.5 spillover columns, which the loader reads as its slots 37-40.
fn faction(m: &mut Map) {
    m.same(0, 19).loc(19, 23).loc(28, 40).run(37, 19, 4);
}

/// `ChrRaces.dbc` 29 → 69. 1.12's mount scale (7), login effect (10), combat stun (11) and
/// starting taxi nodes (14) are gone.
fn chr_races(m: &mut Map) {
    m.same(0, 7)
        .run(8, 7, 2) // BaseLanguage, CreatureType
        .run(12, 9, 2) // ResSicknessSpell, SplashSound
        .run(15, 11, 2) // ClientFileString, CinematicSequence
        .loc(17, 14) // Name
        .run(26, 65, 3); // FacialHairCustomization[2], HairCustomization
}

/// `ChrClasses.dbc` 35 → 60: damage bonus stat, display power, pet name token, name, file name,
/// spell family, flags (whose relic bit 1.12 reads), and the female and male name blocks.
fn chr_classes(m: &mut Map) {
    m.run(0, 0, 1)
        .run(2, 1, 3)
        .loc(5, 4)
        .run(14, 55, 3)
        .loc(17, 21)
        .loc(26, 38);
}

/// `CreatureDisplayInfo.dbc` 12 → 16: a portrait texture column precedes size class.
fn creature_display_info(m: &mut Map) {
    m.same(0, 9).run(9, 10, 3);
}

/// `CreatureFamily.dbc` 18 → 28: pet talent type and category precede the name; slot 18, past
/// the 1.12 width, is the pet talent type.
fn creature_family(m: &mut Map) {
    m.same(0, 8).loc(8, 10).run(17, 27, 1).run(18, 8, 1);
}

/// `SpellItemEnchantment.dbc` 24 → 38: a charges column precedes the effects.
fn spell_item_enchantment(m: &mut Map) {
    m.run(0, 0, 1).run(1, 2, 12).loc(13, 14).run(22, 31, 2);
}

/// `SpellRange.dbc` 22 → 40: 3.3.5 splits min and max range into hostile and friendly columns;
/// 1.12's single values read the hostile ones, and slots 22-23 the friendly pair.
fn spell_range(m: &mut Map) {
    m.run(0, 0, 2)
        .run(2, 3, 1)
        .run(3, 5, 1)
        .loc(4, 6)
        .loc(13, 23)
        .run(22, 2, 1)
        .run(23, 4, 1);
}

/// `AreaTable.dbc` 25 → 36: liquid type grew to four columns.
fn area_table(m: &mut Map) {
    m.same(0, 11).loc(11, 11).run(20, 28, 2).run(22, 33, 3);
}

#[cfg(test)]
mod tests {
    use super::columns;

    /// Installed build-12340 data: every mapped table's file has the map's field count, and every
    /// record parses through the map. Run with `WOW_CLIENT_PROFILE=wrath WOW_LOCALE=enUS
    /// WOW_DATA=<Data>`; skipped otherwise.
    #[test]
    fn installed_every_mapped_table_parses_through_its_map() {
        if std::env::var("WOW_CLIENT_PROFILE").as_deref() != Ok("wrath") {
            return;
        }
        let data = std::env::var("WOW_DATA").expect("WOW_DATA");
        let mut chain = crate::Chain::open_wrath(std::path::Path::new(&data), "enUS").unwrap();
        for &(table, v, w) in MAPPED {
            let bytes = chain
                .read_file(&format!("DBFilesClient\\{table}.dbc"))
                .unwrap_or_else(|e| panic!("{table}: {e:#}"));
            let fields = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
            assert_eq!(fields, w, "{table}: installed field count");
            let mut schema = wrathplus_dbc::Schema::new(table);
            for i in 0..v {
                schema.add_field(wrathplus_dbc::SchemaField::new(
                    format!("c{i}"),
                    wrathplus_dbc::FieldType::UInt32,
                ));
            }
            let rs = crate::dbc::parse(&bytes, schema, table)
                .unwrap_or_else(|e| panic!("{table}: {e:#}"));
            let rows = rs.records().len();
            assert!(rows > 0, "{table}: no records");
            eprintln!("MAPPED {table} rows={rows} fields={fields}");
        }
    }

    /// Every mapped table: name, 1.12 schema slots, build-12340 file fields.
    const MAPPED: &[(&str, usize, usize)] = &[
        ("AreaPOI", 29, 54),
        ("CreatureModelData", 16, 28),
        ("PetPersonality", 19, 24),
        ("Spell", 197, 234),
        ("SpellDispelType", 12, 21),
        ("SkillLine", 32, 56),
        ("TalentTab", 16, 24),
        ("SpellShapeshiftForm", 18, 35),
        ("Faction", 41, 57),
        ("ChrRaces", 29, 69),
        ("ChrClasses", 35, 60),
        ("CreatureDisplayInfo", 12, 16),
        ("CreatureFamily", 19, 28),
        ("SpellItemEnchantment", 24, 38),
        ("AreaTable", 25, 36),
        ("WorldMapArea", 8, 11),
        ("WorldMapContinent", 13, 14),
        ("TaxiPathNode", 9, 11),
        ("Talent", 21, 23),
        ("SkillLineAbility", 15, 14),
        ("SpellRange", 24, 40),
        ("AnimationData", 7, 8),
        ("SoundEntries", 29, 30),
        ("Material", 3, 5),
        ("SpellVisual", 16, 32),
        ("GroundEffectTexture", 7, 11),
        ("SpellVisualKit", 35, 38),
    ];

    #[test]
    fn every_map_is_in_range_and_injective() {
        for &(table, v, w) in MAPPED {
            let map = columns(table, v, w).unwrap();
            let mut seen = std::collections::HashSet::new();
            for c in map.iter().flatten() {
                assert!(*c < w, "{table}: column {c} out of range");
                assert!(seen.insert(*c), "{table}: column {c} mapped twice");
            }
        }
        assert!(columns("Spell.dbc", 197, 234).is_some());
        assert!(
            columns("Spell.dbc", 173, 234).is_none(),
            "the 3.3.5-only slots are required"
        );
        assert!(columns("Spell", 190, 190).is_none());
    }

    #[test]
    fn dispel_gate_is_immunity_possible_not_the_combined_mask() {
        let map = columns("SpellDispelType", 12, 21).unwrap();
        assert_eq!(map[10], Some(19));
        assert_eq!(map[11], Some(20));
        assert_eq!(map[1], Some(1));
    }

    #[test]
    fn native_visual_and_kit_beam_columns() {
        let visual = columns("SpellVisual", 16, 32).unwrap();
        assert_eq!(
            &visual[5..11],
            &[Some(6), Some(7), Some(8), Some(9), Some(10), Some(11)]
        );
        assert_eq!(&visual[11..14], &[None, None, None]);
        assert_eq!(visual[14], Some(12));
        let kit = columns("SpellVisualKit", 35, 38).unwrap();
        assert_eq!(kit[1], None);
        assert_eq!(
            &kit[12..19],
            &[
                Some(14),
                Some(15),
                Some(16),
                Some(17),
                Some(18),
                Some(19),
                Some(20)
            ]
        );
        for (field, column) in kit.iter().enumerate().take(35).skip(19) {
            assert_eq!(*column, Some(field + 2));
        }
    }

    #[test]
    fn remaining_catalog_columns_preserve_native_meaning() {
        let poi = columns("AreaPOI", 29, 54).unwrap();
        assert_eq!(poi[2], Some(2)); // default icon, not another captured-state icon
        assert_eq!(poi[3], Some(11)); // faction
        assert_eq!(&poi[4..7], &[Some(12), Some(13), Some(14)]);
        assert_eq!(poi[10], Some(18)); // name enUS
        assert_eq!(poi[18], Some(34)); // name locale flags
        assert_eq!(poi[19], Some(35)); // description enUS
        assert_eq!(poi[28], Some(52)); // world-state
        let pet = columns("PetPersonality", 19, 24).unwrap();
        assert_eq!(pet[9], Some(17)); // widened localized name flags
        assert_eq!(&pet[10..13], &[Some(18), Some(19), Some(20)]);
        assert_eq!(&pet[13..16], &[Some(21), Some(22), Some(23)]);
        assert_eq!(&pet[16..19], &[None, None, None]); // removed loyalty, not a damage alias
    }

    #[test]
    fn spell_columns_land_on_the_build_12340_fields() {
        let m = columns("Spell", 197, 234).unwrap();
        for (vanilla, wrath) in [
            (18, 28),   // CastingTimeIndex
            (36, 46),   // rangeIndex
            (42, 52),   // Reagent[0]
            (82, 86),   // EffectImplicitTargetA[0]
            (109, 116), // EffectTriggerSpell[0]
            (117, 133), // SpellIconID
            (120, 136), // SpellName enUS
            (156, 204), // ManaCostPercentage
            (165, 214), // PreventionType
            (161, 209), // SpellFamilyFlags[0]
            (173, 211), // SpellFamilyFlags[2]
            (174, 122), // EffectSpellClassMask[0][0]
            (182, 130), // EffectSpellClassMask[2][2]
            (183, 113), // EffectMiscValueB[0]
            (186, 9),   // AttributesEx5
            (187, 11),  // AttributesEx7
            (188, 226), // RuneCostID
            (189, 228), // PowerDisplayId
            (185, 115), // EffectMiscValueB[2]
            (191, 229), // EffectBonusMultiplier[0]
            (194, 232), // SpellDescriptionVariableID
            (195, 222), // RequiredTotemCategoryID[0]
        ] {
            assert_eq!(m[vanilla], Some(wrath), "slot {vanilla}");
        }
        assert_eq!(m[67], None);
    }
}
