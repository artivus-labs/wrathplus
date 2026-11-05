use super::*;

#[test]
fn wrath_resources_preserve_sparse_updates_zero_and_recreate_without_vanilla_aliases() {
    let mut pairs = vec![(24, 123)];
    for slot in 0..7 {
        pairs.push((25 + slot, 100 + u32::from(slot)));
        pairs.push((33 + slot, 1000 + u32::from(slot)));
    }
    let mut created = ObjectFields::from_pairs(&pairs).into_created(ObjectType::Unit);
    created.merge(ObjectFields::from_pairs(&[(26, 437), (31, 0), (39, 1250)]));
    let result = created.wrath_powers().unwrap();
    assert_eq!(result[0], (100, 1000));
    assert_eq!(result[1], (437, 1001));
    assert_eq!(result[6], (0, 1250));
    assert_eq!(created.unit_health(), Some(123));
    assert_eq!(created.unit_power(1), Some(437));
    created.merge(ObjectFields::from_pairs(&[]).into_created(ObjectType::Unit));
    assert_eq!(created.wrath_powers(), Some([(0, 0); 7]));
    assert!(ObjectFields::default().wrath_powers().is_none());
}

#[test]
fn unit_stats_and_resistances_read_indexed_and_gate_range() {
    let f = ObjectFields::from_pairs(&[
        (84, 25),                   // STAT0 strength
        (88, 31),                   // STAT4 spirit
        (99, 120),                  // RESISTANCES[0] = armor
        (101, 15),                  // fire
        (105, 8u32.wrapping_neg()), // arcane, cursed to −8
    ]);
    assert_eq!(f.unit_stat(0), Some(25));
    assert_eq!(f.unit_stat(4), Some(31));
    assert_eq!(f.unit_stat(1), None, "never streamed");
    assert_eq!(f.unit_stat(5), None, "out of range");
    assert_eq!(f.unit_resistance(0), Some(120));
    assert_eq!(f.unit_resistance(2), Some(15));
    assert_eq!(f.unit_resistance(6), Some(-8), "resistances are signed");
    assert_eq!(f.unit_resistance(7), None, "out of range");
}

#[test]
fn attack_power_mods_split_signed_halves() {
    // vmangos `SetInt16Value(index_mod, 0, pos)` and `(index_mod, 1, neg)`: low positive, high
    // negative. pos 30, neg -10.
    let packed = u32::from(30u16) | (u32::from((-10i16) as u16) << 16);
    let f = ObjectFields::from_pairs(&[
        (123, 78),                              // ATTACK_POWER
        (124, packed),                          // ATTACK_POWER_MODS
        (125, 0.1f32.to_bits()),                // MULTIPLIER (stores multiplier − 1.0)
        (126, 52),                              // RANGED_ATTACK_POWER
        (127, u32::from((-3i16) as u16) << 16), // ranged: pos 0, neg −3
        (128, 0.0f32.to_bits()),
    ]);
    assert_eq!(f.unit_attack_power(), Some(78));
    assert_eq!(f.unit_attack_power_mods(), (30, -10));
    assert_eq!(f.unit_attack_power_multiplier(), Some(0.1));
    assert_eq!(f.unit_ranged_attack_power(), Some(52));
    assert_eq!(f.unit_ranged_attack_power_mods(), (0, -3));
    assert_eq!(f.unit_ranged_attack_power_multiplier(), Some(0.0));
    assert_eq!(ObjectFields::default().unit_attack_power_mods(), (0, 0));
}

#[test]
fn damage_and_attack_time_fields_read_their_slots() {
    let f = ObjectFields::from_pairs(&[
        (62, 2900),               // BASEATTACKTIME main
        (63, 1500),               // BASEATTACKTIME offhand
        (64, 2800),               // RANGEDATTACKTIME
        (70, 12.5f32.to_bits()),  // MINDAMAGE
        (71, 19.5f32.to_bits()),  // MAXDAMAGE
        (72, 5.0f32.to_bits()),   // MINOFFHANDDAMAGE
        (73, 9.0f32.to_bits()),   // MAXOFFHANDDAMAGE
        (129, 31.0f32.to_bits()), // MINRANGEDDAMAGE
        (130, 47.0f32.to_bits()), // MAXRANGEDDAMAGE
    ]);
    assert_eq!(f.unit_base_attack_time(0), Some(2900));
    assert_eq!(f.unit_base_attack_time(1), Some(1500));
    assert_eq!(f.unit_base_attack_time(2), None, "out of range");
    assert_eq!(f.unit_ranged_attack_time(), Some(2800));
    assert_eq!(f.unit_min_damage(), Some(12.5));
    assert_eq!(f.unit_max_damage(), Some(19.5));
    assert_eq!(f.unit_min_offhand_damage(), Some(5.0));
    assert_eq!(f.unit_max_offhand_damage(), Some(9.0));
    assert_eq!(f.unit_min_ranged_damage(), Some(31.0));
    assert_eq!(f.unit_max_ranged_damage(), Some(47.0));
}

