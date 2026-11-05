use std::collections::HashMap;

use wrathplus_formats::{SpellDisplay, SpellRange, SpellRangeCatalog};

use super::{catalog_facts, item_facts, Spells};

fn spell(targets: u32, a: [u32; 3], b: [u32; 3], aura: [u32; 3]) -> SpellDisplay {
    SpellDisplay {
        targets,
        effect_implicit_target_a: a,
        effect_implicit_target_b: b,
        effect_apply_aura: aura,
        ..Default::default()
    }
}

/// `0x7fe1b0`: the ally flag first, then the enemy flag, an enemy A or B, then a friendly A or
/// B or the caster with a non-dummy aura.
#[test]
fn polarity_follows_the_classifier() {
    assert_eq!(
        SpellDisplay::benefit(&spell(0x180, [6, 0, 0], [0; 3], [0; 3])),
        1,
        "0x100 wins"
    );
    assert_eq!(
        SpellDisplay::benefit(&spell(0x80, [0; 3], [0; 3], [0; 3])),
        2
    );
    assert_eq!(
        SpellDisplay::benefit(&spell(0, [0, 0, 0], [0, 0x5d, 0], [0; 3])),
        2,
        "an enemy B"
    );
    assert_eq!(
        SpellDisplay::benefit(&spell(0, [0x15, 0, 0], [0; 3], [0; 3])),
        1,
        "a single friend"
    );
    assert_eq!(
        SpellDisplay::benefit(&spell(0, [1, 0, 0], [0; 3], [6, 0, 0])),
        1,
        "self with an aura"
    );
    assert_eq!(
        SpellDisplay::benefit(&spell(0, [1, 0, 0], [0; 3], [4, 0, 0])),
        0,
        "self, a dummy aura"
    );
    assert_eq!(SpellDisplay::benefit(&spell(0, [0; 3], [0; 3], [0; 3])), 0);
}

/// The catalog tail: the attack effect, the reagents, and the range row's two pairs.
#[test]
fn catalog_facts_carry_the_range_row_and_its_friendly_pair() {
    let mut rows = HashMap::new();
    rows.insert(
        5,
        SpellRange {
            min: 0.0,
            max: 30.0,
            flags: 0,
        },
    );
    let ranges = SpellRangeCatalog::from_rows(rows).with_friendly([(5, (0.0, 40.0))].into());
    let spells = Spells {
        ranges,
        ..Spells::empty_for_tests()
    };
    let d = SpellDisplay {
        range_index: 5,
        effects: [78, 0, 0],
        reagents: [
            (17_056, 1),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
        ],
        attributes_ex5: 0x800,
        ..Default::default()
    };
    let f = catalog_facts(&d, &spells);
    assert!(f.attack);
    assert_eq!(f.reagents[0], (17_056, 1));
    let row = f.range.row.unwrap();
    assert_eq!((row.min, row.max), ([0.0, 0.0], [30.0, 40.0]));
    assert_eq!(f.range.attributes_ex5, 0x800);
    let none = catalog_facts(&SpellDisplay::default(), &spells);
    assert!(none.range.row.is_none() && !none.attack);
}

/// Only on-use blocks with a spell, in order, with their raw words.
#[test]
fn item_facts_keep_the_on_use_blocks() {
    use wrathplus_protocol::types::{ItemInfo, ItemSpellEntry};
    let block = |index, spell_id, trigger, charges| ItemSpellEntry {
        index,
        spell_id,
        trigger,
        charges,
        cooldown_ms: 1_000,
        category: 4,
        category_cooldown_ms: 120_000,
    };
    let t = ItemInfo {
        bag_family: 6,
        spells: vec![block(0, 7, 1, 0), block(1, 439, 0, -1), block(2, 0, 0, 0)],
        ..Default::default()
    };
    let f = item_facts(&t);
    assert_eq!(f.bag_family, 6);
    assert_eq!(f.use_spells.len(), 1);
    let s = f.use_spells[0];
    assert_eq!(
        (s.spell, s.charges, s.cooldown_ms, s.category_cooldown_ms),
        (439, -1, 1_000, 120_000)
    );
}
