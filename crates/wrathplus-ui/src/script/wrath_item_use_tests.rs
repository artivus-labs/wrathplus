use std::collections::HashMap;

use super::{ItemUseFacts, ItemUseSpellFacts, ItemUseView};
use crate::script::{
    ContainerSlot, ContainerState, InvSlotView, InventorySlots, ItemTemplateView, RangeUnitFacts,
    SpellInfoView, UiScript, UnitGuids, WrathCurrencyEntry, WrathCurrencySnapshot,
    WrathInstanceInfo,
};

const POTION: u32 = 118;
const ARROW: u32 = 2512;
const BOMB: u32 = 4360;
const SHIELD: u32 = 2129;
const RUBY: u32 = 23094;

fn block(spell: u32, charges: i32) -> ItemUseSpellFacts {
    ItemUseSpellFacts {
        spell,
        charges,
        cooldown_ms: -1,
        category_cooldown_ms: -1,
    }
}

fn template(name: &str) -> ItemTemplateView {
    ItemTemplateView {
        name: name.into(),
        ..Default::default()
    }
}

fn bag(items: &[(u32, u32, u32)]) -> Option<ContainerState> {
    let slots = items
        .iter()
        .map(|&(slot, item_id, count)| {
            (
                slot,
                ContainerSlot {
                    item_id,
                    count,
                    ..Default::default()
                },
            )
        })
        .collect();
    Some(ContainerState {
        num_slots: 16,
        slots,
        ..Default::default()
    })
}

