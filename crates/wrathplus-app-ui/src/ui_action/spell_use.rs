//! The live inputs of the Lua spell and item use queries (`IsUsableSpell 0x541680`,
//! `IsSpellInRange 0x541c60`, `IsUsableItem 0x51c450`, `GetItemCooldown 0x510fc0` and kin), each
//! diff-pushed per frame: the book spells' usable pairs (the pet casts the pet's book), the
//! items' pairs, cooldowns, use-spell enchants and charges, the units and range modifiers the
//! range test reads, the auto-repeat words and the item a cast came from. Spell.dbc facts ride
//! the catalog ([`catalog_facts`]) and the templates' with them ([`push_templates`]).

use std::collections::HashMap;

use bevy::prelude::*;

use wrathplus_formats::SpellDisplay;
use wrathplus_ui::script::{
    ItemUseFacts, ItemUseSpellFacts, ItemUseView, RangeRow, RangeUnitFacts, SpellInfoView,
    SpellRangeFacts, SpellUseView, UiScript,
};

use super::Spells;
use crate::app_core::cooldowns::Cooldowns;
use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::inventory::{carried_counts, collect_inventory, InventoryScope};
use crate::items::Items;
use crate::net::NetCommands;
use crate::spell::usable;

type RangeMods = HashMap<u32, (i32, i32)>;
/// The range test's pushed inputs: unit facts, op 5 modifiers by spell, ranged item scale.
type RangeInputs = (HashMap<u64, RangeUnitFacts>, RangeMods, Option<f32>);

/// What the feed last pushed, for the diffs.
#[derive(Default)]
pub(super) struct UseMemory {
    spells: Option<(HashMap<u32, SpellUseView>, HashMap<u32, SpellUseView>)>,
    items: Option<HashMap<u32, ItemUseView>>,
    range: Option<RangeInputs>,
    auto_repeat: Option<[Option<u32>; 2]>,
    cast_items: Option<Vec<u32>>,
    statics: bool,
}

/// The Spell.dbc columns the use queries read, as [`SpellInfoView`]'s tail: `IsAttackSpell`'s
/// `Effect[0]` (`0x541420`), the classifier `0x7fe1b0`, the reagents, and the range test's facts
/// with the `SpellRange.dbc` row's hostile and friendly pairs (`0x7ff480`).
pub(crate) fn catalog_facts(d: &SpellDisplay, spells: &Spells) -> SpellInfoView {
    let row = spells.ranges.get(d.range_index).map(|r| {
        let friendly = spells
            .ranges
            .friendly(d.range_index)
            .unwrap_or((r.min, r.max));
        RangeRow {
            flags: r.flags,
            min: [r.min, friendly.0],
            max: [r.max, friendly.1],
        }
    });
    SpellInfoView {
        attack: d.is_melee_auto_attack(),
        polarity: polarity(d),
        reagents: d
            .reagents
            .iter()
            .copied()
            .filter(|&(id, _)| id != 0)
            .collect(),
        range: SpellRangeFacts {
            targets: d.targets,
            implicit_a: d.effect_implicit_target_a,
            creature_types: d.target_creature_type,
            attributes: d.attributes,
            attributes_ex2: d.attributes_ex2,
            attributes_ex5: d.attributes_ex5,
            attributes_ex6: d.attributes_ex6,
            row,
        },
        ..Default::default()
    }
}

