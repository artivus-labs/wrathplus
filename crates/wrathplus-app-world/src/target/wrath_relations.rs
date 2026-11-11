//! Proven branch topology of 12340 CanAttack 00729740 / CanAssist 007293d0.
//!
//! Not server spell legality. Callers supply both directional ranks of
//! [`reaction::unit_reaction`] (`007251c0`); `attack` carries the `UNIT_FIELD_FLAGS_2`
//! `0x10000` vehicle guard on [`Context::world`]. Missing creature metadata contributes no flags.

#[cfg(test)]
use std::marker::PhantomData;

use bevy::ecs::system::SystemParam;
use bevy::prelude::{Entity, Query, Res};

#[path = "wrath_reaction.rs"]
pub(super) mod reaction;
use crate::app_core::session::ObjectStore;
use crate::net::GuidIndex;
use reaction::{ReactionState, WorldInputs};
use wrathplus_protocol::types::{ObjectType, OwnerFallback};

/// A live descriptor by guid: the object manager's lookup (`0x468460`), asked afresh at every
/// controller hop. Never a second object cache.
pub trait StoreLookup: Sync {
    fn store(&self, guid: u64) -> Option<&ObjectStore>;

    /// The session tables beside the stores, for a lookup that holds them.
    fn world(&self) -> Option<ReactionState<'_>> {
        None
    }
}

/// A closure as a [`StoreLookup`], for a test whose stores are neither a query nor a param.
#[cfg(test)]
pub(crate) struct ByGuid<'a, F>(F, PhantomData<&'a ObjectStore>);

#[cfg(test)]
impl<'a, F: Fn(u64) -> Option<&'a ObjectStore> + Sync> ByGuid<'a, F> {
    pub(crate) fn new(lookup: F) -> Self {
        Self(lookup, PhantomData)
    }
}

#[cfg(test)]
impl<'a, F: Fn(u64) -> Option<&'a ObjectStore> + Sync> StoreLookup for ByGuid<'a, F> {
    fn store(&self, guid: u64) -> Option<&ObjectStore> {
        (self.0)(guid)
    }
}

/// No object streamed: every referenced owner stays missing.
pub(crate) struct NoStores;

impl StoreLookup for NoStores {
    fn store(&self, _: u64) -> Option<&ObjectStore> {
        None
    }
}

/// The guid index over the stores query, the lookup every Bevy caller holds.
pub struct GuidStores<'a, 'w, 's, 'q> {
    pub index: Option<&'a GuidIndex>,
    pub stores: &'a Query<'w, 's, &'q ObjectStore>,
}

impl StoreLookup for GuidStores<'_, '_, '_, '_> {
    fn store(&self, guid: u64) -> Option<&ObjectStore> {
        guid_store(self.index, self.stores, guid)
    }
}

/// [`GuidStores`]' one read, for a caller that implements [`StoreLookup`] on its own params.
pub fn guid_store<'a>(
    index: Option<&GuidIndex>,
    stores: &'a Query<&ObjectStore>,
    guid: u64,
) -> Option<&'a ObjectStore> {
    let entity: Entity = *index?.0.get(&guid)?;
    stores.get(entity).ok()
}

/// The live inputs as one system param, for a system that holds none of them already.
/// `Option`s: a harness without the net stack or the name cache resolves no owner and no flags.
#[derive(SystemParam)]
pub struct RelationInputs<'w, 's> {
    names: Option<Res<'w, crate::names::NameCache>>,
    index: Option<Res<'w, GuidIndex>>,
    stores: Query<'w, 's, &'static ObjectStore>,
    world: WorldInputs<'w>,
    /// The faction tables and standings the reaction reads beside the context.
    pub(super) factions: Option<Res<'w, super::Factions>>,
    reputations: Option<Res<'w, crate::net::Reputations>>,
}

impl StoreLookup for RelationInputs<'_, '_> {
    fn store(&self, guid: u64) -> Option<&ObjectStore> {
        guid_store(self.index.as_deref(), &self.stores, guid)
    }

    fn world(&self) -> Option<ReactionState<'_>> {
        Some(self.world.state())
    }
}

impl RelationInputs<'_, '_> {
    pub fn context(&self) -> Context<'_> {
        Context::new(self.names.as_deref(), self)
    }

    /// [`super::ring::ring_reaction_in`] on these live inputs: `target`'s reaction toward `me`.
    pub fn ring_reaction(&self, target: Option<&ObjectStore>, me: Option<&ObjectStore>) -> u8 {
        let (factions, reps) = (self.factions.as_deref(), self.reputations());
        super::ring::ring_reaction_in(factions, reps, target, me, self.context())
    }

    /// [`super::ring::can_interact_from_player`] on these live inputs.
    pub(crate) fn can_interact(
        &self,
        target: Option<&ObjectStore>,
        me: Option<&ObjectStore>,
    ) -> bool {
        let (factions, reps) = (self.factions.as_deref(), self.reputations());
        super::ring::can_interact_from_player(factions, reps, target, me, self.context())
    }

    pub(super) fn reputations(&self) -> &crate::net::Reputations {
        static NONE: crate::net::Reputations = crate::net::Reputations(Vec::new());
        self.reputations.as_deref().unwrap_or(&NONE)
    }
}

