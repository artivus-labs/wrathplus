use std::collections::HashMap;

use wrathplus_formats::{SpellCatalog, SpellDisplay};
use wrathplus_protocol::types::{ItemInfo, ItemSpellEntry};
use wrathplus_protocol::ObjectFields;

use super::{resolve_self_res, resurrection_blocked, SelfRes, SPELL_EFFECT_SELF_RESURRECT};
use crate::app_core::state::Spells;
use crate::items::Items;
use crate::net::{ClientCommand, NetCommands};

/// Reincarnation's effect spell, what the server writes into the field.
const REINCARNATION: u32 = 21169;
/// `PLAYER_SELF_RES_SPELL`.
const F_SELF_RES: u16 = wrathplus_protocol::types::Field::PlayerSelfResSpell.index();
/// `UNIT_FIELD_HEALTH` and `UNIT_FIELD_MAXHEALTH`: `unit_is_dead()` also needs a non-zero max.
const F_HEALTH: u16 = wrathplus_protocol::types::Field::UnitHealth.index();
const F_MAXHEALTH: u16 = wrathplus_protocol::types::Field::UnitMaxHealth.index();
/// `PLAYER_FIELD_PACK_SLOT_1`: the backpack's 16 slots, two dwords per guid.
const F_PACK_1: u16 = wrathplus_protocol::types::Field::PlayerPackSlot1.index();

fn commands() -> (NetCommands, crossbeam_channel::Receiver<ClientCommand>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    (NetCommands(tx), rx)
}

fn catalog(pairs: impl IntoIterator<Item = (u32, SpellDisplay)>) -> Spells {
    Spells {
        catalog: SpellCatalog::from_displays(pairs.into_iter().collect::<HashMap<_, _>>()),
        ..Spells::empty_for_tests()
    }
}

fn named(name: &str, effects: [u32; 3]) -> SpellDisplay {
    SpellDisplay {
        name: name.to_string(),
        effects,
        ..Default::default()
    }
}

/// `0x48acba`: nil while health is positive, including a released ghost's 1
/// (`BuildPlayerRepop`).
#[test]
fn the_dead_gate_precedes_the_field() {
    let (net, _rx) = commands();
    let items = Items::default();
    // Nothing streamed: the spell leg answers before the inventory walk.
    let mut objs = crate::items::TestObjects::new();
    let objects = objs.get();
    let spells = catalog([(REINCARNATION, named("Reincarnation", [94, 0, 0]))]);

    let alive = ObjectFields::from_pairs(&[
        (F_MAXHEALTH, 4000),
        (F_HEALTH, 4000),
        (F_SELF_RES, REINCARNATION),
    ]);
    assert_eq!(
        resolve_self_res(&alive, &objects, &items, Some(&spells), &net),
        None,
        "alive with a self-res owed: nil"
    );

    let ghost = ObjectFields::from_pairs(&[
        (F_MAXHEALTH, 4000),
        (F_HEALTH, 1),
        (F_SELF_RES, REINCARNATION),
    ]);
    assert_eq!(
        resolve_self_res(&ghost, &objects, &items, Some(&spells), &net),
        None,
        "a released ghost still holds the field, and still answers nil"
    );

    let dead = ObjectFields::from_pairs(&[
        (F_MAXHEALTH, 4000),
        (F_HEALTH, 0),
        (F_SELF_RES, REINCARNATION),
    ]);
    assert_eq!(
        resolve_self_res(&dead, &objects, &items, Some(&spells), &net),
        Some(SelfRes::Spell {
            spell: REINCARNATION,
            label: "Reincarnation".into(),
        })
    );
}

/// The reference pushes the literal (`0x48ad1d`).
#[test]
fn an_unresolvable_spell_id_reads_unknown_not_nil() {
    let (net, _rx) = commands();
    let items = Items::default();
    let mut objs = crate::items::TestObjects::new();
    let objects = objs.get();
    let dead =
        ObjectFields::from_pairs(&[(F_MAXHEALTH, 4000), (F_HEALTH, 0), (F_SELF_RES, 999_999)]);
    for spells in [None, Some(catalog([]))] {
        assert_eq!(
            resolve_self_res(&dead, &objects, &items, spells.as_ref(), &net),
            Some(SelfRes::Spell {
                spell: 999_999,
                label: "UNKNOWN".into(),
            }),
            "no catalog and an empty catalog are the same miss"
        );
    }
}

