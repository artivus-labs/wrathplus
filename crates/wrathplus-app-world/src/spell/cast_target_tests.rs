use super::*;

/// Selection and caster candidates, with an empty main hand.
fn cands(selection: Option<u64>, caster: Option<u64>) -> CastCandidates {
    CastCandidates {
        selection,
        caster,
        main_hand_item: None,
    }
}

fn spell(targets: u32, implicit: u32) -> SpellDisplay {
    SpellDisplay {
        targets,
        implicit_target_a1: implicit,
        ..Default::default()
    }
}

#[test]
fn auto_self_cast_boots_at_the_reference_default() {
    assert!(
        AutoSelfCast::default().0,
        "build 12340 registers autoSelfCast \"1\""
    );
}

#[test]
fn mask_follows_the_arm_map() {
    assert_eq!(cast_target_mask(&spell(0, 1)), 0, "Ice Armor: self");
    assert_eq!(
        cast_target_mask(&spell(0, 20)),
        0,
        "Battle Shout: no-op arm"
    );
    assert_eq!(cast_target_mask(&spell(0, 6)), TF_UNIT_ENEMY, "Fireball");
    assert_eq!(
        cast_target_mask(&spell(0, 21)),
        TF_UNIT_ASSIST,
        "Arcane Intellect"
    );
    assert_eq!(
        cast_target_mask(&spell(0x8000, 21)),
        TF_CORPSE_ALLY | TF_UNIT_ASSIST,
        "a Targets seed ORs with the switch"
    );
    assert_eq!(
        cast_target_mask(&spell(0x402, 0)),
        TF_UNIT | TF_EXPLICIT_GATE,
        "Skinning: seed only"
    );
}

/// Both stores are present because `CanAttack` refuses outright when either side is missing.
#[test]
fn resolution_wire_shapes() {
    use wrathplus_protocol::ObjectFields;
    // `UNIT_FIELD_FLAGS` bit 3 (player-controlled) on us alone selects `CanAttack`'s mixed
    // arm; with no catalog the reaction resolves neutral, so the enemy bit clears. Both carry
    // `UNIT_FIELD_HEALTH` (22): a store with none reads dead to the gates.
    let me = crate::app_core::session::ObjectStore(ObjectFields::from_fields(&[
        (wrathplus_protocol::types::Field::UnitHealth, 100),
        (wrathplus_protocol::types::Field::UnitFlags, 1 << 3),
    ]));
    let it = crate::app_core::session::ObjectStore(ObjectFields::from_fields(&[
        (wrathplus_protocol::types::Field::ObjectType, 9),
        (wrathplus_protocol::types::Field::UnitHealth, 100),
        (wrathplus_protocol::types::Field::UnitFactionTemplate, 0),
    ]));
    let rel = TargetRelations {
        target_store: Some(&it),
        target_owner_store: None,
        relations: crate::target::RelationContext::none(),
        self_store: Some(&me),
        factions: None,
        reputations: &Reputations(Vec::new()),
        types: Default::default(),
        group: Default::default(),
    };
    let ice_armor = spell(0, 1);
    assert_eq!(
        resolve_cast_target(Some(&ice_armor), &cands(Some(42), Some(1)), true, &rel),
        CastWireTarget::SelfImplicit,
        "a self spell ignores the selection entirely"
    );
    let fireball = spell(0, 6);
    assert_eq!(
        resolve_cast_target(Some(&fireball), &cands(None, Some(1)), true, &rel),
        CastWireTarget::Refused(ERR_NO_TARGET),
        "enemy words retain the no-target refusal"
    );
    // Neutral (3) is attackable by the mixed arm's `< 4` and not assistable by `>= 4`.
    assert_eq!(
        resolve_cast_target(Some(&fireball), &cands(Some(42), Some(1)), true, &rel),
        CastWireTarget::Unit(42)
    );
    let intellect = spell(0, 21);
    assert!(
        !unit_word_binds(TF_UNIT_ASSIST, false, &rel),
        "the cursor's unit binder must reject the same neutral unit as the initial cast arm"
    );
    assert_eq!(
        resolve_cast_target(Some(&intellect), &cands(Some(42), Some(1)), true, &rel),
        CastWireTarget::Unit(1),
        "a friendly-required cast on a non-friend falls back to self"
    );
    assert_eq!(
        resolve_cast_target(Some(&intellect), &cands(Some(42), Some(1)), false, &rel),
        CastWireTarget::Targeting(TF_UNIT_ASSIST),
        "autoSelfCast off leaves the friendly word for the unit cursor"
    );
    assert_eq!(
        resolve_cast_target(Some(&intellect), &cands(None, Some(1)), true, &rel),
        CastWireTarget::Unit(1),
        "no selection at all still self-falls-back"
    );
    // A hostile-required cast never self-binds.
    assert_eq!(
        resolve_cast_target(Some(&fireball), &cands(None, Some(1)), false, &rel),
        CastWireTarget::Refused(ERR_NO_TARGET)
    );
    // Unknown spell: the selection passes through.
    assert_eq!(
        resolve_cast_target(None, &cands(Some(42), Some(1)), true, &rel),
        CastWireTarget::Unit(42)
    );
}