/// The buff-split arrays are ints on the wire, narrowed from the server's floats by
/// `BuildValuesUpdate`; `-4` is an x86 server's word, where arm64 would send 0.
#[test]
fn player_stat_buff_arrays_read_signed_ints() {
    let at = |field: Field| field.index();
    let f = ObjectFields::from_pairs(&[
        (at(Field::UnitPosstat0), 10),
        (at(Field::UnitNegstat0), (-4i32) as u32),
        (at(Field::UnitResistancebuffmodspositive), 30),
        (
            at(Field::UnitResistancebuffmodsnegative) + 2,
            (-20i32) as u32,
        ),
    ]);
    assert_eq!(f.player_posstat(0), Some(10));
    assert_eq!(f.player_negstat(0), Some(-4));
    assert_eq!(f.player_posstat(5), None, "out of range");
    assert_eq!(f.player_negstat(5), None, "out of range");
    assert_eq!(f.player_resistance_buff_pos(0), Some(30));
    assert_eq!(f.player_resistance_buff_neg(2), Some(-20));
    assert_eq!(f.player_resistance_buff_pos(7), None, "out of range");
    assert_eq!(f.player_resistance_buff_neg(7), None, "out of range");
    // A live value: 105 read as an f32 bit pattern would be near 0.
    let live = ObjectFields::from_pairs(&[(at(Field::UnitPosstat0), 105)]);
    assert_eq!(live.player_posstat(0), Some(105));
}

#[test]
fn player_mod_damage_done_reads_int_pos_neg_and_float_pct() {
    let f = ObjectFields::from_pairs(&[
        (1171, 25),                   // MOD_DAMAGE_DONE_POS[0] physical
        (1173, 40),                   // MOD_DAMAGE_DONE_POS[2] fire
        (1178, 15u32.wrapping_neg()), // MOD_DAMAGE_DONE_NEG[0] = −15
        (1185, 1.1f32.to_bits()),     // MOD_DAMAGE_DONE_PCT[0], a true float
    ]);
    assert_eq!(f.player_mod_damage_done_pos(0), Some(25));
    assert_eq!(f.player_mod_damage_done_pos(2), Some(40));
    assert_eq!(f.player_mod_damage_done_neg(0), Some(-15));
    assert_eq!(f.player_mod_damage_done_pct(0), Some(1.1));
    assert_eq!(f.player_mod_damage_done_pct(1), None, "never streamed");
    assert_eq!(f.player_mod_damage_done_pos(7), None, "out of range");
    assert_eq!(f.player_mod_damage_done_neg(7), None, "out of range");
    assert_eq!(f.player_mod_damage_done_pct(7), None, "out of range");
}

#[test]
fn player_skill_unpacks_the_three_dword_triplet() {
    // Slot 1 (718 + 3 = 721): Swords (43) step 0, 25/300, temp -3, perm +5; lo|hi pairs,
    // bonuses signed (`Player.cpp:94-100`).
    let f = ObjectFields::from_pairs(&[
        (639, 43),
        (640, 25 | (300 << 16)),
        (641, u32::from((-3i16) as u16) | (5 << 16)),
    ]);
    assert_eq!(
        f.player_skill(1),
        Some(PlayerSkillSlot {
            skill_id: 43,
            step: 0,
            value: 25,
            max: 300,
            temp_bonus: -3,
            perm_bonus: 5,
        })
    );
    assert_eq!(f.player_skill(0), None);
    assert_eq!(
        ObjectFields::from_pairs(&[(636, 0)])
            .player_skill(0)
            .map(|s| s.skill_id),
        Some(0)
    );
    assert_eq!(f.player_skill(PLAYER_SKILL_SLOTS), None);
}

#[test]
fn player_ammo_id_reads_the_field() {
    let f = ObjectFields::from_pairs(&[(1198, 2512)]);
    assert_eq!(f.player_ammo_id(), Some(2512));
    assert_eq!(ObjectFields::default().player_ammo_id(), None);
}

#[test]
fn player_self_res_spell_collapses_absent_and_zero() {
    // 1224 follows AMMO_ID 1223 and FIELD_BYTES 1222, all set so a wrong index reads a
    // neighbour. 3026 is the rank-1 soulstone's effect spell.
    let f = ObjectFields::from_pairs(&[(1197, 0), (1198, 0), (1199, 3026)]);
    assert_eq!(f.player_self_res_spell(), Some(3026));
    assert_eq!(
        f.player_ammo_id(),
        Some(0),
        "the neighbour is not disturbed"
    );
    assert_eq!(ObjectFields::default().player_self_res_spell(), None);
    assert_eq!(
        ObjectFields::from_pairs(&[(1199, 0)]).player_self_res_spell(),
        None
    );
}