/// `0x7fe1b0`: 1 when the spell aims at friends, 2 at enemies, 0 neither. `Targets & 0x100`
/// friends; `& 0x80` enemies; an enemy implicit target A or B; then, per effect, a friendly A or
/// B, or the caster (1) with an aura other than the dummy (4).
fn polarity(d: &SpellDisplay) -> u8 {
    if d.targets & 0x100 != 0 {
        return 1;
    }
    if d.targets & 0x80 != 0 {
        return 2;
    }
    let ab = |i: usize| [d.effect_implicit_target_a[i], d.effect_implicit_target_b[i]];
    let enemy = |t: u32| matches!(t, 2 | 6 | 0xf | 0x10 | 0x18 | 0x1c | 0x35 | 0x36 | 0x5d);
    if (0..3).any(|i| ab(i).into_iter().any(enemy)) {
        return 2;
    }
    let friend = |t: u32| {
        matches!(
            t,
            3 | 4
                | 5
                | 0x14
                | 0x15
                | 0x1b
                | 0x1d
                | 0x1e
                | 0x1f
                | 0x21
                | 0x22
                | 0x23
                | 0x2d
                | 0x38
                | 0x39
                | 0x3a
                | 0x3b
                | 0x3d
                | 0x3e
        )
    };
    let helps = |i: usize| {
        ab(i)
            .into_iter()
            .any(|t| friend(t) || t == 1 && d.effect_apply_aura[i] != 4)
    };
    u8::from((0..3).any(helps))
}

/// Push templates with their [`ItemUseFacts`]: `BagFamily` and the on-use spell blocks.
pub(crate) fn push_templates(
    script: &mut UiScript,
    views: Vec<(u32, wrathplus_ui::script::ItemTemplateView)>,
    items: &Items,
) {
    for (id, view) in views {
        script.set_item_template(id, view);
        if let Some(t) = items.template_cached(id) {
            script.set_wrath_item_use_facts(id, item_facts(t));
        }
    }
}

fn item_facts(t: &wrathplus_protocol::types::ItemInfo) -> ItemUseFacts {
    ItemUseFacts {
        bag_family: t.bag_family,
        use_spells: t
            .spells
            .iter()
            .filter(|s| s.trigger == 0 && s.spell_id != 0)
            .map(|s| ItemUseSpellFacts {
                spell: s.spell_id,
                charges: s.charges,
                cooldown_ms: s.cooldown_ms,
                category_cooldown_ms: s.category_cooldown_ms,
            })
            .collect(),
    }
}

/// The ranged slot (`[player+0x18f4]` slot 17, `0x7ff480`).
const RANGED_SLOT: u8 = 17;