#[test]
fn assist_word_requires_the_npc_pvp_flag_not_only_a_friendly_reaction() {
    use wrathplus_protocol::ObjectFields;

    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let factions = Factions::from_catalog(
        wrathplus_formats::load_faction_catalog(&mut chain).expect("FactionTemplate.dbc"),
    );
    // Faction templates 35 and 1 are a real friendly pair.
    let me = ObjectStore(ObjectFields::from_fields(&[
        (wrathplus_protocol::types::Field::UnitFactionTemplate, 1),
        (wrathplus_protocol::types::Field::UnitFlags, 0x8),
    ]));
    let quiet = ObjectStore(ObjectFields::from_fields(&[
        (wrathplus_protocol::types::Field::UnitFactionTemplate, 35),
        (wrathplus_protocol::types::Field::UnitFlags, 0),
    ]));
    let pvp = ObjectStore(ObjectFields::from_fields(&[
        (wrathplus_protocol::types::Field::UnitFactionTemplate, 35),
        // `UNIT_FIELD_BYTES_2` byte 1 bit `0x01`, 12340's `UNIT_BYTE2_FLAG_PVP`.
        (wrathplus_protocol::types::Field::UnitBytes2, 0x0000_0100),
    ]));
    let reputations = Reputations(Vec::new());
    let rel = |target| TargetRelations {
        target_store: Some(target),
        target_owner_store: None,
        relations: crate::target::RelationContext::none(),
        self_store: Some(&me),
        factions: Some(&factions),
        reputations: &reputations,
        types: Default::default(),
        group: Default::default(),
    };

    assert!(
        !unit_word_binds(TF_UNIT_ASSIST, false, &rel(&quiet)),
        "a friendly ambient NPC is not assistable"
    );
    assert!(
        unit_word_binds(TF_UNIT_ASSIST, false, &rel(&pvp)),
        "the same friendly NPC becomes assistable with the PvP flag"
    );
}

/// Flamestrike (implicit 16) defers even with a selection, Blizzard (implicit 28) by its
/// bare DEST word; a word of 0 still commits first (`0x6e5338`).
#[test]
fn ground_masks_enter_targeting_mode() {
    let flamestrike = spell(0x40, 16);
    assert_eq!(
        resolve_cast_target(
            Some(&flamestrike),
            &cands(Some(42), Some(1)),
            true,
            &rel_none()
        ),
        CastWireTarget::Targeting(TF_DEST_LOCATION)
    );
    let blizzard = spell(0x40, 28);
    assert_eq!(
        resolve_cast_target(Some(&blizzard), &cands(None, Some(1)), true, &rel_none()),
        CastWireTarget::Targeting(TF_DEST_LOCATION)
    );
    let self_commit_with_ground_arm = spell(0, 16);
    assert_eq!(
        resolve_cast_target(
            Some(&self_commit_with_ground_arm),
            &cands(None, Some(1)),
            true,
            &rel_none()
        ),
        CastWireTarget::SelfImplicit,
        "word==0 commits before the arm-16 defer — the reference's order"
    );
}