#[test]
fn player_field_bytes_splits_into_combo_points_toggles_and_honor_rank() {
    // Bytes 0-3: flags 0x01, byte 1 2, action bars 3, highest rank 5.
    let f = ObjectFields::from_pairs(&[(1197, 0x05_03_02_01)]);
    assert_eq!(f.player_honor_rank(), Some(5));
    assert_eq!(f.player_action_bar_toggles(), Some(3));
    // A full five points with everything else clear.
    let capped = ObjectFields::from_pairs(&[(1197, 0x00_00_05_00)]);
    assert_eq!(capped.player_action_bar_toggles(), Some(0));
    assert_eq!(capped.player_honor_rank(), Some(0));
    // The accessor returns the raw byte, high nibble included; only the binding masks to 4 bits.
    let high = ObjectFields::from_pairs(&[(1197, 0x00_f5_00_00)]);
    assert_eq!(high.player_action_bar_toggles(), Some(0xf5));
    assert_eq!(ObjectFields::default().player_honor_rank(), None);
    assert_eq!(ObjectFields::default().player_action_bar_toggles(), None);
}

/// The nonzero drunk byte catches a wrong shift; the current rank (field 195) and the highest
/// rank (field 1222) are separate fields.
#[test]
fn player_bytes_3_byte_2_is_the_current_pvp_rank_not_the_highest() {
    // byte 0 = 0x01 gender, byte 1 = 0xA0 drunk, byte 2 = 0x0B = internal rank 11 (visual 7).
    let f = ObjectFields::from_pairs(&[(155, 0x00_0B_A0_01)]);
    assert_eq!(f.player_pvp_rank(), Some(11));
    assert_eq!(f.player_drunk_byte(), Some(0xA0), "the neighbouring byte");
    assert_eq!(
        f.player_honor_rank(),
        None,
        "the HIGHEST rank is a different field: absent means absent"
    );
    // A demoted player keeps the higher lifetime rank.
    let demoted = ObjectFields::from_pairs(&[(155, 0x00_03_00_00), (1197, 0x0B_00_00_00)]);
    assert_eq!(demoted.player_pvp_rank(), Some(3));
    assert_eq!(demoted.player_honor_rank(), Some(11));
    assert_eq!(ObjectFields::default().player_pvp_rank(), None);
}

#[test]
fn honor_contributions_standing_and_lifetime_read_their_own_fields() {
    let at = |field: Field| field.index();
    let f = ObjectFields::from_pairs(&[
        (at(Field::PlayerLifetimeHonorableKills), 3_907),
        (at(Field::PlayerYesterdayContribution), 640),
    ]);
    assert_eq!(f.player_lifetime_honorable_kills(), Some(3_907));
    assert_eq!(f.player_yesterday_contribution(), Some(640));
    let empty = ObjectFields::default();
    assert_eq!(empty.player_yesterday_contribution(), None);
    assert_eq!(empty.player_lifetime_honorable_kills(), None);
}

/// A create omits zero fields (vmangos `_SetCreateBits`), so a created store reads absent as 0,
/// like the client's zeroed descriptor; a bare delta reads it as untouched.
#[test]
fn created_store_reads_absent_fields_as_zero_a_delta_as_none() {
    // A broken item's create: MAXDURABILITY present (40), DURABILITY omitted (it is 0).
    let create = ObjectFields::from_pairs(&[(3, 2264), (61, 40)]).into_created(ObjectType::Item);
    assert_eq!(create.item_durability(), Some(0), "created absent = 0");
    assert_eq!(create.item_max_durability(), Some(40));

    // The same mask as a bare delta: absent means "not changed", never 0.
    let delta = ObjectFields::from_pairs(&[(61, 40)]);
    assert_eq!(delta.item_durability(), None, "delta absent = untouched");

    // A guid slot is present only if its low half was sent, even on a created store.
    assert_eq!(create.corpse_owner(), None);
}