/// The item leg (`0x5ed630`); the backpack holds a self-res on an equip trigger, an unrelated
/// on-use spell, and the real one.
#[test]
fn a_zero_field_falls_through_to_a_carried_item() {
    let (net, _rx) = commands();
    let mut items = Items::default();
    let mut objs = crate::items::TestObjects::new();

    let item = |name: &str, spells: Vec<ItemSpellEntry>| ItemInfo {
        spells,
        ..crate::items::test_template(name)
    };
    let block = |spell_id: u32, trigger: u32| ItemSpellEntry {
        index: 0,
        spell_id,
        trigger,
        charges: 0,
        cooldown_ms: -1,
        category: 0,
        category_cooldown_ms: -1,
    };
    // 3026 "Use Soulstone" is the real effect-94 spell; 439 "Healing Potion" is not.
    items.insert_template(10, Some(item("Healthstone", vec![block(439, 0)])));
    items.insert_template(11, Some(item("Worn Trinket", vec![block(3026, 1)])));
    items.insert_template(
        12,
        Some(item("Ankh of Reincarnation", vec![block(3026, 0)])),
    );
    for (guid, entry) in [(0xA0_u64, 10_u32), (0xA1, 11), (0xA2, 12)] {
        objs.spawn(
            guid,
            ObjectFields::from_fields(&[(wrathplus_protocol::types::Field::ObjectEntry, entry)]),
        );
    }
    let objects = objs.get();
    let spells = catalog([
        (
            3026,
            named("Use Soulstone", [SPELL_EFFECT_SELF_RESURRECT, 0, 0]),
        ),
        (439, named("Healing Potion", [6, 0, 0])),
    ]);

    // Backpack slots 23, 24 and 25 hold the three, in that order.
    let mut pairs = vec![(F_MAXHEALTH, 4000), (F_HEALTH, 0)];
    for (i, guid) in [0xA0_u64, 0xA1, 0xA2].iter().enumerate() {
        let f = F_PACK_1 + 2 * i as u16;
        pairs.push((f, (*guid & 0xffff_ffff) as u32));
        pairs.push((f + 1, (*guid >> 32) as u32));
    }
    let dead = ObjectFields::from_pairs(&pairs);

    match resolve_self_res(&dead, &objects, &items, Some(&spells), &net) {
        Some(SelfRes::Item { entry, label, .. }) => {
            assert_eq!(
                entry, 12,
                "the EQUIP-trigger copy and the potion are skipped"
            );
            assert_eq!(label, "Ankh of Reincarnation", "the ITEM names the button");
        }
        other => panic!("expected the item leg, got {other:?}"),
    }

    // Without it the answer is nil, not the spell leg's "UNKNOWN".
    let bare = ObjectFields::from_pairs(&[(F_MAXHEALTH, 4000), (F_HEALTH, 0)]);
    assert_eq!(
        resolve_self_res(&bare, &objects, &items, Some(&spells), &net),
        None
    );
}

/// `0x727860`: an aura applying 314 blocks with its `EffectMiscValue`, unless the self-res spell
/// carries `AttributesEx7` `0x8000000`.
#[test]
fn a_prevent_resurrection_aura_blocks_unless_the_self_res_bypasses_it() {
    const CURSE: u32 = 70_000;
    let curse = SpellDisplay {
        effect_apply_aura: [0, 314, 0],
        effect_misc_value: [0, 12, 0],
        ..Default::default()
    };
    let bypass = SpellDisplay {
        attributes_ex7: 0x0800_0000,
        ..named("Reincarnation", [94, 0, 0])
    };
    let spells = catalog([(CURSE, curse), (REINCARNATION, bypass)]);
    let c = &spells.catalog;
    assert_eq!(resurrection_blocked(None, [1, CURSE], c), Some(12));
    assert_eq!(resurrection_blocked(Some(REINCARNATION), [CURSE], c), None);
    assert_eq!(resurrection_blocked(None, [1, 2], c), None);
}
