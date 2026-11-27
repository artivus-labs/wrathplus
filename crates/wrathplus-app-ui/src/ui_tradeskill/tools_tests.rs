use std::collections::HashMap;

use wrathplus_formats::TotemCategory;
use wrathplus_protocol::types::{Field, ItemInfo};
use wrathplus_protocol::ObjectFields;

use super::*;

const HAMMER: u32 = 5956;
const HAMMER_GUID: u64 = 0x4000_0000_0000_0042;

/// A Rough Copper Vest's tools (session 115138): the Blacksmith Hammer category, met by the hammer
/// in the backpack; an Arclight Spanner category beside it, unmet.
#[test]
fn a_carried_hammer_meets_its_totem_category() {
    let row = |name: &str, kind| TotemCategory {
        name: name.into(),
        kind,
        mask: 1,
    };
    let categories = TotemCategoryCatalog::from_rows(HashMap::from([
        (162, row("Blacksmith Hammer", 2)),
        (14, row("Arclight Spanner", 4)),
    ]));
    // Backpack slot 0 is inventory slot 23.
    let slot = Field::PlayerInvSlotHead.index() + 2 * 23;
    let store = ObjectStore(ObjectFields::from_pairs(&[
        (slot, HAMMER_GUID as u32),
        (slot + 1, (HAMMER_GUID >> 32) as u32),
    ]));
    let mut objs = crate::items::TestObjects::new();
    objs.spawn(
        HAMMER_GUID,
        ObjectFields::from_pairs(&[(Field::ObjectEntry.index(), HAMMER)]),
    );
    let mut items = Items::default();
    items.insert_template(
        HAMMER,
        Some(ItemInfo {
            name: "Blacksmith Hammer".into(),
            totem_category: 162,
            ..Default::default()
        }),
    );
    let (tx, _rx) = crossbeam_channel::unbounded();
    let commands = NetCommands(tx);
    let vest = SpellDisplay {
        totem_categories: [162, 14],
        ..Default::default()
    };
    let objects = objs.get();
    let tools = recipe_tools(
        &vest,
        None,
        Some(&categories),
        &store,
        &objects,
        &items,
        &commands,
    );
    assert_eq!(
        tools,
        [
            ("Blacksmith Hammer".to_string(), true),
            ("Arclight Spanner".to_string(), false)
        ]
    );
    // With no hammer carried the line reads red.
    let empty = ObjectStore(ObjectFields::default());
    let bare = recipe_tools(
        &vest,
        None,
        Some(&categories),
        &empty,
        &objects,
        &items,
        &commands,
    );
    assert_eq!(bare[0], ("Blacksmith Hammer".to_string(), false));
}

/// `GetTradeSkillLine`'s rank and max carry the permanent skill bonus on each nonzero half
/// (`0x51a28a`); the difficulty and the trade link take the raw rank.
#[test]
fn the_rank_bar_adds_the_permanent_bonus() {
    let base = Field::PlayerSkillInfo11.index();
    let store = crate::app_core::session::ObjectStore(ObjectFields::from_pairs(&[
        (base, 171),
        (base + 1, 150 << 16 | 120),
        (base + 2, 10 << 16),
    ]));
    assert_eq!(super::super::skill_rank(&store, 171, false), (120, 150));
    assert_eq!(super::super::skill_rank(&store, 171, true), (130, 160));
}
