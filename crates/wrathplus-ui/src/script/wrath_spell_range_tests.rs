use std::collections::HashMap;

use super::{RangeRow, RangeUnitFacts, SpellRangeFacts};
use crate::script::{SpellBookState, SpellInfoView, SpellSlotView, UiScript, UnitGuids};

const ME: u64 = 1;
const PET: u64 = 2;
const FOE: u64 = 7;
const FRIEND: u64 = 9;
const CORPSE: u64 = 11;

fn ranged(
    id: u32,
    name: &str,
    targets: u32,
    implicit: u32,
    row: RangeRow,
) -> (SpellSlotView, SpellInfoView) {
    let slot = SpellSlotView {
        spell_id: id,
        name: name.into(),
        ..Default::default()
    };
    let info = SpellInfoView {
        spell_id: id,
        range: SpellRangeFacts {
            targets,
            implicit_a: [implicit, 0, 0],
            row: Some(row),
            ..Default::default()
        },
        ..Default::default()
    };
    (slot, info)
}

fn unit(x: f32, reach: f32) -> RangeUnitFacts {
    RangeUnitFacts {
        pos: [x, 0.0, 0.0],
        reach,
        health: 100,
        ..Default::default()
    }
}

/// Fireball (enemy, 0-30 hostile), Heal (ally, 0-40 friendly, 0-30 hostile), a melee strike, a
/// resurrection (dead allies), and a dead-only `0x400` spell.
fn script() -> UiScript {
    let mut s = UiScript::new().unwrap();
    let row = |flags, min, max| RangeRow { flags, min, max };
    let spells = [
        ranged(133, "Fireball", 0, 6, row(0, [0.0, 0.0], [30.0, 30.0])),
        ranged(2050, "Heal", 0, 0x15, row(0, [0.0, 0.0], [30.0, 40.0])),
        ranged(78, "Strike", 0, 6, row(1, [0.0, 0.0], [0.0, 0.0])),
        ranged(
            2006,
            "Resurrection",
            0x8000,
            0,
            row(0, [0.0, 0.0], [30.0, 30.0]),
        ),
        ranged(
            5,
            "Corpse Only",
            0x400 | 2,
            0,
            row(0, [0.0, 0.0], [30.0, 30.0]),
        ),
        ranged(6, "Too Close", 0, 6, row(0, [8.0, 8.0], [30.0, 30.0])),
    ];
    s.set_spellbook(SpellBookState {
        slots: spells.iter().map(|(slot, _)| slot.clone()).collect(),
        ..Default::default()
    });
    s.set_spell_catalog(spells.into_iter().map(|(_, info)| info));
    let mut guids = UnitGuids {
        player: ME,
        pet: PET,
        target: FOE,
        focus: FRIEND,
        mouseover: CORPSE,
        ..Default::default()
    };
    for g in [ME, PET, FOE, FRIEND, CORPSE] {
        guids.held.insert(g, 0);
    }
    s.set_unit_guids(&guids);
    let mut units = HashMap::new();
    units.insert(
        ME,
        RangeUnitFacts {
            player: true,
            ..unit(0.0, 1.5)
        },
    );
    units.insert(PET, unit(2.0, 1.0));
    units.insert(
        FOE,
        RangeUnitFacts {
            attack: true,
            ..unit(31.0, 1.0)
        },
    );
    units.insert(
        FRIEND,
        RangeUnitFacts {
            assist: [true, true],
            player: true,
            ..unit(35.0, 1.5)
        },
    );
    units.insert(
        CORPSE,
        RangeUnitFacts {
            assist: [true, true],
            player: true,
            health: 0,
            ..unit(10.0, 1.5)
        },
    );
    s.set_wrath_range_inputs(units, HashMap::new(), None);
    s
}

/// The target arms of `0x809610`, then the ranged pad of `0x7ff480`: a unit target adds both
/// reaches to the max (31 yards in at 30 + 1.5 + 1.0), and an ally reads the friendly pair.
#[test]
fn the_gate_picks_the_unit_and_the_pad_reaches_it() {
    let s = script();
    s.run(
        r#"
        assert(IsSpellInRange("Fireball") == 1 and IsSpellInRange("Fireball", "target") == 1)
        assert(IsSpellInRange("Fireball", "focus") == nil)  -- an ally for an enemy spell
        assert(IsSpellInRange("Heal", "focus") == 1)          -- 35 <= 40 + 3
        assert(IsSpellInRange("Heal", "target") == nil)
        assert(IsSpellInRange("Heal", "player") == nil)  -- the player is not assistable here
        assert(IsSpellInRange("Fireball", "nonsense") == nil)
        -- A valid token naming nobody falls back to the target (`0x809610` on guid 0).
        assert(IsSpellInRange("Fireball", "party1") == 1)
        "#,
    )
    .unwrap();
}

/// Melee: max(reaches + 4/3, 5); the foe at 31 yards is out, closer is in.
#[test]
fn melee_sums_the_reaches_over_the_floor() {
    let mut s = script();
    s.run("assert(IsSpellInRange('Strike') == 0 and SpellHasRange('Strike') == 1)")
        .unwrap();
    let mut units = HashMap::new();
    units.insert(
        ME,
        RangeUnitFacts {
            player: true,
            ..unit(0.0, 1.5)
        },
    );
    units.insert(
        FOE,
        RangeUnitFacts {
            attack: true,
            ..unit(3.5, 1.0)
        },
    );
    s.set_wrath_range_inputs(units, HashMap::new(), None);
    // 1.5 + 1.0 + 4/3 = 3.83, floored to 5.
    s.run("assert(IsSpellInRange('Strike') == 1)").unwrap();
    // 8 + 2.5 = 10.5 yards minimum.
    s.run("assert(IsSpellInRange('Too Close') == 0)").unwrap();
}

/// The dead-unit arms: a dead ally passes the corpse-ally word, a living unit fails a `0x400`
/// spell, and the min padded by both reaches refuses a unit too close.
#[test]
fn dead_and_too_close_follow_the_words() {
    let s = script();
    s.run(
        r#"
        assert(IsSpellInRange("Resurrection", "mouseover") == 1)
        assert(IsSpellInRange("Resurrection", "focus") == nil)
        assert(IsSpellInRange("Corpse Only", "focus") == nil)
        assert(IsSpellInRange("Too Close", "target") == 1)
        "#,
    )
    .unwrap();
}

/// The range modifier (op 5) scales the row's max before the pad.
#[test]
fn the_range_modifier_applies_before_the_pad() {
    let mut s = script();
    let mut units = HashMap::new();
    units.insert(
        ME,
        RangeUnitFacts {
            player: true,
            ..unit(0.0, 1.5)
        },
    );
    units.insert(
        FOE,
        RangeUnitFacts {
            attack: true,
            ..unit(37.0, 1.0)
        },
    );
    s.set_wrath_range_inputs(units.clone(), HashMap::new(), None);
    s.run("assert(IsSpellInRange('Fireball') == 0)").unwrap();
    // (5 + 30) * 100% = 35, + 2.5 pad = 37.5.
    s.set_wrath_range_inputs(units, [(133, (5, 100))].into_iter().collect(), None);
    s.run("assert(IsSpellInRange('Fireball') == 1)").unwrap();
}

/// No caster object (no player pushed), and an unheld unit, answer nil.
#[test]
fn a_missing_caster_or_unit_answers_nil() {
    let mut s = script();
    s.set_wrath_range_inputs(HashMap::new(), HashMap::new(), None);
    s.run("assert(IsSpellInRange('Fireball') == nil and SpellHasRange('Fireball') == nil)")
        .unwrap();
}