/// Live, caller-owned inputs of `CanAttack`/`CanAssist`: the creature template cache (for
/// query-response `type_flags`) and the guid lookup the controller resolver walks.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub(crate) names: Option<&'a crate::names::NameCache>,
    pub(crate) stores: &'a dyn StoreLookup,
    /// The group roster, forced reactions and vehicles, as `stores` supplies them.
    pub(crate) world: ReactionState<'a>,
}

impl<'a> Context<'a> {
    /// No template cache, no streamed owner and no session tables: creature metadata contributes
    /// no flags and every controller hop misses (a caller that holds none of them).
    pub fn none() -> Self {
        Self::new(None, &NoStores)
    }

    pub fn new(names: Option<&'a crate::names::NameCache>, stores: &'a dyn StoreLookup) -> Self {
        Self {
            names,
            stores,
            world: stores.world().unwrap_or_default(),
        }
    }

    /// Creature metadata flags off the template cache, for a Unit object only (a player has no
    /// creature record). Missing metadata contributes no flags.
    pub(crate) fn creature_flags(&self, unit: &ObjectStore) -> u32 {
        if unit.0.object_type() != Some(ObjectType::Unit) {
            return 0;
        }
        unit.0
            .object_entry()
            .and_then(|entry| self.names?.creature_record(entry))
            .map_or(0, |record| record.type_flags)
    }
}

const PC: u32 = 8;
const RAID_UNIT: u32 = 0x0400_0000;

/// Resolves CHARMEDBY, otherwise CREATEDBY, at most twice (00718b70). An unresolved
/// nonzero owner never falls back to the original unit. Direct players need no lookup.
pub(super) fn controller<'a>(
    unit: &'a ObjectStore,
    stores: &'a dyn StoreLookup,
) -> Option<&'a ObjectStore> {
    let mut current = unit;
    for _ in 0..2 {
        let Some(owner) = current.0.unit_owner(OwnerFallback::CreatedBy) else {
            return (current.0.object_type() == Some(ObjectType::Player)).then_some(current);
        };
        current = stores.store(owner)?;
        if current.0.object_type() == Some(ObjectType::Player) {
            return Some(current);
        }
    }
    None
}

/// `UNIT_FIELD_FLAGS_2`, 0 when absent.
fn flags_2(unit: &ObjectStore) -> u32 {
    unit.0
        .get(wrathplus_protocol::types::Field::UnitFlags2)
        .unwrap_or(0)
}

fn same_duel(actor: &ObjectStore, target: &ObjectStore) -> bool {
    actor.0.player_duel_team() != 0
        && target.0.player_duel_team() != 0
        && actor.0.player_duel_arbiter() == target.0.player_duel_arbiter()
}

/// Creature flags are query-response type_flags, never UNIT_FLAGS or NPC service bits.
pub(super) fn attack(
    actor: &ObjectStore,
    target: &ObjectStore,
    toward: u8,
    reverse: u8,
    ctx: Context,
) -> bool {
    let actor_creature_flags = ctx.creature_flags(actor);
    let (a, t) = (actor.0.unit_flags(), target.0.unit_flags());
    let (apc, tpc) = (a & PC != 0, t & PC != 0);
    if actor.0.object_type() == Some(ObjectType::Player) && actor.0.player_flags() & (1 << 19) != 0
    {
        return false;
    }
    if target.0.object_type() == Some(ObjectType::Player)
        && target.0.player_flags() & 0x10 != 0
        && actor_creature_flags & 2 == 0
    {
        return false;
    }
    if t & ((1 << 1) | (1 << 20) | (1 << 7) | (1 << 16) | (1 << 25)) != 0 {
        return false;
    }
    if (apc && t & 0x100 != 0)
        || (!apc && t & 0x200 != 0)
        || (tpc && a & 0x100 != 0)
        || (!tpc && a & 0x200 != 0)
    {
        return false;
    }
    // `0x729886`: either side's `UNIT_FIELD_FLAGS_2 & 0x10000` arms the vehicle leg on both.
    if (flags_2(actor) | flags_2(target)) & 0x1_0000 != 0
        && (ctx.world.guarded_vehicle(actor) || ctx.world.guarded_vehicle(target))
    {
        return false;
    }
    let friendly = toward >= 4 || actor_creature_flags & RAID_UNIT != 0;
    let sanctuary = actor.0.unit_is_in_sanctuary() || target.0.unit_is_in_sanctuary();
    match (apc, tpc) {
        (false, false) => toward < 2 || reverse < 2,
        (true, true) => {
            if friendly {
                return false;
            }
            if let (Some(ac), Some(tc)) = (
                controller(actor, ctx.stores),
                controller(target, ctx.stores),
            ) {
                if same_duel(ac, tc) {
                    return true;
                }
                if target.0.unit_is_pvp() {
                    return !sanctuary;
                }
                if actor.0.unit_is_ffa_pvp() && target.0.unit_is_ffa_pvp() {
                    return true;
                }
                let raw = actor.0.unit_pvp_state_flags().unwrap_or(0)
                    | target.0.unit_pvp_state_flags().unwrap_or(0);
                if raw & 2 == 0 {
                    return false;
                }
            }
            !sanctuary
        }
        _ => {
            !(apc && target.0.unit_is_in_sanctuary() || tpc && actor.0.unit_is_in_sanctuary())
                && !friendly
        }
    }
}