#[test]
fn corpse_descriptor_indices_and_packing() {
    // 10 display · 11 item[0] · 30/31 bytes · 33 flags · 34 dynamic flags (AzerothCore
    // `UpdateFields.h`).
    let corpse = ObjectFields::from_pairs(&[
        (10, 49),
        // Slot 4 (chest): DisplayInfoID 902 | InventoryType 5 << 24 (`Player.cpp:4822`).
        (11 + 4, 902 | (5 << 24)),
        // BYTES_1: (0) | race<<8 | gender<<16 | skin<<24 (`Corpse.cpp:228`).
        (30, (6 << 8) | (1 << 16) | (3 << 24)),
        // BYTES_2: face | hairstyle<<8 | haircolor<<16 | facialhair<<24 (`Corpse.cpp:229`).
        (31, 7 | (2 << 8) | (4 << 16) | (5 << 24)),
        (33, 0x04 | 0x08),
        (34, 0x01),
    ])
    .into_created(ObjectType::Corpse);

    assert_eq!(corpse.corpse_display_id(), Some(49));
    assert_eq!(
        corpse.corpse_item(4),
        Some((902, 5)),
        "display | invType<<24"
    );
    assert_eq!(
        corpse.corpse_item(3),
        None,
        "an empty slot is absent, not (0, 0)"
    );

    let look = corpse.corpse_look().expect("both BYTES words present");
    assert_eq!(
        (
            look.race,
            look.sex,
            look.skin,
            look.face,
            look.hair_style,
            look.hair_color,
            look.facial_hair
        ),
        (6, 1, 3, 7, 2, 4, 5),
        "the seven bytes the reference loads at [descr+0x69..+0x6f], in that order"
    );

    // Three independent bits; BONES picks the skeleton model.
    assert!(!corpse.corpse_is_bones(), "0x01 clear");
    assert!(corpse.corpse_hides_helm(), "0x08 set");
    assert!(!corpse.corpse_hides_cloak(), "0x10 clear");
    assert!(
        corpse.corpse_lootable(),
        "DYNAMIC_FLAGS bit 0 — a different field"
    );

    // CORPSE_END is 36, so a created corpse reads 0, not None, for every field below it.
    let bare = ObjectFields::from_pairs(&[(10, 49)]).into_created(ObjectType::Corpse);
    assert_eq!(bare.corpse_flags(), 0);
    assert!(
        bare.corpse_look().is_some(),
        "absent BYTES read as the zero default"
    );
    assert!(!bare.corpse_lootable());
}

/// Absent reads 0 only inside the object's own descriptor: a creature has no player block, so a
/// player-block read off one is `None`.
#[test]
fn a_creature_has_no_player_block_to_be_absent_from() {
    // PLAYER_FIELD_MOD_DAMAGE_DONE_PCT[0], past a unit's descriptor.
    const MOD_DAMAGE_DONE_PCT: u16 = Field::PlayerModDamageDonePct.index();
    // UNIT_FIELD_BASEATTACKTIME, well inside it.
    const BASEATTACKTIME: u16 = Field::UnitBaseattacktime.index();

    let pet = ObjectFields::from_pairs(&[(2, 0x09), (84, 33)]).into_created(ObjectType::Unit);
    assert_eq!(
        pet.player_mod_damage_done_pct(0),
        None,
        "a creature cannot answer a PLAYER-block field"
    );
    assert_eq!(
        pet.unit_base_attack_time(0),
        Some(0),
        "…while a field it DOES have still reads the create's zero"
    );
    assert_eq!(pet.unit_stat(0), Some(33), "and a present field is itself");

    // On a player the same index is inside the descriptor, so absent reads 0.
    let player = ObjectFields::from_pairs(&[(2, 0x19), (BASEATTACKTIME, 1800)])
        .into_created(ObjectType::Player);
    assert_eq!(player.player_mod_damage_done_pct(0), Some(0.0));
    assert_eq!(player.unit_base_attack_time(0), Some(1800));

    // A bare delta reads absent as None either way.
    let delta = ObjectFields::from_pairs(&[(MOD_DAMAGE_DONE_PCT, 1.0f32.to_bits())]);
    assert_eq!(delta.player_mod_damage_done_pct(0), Some(1.0));
    assert_eq!(delta.unit_base_attack_time(0), None);
}

/// A re-create replaces the store, so a field that dropped to 0 out of view does not survive.
#[test]
fn merge_overlays_deltas_and_replaces_on_recreate() {
    let mut store =
        ObjectFields::from_pairs(&[(3, 2264), (60, 40), (61, 40)]).into_created(ObjectType::Item);

    // A durability delta overlays; the store stays created.
    store.merge(ObjectFields::from_pairs(&[(60, 30)]));
    assert_eq!(store.item_durability(), Some(30));
    assert_eq!(
        store.item_max_durability(),
        Some(40),
        "untouched field kept"
    );

    // Re-create after the item broke out of view: the fresh snapshot omits DURABILITY (0).
    store.merge(ObjectFields::from_pairs(&[(3, 2264), (61, 40)]).into_created(ObjectType::Item));
    assert_eq!(
        store.item_durability(),
        Some(0),
        "a re-create replaces — the stale 30 must not survive an overlay"
    );
}

/// Like the reference's per-field notifier, which fires on a memcmp difference with the old
/// value: moved dwords only, ascending, absent as 0, an unchanged resend silent.
#[test]
fn merge_diff_reports_each_moved_dword_once_with_its_old_value() {
    let mut store =
        ObjectFields::from_pairs(&[(24, 100), (32, 100), (54, 9)]).into_created(ObjectType::Unit);
    let mut edges = Vec::new();
    // Health drops, level rises, max-health re-sent as it was, a never-seen field appears.
    store.merge_diff(
        ObjectFields::from_pairs(&[(54, 10), (24, 40), (32, 100), (59, 8)]),
        |i, old, new| edges.push((i, old, new)),
    );
    assert_eq!(
        edges,
        vec![(24, 100, 40), (54, 9, 10), (59, 0, 8)],
        "moved dwords only, ascending; absent-before reads 0; an unchanged resend is silent"
    );
    assert_eq!(store.unit_health(), Some(40), "the delta still lands");
    assert_eq!(store.unit_flags(), 8);
}

