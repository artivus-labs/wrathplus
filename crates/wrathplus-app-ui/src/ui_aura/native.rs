//! Packet auras use native polarity and full/remaining durations, never descriptor slots.
use std::collections::BTreeSet;

use crate::app_core::session::ObjectStore;
use crate::app_core::state::PlayerActions;
use crate::net::wrath_auras::StampedAura;
use wrathplus_formats::{SpellCatalog, SpellDisplay};
use wrathplus_ui::script::AuraState;

/// Native writer 00727e70 only includes effects marked present in the received aura.
/// An unloaded spell row is explicitly treated as no 295; catalog arrival rebuilds lists.
pub(super) fn has_active_aura_295(catalog: Option<&SpellCatalog>, auras: &[StampedAura]) -> bool {
    auras.iter().any(|stamp| {
        catalog
            .and_then(|c| c.get(stamp.aura.spell_id))
            .is_some_and(|spell| {
                spell
                    .effect_apply_aura
                    .iter()
                    .enumerate()
                    .any(|(i, &kind)| kind == 295 && stamp.aura.flags & (1 << i) != 0)
            })
    })
}

/// Shared UnitBuff/UnitDebuff/UnitAura guard (006147c0/00514050), not CanAssist.
pub(super) fn all_visible(has_295: bool, local_guid: u64, reaction: u8, type_flags: u32) -> bool {
    !has_295 || local_guid == 0 || reaction >= 4 || type_flags & 0x0400_0000 != 0
}

/// What [`list`] reads beside the auras: the catalog, the object manager's lookup for a caster's
/// links (`0x60b6a6`), and the player's learned spells and the dispel types they remove, which
/// the `RAID` list reads as each aura enters it (`0x72c8a1`).
pub(super) struct ListCtx<'a> {
    pub(super) catalog: Option<&'a SpellCatalog>,
    pub(super) held: &'a dyn Fn(u64) -> Option<&'a ObjectStore>,
    pub(super) known: Option<&'a BTreeSet<u32>>,
    pub(super) dispels: &'a [u32],
}

impl<'a> ListCtx<'a> {
    pub(super) fn new(
        catalog: Option<&'a SpellCatalog>,
        held: &'a dyn Fn(u64) -> Option<&'a ObjectStore>,
        actions: Option<&'a PlayerActions>,
        dispels: &'a [u32],
    ) -> Self {
        let known = actions.map(|a| &a.spells);
        ListCtx {
            catalog,
            held,
            known,
            dispels,
        }
    }
}

#[cfg(test)]
fn nothing_held<'a>(_: u64) -> Option<&'a ObjectStore> {
    None
}

/// A context over `catalog` alone: no objects, no spells.
#[cfg(test)]
pub(super) fn bare(catalog: Option<&SpellCatalog>) -> ListCtx<'_> {
    ListCtx::new(catalog, &nothing_held, None, &[])
}

/// `0x542170`: each learned spell's `SPELL_EFFECT_DISPEL` (38) not aimed at an enemy (implicit
/// target A 6) adds its misc value to the dispel types `0x53bdf0` searches. Rebuilt into `out`
/// when the catalog (`catalog_moved`) or the book moved.
pub(super) fn dispel_types(
    out: &mut Vec<u32>,
    catalog: Option<&SpellCatalog>,
    actions: Option<&bevy::prelude::Res<PlayerActions>>,
    catalog_moved: bool,
) {
    if !catalog_moved && !actions.is_some_and(bevy::prelude::DetectChanges::is_changed) {
        return;
    }
    out.clear();
    let known = actions.into_iter().flat_map(|a| &a.spells);
    for d in known.filter_map(|&id| catalog?.get(id)) {
        for i in 0..3 {
            let misc = d.effect_misc_value[i] as u32;
            if d.effects[i] == 38 && d.effect_implicit_target_a[i] != 6 && !out.contains(&misc) {
                out.push(misc);
            }
        }
    }
}

