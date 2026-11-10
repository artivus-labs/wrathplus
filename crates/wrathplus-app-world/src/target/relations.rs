//! The three unit-relationship predicates the reference shares across systems, may I attack,
//! interact with or help this unit, as the store-only entry points every consumer calls:
//!
//! - `CanAttack 0x606980` forwards to [`super::ring::can_attack_from_player`],
//! - `CanInteract 0x6067f0` forwards to [`super::ring::can_interact_from_player`],
//! - `CanAssist 0x6066f0` is derived here, on [`ring_reaction`].
//!
//! The first two live in [`super::ring`] beside the reaction directions they turn on:
//! `0x6061e0(this = player)` answers a reputation faction with the at-war bit,
//! `0x6061e0(this = unit)` with the standing, and the two often disagree.
//!
//! A corpse has its own reaction gate, `0x6067d0` ([`corpse_friendly`]).

use wrathplus_protocol::types::{Field, ObjectType, OwnerFallback};

use crate::app_core::session::ObjectStore;
use crate::net::Reputations;

use super::wrath_relations::Context;
use super::Factions;

/// `CanAttack 0x606980`, the one attackability question of the world cursor's sword
/// (`0x48269a`), the combat flash, the TAB scan's filter 3, `UnitCanAttack` and hostile spell
/// targeting. Its mixed arm is `UnitReaction(player → target) < 4`, which answers a reputation
/// faction with the at-war bit, never the standing, so a not-at-war neutral faction (Cenarion
/// Circle) is not attackable.
pub fn can_attack(
    store: Option<&ObjectStore>,
    factions: Option<&Factions>,
    reputations: &Reputations,
    self_store: Option<&ObjectStore>,
    ctx: Context,
) -> bool {
    super::ring::can_attack_from_player(factions, reputations, store, self_store, ctx)
}

/// `CanInteract 0x6067f0`, may I take a service from this unit: the world-cursor classifier runs it
/// at `0x482310` (through `CanInteractNow 0x606880`) to choose between the NPC service ladder and
/// the loot, skin and attack block.
pub fn can_interact(
    store: Option<&ObjectStore>,
    factions: Option<&Factions>,
    reputations: &Reputations,
    self_store: Option<&ObjectStore>,
    ctx: Context,
) -> bool {
    super::ring::can_interact_from_player(factions, reputations, store, self_store, ctx)
}

/// `UNIT_FLAG_NOT_SELECTABLE`, bit 25 (vmangos `UnitDefines.h:570`): `CanAssist`'s first refusal
/// and one of `CanAttack`'s.
const UNIT_FLAG_NOT_SELECTABLE: u32 = 1 << 25;

/// `IsSelectable`, CGUnit_C's and CGPlayer_C's vtable `+0xa8` (`0x6e6ec0`, a jump to `+0xa4`,
/// `0x71a390`): `UNIT_FLAG_NOT_SELECTABLE` clear, or the unit is both created by the active player
/// (`UNIT_FIELD_CREATEDBY` against `0x4d3790`) and slot 0 of PetInfo's pet list (`0x5d3390(0)`),
/// the `SMSG_PET_SPELLS` guid `0x5d6900` stores, here `pet_guid` (0: no pet). Unverified for
/// 12340: that the other vtables' slot is a `xor eax,eax` stub (1.12 `0x469fe0`), so a GameObject,
/// item or corpse never becomes the selection.
///
/// `None` passes: `SetSelection` makes the call (`0x524cfa`) only on an object the manager
/// resolved (`0x524cd7`), which keeps an out-of-range party member selectable. A store whose
/// `OBJECT_FIELD_TYPE` has not streamed passes the same way.
pub(crate) fn is_selectable(
    store: Option<&ObjectStore>,
    self_guid: Option<u64>,
    pet_guid: u64,
) -> bool {
    let Some(store) = store else {
        return true; // no resolved object: the reference skips the call (`0x524cd7`)
    };
    // Only CGUnit_C and CGPlayer_C override the slot (the 1.12 reading above).
    if !matches!(
        store.0.object_type(),
        None | Some(ObjectType::Unit) | Some(ObjectType::Player)
    ) {
        return false;
    }
    store.0.unit_flags() & UNIT_FLAG_NOT_SELECTABLE == 0
        || (self_guid.is_some()
            && store.0.unit_created_by() == self_guid
            && store.0.get_field_guid(Field::ObjectGuid, 0) == Some(pet_guid))
}

