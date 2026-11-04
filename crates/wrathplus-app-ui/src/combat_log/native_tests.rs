use super::*;
use wrathplus_protocol::wrath_combat::{AttackerState, MeleeDamage};

fn recorded(ev: SessionEvent) -> Vec<Pending> {
    let mut log = NativeCombatLog::default();
    log.record(&ev);
    log.0
}

fn swing(hit_info: u32, damage_total: u32, target_state: u8) -> SessionEvent {
    SessionEvent::WrathAttackerState(Box::new(AttackerState {
        hit_info,
        attacker: 1,
        victim: 2,
        damage: damage_total,
        overkill: 3,
        damages: vec![
            MeleeDamage {
                school_mask: 1,
                damage_float: damage_total as f32,
                damage: damage_total,
                absorb: Some(4),
                resist: Some(0),
            },
            MeleeDamage {
                school_mask: 4,
                damage_float: 0.0,
                damage: 0,
                absorb: Some(1),
                resist: Some(2),
            },
        ],
        target_state,
        attacker_state: 0,
        melee_spell_id: 0,
        blocked: Some(0),
        rage_gain: None,
    }))
}

/// `0x7512b0`: a landed swing sums the school rows' absorbs and resists into group `0x10`, the
/// hit bits giving critical, glancing and crushing; a zero swing is a typed miss.
#[test]
fn a_swing_lands_as_swing_damage_or_misses_by_victim_state() {
    let hit = recorded(swing(0x200 | 0x2_0000, 50, 1));
    assert_eq!(hit.len(), 1);
    assert_eq!(
        (hit[0].event, hit[0].source, hit[0].dest),
        (event::SWING_DAMAGE, 1, 2)
    );
    assert_eq!(
        hit[0].suffix,
        [
            V::Int(50),
            V::Int(3),
            V::Int(1),
            V::Int(2),
            V::Nil,
            V::Int(5),
            V::Int(1),
            V::Nil,
            V::Int(1),
        ]
    );
    let dodge = recorded(swing(0, 0, 2));
    assert_eq!(dodge[0].event, event::SWING_MISSED);
    assert_eq!(dodge[0].suffix, [V::Str("DODGE".into())]);
    let absorbed = recorded(swing(0x20, 0, 1));
    assert_eq!(absorbed[0].suffix, [V::Str("ABSORB".into()), V::Int(5)]);
}

/// Heals keep overheal and absorb (group `0x20`); periodic ticks split by kind; misses take the
/// `SpellMissInfo` name; environmental damage leads with its type.
#[test]
fn spell_entries_carry_the_native_suffix_groups() {
    let heal = recorded(SessionEvent::SpellHealLog(SpellHealLog {
        target: 2,
        healer: 1,
        spell_id: 2050,
        amount: 300,
        critical: true,
        overheal: 120,
        absorb: 0,
    }));
    assert_eq!(
        (heal[0].event, heal[0].spell),
        (event::SPELL_HEAL, Some(2050))
    );
    assert_eq!(
        heal[0].suffix,
        [V::Int(300), V::Int(120), V::Int(0), V::Int(1)]
    );

    let ticks = recorded(SessionEvent::PeriodicAuraLog(PeriodicAuraLog {
        target: 2,
        caster: 1,
        spell_id: 589,
        ticks: vec![
            PeriodicTick::Damage {
                amount: 40,
                school: 5,
                absorb: 0,
                resist: 0,
                overkill: 0,
                critical: false,
            },
            PeriodicTick::Energize {
                power: 0,
                amount: 25,
            },
        ],
    }));
    assert_eq!(
        ticks.iter().map(|p| p.event).collect::<Vec<_>>(),
        [event::SPELL_PERIODIC_DAMAGE, event::SPELL_PERIODIC_ENERGIZE]
    );
    assert_eq!(
        ticks[0].suffix[2],
        V::Int(1 << 5),
        "school index 5 is mask 0x20"
    );

    let misses = recorded(SessionEvent::SpellLogMiss(SpellLogMiss {
        spell_id: 133,
        caster: 1,
        misses: vec![(2, 2), (3, 11)],
    }));
    assert_eq!(
        misses
            .iter()
            .map(|p| (p.dest, p.suffix.clone()))
            .collect::<Vec<_>>(),
        [
            (2, vec![V::Str("RESIST".into())]),
            (3, vec![V::Str("REFLECT".into())])
        ]
    );

    let fall = recorded(SessionEvent::WrathEnvironmentalDamage(
        wrathplus_protocol::wrath_environment::EnvironmentalDamage {
            victim: 2,
            damage_type: 2,
            damage: 90,
            resisted: 0,
            absorbed: 0,
        },
    ));
    assert_eq!(
        (fall[0].event, fall[0].source),
        (event::ENVIRONMENTAL_DAMAGE, 0)
    );
    assert_eq!(fall[0].suffix[0], V::Str("FALLING".into()));
    assert!(recorded(SessionEvent::CancelAutoRepeat).is_empty());
}

