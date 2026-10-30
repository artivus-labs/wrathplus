//! Build 12340's trainer window laws: the icon (`GetTrainerServiceIcon 00595150`) reads the
//! service spell itself, where the tooltip (`SetTrainerService 0062e900`) hops through a learn
//! effect to the taught spell, so the two can name different spells on one row. Do not unify them.

use wrathplus_formats::{
    SkillLineCatalog, SpellCatalog, SpellDisplay, SPELL_ATTR_IS_TRADESKILL,
    SPELL_EFFECT_LEARN_PET_SPELL, SPELL_EFFECT_LEARN_SPELL, SPELL_EFFECT_SKILL_STEP,
};
use wrathplus_protocol::types::trainer_spell_state;
use wrathplus_ui::script::{TrainerServiceCategory, TrainerTooltip};

use crate::entities::ItemDisplays;
use crate::items::Items;
use crate::net::NetCommands;
use crate::ui_items::item_icon;

use super::TRAINER_TYPE_TRADESKILL;

/// `Effect[0]` creating an item: `CREATE_ITEM` (24), `CREATE_ITEM_2` (157) or
/// `CREATE_RANDOM_ITEM` (59), the item arm of the icon, tooltip and description.
fn creates_item(d: &SpellDisplay) -> bool {
    matches!(d.effects[0], 24 | 157 | 59)
}

/// `GetTrainerServiceIcon 00595150`: at a tradeskill trainer (`[0xc0e498] == 2`), a service
/// spell whose `Effect[0]` creates an item shows that item's icon; otherwise the spell's own
/// `SpellIconID`. Labelled: the reference reads the item's display id off the client's `Item.dbc`
/// (`0065c290`); the template cache stands in, `None` until its answer re-fires `TRAINER_UPDATE`.
/// Unlike 1.12, no learn effect is followed: 3.3.5 lists the taught spell itself.
pub(super) fn service_icon(
    wire_spell: u32,
    trainer_type: u32,
    spells: &SpellCatalog,
    icons: Option<&ItemDisplays>,
    items: &Items,
    commands: &NetCommands,
) -> Option<String> {
    let wire = spells.get(wire_spell)?;
    if trainer_type == TRAINER_TYPE_TRADESKILL && creates_item(wire) {
        return items
            .template(wire.effect_item_type[0], 0, commands)
            .map(|t| t.display_info_id)
            .and_then(|d| item_icon(icons, d));
    }
    wire.icon.clone()
}

/// `SetTrainerService 0062e900` picks the subject the tooltip builders describe: the first learn
/// effect (36, or 57 for the pet caster) whose taught spell resolves; an unresolvable one moves to
/// the next slot. The taught spell shows as its item when it is a trade spell whose `Effect[0]`
/// creates a nonzero `EffectItemType[0]`, else as itself; with no learn effect, the service spell.
pub(super) fn service_tooltip(wire_spell: u32, spells: &SpellCatalog) -> TrainerTooltip {
    let wire_only = TrainerTooltip::Spell {
        spell_id: wire_spell,
        alt_caster: false,
    };
    let Some(wire) = spells.get(wire_spell) else {
        return wire_only;
    };
    for i in 0..3 {
        let effect = wire.effects[i];
        if effect != SPELL_EFFECT_LEARN_SPELL && effect != SPELL_EFFECT_LEARN_PET_SPELL {
            continue;
        }
        let trigger = wire.effect_trigger_spell[i];
        let Some(taught) = spells.get(trigger) else {
            continue;
        };
        let item = taught.effect_item_type[0];
        if taught.attributes & SPELL_ATTR_IS_TRADESKILL != 0 && creates_item(taught) && item != 0 {
            return TrainerTooltip::Item(item);
        }
        return TrainerTooltip::Spell {
            spell_id: trigger,
            alt_caster: effect == SPELL_EFFECT_LEARN_PET_SPELL,
        };
    }
    // 12340's spell builder `0x6239ae`/`0x62526a` appends the product of a direct trade recipe.
    let item_id = wire.effect_item_type[0];
    if wire.attributes & SPELL_ATTR_IS_TRADESKILL != 0 && creates_item(wire) && item_id != 0 {
        return TrainerTooltip::Recipe {
            spell_id: wire_spell,
            item_id,
        };
    }
    wire_only
}

/// `SPELL_ATTR0_PASSIVE`, the tradeskill builder's third group.
const SPELL_ATTR_PASSIVE: u32 = 0x40;
/// `TARGET_UNIT_PET`: a learn effect aimed at the pet keys nothing.
const TARGET_UNIT_PET: u32 = 5;

/// The group key of 12340's list builder `0x596450`, with its header name (`00594e80`).
/// - A tradeskill trainer (`[0xc0e498] == 2`) keys on the service spell: 3 when it is passive
///   (`Attributes & 0x40`), else 1 when any effect steps a skill (44), else 2, also for a spell
///   with no record. The header is the `[0xa13e08]` key's string, `TRADESKILL_SERVICE_STEP`,
///   `_LEARN` or `_PASSIVE`.
/// - Every other type keys on `0x594ae0`: the `SkillLineAbility` line (`0x71a700`) of the taught
///   spell, which is the trigger of the first `LEARN_SPELL` (36) effect not aimed at the pet,
///   else the service spell itself. The header is the `SkillLine.dbc` name, and 0 drops the
///   service. There is no known-services group, which is 1.12's.
pub(super) fn service_group(
    wire_spell: u32,
    trainer_type: u32,
    spells: &SpellCatalog,
    skill_lines: Option<&SkillLineCatalog>,
    // The VM's own `GlobalStrings.lua`, for the header labels.
    get: &dyn Fn(&str) -> Option<String>,
) -> (u32, String) {
    let display = spells.get(wire_spell);
    if trainer_type == TRAINER_TYPE_TRADESKILL {
        let key = match display {
            Some(d) if d.attributes & SPELL_ATTR_PASSIVE != 0 => 3,
            Some(d) if d.effects.contains(&SPELL_EFFECT_SKILL_STEP) => 1,
            _ => 2,
        };
        let label = [
            "",
            "TRADESKILL_SERVICE_STEP",
            "TRADESKILL_SERVICE_LEARN",
            "TRADESKILL_SERVICE_PASSIVE",
        ][key];
        return (key as u32, get(label).unwrap_or_default());
    }
    let taught = display
        .and_then(|d| {
            (0..3)
                .find(|&i| {
                    d.effects[i] == SPELL_EFFECT_LEARN_SPELL
                        && d.effect_implicit_target_a[i] != TARGET_UNIT_PET
                })
                .map(|i| d.effect_trigger_spell[i])
        })
        .unwrap_or(wire_spell);
    let line = skill_lines
        .and_then(|c| c.spell_to_line(taught))
        .unwrap_or(0);
    if line == 0 {
        return (0, String::new());
    }
    let name = skill_lines
        .and_then(|c| c.line(line))
        .map(|l| l.name.clone())
        .unwrap_or_else(|| format!("Skill {line}"));
    (line, name)
}

/// A wire `state` byte's category (`00594260`): 0 "available", 2 "used", anything else
/// "unavailable".
pub(super) fn category(state: u8) -> TrainerServiceCategory {
    match state {
        trainer_spell_state::GRAY => TrainerServiceCategory::Used,
        trainer_spell_state::GREEN => TrainerServiceCategory::Available,
        _ => TrainerServiceCategory::Unavailable,
    }
}