/// Computes and diff-pushes the use queries' live inputs.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system's full input set
pub(super) fn feed_spell_use(
    script: Option<NonSendMut<UiScript>>,
    spells: Option<Res<Spells>>,
    targeting: crate::spell::cast_target::CastTargeting,
    walk: (
        Res<Cooldowns>,
        Res<crate::spell::SpellModifiers>,
        Res<crate::net::wrath_combo_points::NativeComboPoints>,
        crate::target::RelationInputs,
        Option<Res<crate::app_core::state::Factions>>,
        Res<crate::net::Reputations>,
    ),
    clock: Res<crate::app_core::ui_flags::UiClock>,
    casts: (
        Res<crate::spell::AutoRepeatActive>,
        Res<crate::spell::LastItemCast>,
        Res<crate::spell::ActiveChannel>,
        Query<&crate::creature_anim::Casting, With<SelfPlayer>>,
        Res<crate::app_core::session::SelfGuid>,
        Res<crate::spell::PendingCast>,
    ),
    stores: Query<&ObjectStore>,
    objects: crate::net::Objects,
    items: Res<Items>,
    commands: Res<NetCommands>,
    statics: (
        Option<Res<crate::ui_native::wrath_sockets::Gems>>,
        Option<Res<crate::items::ItemSubClasses>>,
    ),
    mut memory: Local<crate::app_core::vm_memo::VmMemo<UseMemory>>,
) {
    let (Some(mut script), Some(spells)) = (script, spells) else {
        return;
    };
    let memory = memory.get(&script);
    let (cooldowns, mods, combo, relations, factions, reputations) = &walk;
    let gems = statics.0.as_deref().map(|g| &g.0);
    if !memory.statics {
        memory.statics = true;
        if let Some(g) = gems {
            script.set_wrath_gem_enchants(g.gem_enchants().collect());
        }
        if let Some(s) = statics.1.as_deref() {
            script.set_wrath_subclass_masks(s.0.mask_groups().to_vec());
        }
    }
    let Some(me) = targeting.self_store.iter().next() else {
        return;
    };
    let target_store = targeting.selection.target.and_then(|e| stores.get(e).ok());
    let carried = carried_counts(&me.0, &objects);
    let ctx_for = |store| usable::UsableCtx {
        store,
        target_store,
        factions: factions.as_deref(),
        reputations,
        cooldowns,
        carried: &carried,
        spell_mods: mods,
        combo,
        relations: relations.context(),
    };
    let me_guid = casts.4 .0;
    let pet_guid = me.0.unit_summon().or(me.0.unit_charm()).filter(|&g| g != 0);
    let pet_store = pet_guid
        .and_then(|g| objects.entity(g))
        .and_then(|e| stores.get(e).ok());

    // The books' pairs, each with its caster.
    let (player_ids, pet_ids) = script.wrath_spell_use_book_ids();
    let me_ctx = ctx_for(me);
    let pet_ctx = pet_store.map(ctx_for);
    let pair = |id: u32, ctx: &usable::UsableCtx| -> Option<(u32, SpellUseView)> {
        let d = spells.catalog.get(id)?;
        let (usable, not_enough_mana) =
            usable::spell_usable(id, d, &spells, ctx, &objects, &items, &commands);
        Some((
            id,
            SpellUseView {
                usable,
                not_enough_mana,
            },
        ))
    };
    let book = (
        player_ids
            .iter()
            .filter_map(|&id| pair(id, &me_ctx))
            .collect(),
        pet_ctx.as_ref().map_or_else(HashMap::new, |p| {
            pet_ids.iter().filter_map(|&id| pair(id, p)).collect()
        }),
    );

    // The items: every carried or worn copy's entry, then every other cached template's
    // cooldown.
    let now = std::time::Instant::now();
    let item_views = item_views(&ItemInputs {
        me,
        carried: &carried,
        objects: &objects,
        items: &items,
        spells: &spells,
        cooldowns,
        gems,
        clock: &clock,
        now,
        usable: &|id, d| usable::spell_usable(id, d, &spells, &me_ctx, &objects, &items, &commands),
    });

    // The range inputs: the player, the pet and every held unit; the books' and the items'
    // range modifiers; the ranged slot's scale.
    let mut guids = script.held_unit_guids();
    guids.extend(me_guid);
    guids.extend(pet_guid);
    let units: HashMap<u64, RangeUnitFacts> = guids
        .into_iter()
        .filter_map(|g| Some((g, targeting.range_unit_facts(g)?)))
        .collect();
    let item_spells = item_views.keys().filter_map(|&id| {
        let t = items.template_cached(id)?;
        t.use_spell.as_ref().map(|u| u.spell_id)
    });
    let range_mods: HashMap<u32, (i32, i32)> = player_ids
        .iter()
        .chain(&pet_ids)
        .copied()
        .chain(item_spells)
        .filter_map(|id| {
            let m = mods.modifiers(spells.catalog.get(id)?, crate::spell::OP_RANGE)?;
            Some((id, (m.flat, m.pct)))
        })
        .collect();
    let ranged_mod =
        me.0.player_inv_slot(RANGED_SLOT)
            .and_then(|g| objects.object(g))
            .and_then(|o| o.object_entry())
            .and_then(|entry| items.template_cached(entry))
            .map(|t| t.ranged_mod_range);
    let range = (units, range_mods, ranged_mod);

    let (auto_repeat, last_item, channel, casting, _, pending) = &casts;
    // `0xd397cc` is the autoRangedCombat queue (`0x7fe140` from `0x6e2be0`), which the client
    // does not run, so it stays empty.
    let auto = [auto_repeat.0, None];
    let current = casting
        .iter()
        .next()
        .map(|c| c.spell_id)
        .or(channel.current(now));
    // The client's spell object holds the item from the `CMSG_USE_ITEM` send until the reply
    // ([`crate::spell::PendingCast::item_use`]), then through the cast bar or channel the
    // `SMSG_SPELL_START` named it for.
    let mut cast_items: Vec<u32> = pending
        .item_use(now)
        .into_iter()
        .chain(last_item.0.filter(|&(spell, _)| current == Some(spell)))
        .filter_map(|(_, guid)| objects.object(guid)?.object_entry())
        .collect();
    cast_items.dedup();

    if memory.spells.as_ref() != Some(&book) {
        script.set_wrath_spell_use(book.0.clone(), book.1.clone());
        memory.spells = Some(book);
    }
    if memory.items.as_ref() != Some(&item_views) {
        script.set_wrath_item_use(item_views.clone());
        memory.items = Some(item_views);
    }
    if memory.range.as_ref() != Some(&range) {
        script.set_wrath_range_inputs(range.0.clone(), range.1.clone(), range.2);
        memory.range = Some(range);
    }
    if memory.auto_repeat != Some(auto) {
        script.set_wrath_auto_repeat_spell(auto);
        memory.auto_repeat = Some(auto);
    }
    if memory.cast_items.as_ref() != Some(&cast_items) {
        script.set_wrath_cast_items(cast_items.clone());
        memory.cast_items = Some(cast_items);
    }
}