fn aura(slot: u8, spell_id: u32, stacks: u8, caster: Option<u64>, helpful: bool) -> NativeAura {
    NativeAura {
        slot,
        spell_id,
        flags: if helpful { 0x10 } else { 0x80 },
        level: 80,
        stacks,
        caster_guid: caster,
        max_duration_ms: None,
        remaining_ms: None,
    }
}

fn snapshot(guid: u64, auras: Vec<NativeAura>, updated: &[u8]) -> SessionEvent {
    SessionEvent::UnitAuras {
        guid,
        auras,
        updated_slots: updated.to_vec(),
    }
}

/// `0x751920`/`0x7519e0`/`0x751ad0`: a start has no destination, a launch its unit target; our
/// refused cast carries its `SPELL_FAILED_*` key for the feed to resolve; success is no failure.
#[test]
fn casts_log_start_success_and_our_failures() {
    let start = recorded(SessionEvent::SpellStart {
        caster: 1,
        spell_id: 133,
        cast_flags: 0,
        cast_time_ms: 1500,
        target: Some(2),
        ammo_display_id: None,
        cast_count: Some(1),
    });
    assert_eq!(
        (start[0].event, start[0].source, start[0].dest),
        (event::SPELL_CAST_START, 1, 0)
    );
    let go = recorded(SessionEvent::SpellGo {
        caster: 1,
        spell_id: 133,
        cast_flags: 0,
        hits: vec![2],
        misses: vec![],
        target: Some(2),
        go_target: None,
        dest: None,
        ammo_display_id: None,
        item_caster: None,
        cast_count: Some(1),
    });
    assert_eq!((go[0].event, go[0].dest), (event::SPELL_CAST_SUCCESS, 2));
    let failure = |reason| {
        recorded(SessionEvent::WrathSpellFailure(
            wrathplus_protocol::wrath_spell_failure::SpellFailure {
                caster: 1,
                cast_count: 1,
                spell_id: 133,
                reason,
            },
        ))
    };
    let failed = failure(0x43);
    assert_eq!(
        (failed[0].event, failed[0].fill, failed[0].suffix.clone()),
        (
            event::SPELL_CAST_FAILED,
            Fill::FailedKey,
            vec![V::Str("SPELL_FAILED_NOT_READY".into())]
        )
    );
    assert!(failure(0).is_empty());
}

