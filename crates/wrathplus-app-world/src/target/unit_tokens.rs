//! The unit-token resolver, world state only (the guid index, the pet bar's guid, the hover pick,
//! the group roster and the object stores), so world systems resolve `"target"`, `"party1"` or
//! `"mouseover"` without the UI layer; `ui_unit` re-exports it from its old path.

use bevy::prelude::*;
use wrathplus_ui::script::{parse_unit_token, UnitBase, UnitTokenParse};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::PetBar;
use crate::app_core::state::Selection;
use crate::app_core::{group::GroupState, raid_rows::raid_row_guid};
use crate::net::{Guid, GuidIndex};
use crate::target::{Hovered, HoveredObject};

/// The one unit-token resolver, the reference's `0x515970`: case-insensitive compares, then the
/// object manager; a caller wanting a type tests the resolved unit. [`Selection`] is a parameter
/// because [`crate::target::SelectCommit`] holds it as `ResMut`. `npc` is recognised but
/// unresolved here, a quiet nil.
#[derive(bevy::ecs::system::SystemParam)]
pub struct UnitTokens<'w, 's> {
    /// `Option`, like the two below: a UI-only harness lacks their plugins; a bare `Res` panics.
    index: Option<Res<'w, GuidIndex>>,
    pet: Option<Res<'w, PetBar>>,
    hovered: Option<Res<'w, Hovered>>,
    hovered_go: Option<Res<'w, HoveredObject>>,
    group: Res<'w, GroupState>,
    arena: Option<Res<'w, crate::app_core::arena::ArenaOpponents>>,
    pub stores: Query<'w, 's, &'static ObjectStore>,
    me: Query<'w, 's, (Entity, &'static Guid), With<SelfPlayer>>,
}

impl UnitTokens<'_, '_> {
    fn me(&self) -> Option<(Entity, u64)> {
        self.me.iter().next().map(|(e, g)| (e, g.0))
    }

    /// The guid lookup (`0x468460`), `pub(crate)` for `TargetLastEnemy`, which starts from a guid.
    pub fn held(&self, guid: u64) -> Option<(Entity, u64)> {
        Some((*self.index.as_ref()?.0.get(&guid)?, guid))
    }

    /// Resolve `token` as `0x515970` does: its base, then a hop for each `target` after it, the
    /// text read by [`parse_unit_token`]. The VM resolves through build 12340's `0x60abf0`, which
    /// also takes `pet` suffixes and group members' names. A token that names nobody, or a base
    /// or hop the object manager does not hold, is `None`.
    pub fn resolve(&self, token: &str, selection: &Selection) -> Option<(Entity, u64)> {
        let UnitTokenParse::Unit { base, hops } = parse_unit_token(token) else {
            return None;
        };
        let mut unit = self.base(base, selection)?;
        for _ in 0..hops {
            unit = self.target_of(unit.0)?;
        }
        Some(unit)
    }

    /// The held unit a token's base names, before any `target` hop.
    fn base(&self, base: UnitBase, selection: &Selection) -> Option<(Entity, u64)> {
        match base {
            UnitBase::Player => self.me(),
            UnitBase::Target => selection.target.zip(selection.guid),
            // The same pick `ui_tooltip` pushes `"mouseover"` from.
            UnitBase::Mouseover => self
                .hovered
                .as_ref()?
                .mouseover(&self.hovered_go.as_deref().copied().unwrap_or_default()),
            // Off the pet bar's cached guid, as the `"pet"` snapshot reads it.
            UnitBase::Pet => {
                let guid = self.pet.as_ref()?.guid();
                (guid != 0).then(|| self.held(guid)).flatten()
            }
            UnitBase::PartyPet(row) => self
                .party_member(row)
                .and_then(|member| self.pet_of(member, false)),
            UnitBase::RaidPet(row) => self
                .raid_member(row)
                .and_then(|member| self.pet_of(member, true)),
            UnitBase::Raid(row) => self.raid_member(row).and_then(|guid| self.held(guid)),
            UnitBase::Party(row) => self.party_member(row).and_then(|guid| self.held(guid)),
            // The arena feed's slots (`0xbe9f48`, `0xbe9ef8`), held or nobody.
            UnitBase::Arena(row) => self.arena_slot(row, false),
            UnitBase::ArenaPet(row) => self.arena_slot(row, true),
            // The interaction NPC is unresolved here, a quiet nil; so are the Wrath bases this
            // client does not yet track.
            UnitBase::Npc | UnitBase::Focus | UnitBase::Vehicle | UnitBase::Boss(_) => None,
        }
    }

    /// One `target` hop (`0x515a0f`-`0x515a25`): the held unit's `UNIT_FIELD_TARGET`, held in turn.
    /// The lookup takes the unit typemask, so a hop off any other object, or off a unit with no
    /// target, is nobody.
    fn target_of(&self, unit: Entity) -> Option<(Entity, u64)> {
        let store = self.stores.get(unit).ok().filter(|s| s.is_unit())?;
        self.held(store.0.unit_target()?)
    }

    /// The held opponent, or its stored pet, in 0-based arena slot `row`.
    fn arena_slot(&self, row: u32, pet: bool) -> Option<(Entity, u64)> {
        let table = self.arena.as_ref()?;
        let slots = if pet { &table.pets } else { &table.arena };
        let guid = *slots.get(usize::try_from(row).ok()?)?;
        (guid != 0).then(|| self.held(guid)).flatten()
    }

    /// The guid in 0-based party slot `row`, the one `partyN` and `partypetN` share.
    fn party_member(&self, row: u32) -> Option<u64> {
        let slot = usize::try_from(row).ok()?;
        self.group.party_slots().nth(slot).map(|m| m.guid)
    }

    /// The guid on 0-based raid row `row`, the one `raidN` and `GetRaidRosterInfo` share.
    fn raid_member(&self, row: u32) -> Option<u64> {
        let index = usize::try_from(row).ok()?.checked_add(1)?;
        raid_row_guid(&self.group, self.me().map(|(_, g)| g), index)
    }

    /// A group member's pet, held: `CHARM`, else `SUMMON`, off the member's descriptor while it is
    /// held, else off the roster record's pet guid (`partypetN` `0x4e81d0`, `raidpetN` `0x491960`).
    /// A pet the object manager does not hold names no unit here, as any unheld unit.
    fn pet_of(&self, member: u64, raid: bool) -> Option<(Entity, u64)> {
        let live = self
            .held(member)
            .and_then(|(entity, _)| self.stores.get(entity).ok())
            .map(|store| &store.0);
        let pet = if raid {
            self.group.raid_pet_guid(member, live)
        } else {
            self.group.party_pet_guid(member, live)
        };
        self.held(pet?)
    }
}

/// Resolve a UnitPopup token to the other player's guid it names: `"target"` when it is a player,
/// `"partyN"` through the roster; `"player"` and anything unresolved answer `None`.
pub fn player_token_guid(token: &str, selection: &Selection, group: &GroupState) -> Option<u64> {
    match token {
        "target" => selection
            .guid
            .filter(|g| wrathplus_protocol::guid::is_player(*g)),
        "player" => None,
        tok => tok
            .strip_prefix("party")
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| group.party_slots().nth(n))
            .map(|m| m.guid),
    }
}