/// A potion (helpful, charges -1), arrows, a bomb (harmful, ranged), a worn shield, a ruby.
fn script() -> UiScript {
    let mut s = UiScript::new().unwrap();
    s.set_spell_catalog([
        SpellInfoView {
            spell_id: 439,
            name: "Healing Potion".into(),
            polarity: 1,
            ..Default::default()
        },
        SpellInfoView {
            spell_id: 4064,
            name: "Rough Copper Bomb".into(),
            polarity: 2,
            range: crate::script::SpellRangeFacts {
                implicit_a: [6, 0, 0],
                row: Some(crate::script::RangeRow {
                    flags: 0,
                    min: [0.0; 2],
                    max: [30.0; 2],
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    ]);
    s.set_item_template(POTION, template("Minor Healing Potion"));
    s.set_wrath_item_use_facts(
        POTION,
        ItemUseFacts {
            bag_family: 0,
            use_spells: vec![block(439, -1)],
        },
    );
    s.set_item_template(
        ARROW,
        ItemTemplateView {
            inventory_type: 24,
            ..template("Rough Arrow")
        },
    );
    s.set_wrath_item_use_facts(
        ARROW,
        ItemUseFacts {
            bag_family: 1,
            use_spells: vec![],
        },
    );
    s.set_item_template(BOMB, template("Rough Copper Bomb"));
    s.set_wrath_item_use_facts(
        BOMB,
        ItemUseFacts {
            bag_family: 0,
            use_spells: vec![block(4064, 0)],
        },
    );
    s.set_item_template(
        SHIELD,
        ItemTemplateView {
            inventory_type: 14,
            item_type: Some("Armor".into()),
            item_sub_type: Some("Shields".into()),
            ..template("Large Round Shield")
        },
    );
    s.set_item_template(RUBY, template("Bold Ruby"));
    s.set_container(0, bag(&[(1, POTION, 3), (2, ARROW, 200), (3, BOMB, 2)]));
    s.set_container(-1, bag(&[(1, POTION, 5)]));
    let mut worn: InventorySlots = Default::default();
    worn[17] = Some(InvSlotView {
        item_id: SHIELD,
        count: 1,
        ..Default::default()
    });
    s.set_inventory_slots(worn);
    s
}

#[test]
fn item_count_sums_bags_worn_currency_and_the_bank_on_request() {
    let mut s = script();
    s.set_wrath_currency_snapshot(WrathCurrencySnapshot {
        entries: vec![WrathCurrencyEntry {
            item_id: 29434,
            count: 12,
            ..Default::default()
        }],
        ..Default::default()
    });
    s.run(
        r#"
        assert(GetItemCount(118) == 3 and GetItemCount(118, 1) == 8)
        assert(GetItemCount("Minor Healing Potion", true) == 8)
        assert(GetItemCount("|cffffffff|Hitem:118:0:0:0|h[Minor Healing Potion]|h|r") == 3)
        assert(GetItemCount(2129) == 1 and GetItemCount(29434) == 12)
        assert(GetItemCount(999) == 0 and GetItemCount("nothing") == 0)
        "#,
    )
    .unwrap();
}

#[test]
fn family_consumable_and_polarity_read_the_template() {
    let s = script();
    s.run(
        r#"
        assert(GetItemFamily(2512) == 1 and GetItemFamily(118) == 0)
        assert(select('#', GetItemFamily(2129)) == 0)
        assert(IsConsumableItem(118) == 1 and IsConsumableItem(2512) == 1)
        assert(IsConsumableItem(4360) == nil and IsConsumableItem(2129) == nil)
        assert(IsHelpfulItem(118) == 1 and IsHarmfulItem(118) == nil)
        assert(IsHarmfulItem("Rough Copper Bomb") == 1 and IsHelpfulItem(4360) == nil)
        assert(IsHarmfulItem(2129) == nil)
        "#,
    )
    .unwrap();
}

/// Usable, range and cooldown: the app's pair, the bomb's range through the range test, and the
/// cooldown's empty answers by use spell.
#[test]
fn usable_range_and_cooldown_read_the_live_views() {
    let mut s = script();
    s.set_now(100.0);
    let mut items = HashMap::new();
    items.insert(
        POTION,
        ItemUseView {
            usable: true,
            cooldown: Some((90_000, 60_000, true)),
            ..Default::default()
        },
    );
    s.set_wrath_item_use(items);
    s.set_unit_guids(&UnitGuids {
        player: 1,
        target: 7,
        held: [(1, 0), (7, 0)].into_iter().collect(),
        ..Default::default()
    });
    let facts = |x: f32, attack| RangeUnitFacts {
        pos: [x, 0.0, 0.0],
        reach: 1.0,
        health: 10,
        attack,
        ..Default::default()
    };
    s.set_wrath_range_inputs(
        [(1, facts(0.0, false)), (7, facts(20.0, true))]
            .into_iter()
            .collect(),
        HashMap::new(),
        None,
    );
    s.run(
        r#"
        local a, b = IsUsableItem(118); assert(a == 1 and b == nil)
        a, b = IsUsableItem(4360); assert(select('#', IsUsableItem(4360)) == 2 and a == nil and b == nil)
        assert(ItemHasRange(4360) == 1 and ItemHasRange(118) == nil)
        assert(IsItemInRange(4360, "target") == 1 and IsItemInRange(4360) == 1)
        assert(IsItemInRange(118, "target") == nil)
        local s, d, e = GetItemCooldown(118); assert(s == 90 and d == 60 and e == 1)
        s, d, e = GetItemCooldown(4360); assert(s == 0 and d == 0 and e == 1)
        s, d, e = GetItemCooldown(2129); assert(s == 0 and d == 0 and e == 0)
        "#,
    )
    .unwrap();
}

/// On an arena map a used-up potion is refused unless flagged usable in arena.
#[test]
fn the_arena_gate_refuses_spent_items() {
    let mut s = script();
    let usable = ItemUseView {
        usable: true,
        ..Default::default()
    };
    s.set_wrath_item_use([(POTION, usable)].into_iter().collect());
    s.set_wrath_instance_info(Some(WrathInstanceInfo {
        instance_type: 4,
        ..Default::default()
    }));
    s.run("assert(IsUsableItem(118) == nil)").unwrap();
    s.set_item_template(
        POTION,
        ItemTemplateView {
            flags: 0x20_0000,
            ..template("Minor Healing Potion")
        },
    );
    s.run("assert(IsUsableItem(118) == 1)").unwrap();
}

/// A broken copy drops out of the count unless `includeCharges` is passed, which for a
/// non-stacking item with spent charges counts the charges.
#[test]
fn item_count_skips_broken_copies_and_counts_charges() {
    let mut s = script();
    let mut worn: InventorySlots = Default::default();
    worn[17] = Some(InvSlotView {
        item_id: SHIELD,
        count: 1,
        durability: Some((0, 50)),
        ..Default::default()
    });
    s.set_inventory_slots(worn);
    s.run("assert(GetItemCount(2129) == 0 and GetItemCount(2129, nil, 1) == 1)")
        .unwrap();
    let wand = ItemUseView {
        charges: [7, 9],
        ..Default::default()
    };
    s.set_wrath_item_use([(POTION, wand)].into_iter().collect());
    s.set_item_template(
        POTION,
        ItemTemplateView {
            stackable: 1,
            ..template("Minor Healing Potion")
        },
    );
    s.run("assert(GetItemCount(118, false, true) == 7 and GetItemCount(118, true, true) == 9)")
        .unwrap();
}

#[test]
fn equipped_item_type_matches_slot_keys_and_class_names() {
    let mut s = script();
    s.run(
        r#"
        assert(IsEquippedItemType("INVTYPE_SHIELD") == 1 and IsEquippedItemType("Shields") == 1)
        assert(IsEquippedItemType("armor") == 1 and IsEquippedItemType("INVTYPE_HEAD") == nil)
        assert(IsEquippedItemType("Daggers") == nil and IsEquippedItemType("Off Hand") == nil)
        assert(not pcall(IsEquippedItemType, "") and not pcall(IsEquippedItemType))
        "#,
    )
    .unwrap();
    // `ItemSubClassMask.dbc`: class 4 (Armor), subclass 6 (Shield) in the mask.
    s.set_wrath_subclass_masks(vec![(4, 1 << 6, "Off Hand".into())]);
    s.set_item_template(
        SHIELD,
        ItemTemplateView {
            inventory_type: 14,
            class: 4,
            subclass: 6,
            ..template("Large Round Shield")
        },
    );
    s.run("assert(IsEquippedItemType('off hand') == 1)")
        .unwrap();
}

/// The casting item entries the app pushes.
#[test]
fn current_item_reads_the_casts_items() {
    let mut s = script();
    s.set_wrath_cast_items(vec![POTION]);
    s.run("assert(IsCurrentItem(118) == 1 and IsCurrentItem(4360) == nil)")
        .unwrap();
}

/// A link's gem enchant through `GemID`; a name through the socketed item's gems; nothing for an
/// empty socket, a bad index or an uncached gem; the usage for a missing index.
#[test]
fn item_gem_reads_links_and_named_items() {
    let mut s = script();
    s.set_wrath_gem_enchants([(3261, RUBY)].into_iter().collect());
    s.set_wrath_item_gems(vec![(0, 1, [None, Some(RUBY), None])]);
    s.run(
        r#"
        local link = "|cffffffff|Hitem:2129:0:3261:0:0:0:0:0:0|h[Large Round Shield]|h|r"
        local name, gem = GetItemGem(link, 1)
        assert(name == "Bold Ruby" and gem:find("Hitem:23094"))
        assert(select('#', GetItemGem(link, 2)) == 0 and select('#', GetItemGem(link, 4)) == 0)
        assert(GetItemGem("Minor Healing Potion", 2) == "Bold Ruby")
        assert(select('#', GetItemGem("Minor Healing Potion", 1)) == 0)
        assert(not pcall(GetItemGem, link) and not pcall(GetItemGem, nil, 1))
        "#,
    )
    .unwrap();
    s.set_wrath_gem_enchants([(3261, 99_999)].into_iter().collect());
    s.run(r#"assert(select('#', GetItemGem("item:1:0:3261", 1)) == 0)"#)
        .unwrap();
}