/// The snapshot diff: a unit's first snapshot logs nothing; then a new slot applies, a stack
/// change doses, a re-sent unchanged slot refreshes, a vacated slot (even one the packet does not
/// name) is removed and a swapped spell is removed then applied.
#[test]
fn aura_snapshots_diff_into_applied_dose_refresh_and_removed() {
    let mut log = NativeCombatLog::default();
    log.record(&snapshot(
        9,
        vec![aura(0, 100, 1, Some(1), true), aura(1, 200, 0, None, false)],
        &[0, 1],
    ));
    assert!(log.0.is_empty(), "first sight");
    log.record(&snapshot(
        9,
        vec![
            aura(0, 100, 3, Some(1), true),
            aura(2, 300, 0, Some(4), false),
            aura(3, 400, 0, None, true),
        ],
        &[0, 2, 3],
    ));
    log.record(&snapshot(
        9,
        vec![
            aura(0, 100, 3, Some(1), true),
            aura(2, 500, 0, Some(4), false),
            aura(3, 400, 0, None, true),
        ],
        &[0, 2],
    ));
    let got: Vec<_> = log
        .0
        .iter()
        .map(|p| (p.event, p.source, p.spell, p.suffix.clone(), p.aura))
        .collect();
    let buff = || V::Str("BUFF".into());
    let debuff = || V::Str("DEBUFF".into());
    assert_eq!(
        got,
        [
            (
                event::SPELL_AURA_APPLIED_DOSE,
                1,
                Some(100),
                vec![buff(), V::Int(3)],
                Some(AuraGate::Dose)
            ),
            (
                event::SPELL_AURA_REMOVED,
                9,
                Some(200),
                vec![debuff()],
                Some(AuraGate::Removed)
            ),
            (
                event::SPELL_AURA_APPLIED,
                4,
                Some(300),
                vec![debuff()],
                Some(AuraGate::Shown)
            ),
            (
                event::SPELL_AURA_APPLIED,
                9,
                Some(400),
                vec![buff()],
                Some(AuraGate::Shown)
            ),
            (
                event::SPELL_AURA_REFRESH,
                1,
                Some(100),
                vec![buff()],
                Some(AuraGate::Shown)
            ),
            (
                event::SPELL_AURA_REMOVED,
                4,
                Some(300),
                vec![debuff()],
                Some(AuraGate::Removed)
            ),
            (
                event::SPELL_AURA_APPLIED,
                4,
                Some(500),
                vec![debuff()],
                Some(AuraGate::Shown)
            ),
        ]
    );
    // A unit leaving view forgets its snapshot: its next one is a first sight again.
    log.0.clear();
    log.record(&SessionEvent::ObjectsRemoved(vec![9]));
    log.record(&snapshot(9, vec![], &[]));
    assert!(log.0.is_empty());
}

/// Execute rows by effect; the dispel byte is the aura type, and a steal logs `SPELL_STOLEN`.
#[test]
fn execute_rows_and_dispels_take_their_native_events() {
    const TOTEM: u64 = 0xf130_0000_0000_0005;
    const FLAG: u64 = 0xf110_0000_0000_0006;
    let rows = recorded(SessionEvent::SpellLogExecute(SpellLogExecute {
        caster: 1,
        spell_id: 77,
        effects: vec![
            (
                68,
                vec![ExecuteLog::InterruptCast {
                    target: 2,
                    spell_id: 116,
                }],
            ),
            (
                19,
                vec![ExecuteLog::ExtraAttacks {
                    target: 1,
                    count: 2,
                }],
            ),
            (
                111,
                vec![
                    ExecuteLog::DurabilityDamage {
                        target: 2,
                        item_entry: -1,
                        slot: -1,
                    },
                    ExecuteLog::DurabilityDamage {
                        target: 2,
                        item_entry: 4500,
                        slot: 3,
                    },
                ],
            ),
            (28, vec![ExecuteLog::Target { target: TOTEM }]),
            (83, vec![ExecuteLog::Target { target: FLAG }]),
            (113, vec![ExecuteLog::Target { target: 2 }]),
            (24, vec![ExecuteLog::CreateItem { item_entry: 9 }]),
        ],
    }));
    assert_eq!(
        rows.iter()
            .map(|p| (p.event, p.dest, p.fill, p.suffix.clone()))
            .collect::<Vec<_>>(),
        [
            (
                event::SPELL_INTERRUPT,
                2,
                Fill::ExtraSpell,
                vec![V::Int(116)]
            ),
            (event::SPELL_EXTRA_ATTACKS, 1, Fill::None, vec![V::Int(2)]),
            (event::SPELL_DURABILITY_DAMAGE_ALL, 2, Fill::None, vec![]),
            (
                event::SPELL_DURABILITY_DAMAGE,
                2,
                Fill::Item,
                vec![V::Int(4500)]
            ),
            (event::SPELL_SUMMON, TOTEM, Fill::None, vec![]),
            (event::SPELL_CREATE, FLAG, Fill::None, vec![]),
            (event::SPELL_RESURRECT, 2, Fill::None, vec![]),
        ]
    );
    let log = SpellDispelLog {
        victim: 2,
        caster: 1,
        spell_id: 527,
        flag: 0,
        auras: vec![
            wrathplus_protocol::types::DispelledAura {
                spell_id: 589,
                flag: 1,
            },
            wrathplus_protocol::types::DispelledAura {
                spell_id: 17,
                flag: 0,
            },
        ],
    };
    let dispels = recorded(SessionEvent::SpellDispelLog(log.clone()));
    assert_eq!(
        dispels.iter().map(|p| p.suffix.clone()).collect::<Vec<_>>(),
        [
            vec![V::Int(589), V::Str("DEBUFF".into())],
            vec![V::Int(17), V::Str("BUFF".into())]
        ]
    );
    let stolen = recorded(SessionEvent::WrathSpellLog {
        packet: wrathplus_protocol::wrath_logs::Packet::SpellSteal(log),
    });
    assert!(stolen
        .iter()
        .all(|p| p.event == event::SPELL_STOLEN && p.fill == Fill::ExtraSpell));
}