/// A bare SOURCE word raises the cursor like DEST (`0x6e6320` tests `& 0x60`); spell 265,
/// `Targets 0x20` with implicit 15, is the shipped case.
#[test]
fn the_source_location_word_enters_targeting_mode() {
    let area_death = spell(0x20, 15);
    assert_eq!(
        resolve_cast_target(
            Some(&area_death),
            &cands(Some(42), Some(1)),
            true,
            &rel_none()
        ),
        CastWireTarget::Targeting(TF_SOURCE_LOCATION)
    );
    assert_eq!(
        resolve_cast_target(Some(&area_death), &cands(None, Some(1)), true, &rel_none()),
        CastWireTarget::Targeting(TF_SOURCE_LOCATION),
        "and with nothing selected — autoSelfCast has no location to give either"
    );
    // Both bits, which `BindLocation` takes over two clicks; no shipped row carries it.
    let both = spell(0x60, 0);
    assert_eq!(
        resolve_cast_target(Some(&both), &cands(None, Some(1)), true, &rel_none()),
        CastWireTarget::Targeting(LOCATION_BITS)
    );
}

/// A pet's location spell raises the cursor as ours would; a unit word stays a pet order.
#[test]
fn a_pet_spell_takes_the_cursor_only_for_a_location() {
    assert_eq!(pet_cursor_word(&spell(0x40, 0)), Some(TF_DEST_LOCATION));
    assert_eq!(pet_cursor_word(&spell(0x20, 15)), Some(TF_SOURCE_LOCATION));
    assert_eq!(pet_cursor_word(&spell(0, 16)), Some(TF_DEST_LOCATION));
    assert_eq!(pet_cursor_word(&spell(u32::from(TF_UNIT), 0)), None);
    assert_eq!(
        pet_cursor_word(&spell(0x42, 0)),
        None,
        "a unit bit beside the location"
    );
    assert_eq!(pet_cursor_word(&spell(0, 0)), None);
}

/// STRING (bit 13) has no seam; `0x42` (spell 28218) would bind its unit first in the
/// reference.
#[test]
fn non_unit_masks_refuse() {
    for targets in [0x2000u32, 0x42] {
        let s = spell(targets, 0);
        assert_eq!(
            resolve_cast_target(Some(&s), &cands(Some(42), Some(1)), true, &rel_none()),
            CastWireTarget::Refused(ERR_INVALID_TARGET),
            "Targets {targets:#x} must stay refused"
        );
    }
}

/// 100 of the 103 LOCKED rows carry implicit arm 23 (word `0x4800`) and one arm 25
/// (`0x4002`): the seam tests are masks, so each raises the cursor with its whole word.
#[test]
fn the_seam_predicates_are_masks_and_the_word_travels() {
    for (targets, implicit) in [
        (0x10u32, 0),
        (0x4000, 0),
        (0x4000, 23),
        (0x4000, 25),
        (0x0, 23),
    ] {
        let s = spell(targets, implicit);
        let word = cast_target_mask(&s);
        assert_eq!(
            resolve_cast_target(Some(&s), &cands(Some(42), Some(1)), true, &rel_none()),
            CastWireTarget::Targeting(word),
            "Targets {targets:#x} + implicit arm {implicit} must raise the cursor with its word"
        );
        assert_eq!(
            resolve_cast_target(Some(&s), &cands(None, Some(1)), false, &rel_none()),
            CastWireTarget::Targeting(word),
            "no selection and no autoSelfCast change nothing — no unit clears these bits"
        );
    }
    // The shipped lock word carries both seams' bits.
    let opening = cast_target_mask(&spell(0x4000, 23));
    assert_eq!(opening & (TF_ITEM | TF_LOCKED), TF_LOCKED);
    assert_eq!(opening & (TF_GAMEOBJECT | TF_LOCKED), opening);
}