/// What [`item_views`] reads.
struct ItemInputs<'a, 'w, 's> {
    me: &'a ObjectStore,
    carried: &'a HashMap<u32, u32>,
    objects: &'a crate::net::Objects<'w, 's>,
    items: &'a Items,
    spells: &'a Spells,
    cooldowns: &'a Cooldowns,
    gems: Option<&'a wrathplus_formats::gems::GemCatalog>,
    clock: &'a crate::app_core::ui_flags::UiClock,
    now: std::time::Instant,
    /// The usable walk with the player as the caster.
    usable: &'a dyn Fn(u32, &SpellDisplay) -> (bool, bool),
}

/// The items' [`ItemUseView`]s by entry: for each carried or worn entry the pair, the cooldown,
/// the worn copy's use-spell enchant and the copies' charges; for any other cached template with
/// a use spell, its cooldown.
fn item_views(i: &ItemInputs<'_, '_, '_>) -> HashMap<u32, ItemUseView> {
    let copies = |scope| {
        let mut by_entry: HashMap<u32, Vec<(u8, u8, u64)>> = HashMap::new();
        for (bag, slot, guid) in collect_inventory(&i.me.0, i.objects, scope) {
            if let Some(entry) = i.objects.object(guid).and_then(|o| o.object_entry()) {
                by_entry.entry(entry).or_default().push((bag, slot, guid));
            }
        }
        by_entry
    };
    let held = copies(InventoryScope::DEFAULT);
    let banked = copies(InventoryScope::QUEST_ITEMS);
    let mut out = HashMap::new();
    for &entry in held.keys().chain(i.carried.keys()) {
        if out.contains_key(&entry) {
            continue;
        }
        let t = i.items.template_cached(entry);
        let enchant = t
            .filter(|t| t.use_spell.is_none())
            .and_then(|_| worn_enchant(i, held.get(&entry).map_or(&[][..], Vec::as_slice)));
        let spell = t.and_then(|t| t.use_spell.as_ref()).map(|u| u.spell_id);
        let spell = spell.or(enchant.map(|(s, _)| s));
        let d = spell.and_then(|s| i.spells.catalog.get(s));
        let category = t
            .and_then(|t| t.use_spell.as_ref())
            .map_or(0, |u| u.category);
        let category = if category == 0 {
            d.map_or(0, |d| d.category)
        } else {
            category
        };
        // `0x80ac10`: an on-hold record refuses; a spell with no record is usable (`0x51c450`).
        let (usable, not_enough_mana) = match (spell, d) {
            (Some(s), _) if i.cooldowns.has_on_hold_record(s, entry, category) => (false, false),
            (Some(s), Some(d)) => (i.usable)(s, d),
            _ => (true, false),
        };
        let charges = |copies: Option<&Vec<(u8, u8, u64)>>| -> u32 {
            copies.map_or(0, |c| {
                c.iter().map(|&(_, _, g)| copy_charges(i, t, g)).sum()
            })
        };
        out.insert(
            entry,
            ItemUseView {
                usable,
                not_enough_mana,
                cooldown: cooldown(i, spell, entry, d),
                enchant_spell: enchant,
                charges: [charges(held.get(&entry)), charges(banked.get(&entry))],
            },
        );
    }
    for entry in i.items.cached_template_ids() {
        if out.contains_key(&entry) {
            continue;
        }
        let Some(u) = i
            .items
            .template_cached(entry)
            .and_then(|t| t.use_spell.as_ref())
        else {
            continue;
        };
        let d = i.spells.catalog.get(u.spell_id);
        if let Some(cd) = cooldown(i, Some(u.spell_id), entry, d) {
            out.insert(
                entry,
                ItemUseView {
                    cooldown: Some(cd),
                    ..Default::default()
                },
            );
        }
    }
    out
}

