//! Cast contexts keep candidate identity, relations and range inputs together.
use super::*;

impl CastTargeting<'_, '_> {
    /// This frame's [`CastContext`].
    pub fn context(&self) -> CastContext<'_> {
        self.context_at(None)
    }

    /// An explicit unit replaces the candidate, never the player's selection. `Some(None)`
    /// retains zero intent without replacing it with an unrelated selection.
    pub fn context_at(&self, explicit: Option<Option<u64>>) -> CastContext<'_> {
        let guid = explicit.unwrap_or(self.selection.guid);
        let entity = match explicit {
            None => self.selection.target,
            Some(_) => guid.and_then(|g| self.index.as_ref()?.0.get(&g).copied()),
        };
        let target_store = entity.and_then(|e| self.stores.get(e).ok());
        let owner_store = |store| owner_store(store, self.index.as_deref(), &self.stores);
        let self_store = self.self_store.iter().next();
        let rel = TargetRelations {
            target_store,
            target_owner_store: owner_store(target_store),
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
                target_guid: guid,
                self_owner_store: owner_store(self_store),
                roster: self.roster.as_deref(),
            },
        };
        CastContext {
            selection_guid: guid,
            explicit_guid: explicit.flatten(),
            self_guid: self.self_guid.0,
            auto_self_cast: self.auto_self_cast.0,
            rel,
            range: RangeInputs {
                friendly: rel.assistable(guid == self.self_guid.0),
                ranged_mod: self.range_units.ranged_mod(),
                attack_target: self.range_units.caster_attack_target(),
                self_pos: self.self_transform.iter().next().map(|t| t.translation),
                target_pos: entity
                    .and_then(|e| self.transforms.get(e).ok())
                    .map(|t| t.translation),
                caster: self.range_units.caster(),
                target: entity
                    .and_then(|e| self.range_units.unit(e))
                    .map(crate::spell::cast_range::RangeTarget::Unit)
                    .unwrap_or_default(),
            },
            main_hand_item: self
                .self_store
                .iter()
                .next()
                .and_then(|s| s.0.player_inv_slot(EQUIPMENT_SLOT_MAINHAND))
                .filter(|&g| g != 0),
            self_move_flags: self.player.as_deref().map_or(0, |p| p.move_flags()),
        }
    }
}