/// `UNIT_FLAG_POSSESSED`, `UNIT_FIELD_FLAGS` bit 24 (vmangos `UnitDefines.h:569`), which the
/// reference reads as `[[unit+0x110]+0xA3] & 1`.
pub const UNIT_FLAG_POSSESSED: u32 = 0x0100_0000;

/// Whether the active player possesses this unit (Mind Control, an Eye of Kilrogg, Eyes of the
/// Beast): `UNIT_FLAG_POSSESSED` set and `UNIT_FIELD_CHARMEDBY`, else `UNIT_FIELD_CREATEDBY`, the
/// active player's guid. The own-pet resolver `0x5ee5a0` (`0x5ee626`, `0x5ee62f`) and the world
/// pick's filter `0x480610` (`0x48064f`-`0x480674`) read this same clause. No store, no owner or
/// no active player is no possession.
pub fn is_possessed_by(store: Option<&ObjectStore>, self_guid: Option<u64>) -> bool {
    let (Some(store), Some(me)) = (store, self_guid) else {
        return false;
    };
    store.0.unit_flags() & UNIT_FLAG_POSSESSED != 0
        && store.0.unit_owner(OwnerFallback::CreatedBy) == Some(me)
}

/// `0x6067d0(player, corpse)`, the corpse's reaction gate: `0x6064c0` compares the player's
/// template (`UNIT_FIELD_FACTIONTEMPLATE`) toward the dead player's race's (`0x5d7120`: the race
/// byte of `CORPSE_FIELD_BYTES_1`, then `ChrRaces.dbc` column 2) with the template comparator
/// `0x606640` alone, no reputation, and passes at friendly (`setge` 4). A missing row on either
/// side is neutral (`0x60651b`), so it fails, as does a missing catalog or player.
pub fn corpse_friendly(
    corpse: &ObjectStore,
    factions: Option<&Factions>,
    self_store: Option<&ObjectStore>,
) -> bool {
    use wrathplus_formats::Reaction;
    let reaction = (|| {
        let catalog = factions?.catalog();
        let own = catalog.template(self_store?.0.unit_faction_template()?)?;
        Some(own.reaction_toward(catalog.race_template(corpse.0.corpse_race())?))
    })();
    reaction.unwrap_or(Reaction::Neutral) >= Reaction::Friendly
}

/// `CanAssist 007293d0` (Lua `UnitCanAssist`) from the local player: 12340's gate
/// ([`super::wrath_relations::assist_in`] strict), on the player's reaction toward the unit and `ctx`'s live
/// owner lookup and creature metadata.
pub fn can_assist(
    store: Option<&ObjectStore>,
    factions: Option<&Factions>,
    reputations: &Reputations,
    self_store: Option<&ObjectStore>,
    ctx: Context,
) -> bool {
    can_assist_in(store, factions, reputations, self_store, ctx, false)
}