/// `UNIT_DIED`: health falling to 0 after a living reading; a unit first seen dead logs nothing.
#[test]
fn a_unit_dies_when_its_health_falls_to_zero() {
    use wrathplus_protocol::types::{Field, ObjectFields};
    let unit = |health: u32| {
        crate::app_core::session::ObjectStore(ObjectFields::from_field_slots(&[
            (Field::ObjectType, 0, 9),
            (Field::UnitHealth, 0, health),
        ]))
    };
    let (alive, dead) = (unit(40), unit(0));
    let mut health = HashMap::new();
    let mut log = NativeCombatLog::default();
    deaths([(5, &dead), (6, &alive)], &mut health, &mut log, |_| {
        event::UNIT_DIED
    });
    assert!(log.0.is_empty());
    deaths([(6, &dead), (5, &dead)], &mut health, &mut log, |_| {
        event::UNIT_DIED
    });
    assert_eq!(
        log.0
            .iter()
            .map(|p| (p.event, p.source, p.dest))
            .collect::<Vec<_>>(),
        [(event::UNIT_DIED, 0, 6)]
    );
}

/// `0x752ed0`'s split: a gas cloud dissipates, a totem-slot summon is destroyed, the rest die; the
/// event `deaths` logs is the one its picker returns.
#[test]
fn a_gas_cloud_dissipates_and_a_totem_is_destroyed() {
    assert_eq!(death_event(Some(13), Some(1)), event::UNIT_DISSIPATES);
    assert_eq!(death_event(Some(4), Some(1)), event::UNIT_DESTROYED);
    assert_eq!(death_event(None, Some(4)), event::UNIT_DESTROYED);
    assert_eq!(
        death_event(Some(7), Some(5)),
        event::UNIT_DIED,
        "a minipet's slot"
    );
    assert_eq!(death_event(Some(7), Some(0)), event::UNIT_DIED);
    assert_eq!(death_event(None, None), event::UNIT_DIED);

    use wrathplus_protocol::types::{Field, ObjectFields};
    let unit = |health: u32| {
        crate::app_core::session::ObjectStore(ObjectFields::from_field_slots(&[
            (Field::ObjectType, 0, 9),
            (Field::UnitHealth, 0, health),
        ]))
    };
    let (alive, dead) = (unit(40), unit(0));
    let mut health = HashMap::new();
    let mut log = NativeCombatLog::default();
    let totem = |_: &crate::app_core::session::ObjectStore| event::UNIT_DESTROYED;
    deaths([(7, &alive)], &mut health, &mut log, totem);
    deaths([(7, &dead)], &mut health, &mut log, totem);
    assert_eq!(
        log.0
            .iter()
            .map(|p| (p.event, p.source, p.dest))
            .collect::<Vec<_>>(),
        [(event::UNIT_DESTROYED, 0, 7)]
    );
}

