//! The units the Lua range test reads (`IsSpellInRange 0x541c60`, `IsItemInRange 0x51c9c0`): each
//! held unit's position, reach and motion (`0x7ff480`, `0x803850`) and its relations to the
//! player (`0x809610`'s arms), on the same inputs the cast binder asks.

use wrathplus_ui::script::RangeUnitFacts;

use super::{owner_store, CastTargeting, GroupInputs, TargetRelations};
use crate::creature_type::CreatureTypeSources;
use crate::target::RelationContext;

impl CastTargeting<'_, '_> {
    /// `guid`'s [`RangeUnitFacts`], `None` while it has no streamed unit, store or position.
    pub fn range_unit_facts(&self, guid: u64) -> Option<RangeUnitFacts> {
        let entity = *self.index.as_deref()?.0.get(&guid)?;
        let store = self.stores.get(entity).ok()?;
        let pos = self.transforms.get(entity).ok()?.translation;
        let unit = self.range_units.unit(entity)?;
        let self_store = self.self_store.iter().next();
        let rel = TargetRelations {
            target_store: Some(store),
            target_owner_store: owner_store(Some(store), self.index.as_deref(), &self.stores),
            self_store,
            relations: RelationContext::new(self.names.as_deref(), self),
            factions: self.factions.as_deref(),
            reputations: &self.reputations,
            types: CreatureTypeSources {
                names: self.names.as_deref(),
                forms: self.spells.as_deref().map(|s| &s.forms),
            },
            group: GroupInputs {
                self_guid: self.self_guid.0,
                target_guid: Some(guid),
                self_owner_store: owner_store(self_store, self.index.as_deref(), &self.stores),
                roster: self.roster.as_deref(),
            },
        };
        let is_self = self.self_guid.0 == Some(guid);
        let lenient = is_self
            || crate::target::relations::can_assist_in(
                rel.target_store,
                rel.factions,
                rel.reputations,
                rel.self_store,
                rel.relations,
                true,
            );
        let f = &store.0;
        Some(RangeUnitFacts {
            pos: [pos.x, pos.y, pos.z],
            reach: unit.reach,
            player: unit.player,
            running: unit.motion.moves_at_run_speed(),
            health: f.unit_health().unwrap_or(0),
            unit_flags: f.unit_flags(),
            dynamic_flags: f.unit_dynamic_flags(),
            creature_type: rel.types.of(store),
            pet_number: f.unit_pet_number(),
            assist: [rel.assistable(is_self), lenient],
            attack: rel.attackable(is_self),
            party: rel.in_party(is_self),
            raid: rel.in_group(is_self),
        })
    }
}
