use std::collections::HashMap;

use super::SpellUseView;
use crate::script::{
    ContainerSlot, ContainerState, PetBookState, SpellBookState, SpellInfoView, SpellSlotView,
    UiScript, UnitGuids,
};

fn slot(id: u32, name: &str, rank: Option<&str>) -> SpellSlotView {
    SpellSlotView {
        spell_id: id,
        name: name.into(),
        rank: rank.map(str::to_string),
        ..Default::default()
    }
}

fn row(id: u32) -> SpellInfoView {
    SpellInfoView {
        spell_id: id,
        ..Default::default()
    }
}

/// Fireball 1 and 2, Shoot, a shot with arrows, Attack; a pet bite.
fn script() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_spellbook(SpellBookState {
        slots: vec![
            slot(133, "Fireball", Some("Rank 1")),
            slot(143, "Fireball", Some("Rank 2")),
            slot(5019, "Shoot", None),
            slot(700, "Arcane Shot", None),
            slot(6603, "Attack", None),
            slot(2050, "Lesser Heal", Some("Rank 1")),
        ],
        ..Default::default()
    });
    s.set_pet_book(PetBookState {
        slots: vec![slot(17253, "Bite", Some("Rank 1"))],
        ..Default::default()
    });
    s.set_spell_catalog([
        SpellInfoView {
            polarity: 2,
            ..row(133)
        },
        SpellInfoView {
            polarity: 2,
            ..row(143)
        },
        SpellInfoView {
            polarity: 2,
            ..row(5019)
        },
        SpellInfoView {
            polarity: 2,
            reagents: vec![(2512, 2), (0, 0)],
            ..row(700)
        },
        SpellInfoView {
            attack: true,
            ..row(6603)
        },
        SpellInfoView {
            polarity: 1,
            ..row(2050)
        },
        row(17253),
    ]);
    s
}

/// The reader `0x540670`: a slot and book, a name with an embedded or separate rank, `!`, and a
/// miss answering nothing; a bad slot raises with the verb.
#[test]
fn the_spell_reader_takes_slots_names_and_ranks() {
    let s = script();
    s.run(
        r#"
        assert(IsHarmfulSpell(1, "spell") == 1)
        assert(IsHarmfulSpell("Fireball") == 1 and IsHarmfulSpell("fireball(rank 1)") == 1)
        assert(IsHarmfulSpell("Fireball", "Rank 2") == 1 and IsHarmfulSpell("!Fireball") == 1)
        assert(select('#', IsHarmfulSpell("Frostbolt")) == 0)
        assert(select('#', IsHarmfulSpell("Fireball", "Rank 9")) == 0)
        -- An empty slot answers like a spell with no record: one nil.
        assert(select('#', IsHarmfulSpell(40, "spell")) == 1 and IsHarmfulSpell(40, "spell") == nil)
        local ok, err = pcall(IsHarmfulSpell, 0, "spell")
        assert(not ok and tostring(err):find("IsHarmfulSpell%(%): Invalid spell slot"))
        assert(not pcall(IsHarmfulSpell, nil))
        "#,
    )
    .unwrap();
}

#[test]
fn the_static_predicates_read_the_catalog() {
    let s = script();
    s.run(
        r#"
        assert(IsHelpfulSpell("Lesser Heal") == 1 and IsHarmfulSpell("Lesser Heal") == nil)
        assert(IsHelpfulSpell("Fireball") == nil)
        assert(IsAttackSpell("Attack") == 1 and IsAttackSpell("Fireball") == nil)
        assert(IsConsumableSpell("Arcane Shot") == 1 and IsConsumableSpell("Fireball") == nil)
        assert(IsAttackSpell(1, "pet") == nil and IsHelpfulSpell(1, "pet") == nil)
        "#,
    )
    .unwrap();
}

/// `GetSpellCount` divides the carried reagent by its count; 0 for a spell without reagents.
#[test]
fn spell_count_divides_the_carried_reagents() {
    let mut s = script();
    s.set_unit_guids(&UnitGuids {
        player: 1,
        ..Default::default()
    });
    let mut slots = HashMap::new();
    slots.insert(
        1,
        ContainerSlot {
            item_id: 2512,
            count: 7,
            ..Default::default()
        },
    );
    s.set_container(
        0,
        Some(ContainerState {
            num_slots: 16,
            slots,
            ..Default::default()
        }),
    );
    s.run(
        r#"
        assert(GetSpellCount("Arcane Shot") == 3)
        assert(GetSpellCount("Fireball") == 0 and GetSpellCount(40, "spell") == 0)
        "#,
    )
    .unwrap();
    s.set_unit_guids(&UnitGuids::default());
    s.run("assert(GetSpellCount('Arcane Shot') == 0)").unwrap();
}

/// The live pair, two values always, from the book the spell came from; an unpushed spell
/// answers nil, nil. Either auto-repeat word lights the player's spell.
#[test]
fn usable_and_auto_repeat_read_the_live_views() {
    let mut s = script();
    let view = |usable, not_enough_mana| SpellUseView {
        usable,
        not_enough_mana,
    };
    let player = [(133, view(true, false)), (143, view(false, true))];
    let pet = [(17253, view(true, false))];
    s.set_wrath_spell_use(player.into_iter().collect(), pet.into_iter().collect());
    s.set_wrath_auto_repeat_spell([None, Some(5019)]);
    s.run(
        r#"
        local a, b = IsUsableSpell("Fireball(Rank 1)"); assert(a == 1 and b == nil)
        a, b = IsUsableSpell(2, "spell"); assert(a == nil and b == 1)
        assert(select('#', IsUsableSpell("Shoot")) == 2 and IsUsableSpell("Shoot") == nil)
        assert(IsUsableSpell(1, "pet") == 1 and IsUsableSpell("Bite") == 1)
        assert(IsAutoRepeatSpell("Shoot") == 1 and IsAutoRepeatSpell("Fireball") == nil)
        "#,
    )
    .unwrap();
    s.set_wrath_auto_repeat_spell([Some(133), None]);
    s.run("assert(IsAutoRepeatSpell('Shoot') == nil and IsAutoRepeatSpell(1, 'spell') == 1)")
        .unwrap();
}

/// `0x53b6c0`: the 0-based book index, nil when absent, the usage for a non-positive id.
#[test]
fn find_slot_by_id_answers_the_raw_index() {
    let s = script();
    s.run(
        r#"
        assert(FindSpellBookSlotByID(143) == 1 and FindSpellBookSlotByID(133) == 0)
        assert(FindSpellBookSlotByID(17253, true) == 0 and FindSpellBookSlotByID(17253) == nil)
        assert(FindSpellBookSlotByID(99999) == nil)
        assert(not pcall(FindSpellBookSlotByID, 0) and not pcall(FindSpellBookSlotByID, "x"))
        "#,
    )
    .unwrap();
}