/// `0x750d40`, `0x751050` and `0x752de0`: a resisted proc, an enchant landing and fading, and a
/// split damage share.
#[test]
fn proc_resists_enchants_and_split_damage_take_their_native_events() {
    let resist = recorded(SessionEvent::ProcResist(SpellOutcomeLog {
        caster: 1,
        target: 2,
        spell_id: 20_271,
        log_format: 0,
    }));
    assert_eq!(
        resist
            .iter()
            .map(|p| (p.event, p.source, p.dest, p.spell, p.suffix.clone()))
            .collect::<Vec<_>>(),
        [(
            event::SPELL_MISSED,
            1,
            2,
            Some(20_271),
            vec![V::Str("RESIST".into()), V::Int(0)]
        )]
    );

    let enchant = |caster| {
        recorded(SessionEvent::EnchantmentLog(EnchantmentLog {
            target: 2,
            caster,
            item_entry: 19_019,
            enchant_id: 1900,
        }))
    };
    for (caster, event) in [(1, event::ENCHANT_APPLIED), (0, event::ENCHANT_REMOVED)] {
        let entries = enchant(caster);
        assert_eq!(
            entries
                .iter()
                .map(|p| (p.event, p.source, p.dest, p.spell, p.fill))
                .collect::<Vec<_>>(),
            [(event, caster, 2, None, Fill::Enchant)]
        );
        assert_eq!(entries[0].suffix, [V::Int(1900), V::Int(19_019)]);
    }

    let catalog = crate::items::Enchants(wrathplus_formats::EnchantCatalog::from_rows(
        HashMap::new(),
        HashMap::from([
            (1900, "Crusader".to_string()),
            (7, "Flametongue".to_string()),
        ]),
        HashMap::from([(1900, 0), (7, 0x2), (8, 0)]),
    ));
    assert_eq!(
        enchant_name(Some(&catalog), 1900),
        Some(V::Str("Crusader".into()))
    );
    assert_eq!(
        enchant_name(Some(&catalog), 7),
        None,
        "Flags & 0x2 logs nothing"
    );
    assert_eq!(
        enchant_name(Some(&catalog), 8),
        Some(V::Nil),
        "an empty name"
    );
    assert_eq!(enchant_name(Some(&catalog), 9), None, "no row");
    assert_eq!(enchant_name(None, 9), Some(V::Nil));

    let damage = |hit_info, periodic| {
        recorded(SessionEvent::SpellDamageLog(SpellDamageLog {
            target: 2,
            attacker: 1,
            spell_id: 6229,
            damage: 120,
            school: 5,
            absorb: 0,
            resist: 0,
            periodic,
            blocked: 0,
            hit_info,
            overkill: 0,
        }))
    };
    let split = damage(SPELL_HIT_SPLIT | SPELL_HIT_CRIT, true);
    assert_eq!(
        (split[0].event, split[0].source, split[0].dest),
        (event::DAMAGE_SPLIT, 1, 2)
    );
    assert_eq!(split[0].suffix[0], V::Int(120));
    assert_eq!(split[0].suffix[6], V::Int(1), "critical");
    assert_eq!(damage(0, false)[0].event, event::SPELL_DAMAGE);
    assert_eq!(damage(0, true)[0].event, event::SPELL_PERIODIC_DAMAGE);
}

/// `0x751b80` through its two packet callers: a spell-log miss row is `DAMAGE_SHIELD_MISSED`
/// (`0x6d3090`'s constant flag), a zero miss type logs nothing, and an immune target is
/// `DAMAGE_SHIELD_MISSED` `IMMUNE` for AzerothCore's zero log format (`0x6ce3d0`).
#[test]
fn spell_log_misses_and_immunities_are_damage_shield_misses() {
    let misses = recorded(SessionEvent::SpellLogMiss(SpellLogMiss {
        spell_id: 26_992,
        caster: 1,
        misses: vec![(2, 2), (3, 0)],
    }));
    assert_eq!(
        misses
            .iter()
            .map(|p| (p.event, p.source, p.dest, p.spell))
            .collect::<Vec<_>>(),
        [(event::DAMAGE_SHIELD_MISSED, 1, 2, Some(26_992))]
    );

    let immune = |log_format| {
        recorded(SessionEvent::SpellOrDamageImmune(SpellOutcomeLog {
            caster: 1,
            target: 2,
            spell_id: 26_992,
            log_format,
        }))
    };
    for (log_format, event) in [
        (0, event::DAMAGE_SHIELD_MISSED),
        (1, event::SPELL_PERIODIC_MISSED),
    ] {
        assert_eq!(
            immune(log_format)
                .iter()
                .map(|p| (p.event, p.source, p.dest, p.suffix.clone()))
                .collect::<Vec<_>>(),
            [(event, 1, 2, vec![V::Str("IMMUNE".into())])]
        );
    }
}