/// The `RAID` list's test (`0x72c8a1`): a helpful aura of a learned spell `0x53d8a0` lets reach
/// others (no `AttributesEx3 & 0x4000`, implicit target A not the caster, no area or persistent
/// area effect), or a harmful one whose dispel type the player removes (`0x53bdf0`).
fn in_raid_list(ctx: &ListCtx, d: Option<&SpellDisplay>, spell_id: u32, helpful: bool) -> bool {
    const AREA: [u32; 7] = [0x1b, 0x23, 0x41, 0x77, 0x80, 0x81, 0x8f];
    d.is_some_and(|d| match helpful {
        true => {
            ctx.known.is_some_and(|k| k.contains(&spell_id))
                && d.attributes_ex3 & 0x4000 == 0
                && d.effect_implicit_target_a[0] != 1
                && !d.effects.iter().any(|e| AREA.contains(e))
        }
        false => ctx.dispels.contains(&d.dispel),
    })
}

/// `unit`'s list: the record's caster is the wire's, or `unit` itself under `AFLAG_CASTER`
/// (`0x716553`); a roster record's has none, so it passes `unit` 0 and casters of 0.
pub(super) fn list(
    ctx: &ListCtx,
    unit: u64,
    auras: &[StampedAura],
    all_visible: bool,
    now: f64,
    script_now: f64,
) -> Vec<AuraState> {
    if !all_visible {
        return Vec::new();
    }
    auras
        .iter()
        .filter(|s| super::shown_in_aura_ui(ctx.catalog, s.aura.spell_id))
        .map(|s| {
            let a = &s.aura;
            let d = ctx.catalog.and_then(|c| c.get(a.spell_id));
            let mut state = super::unit_aura_state(ctx.catalog, 0, a.spell_id, a.stacks, 0);
            // The search's sign and cancel tests read flag bits 0x80 and 0x10 (`0x72caa1`,
            // `0x72cad0`).
            state.helpful = a.flags & 0x80 == 0;
            state.cancelable = a.flags & 0x10 != 0;
            state.duration = a.max_duration_ms.map_or(0.0, |ms| f64::from(ms) / 1000.0);
            state.expiration_time = a.remaining_ms.map_or(0.0, |ms| {
                script_now + s.received_at + f64::from(ms) / 1000.0 - now
            });
            state.until_cancelled = a.remaining_ms.is_none();
            state.caster = a.caster_guid.unwrap_or(unit);
            if let Some(c) = (ctx.held)(state.caster).filter(|c| c.is_unit()) {
                let c = &c.0;
                state.caster_links = [
                    c.unit_charmed_by().or(c.unit_created_by()).unwrap_or(0),
                    c.unit_charm().or(c.unit_summon()).unwrap_or(0),
                ];
            }
            state.rank = d.and_then(|d| d.rank.clone());
            state.consolidate = d.is_some_and(|d| d.attributes_ex7 & 0x1000_0000 != 0);
            state.raid = in_raid_list(ctx, d, a.spell_id, state.helpful);
            state
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_formats::SpellDisplay;
    use wrathplus_protocol::wrath_auras::NativeAura;
    #[test]
    fn native_295_guard_uses_present_effects_and_missing_rows_are_visible() {
        let catalog = SpellCatalog::from_displays(std::collections::HashMap::from([(
            123,
            wrathplus_formats::SpellDisplay {
                effect_apply_aura: [0, 295, 0],
                ..Default::default()
            },
        )]));
        let stamp = |flags| StampedAura {
            aura: NativeAura {
                slot: 1,
                spell_id: 123,
                flags,
                level: 80,
                stacks: 1,
                caster_guid: None,
                max_duration_ms: None,
                remaining_ms: None,
            },
            received_at: 0.0,
        };
        let missing = SpellCatalog::from_displays(Default::default());
        assert!(!has_active_aura_295(Some(&missing), &[stamp(2)]));
        assert!(!has_active_aura_295(None, &[stamp(2)]));
        assert!(!has_active_aura_295(Some(&catalog), &[stamp(1)]));
        assert!(has_active_aura_295(Some(&catalog), &[stamp(2)]));
        for slot in 0..3 {
            let mut effect_apply_aura = [0; 3];
            effect_apply_aura[slot] = 295;
            let catalog = SpellCatalog::from_displays(std::collections::HashMap::from([(
                123,
                wrathplus_formats::SpellDisplay {
                    effect_apply_aura,
                    ..Default::default()
                },
            )]));
            for present in 0..3 {
                assert_eq!(
                    has_active_aura_295(Some(&catalog), &[stamp(1 << present)]),
                    slot == present
                );
            }
        }
        assert!(!all_visible(true, 7, 3, 0));
        assert!(all_visible(true, 7, 4, 0));
        assert!(all_visible(true, 0, 3, 0));
        assert!(all_visible(true, 7, 3, 0x04000000));
        assert!(all_visible(false, 7, 3, 0));
        assert!(list(
            &bare(Some(&catalog)),
            0,
            &[stamp(2), stamp(0x82)],
            false,
            0.0,
            0.0
        )
        .is_empty());
    }

    #[test]
    fn native_slots_do_not_encode_polarity_and_duration_is_not_remaining() {
        let mut aura = NativeAura {
            slot: 63,
            spell_id: 123,
            flags: 0x10,
            level: 80,
            stacks: 0,
            caster_guid: Some(9),
            max_duration_ms: Some(30000),
            remaining_ms: Some(18000),
        };
        let stamp = StampedAura {
            aura: aura.clone(),
            received_at: 4.0,
        };
        let list = list(&bare(None), 5, &[stamp], true, 7.0, 100.0);
        assert!(list[0].helpful);
        assert_eq!(list[0].caster, 9, "the wire's caster");
        assert_eq!(list[0].count, 0);
        assert_eq!(list[0].duration, 30.0);
        assert_eq!(list[0].expiration_time, 115.0);
        aura.slot = 0;
        aura.flags = 0x90;
        aura.remaining_ms = None;
        aura.max_duration_ms = None;
        aura.caster_guid = None;
        let stamp = |aura| StampedAura {
            aura,
            received_at: 4.0,
        };
        let list = super::list(&bare(None), 5, &[stamp(aura.clone())], true, 7.0, 100.0);
        assert!(!list[0].helpful, "0x72caa1 reads 0x80");
        assert!(list[0].cancelable, "0x72cad0 reads 0x10 alone");
        assert_eq!(list[0].expiration_time, 0.0);
        assert!(list[0].until_cancelled);
        assert_eq!(
            list[0].caster, 5,
            "AFLAG_CASTER: the unit itself (0x716558)"
        );

        // The record's rank and consolidate bit, a held caster's links, and the RAID tests.
        let display = |rank: &str, ex3, ex7, target_a, dispel, effects| SpellDisplay {
            rank: Some(rank.into()),
            attributes_ex3: ex3,
            attributes_ex7: ex7,
            effect_implicit_target_a: [target_a, 0, 0],
            effects,
            dispel,
            ..Default::default()
        };
        let catalog = SpellCatalog::from_displays(std::collections::HashMap::from([
            (1, display("Rank 2", 0, 0x1000_0000, 21, 0, [6, 0, 0])),
            (2, display("", 0, 0, 1, 0, [6, 0, 0])),
            (3, display("", 0x4000, 0, 21, 0, [6, 0, 0])),
            (4, display("", 0, 0, 21, 0, [35, 0, 0])),
            (5, display("", 0, 0, 6, 1, [6, 0, 0])),
            (6, display("", 0, 0, 6, 2, [6, 0, 0])),
        ]));
        use wrathplus_protocol::types::{Field, ObjectFields};
        let totem = ObjectStore(ObjectFields::from_pairs(&[
            (Field::ObjectType.index(), 0x9),
            (Field::UnitCreatedby.index(), 0x10),
            (Field::UnitCreatedby.index() + 1, 0),
        ]));
        let held = |g: u64| (g == 0x77).then_some(&totem);
        let known = std::collections::BTreeSet::from([1, 2, 3, 4]);
        let actions = PlayerActions {
            spells: known,
            ..Default::default()
        };
        let ctx = ListCtx::new(Some(&catalog), &held, Some(&actions), &[1]);
        let auras: Vec<_> = (1..=6)
            .map(|spell_id| NativeAura {
                spell_id,
                flags: if spell_id < 5 { 0x10 } else { 0x80 },
                caster_guid: Some(0x77),
                ..aura.clone()
            })
            .map(stamp)
            .collect();
        let list = super::list(&ctx, 5, &auras, true, 7.0, 100.0);
        let a = &list[0];
        assert_eq!((a.rank.as_deref(), a.consolidate), (Some("Rank 2"), true));
        assert_eq!(a.caster_links, [0x10, 0], "CREATEDBY, no pet (0x60b6c0)");
        let raid: Vec<bool> = list.iter().map(|a| a.raid).collect();
        assert_eq!(
            raid,
            [true, false, false, false, true, false],
            "0x53d8a0, 0x53bdf0"
        );
    }
}