/// A re-create refreshes a live guid in place and notifies over both masks: an omitted field is
/// an edge to 0.
#[test]
fn merge_diff_on_a_recreate_reports_the_replace_on_both_masks() {
    let mut store =
        ObjectFields::from_pairs(&[(3, 2264), (46, 30), (47, 40)]).into_created(ObjectType::Item);
    let mut edges = Vec::new();
    store.merge_diff(
        ObjectFields::from_pairs(&[(3, 2264), (47, 40), (12, 7)]).into_created(ObjectType::Item),
        |i, old, new| edges.push((i, old, new)),
    );
    assert_eq!(
        edges,
        vec![(12, 0, 7), (46, 30, 0)],
        "the omitted durability is a 30 → 0 edge; the identical entry and max are silent"
    );
    assert_eq!(store.item_durability(), Some(0));
}

#[test]
fn created_as_reads_back_off_the_created_length() {
    for t in [
        ObjectType::Object,
        ObjectType::Item,
        ObjectType::Container,
        ObjectType::Unit,
        ObjectType::Player,
        ObjectType::GameObject,
        ObjectType::DynamicObject,
        ObjectType::Corpse,
    ] {
        assert_eq!(
            ObjectFields::default().into_created(t).created_as(),
            Some(t)
        );
    }
    assert_eq!(ObjectFields::from_pairs(&[(22, 1)]).created_as(), None);
}

/// A live vmangos capture of a Blizzard (spell 10) dynamic-object create: caster guid 26, area
/// spell, radius 8.0, the cast point in POS, FACING never sent.
#[test]
fn dynamicobject_fields_read_the_live_blizzard_capture() {
    let f = ObjectFields::from_pairs(&[
        (0, 6),                 // OBJECT_FIELD_GUID lo
        (1, 0xf100_0000),       // guid hi, HIGHGUID dynobj
        (2, 0x41),              // TYPE: OBJECT | DYNAMICOBJECT
        (3, 10),                // ENTRY = the spell id
        (4, 1.0f32.to_bits()),  // SCALE_X
        (6, 26),                // DYNAMICOBJECT_CASTER lo
        (8, 1),                 // BYTES: area spell
        (9, 10),                // SPELLID
        (10, 8.0f32.to_bits()), // RADIUS
    ])
    .into_created(ObjectType::DynamicObject);
    assert_eq!(f.dynamicobject_caster(), Some(26));
    assert_eq!(f.dynamicobject_bytes(), Some(1));
    assert_eq!(f.dynamicobject_spell_id(), Some(10));
    assert_eq!(f.dynamicobject_radius(), Some(8.0));
    // Not a dynobj (a corpse never streams SPELLID): the id gate reads None, not Some(0).
    assert_eq!(
        ObjectFields::from_pairs(&[(6, 26)]).dynamicobject_spell_id(),
        None
    );
}

/// `0x605f30`: health 0 (`0x605f3b`), or `PLAYER_FLAGS & 0x10` (`0x605f59`); a released ghost has
/// health 1, so only the flag catches it, and feign death is neither.
#[test]
fn dead_or_ghost_is_health_zero_or_the_ghost_flag() {
    let unit = |pairs: &[(u16, u32)]| ObjectFields::from_pairs(pairs);
    let alive = unit(&[(FIELD_UNIT_HEALTH, 100), (FIELD_UNIT_MAXHEALTH, 100)]);
    let corpse = unit(&[(FIELD_UNIT_HEALTH, 0), (FIELD_UNIT_MAXHEALTH, 100)]);
    let ghost = unit(&[
        (FIELD_UNIT_HEALTH, 1),
        (FIELD_UNIT_MAXHEALTH, 100),
        (FIELD_PLAYER_FLAGS, 0x10),
    ]);
    let feigning = unit(&[
        (FIELD_UNIT_HEALTH, 100),
        (FIELD_UNIT_MAXHEALTH, 100),
        (FIELD_UNIT_DYNAMIC_FLAGS, 0x20),
    ]);
    assert!(!alive.is_dead_or_ghost());
    assert!(corpse.is_dead_or_ghost());
    assert!(!ghost.unit_is_dead(), "a ghost has health");
    assert!(ghost.is_dead_or_ghost(), "the ghost-flag leg");
    assert!(
        !feigning.is_dead_or_ghost(),
        "feign death is 0x605f90's, not this"
    );
}