/// [`can_assist`] with `0x7293d0`'s third argument ([`super::wrath_relations::assist_in`]).
pub fn can_assist_in(
    store: Option<&ObjectStore>,
    factions: Option<&Factions>,
    reputations: &Reputations,
    self_store: Option<&ObjectStore>,
    ctx: Context,
    lenient: bool,
) -> bool {
    let (Some(store), Some(actor)) = (store, self_store) else {
        return false;
    };
    let toward = super::ring::reaction_between(
        factions,
        reputations,
        self_store,
        Some(store),
        self_store,
        ctx,
    );
    super::wrath_relations::assist_in(actor, store, toward, ctx, lenient)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spell::targeting::corpse_fixture as fx;

    /// `0x6067d0`: the caster's template toward the corpse's race's, friendly or better; every gap
    /// is neutral (`0x60651b`) and fails.
    #[test]
    fn the_corpse_gate_reads_the_race_template() {
        let factions = fx::factions();
        let me = fx::caster(1.5);
        let gate = |corpse: &ObjectStore| corpse_friendly(corpse, Some(&factions), Some(&me));
        assert!(
            gate(&fx::corpse(fx::HUMAN, false)),
            "a human toward a human"
        );
        assert!(!gate(&fx::corpse(fx::ORC, false)), "a human toward an orc");
        assert!(!gate(&fx::corpse(0, false)), "race 0 has no ChrRaces row");
        assert!(
            !gate(&ObjectStore(Default::default())),
            "no BYTES_1 reads race 0"
        );
        let human = fx::corpse(fx::HUMAN, false);
        assert!(!corpse_friendly(&human, None, Some(&me)), "no catalog");
        assert!(!corpse_friendly(&human, Some(&factions), None), "no player");
        let no_template = ObjectStore(wrathplus_protocol::ObjectFields::from_fields(&[(
            wrathplus_protocol::types::Field::UnitFactionTemplate,
            999,
        )]));
        assert!(
            !corpse_friendly(&human, Some(&factions), Some(&no_template)),
            "a template the table lacks"
        );
    }

    /// The same gate on the shipped `ChrRaces.dbc` and `FactionTemplate.dbc`: a human (template 1)
    /// stands friendly toward every Alliance race's corpse and not toward a Horde one's.
    #[test]
    fn the_corpse_gate_splits_the_sides_on_the_shipped_tables() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let factions = Factions::from_catalog(
            wrathplus_formats::load_faction_catalog(&mut chain).expect("faction tables"),
        );
        let me = fx::caster(1.5);
        let gate = |race| corpse_friendly(&fx::corpse(race, false), Some(&factions), Some(&me));
        for race in [1, 3, 4, 7] {
            assert!(gate(race), "Alliance race {race}");
        }
        for race in [2, 5, 6, 8] {
            assert!(!gate(race), "Horde race {race}");
        }
        assert!(!gate(0) && !gate(10), "a race with no row");
    }

    /// `0x480658`-`0x480667`: the flag, then the charmer if there is one, else the creator; an
    /// unset owner or an unknown active player is no possession.
    #[test]
    fn possession_is_the_flag_and_the_charmer_else_the_creator() {
        let store = |flags: u32, charmed_by: u32, created_by: u32| {
            ObjectStore(wrathplus_protocol::ObjectFields::from_field_slots(&[
                (wrathplus_protocol::types::Field::UnitFlags, 0, flags),
                (
                    wrathplus_protocol::types::Field::UnitCharmedby,
                    0,
                    charmed_by,
                ),
                (wrathplus_protocol::types::Field::UnitCharmedby, 1, 0),
                (
                    wrathplus_protocol::types::Field::UnitCreatedby,
                    0,
                    created_by,
                ),
                (wrathplus_protocol::types::Field::UnitCreatedby, 1, 0),
            ]))
        };
        let me = Some(0x77);
        let flag = UNIT_FLAG_POSSESSED;
        assert!(
            is_possessed_by(Some(&store(flag, 0x77, 0)), me),
            "charmed by us"
        );
        assert!(
            is_possessed_by(Some(&store(flag, 0, 0x77)), me),
            "created by us"
        );
        assert!(
            !is_possessed_by(Some(&store(flag, 0x99, 0x77)), me),
            "a charmer other than us comes before our creation"
        );
        assert!(!is_possessed_by(Some(&store(0, 0x77, 0x77)), me), "no flag");
        assert!(!is_possessed_by(Some(&store(flag, 0, 0)), me), "no owner");
        assert!(
            !is_possessed_by(Some(&store(flag, 0, 0)), None),
            "no active player"
        );
        assert!(!is_possessed_by(None, me), "no descriptor");
    }
}