/// `CanAssist` (`0x7293d0`) with its third argument: `lenient` (1) skips the immune-to-PC/NPC test against the
/// actor's controller bit and the non-PvP refusal of a player-controlled actor (`0x7293f0`
/// branch), so only the not-selectable bit, the reaction, the duel and FFA arms and the
/// sanctuary arm decide.
pub(super) fn assist_in(
    actor: &ObjectStore,
    target: &ObjectStore,
    toward: u8,
    ctx: Context,
    lenient: bool,
) -> bool {
    let actor_creature_flags = ctx.creature_flags(actor);
    let (apc, tpc) = (
        actor.0.unit_flags() & PC != 0,
        target.0.unit_flags() & PC != 0,
    );
    let t = target.0.unit_flags();
    if t & (1 << 25) != 0 || (!lenient && t & if apc { 0x100 } else { 0x200 } != 0) {
        return false;
    }
    if toward < 4 && actor_creature_flags & RAID_UNIT == 0 {
        return false;
    }
    if tpc {
        if let (Some(ac), Some(tc)) = (
            controller(actor, ctx.stores),
            controller(target, ctx.stores),
        ) {
            if tc.0.player_duel_team() != 0
                && (ac.0.player_duel_arbiter() != tc.0.player_duel_arbiter()
                    || ac.0.player_duel_team() != tc.0.player_duel_team())
            {
                return false;
            }
        }
        if target.0.unit_is_ffa_pvp() && !actor.0.unit_is_ffa_pvp() {
            return false;
        }
        return !(actor.0.unit_is_in_sanctuary()
            && !target.0.unit_is_in_sanctuary()
            && target.0.unit_is_pvp());
    }
    lenient
        || !apc
        || target.0.unit_is_pvp()
        || ctx.creature_flags(target) & (0x1000 | RAID_UNIT) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strict mode every caller but the range test asks.
    fn assist(actor: &ObjectStore, target: &ObjectStore, toward: u8, ctx: Context) -> bool {
        assist_in(actor, target, toward, ctx, false)
    }
    use crate::names::{CreatureRecord, NameCache};
    use std::collections::HashMap;
    use wrathplus_protocol::types::{Field, ObjectFields};

    const ENTRY: u32 = 1423;

    fn unit(player: bool, flags: u32, pvp: u8, owner: u64) -> ObjectStore {
        let mut pairs = vec![];
        for (field, value) in [
            (Field::ObjectType, if player { 0x19 } else { 9 }),
            (Field::ObjectEntry, if player { 0 } else { ENTRY }),
            (Field::UnitFlags, flags),
            (Field::UnitBytes2, u32::from(pvp) << 8),
        ] {
            pairs.push((field.span().unwrap().index, value));
        }
        let index = Field::UnitCreatedby.span().unwrap().index;
        pairs.extend([(index, owner as u32), (index + 1, (owner >> 32) as u32)]);
        ObjectStore(ObjectFields::from_pairs(&pairs))
    }

    fn names(type_flags: u32) -> NameCache {
        let mut names = NameCache::default();
        names.insert_creature(
            ENTRY,
            Some(CreatureRecord {
                name: "Stormwind Guard".into(),
                subname: None,
                creature_type: 7,
                pet_family: 0,
                rank: 0,
                type_flags,
                civilian: false,
                racial_leader: false,
                display_id: 0,
                modifiers: None,
            }),
        );
        names
    }

    #[test]
    fn controller_resolves_two_distinct_hops_and_never_reuses_a_missing_owner() {
        let pet = unit(false, PC, 0, 11);
        let guardian = unit(false, PC, 0, 22);
        let player = unit(true, PC, 0, 0);
        let visits = std::sync::Mutex::new(Vec::new());
        let lookup = ByGuid::new(|guid| {
            visits.lock().unwrap().push(guid);
            match guid {
                11 => Some(&guardian),
                22 => Some(&player),
                _ => None,
            }
        });
        let resolved = controller(&pet, &lookup);
        assert_eq!(*visits.lock().unwrap(), [11, 22]);
        assert_eq!(resolved.unwrap().0.object_type(), Some(ObjectType::Player));
        assert!(controller(&unit(true, PC, 0, 99), &NoStores).is_none());
        // A guardian that names itself again never resolves past the second hop.
        let looping = unit(false, PC, 0, 11);
        assert!(controller(&pet, &ByGuid::new(|_| Some(&looping))).is_none());
        let unreachable =
            ByGuid::new(|_| -> Option<&ObjectStore> { panic!("direct player lookup") });
        assert!(controller(&player, &unreachable).is_some());
    }

    #[test]
    fn creature_metadata_is_read_off_a_unit_template_only() {
        let names = names(0x1000);
        let ctx = Context::new(Some(&names), &NoStores);
        assert_eq!(ctx.creature_flags(&unit(false, 0, 0, 0)), 0x1000);
        assert_eq!(
            ctx.creature_flags(&unit(true, 0, 0, 0)),
            0,
            "a player has no record"
        );
        assert_eq!(Context::none().creature_flags(&unit(false, 0, 0, 0)), 0);
    }

    #[test]
    fn assist_creature_metadata_is_not_a_unit_flag_or_owner_pvp_shortcut() {
        let actor = unit(true, PC, 0, 0);
        let target = unit(false, 0, 0, 99);
        let pvp_owner = unit(true, PC, 1, 0);
        let owner = ByGuid::new(|_| Some(&pvp_owner));
        for metadata in [0, 0x1000, RAID_UNIT] {
            let names = names(metadata);
            let ctx = Context::new(Some(&names), &owner);
            assert_eq!(assist(&actor, &target, 4, ctx), metadata != 0);
        }
        // UNIT_FLAGS 0x1000 is not the creature CAN_ASSIST bit.
        assert!(!assist(
            &actor,
            &unit(false, 0x1000, 0, 0),
            4,
            Context::none()
        ));
        assert!(assist(&actor, &unit(false, 0, 1, 0), 4, Context::none()));
        // Lenient (`0x7293d0`'s third argument 1): a non-PvP creature passes for a PvP actor.
        assert!(assist_in(&actor, &target, 4, Context::none(), true));
        assert!(!assist(&actor, &target, 4, Context::none()));
        let names = names(0x1000);
        assert!(!assist(
            &actor,
            &target,
            3,
            Context::new(Some(&names), &NoStores)
        ));
    }

    #[test]
    fn assist_refuses_a_target_whose_controller_duels_someone_else() {
        let actor = unit(true, PC, 0, 0);
        let at = |field: Field| field.span().unwrap().index;
        let duellist = ObjectStore(ObjectFields::from_pairs(&[
            (at(Field::ObjectType), 0x19),
            (at(Field::UnitFlags), PC),
            (at(Field::PlayerDuelTeam), 1),
        ]));
        let pet = unit(false, PC, 0, 7);
        let stores = HashMap::from([(7u64, duellist)]);
        let lookup = ByGuid::new(|guid| stores.get(&guid));
        assert!(!assist(&actor, &pet, 4, Context::new(None, &lookup)));
        assert!(
            assist(&actor, &pet, 4, Context::none()),
            "an unresolved owner skips the duel leg"
        );
    }

    #[test]
    fn native_player_attack_ffa_precedes_sanctuary_but_target_pvp_does_not() {
        let none = Context::none();
        let actor = unit(true, PC, 4 | 8, 0);
        let target = unit(true, PC, 4, 0);
        assert!(attack(&actor, &target, 1, 1, none));
        assert!(!attack(&actor, &unit(true, PC, 1 | 4, 0), 1, 1, none));
        assert!(!attack(
            &unit(true, PC, 0, 0),
            &unit(true, PC, 0, 0),
            1,
            1,
            none
        ));
        assert!(attack(
            &unit(true, PC, 2, 0),
            &unit(true, PC, 0, 0),
            1,
            1,
            none
        ));
    }

    #[test]
    fn attack_resolves_a_pets_controller_before_the_pvp_legs() {
        let me = unit(true, PC, 0, 0);
        let pet = unit(false, PC, 0, 5);
        let owner = unit(true, PC, 0, 0);
        let lookup = ByGuid::new(|guid| (guid == 5).then_some(&owner));
        // Both controllers resolve and nothing flags PvP: refused.
        assert!(!attack(&me, &pet, 1, 1, Context::new(None, &lookup)));
        // The owner never streamed: the missing-controller branch only checks sanctuary.
        assert!(attack(&me, &pet, 1, 1, Context::none()));
    }
}