/// `Unit::SetFeignDeath` sets only `UNIT_DYNFLAG_DEAD`: the raw predicates stay false, the
/// reads-dead ones (`0x605f90`, `UnitHealth`, `UnitMana`) flip, and the maxima stay.
#[test]
fn feign_death_reads_dead_without_touching_the_raw_health_field() {
    const MANA: u8 = 0;
    let unit = |pairs: &[(u16, u32)]| ObjectFields::from_pairs(pairs);
    let alive = unit(&[
        (FIELD_UNIT_HEALTH, 1200),
        (FIELD_UNIT_MAXHEALTH, 1500),
        (FIELD_UNIT_POWER1, 300),
        (FIELD_UNIT_MAXPOWER1, 900),
    ]);
    let feigning = unit(&[
        (FIELD_UNIT_HEALTH, 1200),
        (FIELD_UNIT_MAXHEALTH, 1500),
        (FIELD_UNIT_POWER1, 300),
        (FIELD_UNIT_MAXPOWER1, 900),
        (FIELD_UNIT_DYNAMIC_FLAGS, 0x20),
    ]);
    let corpse = unit(&[(FIELD_UNIT_HEALTH, 0), (FIELD_UNIT_MAXHEALTH, 1500)]);

    // The raw predicate ignores the flag.
    assert!(!alive.unit_is_dead());
    assert!(!feigning.unit_is_dead(), "feign leaves HEALTH alone");
    assert!(corpse.unit_is_dead());
    assert_eq!(feigning.unit_health(), Some(1200), "the raw field survives");

    // The reads-dead predicate flips for both, plus the stand-state leg.
    assert!(!alive.unit_reads_dead());
    assert!(feigning.unit_reads_dead(), "0x605f9d — the dynflag leg");
    assert!(corpse.unit_reads_dead());
    assert!(
        unit(&[
            (FIELD_UNIT_HEALTH, 1200),
            (FIELD_UNIT_MAXHEALTH, 1500),
            (FIELD_UNIT_BYTES_1, 7),
        ])
        .unit_reads_dead(),
        "0x605faa — stand state 7 (inert against vmangos, kept as the reference has it)"
    );

    // Current values go to 0 and the maxima stay, so the bar shows empty.
    assert_eq!(alive.unit_shown_health(), Some(1200));
    assert_eq!(feigning.unit_shown_health(), Some(0));
    assert_eq!(feigning.unit_max_health(), Some(1500));
    assert_eq!(alive.unit_shown_power(MANA), Some(300));
    assert_eq!(feigning.unit_shown_power(MANA), Some(0));
    assert_eq!(feigning.unit_max_power(MANA), Some(900));
    // Out-of-range power slots stay None under the flag, as the ungated reader has them.
    assert_eq!(feigning.unit_shown_power(7), None);
    // An empty store has no flag and no health.
    assert!(!ObjectFields::default().unit_reads_dead());
    assert_eq!(ObjectFields::default().unit_shown_health(), None);
}

/// The server sends rage and runic power max 1000 and happiness max 1050000; the table at
/// `0xaf5220` (`0x7fde00`) divides them to the 100 and 1050 the reference shows.
#[test]
fn rage_and_happiness_divide_for_display_but_not_for_the_raw_readers() {
    const RAGE: u8 = 1;
    const HAPPINESS: u8 = 4;
    const MANA: u8 = 0;
    let warrior = ObjectFields::from_pairs(&[
        (FIELD_UNIT_POWER1 + u16::from(RAGE), 350),
        (FIELD_UNIT_MAXPOWER1 + u16::from(RAGE), 1000),
        (FIELD_UNIT_POWER1 + 6, 250),
    ]);
    assert_eq!(warrior.unit_power(RAGE), Some(350), "the wire value");
    assert_eq!(warrior.unit_shown_power(RAGE), Some(35));
    assert_eq!(warrior.unit_shown_power(6), Some(25), "runic power too");
    assert_eq!(warrior.unit_shown_max_power(RAGE), Some(100), "not 1000");

    let pet = ObjectFields::from_pairs(&[
        (FIELD_UNIT_POWER1 + u16::from(HAPPINESS), 1_020_000),
        (FIELD_UNIT_MAXPOWER1 + u16::from(HAPPINESS), 1_050_000),
    ]);
    assert_eq!(
        pet.unit_power(HAPPINESS),
        Some(1_020_000),
        "raw — the happiness bucket thresholds are on this scale"
    );
    assert_eq!(pet.unit_shown_power(HAPPINESS), Some(1020));
    assert_eq!(pet.unit_shown_max_power(HAPPINESS), Some(1050));

    // Mana, focus and energy divide by 1.
    let caster = ObjectFields::from_pairs(&[
        (FIELD_UNIT_POWER1 + u16::from(MANA), 4200),
        (FIELD_UNIT_MAXPOWER1 + u16::from(MANA), 8000),
    ]);
    assert_eq!(caster.unit_shown_power(MANA), caster.unit_power(MANA));
    assert_eq!(
        caster.unit_shown_max_power(MANA),
        caster.unit_max_power(MANA)
    );
    assert_eq!(caster.unit_shown_power(RAGE), None);
    assert_eq!(caster.unit_shown_max_power(RAGE), None);
    // The dead gate still wins over the divide, and only for the current value.
    let feigning_warrior = ObjectFields::from_pairs(&[
        (FIELD_UNIT_POWER1 + u16::from(RAGE), 350),
        (FIELD_UNIT_MAXPOWER1 + u16::from(RAGE), 1000),
        (FIELD_UNIT_DYNAMIC_FLAGS, 0x20),
    ]);
    assert_eq!(feigning_warrior.unit_shown_power(RAGE), Some(0));
    assert_eq!(feigning_warrior.unit_shown_max_power(RAGE), Some(100));
}