/// Rockbiter Weapon 8017 (`Targets 0x10`, `Attributes 0x50200`, implicit 0) binds the main
/// hand whatever the selection; an empty hand refuses; the same word without the attribute
/// raises the item cursor.
#[test]
fn the_imbue_attribute_binds_the_main_hand_without_a_cursor() {
    let rockbiter = SpellDisplay {
        targets: 0x10,
        attributes: 0x0005_0200,
        effects: [wrathplus_formats::SPELL_EFFECT_ENCHANT_ITEM_TEMPORARY, 0, 0],
        ..Default::default()
    };
    assert!(rockbiter.targets_main_hand_item());
    const WEAPON: u64 = 0xdead_beef;
    let armed = CastCandidates {
        selection: Some(42),
        caster: Some(1),
        main_hand_item: Some(WEAPON),
    };
    assert_eq!(
        resolve_cast_target(Some(&rockbiter), &armed, true, &rel_none()),
        CastWireTarget::Item(WEAPON),
        "the imbue binds the equipped weapon on the press"
    );
    assert_eq!(
        resolve_cast_target(
            Some(&rockbiter),
            &CastCandidates {
                selection: None,
                ..armed
            },
            false,
            &rel_none()
        ),
        CastWireTarget::Item(WEAPON),
        "neither the selection nor autoSelfCast has anything to do with it"
    );
    // Unarmed: the cast tail's second test of the bit (`0x6e504b`) refuses, skipping the
    // cursor arm (`0x6e50ab`).
    assert_eq!(
        resolve_cast_target(
            Some(&rockbiter),
            &cands(Some(42), Some(1)),
            true,
            &rel_none()
        ),
        CastWireTarget::RefusedAtArm(ERR_MAINHAND_EMPTY),
        "no main hand — \"Your weapon hand is empty\", never the item cursor"
    );
    // A stone, poison, oil or enchant: the same word without the attribute.
    let stone = SpellDisplay {
        targets: 0x10,
        attributes: 0x0004_0000,
        effects: [wrathplus_formats::SPELL_EFFECT_ENCHANT_ITEM_TEMPORARY, 0, 0],
        ..Default::default()
    };
    assert!(!stone.targets_main_hand_item());
    assert_eq!(
        resolve_cast_target(Some(&stone), &armed, true, &rel_none()),
        CastWireTarget::Targeting(TF_ITEM),
        "without the attribute the cursor comes up even with a weapon equipped"
    );
}

/// A dead unit's bind clears the `0x400` bit whoever it is (`6e5d89`, `6e5ddb`); a living
/// unit's never does, and the gate refuses it before the relation checks are asked.
#[test]
fn a_dead_units_bind_clears_the_0x400_bit() {
    use wrathplus_protocol::ObjectFields;

    let health = |hp| {
        ObjectStore(ObjectFields::from_fields(&[(
            wrathplus_protocol::types::Field::UnitHealth,
            hp,
        )]))
    };
    let (corpse, live) = (health(0), health(100));
    let rel = |target| TargetRelations {
        target_store: Some(target),
        ..rel_none()
    };
    for is_self in [false, true] {
        assert!(
            unit_word_binds(0x402, is_self, &rel(&corpse)),
            "a corpse, self {is_self}"
        );
        assert!(
            !unit_word_binds(0x402, is_self, &rel(&live)),
            "a living unit keeps the bit, self {is_self}"
        );
    }
}

fn rel_none() -> TargetRelations<'static> {
    static EMPTY: Reputations = Reputations(Vec::new());
    TargetRelations {
        target_store: None,
        target_owner_store: None,
        relations: crate::target::RelationContext::none(),
        self_store: None,
        factions: None,
        reputations: &EMPTY,
        types: Default::default(),
        group: Default::default(),
    }
}