/// The use spell's cooldown (`0x807980`) on the frame's one clock pair.
fn cooldown(
    i: &ItemInputs<'_, '_, '_>,
    spell: Option<u32>,
    entry: u32,
    d: Option<&SpellDisplay>,
) -> Option<(i64, u32, bool)> {
    let info = i.cooldowns.info(spell?, entry, d, i.now);
    info.ui_triple(i.clock.anchor, i.clock.ui_now)
}

/// A worn copy's use-spell enchant (`0x5a86b0`): the first worn copy whose enchant spell has
/// charges left (walk flags `0xa1`), else the first worn copy (`0x81`), through `0x707c60`'s
/// enchant slots in order.
fn worn_enchant(i: &ItemInputs<'_, '_, '_>, copies: &[(u8, u8, u64)]) -> Option<(u32, i32)> {
    let gems = i.gems?;
    let enchant_of = |guid: u64| -> Option<(u32, i32, u32)> {
        let o = i.objects.object(guid)?;
        (0..12u8).find_map(|slot| {
            let id = o.item_enchant(slot).filter(|&e| e > 0)? as u32;
            gems.use_spell(id)
                .map(|(s, c)| (s, c, o.item_enchant_charges(slot)))
        })
    };
    let worn: Vec<(u32, i32, u32)> = copies
        .iter()
        .filter(|&&(bag, slot, _)| bag == 255 && slot < 19)
        .filter_map(|&(_, _, g)| enchant_of(g))
        .collect();
    worn.iter()
        .find(|e| e.2 != 0)
        .or(worn.first())
        .map(|&(s, c, _)| (s, c))
}

/// One copy's `|charges|` (`0x753f60` → `0x707dc0`): its use block's `ITEM_FIELD_SPELL_CHARGES`,
/// or for a template without one its use-spell enchant slot's charges.
fn copy_charges(
    i: &ItemInputs<'_, '_, '_>,
    t: Option<&wrathplus_protocol::types::ItemInfo>,
    guid: u64,
) -> u32 {
    let Some(o) = i.objects.object(guid) else {
        return 0;
    };
    match t.and_then(|t| t.use_spell_index()) {
        Some(block) => o.item_spell_charges(block).unwrap_or(0).unsigned_abs(),
        None => (0..12u8)
            .find(|&slot| {
                let id = o.item_enchant(slot).filter(|&e| e > 0);
                id.zip(i.gems)
                    .is_some_and(|(e, g)| g.use_spell(e as u32).is_some())
            })
            .map_or(0, |slot| o.item_enchant_charges(slot)),
    }
}

#[cfg(test)]
#[path = "spell_use_tests.rs"]
mod tests;