/// `UNIT_FIELD_BASE_MANA` and `UNIT_FIELD_BASE_HEALTH` are fields 162 and 163
/// (`UpdateFields_1_12_1.h:101-102`), the unit block's `+0x270` and `+0x274` `0x612c50` reads.
#[test]
fn base_mana_and_base_health_read_fields_162_and_163() {
    let unit = ObjectFields::from_pairs(&[(120, 1500), (121, 1689)]);
    assert_eq!(unit.unit_base_mana(), Some(1500));
    assert_eq!(unit.unit_base_health(), Some(1689));
}

#[test]
fn native_semantic_reads_keep_sparse_and_create_zero_fill() {
    let health = 24;
    let mut fields = ObjectFields::from_pairs(&[(health, 73)]);
    assert_eq!(fields.get(Field::UnitHealth), Some(73));
    assert_eq!(fields.get(Field::UnitMaxHealth), None);
    assert!(!fields.field_present(Field::UnitMaxHealth, 0));
    fields = fields.into_created(ObjectType::Unit);
    assert_eq!(fields.get(Field::UnitMaxHealth), Some(0));
    fields.merge(ObjectFields::from_pairs(&[(health, 0)]));
    assert_eq!(fields.get(Field::UnitHealth), Some(0));
    assert_eq!(fields.created_as(), Some(ObjectType::Unit));
    let fields = ObjectFields::from_pairs(&[]).into_created(ObjectType::Unit);
    assert_eq!(fields.get_at(Field::UnitPower, 7), None);
}

#[test]
fn semantic_guid_and_float_reads_obey_span_boundaries() {
    let fields = ObjectFields::from_pairs(&[(0, 7), (1, 3), (4, 1.5f32.to_bits())]);
    assert_eq!(
        fields.get_field_guid(Field::ObjectGuid, 0),
        Some((3u64 << 32) | 7)
    );
    assert_eq!(fields.get_field_guid(Field::ObjectGuid, 1), None);
    assert_eq!(fields.get_float(Field::ObjectScaleX, 0), Some(1.5));
    assert_eq!(fields.get_float(Field::ObjectScaleX, 1), None);
}

#[test]
fn unit_accessors_read_native_wrath_without_vanilla_aliases() {
    let fields = ObjectFields::from_pairs(&[
        (22, 999), // Wrath channel object: not vanilla health.
        (24, 81),
        (25, 14),
        (32, 100),
        (33, 70),
        (54, 22),
        (64, 1.25f32.to_bits()),
        (65, 2.5f32.to_bits()),
        (67, 1234),
    ])
    .into_created(ObjectType::Unit);
    assert_eq!(fields.unit_health(), Some(81));
    assert_eq!(fields.unit_max_health(), Some(100));
    assert_eq!(fields.unit_power(0), Some(14));
    assert_eq!(fields.unit_max_power(0), Some(70));
    assert_eq!(fields.unit_power(7), None);
    assert_eq!(fields.unit_level(), Some(22));
    assert_eq!(fields.unit_displayid(), Some(1234));
}

#[test]
fn semantic_guids_preserve_low_half_presence_and_optional_high_half() {
    let owner = Field::UnitSummonedby.span().unwrap().index;
    let fields = ObjectFields::from_pairs(&[(owner, 42)]);
    assert_eq!(fields.unit_summoned_by(), Some(42));
    assert_eq!(fields.get_field_guid(Field::UnitSummonedby, 1), None);
    let high_only = ObjectFields::from_pairs(&[(owner + 1, 3)]).into_created(ObjectType::Unit);
    assert_eq!(high_only.unit_summoned_by(), None);
    assert_eq!(high_only.get_field_guid(Field::UnitSummonedby, 0), None);
    let fields = fields.into_created(ObjectType::Unit);
    assert_eq!(fields.unit_summoned_by(), Some(42));
    assert_eq!(fields.unit_charmed_by(), None);
}

