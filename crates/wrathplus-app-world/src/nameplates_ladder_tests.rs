//! The `ShouldShowName` ladder's test, split out of `nameplates.rs`.

use super::*;

/// `0x729c70`'s kind ladder over the registered defaults (guild, PvP title, both reactions'
/// player and pet names on; NPC, totems, guardians, non-combat off).
#[test]
fn the_unit_should_show_name_ladder() {
    use crate::vplates::UnitSort;
    let subject = |sort| NameSubject {
        sort,
        friendly: false,
        attackable: false,
        stealthed: false,
        ours: false,
        creator_flag_200: false,
    };
    let cfg = NameConfig::default();
    assert!(cfg.shows(&subject(UnitSort::Player)), "enemy player on");
    assert!(!cfg.shows(&NameSubject {
        stealthed: true,
        ..subject(UnitSort::Player)
    }));
    assert!(cfg.shows(&NameSubject {
        friendly: true,
        stealthed: true,
        ..subject(UnitSort::Player)
    }));
    assert!(!cfg.shows(&subject(UnitSort::Creature)), "UnitNameNPC is 0");
    assert!(cfg.shows(&subject(UnitSort::Pet)), "pets are on");
    assert!(!cfg.shows(&NameSubject {
        stealthed: true,
        attackable: true,
        ..subject(UnitSort::Pet)
    }));
    let npc_on = NameConfig(name_bit::NPC);
    assert!(npc_on.shows(&subject(UnitSort::Creature)));
    assert!(
        !npc_on.shows(&subject(UnitSort::NonCombat)),
        "critters have their own"
    );
    // A guardian answers to the enemy bit whatever its reaction.
    let friendly_guardians = NameConfig(name_bit::FRIENDLY_GUARDIAN);
    let friendly_guardian = NameSubject {
        friendly: true,
        ..subject(UnitSort::Guardian)
    };
    assert!(!friendly_guardians.shows(&friendly_guardian));
    assert!(NameConfig(name_bit::ENEMY_GUARDIAN).shows(&friendly_guardian));
    // Totems split on attackable; our pet is a friendly pet.
    let totems = NameConfig(name_bit::ENEMY_TOTEM);
    assert!(totems.shows(&NameSubject {
        attackable: true,
        ..subject(UnitSort::Totem)
    }));
    assert!(!totems.shows(&subject(UnitSort::Totem)));
    assert!(NameConfig(name_bit::FRIENDLY_PET).shows(&NameSubject {
        ours: true,
        ..subject(UnitSort::Pet)
    }));
    assert!(NameConfig(name_bit::NON_COMBAT).shows(&NameSubject {
        creator_flag_200: true,
        ..subject(UnitSort::Guardian)
    }));
    // `IsSelectable` (`0x71a390`); the Steam Cloud's flags are `0x02000300`.
    use wrathplus_protocol::types::Field;
    const ME: u64 = 0x1;
    const PET: u64 = 0x50;
    let flagged = |guid: u64, creator: u64| {
        ObjectStore(wrathplus_protocol::ObjectFields::from_pairs(&[
            (Field::ObjectGuid.index(), guid as u32),
            (Field::UnitFlags.index(), 0x0200_0300),
            (Field::UnitCreatedby.index(), creator as u32),
        ]))
    };
    let selectable =
        |s: ObjectStore| crate::target::relations::is_selectable(Some(&s), Some(ME), PET);
    assert!(!selectable(flagged(0x40, 0)), "the Steam Cloud");
    assert!(!selectable(flagged(0x40, ME)), "ours, not the pet");
    assert!(selectable(flagged(PET, ME)), "ours and the pet");
}