#[test]
fn native_item_container_and_corpse_readers_use_their_layout() {
    let item = ObjectFields::from_pairs(&[
        (14, 3),
        (16, (-2i32) as u32),
        (55, 789),
        (56, 800),
        (60, 17),
        (61, 20),
    ])
    .into_created(ObjectType::Item);
    assert_eq!(item.item_stack_count(), Some(3));
    assert_eq!(item.item_spell_charges(0), Some(-2));
    assert_eq!(item.item_enchant(11), Some(789));
    assert_eq!(item.item_enchant(12), None);
    assert_eq!(item.item_durability(), Some(17));
    assert_eq!(item.item_max_durability(), Some(20));
    let bag = ObjectFields::from_pairs(&[(64, 36), (66, 123)]);
    assert_eq!(bag.container_num_slots(), Some(36));
    assert_eq!(bag.container_slot(0), Some(123));
    let corpse = ObjectFields::from_pairs(&[
        (6, 123),
        (10, 456),
        (11, 99 | (4 << 24)),
        (30, 1 << 8),
        (33, 8),
    ]);
    assert_eq!(corpse.corpse_owner(), Some(123));
    assert_eq!(corpse.corpse_display_id(), Some(456));
    assert_eq!(corpse.corpse_item(0), Some((99, 4)));
    assert_eq!(corpse.corpse_item(19), None);
    assert_eq!(corpse.corpse_race(), 1);
    assert!(corpse.corpse_hides_helm());
}

#[test]
fn native_wrath_quest_log_preserves_last_slot_and_u16_counters() {
    // AzerothCore UpdateFields.h: PLAYER_QUEST_LOG_1 starts at158, five dwords per slot.
    let fields = ObjectFields::from_pairs(&[
        (278, 987),
        (279, 1),
        (280, 300 | (65535 << 16)),
        (281, 64 | (1024 << 16)),
        (282, 12345),
    ]);
    assert_eq!(fields.player_quest_log_slots(), 25);
    let quest = fields.player_quest_log(24).unwrap();
    assert_eq!(quest.quest_id, 987);
    assert_eq!(quest.counters, [300, 65535, 64, 1024]);
    assert_eq!(quest.state, quest_slot_state::COMPLETE);
    assert_eq!(quest.timer, 12345);
    assert_eq!(fields.player_quest_log(25), None);
    assert_eq!(fields.player_quest_log(0), None);
    let sparse = ObjectFields::from_pairs(&[(158, 42)]);
    assert_eq!(sparse.player_quest_log(0).unwrap().counters, [0; 4]);
}

#[test]
fn player_public_and_private_bytes_do_not_collide() {
    let (public, private, private2) = (153, 1197, 1229);
    assert_eq!(Field::PlayerBytes.span().unwrap().index, public);
    assert_eq!(Field::PlayerFieldBytes.span().unwrap().index, private);
    assert_eq!(Field::PlayerFieldBytes2.span().unwrap().index, private2);
    let fields =
        ObjectFields::from_pairs(&[(public, 0x04030201), (private, 0x09080706), (private2, 42)]);
    assert_eq!(fields.player_bytes(), Some(0x04030201));
    assert_eq!(fields.get(Field::PlayerFieldBytes), Some(0x09080706));
    assert_eq!(fields.get(Field::PlayerFieldBytes2), Some(42));
    assert_eq!(
        Field::at(ObjectType::Player, public),
        Some((Field::PlayerBytes, 0))
    );
    assert_eq!(
        Field::at(ObjectType::Player, private),
        Some((Field::PlayerFieldBytes, 0))
    );
}

#[test]
fn native_power_arrays_preserve_all_slots() {
    let slots = 7;
    let current = Field::UnitPower.span().unwrap();
    let maximum = Field::UnitMaxPower.span().unwrap();
    let mut pairs = Vec::new();
    for slot in 0..slots {
        pairs.push((current.index + slot, 100 + u32::from(slot)));
        pairs.push((maximum.index + slot, 1000 + u32::from(slot)));
    }
    let mut fields = ObjectFields::from_pairs(&pairs);
    assert_eq!(fields.unit_power_slots(), slots as u8);
    for slot in 0..slots as u8 {
        assert_eq!(fields.unit_power(slot), Some(100 + u32::from(slot)));
        assert_eq!(fields.unit_max_power(slot), Some(1000 + u32::from(slot)));
    }
    assert_eq!(fields.unit_power(slots as u8), None);
    assert_eq!(fields.unit_max_power(slots as u8), None);
    if true {
        assert_eq!(fields.wrath_powers().unwrap()[6], (106, 1006));
        fields.merge(ObjectFields::from_pairs(&[(current.index + 6, 0)]));
        assert_eq!(fields.wrath_powers().unwrap()[6], (0, 1006));
        assert_eq!(fields.wrath_powers().unwrap()[0], (100, 1000));
    }
    let flags = Field::UnitDynamicFlags.span().unwrap();
    fields.merge(ObjectFields::from_pairs(&[(
        flags.index,
        unit::UNIT_DYNFLAG_DEAD,
    )]));
    assert_eq!(fields.unit_shown_power((slots - 1) as u8), Some(0));
    assert_eq!(fields.unit_shown_power(slots as u8), None);
    assert!(
        ObjectFields::from_pairs(&[(Field::UnitPower.span().unwrap().index, 1)])
            .wrath_powers()
            .is_none()
    );
}
