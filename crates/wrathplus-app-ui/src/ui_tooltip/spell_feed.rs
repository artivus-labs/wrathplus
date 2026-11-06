//! The spell tooltip views: the builder `0x52e610`'s cells resolved here where the catalogs live,
//! and the feed that pushes a view for every spell the UI can hover before it is hovered. The
//! reference builds at the hover and keeps nothing; the feed keeps what each pushed view read
//! ([`super::spell_deps`]) and builds again only the views a change of the player's state reaches.

use std::cell::RefCell;
use std::collections::HashMap;

use bevy::prelude::*;

use wrathplus_ui::script::UiScript;
use wrathplus_ui::strings::Arg;

use super::keyed;
use super::spell_deps::{Chance, Changes, Deps, RangeSeen, Reagents, Seen, UnitField};
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::PlayerActions;
use crate::app_core::state::Spells;
use crate::inventory::count_of;
use crate::inventory::InventoryScope;
use crate::items::Items;
use crate::net::{NetCommands, Objects};
use crate::spell::usable::{self, CostBasis};
use crate::ui_trainer::TrainerTooltipSubjects;

/// The unit a view's caster cells are computed against, the spell builder `0x52e610`'s fourth
/// argument: the level the `$`-tokens, the cost and the cast time scale by, and the cost's basis.
#[derive(Clone, Copy)]
pub(crate) enum ViewCaster<'a> {
    /// The active player ([`ViewCtx::store`]), selector 0.
    Player,
    /// Selector 1: the player's charm, else its summon (`0x6e3159`-`0x6e317f`, `0x6e31fe`-
    /// `0x6e3228`), `None` when neither is streamed.
    Pet(Option<&'a wrathplus_protocol::ObjectFields>),
}

impl ViewCaster<'_> {
    /// The pet's `[vtbl+0xa8]`, `CGUnit`'s `0x60cd80`: `UNIT_FIELD_LEVEL × 5`, which
    /// [`wrathplus_formats::SpellDisplay::skill_level`] caps at `maxLevel × 5` and divides by 5 as
    /// `0x60cdb2`-`0x60cdb9` and `0x6e3195` do. No unit reads 0 (`0x6e31a4`).
    fn pet_skill_value(pet: Option<&wrathplus_protocol::ObjectFields>) -> u32 {
        pet.and_then(|p| p.unit_level())
            .unwrap_or(0)
            .saturating_mul(5)
    }
}

/// The caster-dependent inputs of a spell view: the worn set (`0x5f0c50`), the bags, the form and
/// the bind point `$z` names.
pub(crate) struct ViewCtx<'a, 'w, 's> {
    pub(super) home_area: Option<&'a str>,
    /// The caster's shapeshift form, which the required-form line's colour reads (`0x52f1f2`).
    pub(super) form: u8,
    /// The active player.
    pub(super) store: Option<&'a ObjectStore>,
    /// Whom the level terms and the cost read: the player or its pet. The form and the reaches
    /// are that same unit's.
    pub(super) caster: ViewCaster<'a>,
    /// The caster as `0x7ff480` reads it: its `UNIT_FIELD_COMBATREACH` (1.5 is the descriptor
    /// default) and its motion.
    pub(super) range_caster: wrathplus_formats::RangeUnit,
    /// The caster's auto-attack target, which `0x7ff480` looks up itself (`[caster+0xc48]`) for
    /// its melee arm alone, reading its reach and motion; with none, the caster's reach counts
    /// twice and there is no moving bonus.
    pub(super) attack_target: Option<wrathplus_formats::RangeUnit>,
    /// The object index the worn-item search and each reagent's carried count resolve through.
    pub(super) objects: &'a Objects<'w, 's>,
    pub(super) items: &'a Items,
    pub(super) commands: &'a NetCommands,
    pub(super) sub_classes: Option<&'a wrathplus_formats::ItemSubClassCatalog>,
    /// The spell-to-line hop the `$`-tokens' skill level reads ([`crate::spell::spell_skill_value`]).
    pub(super) skill_lines: Option<&'a wrathplus_formats::SkillLineCatalog>,
    /// The talent spell-modifier tables: the cost cell shows the modified cost (`power_cost`).
    pub(super) spell_mods: &'a crate::spell::SpellModifiers,
    /// The VM's `GlobalStrings.lua`, where the keyed cells and the `$` tokens' templates resolve.
    pub(super) get: &'a dyn Fn(&str) -> Option<String>,
}

/// The player's worn ranged weapon and main hand as [`super::cast_cells::Weapons`], each read only
/// where the spell reads it: the ranged slot for a ranged spell in the player's view (`007ff480`
/// and `007ff180` read the caster's), the main hand for a weapon-speed category in either
/// (`007fef60` reads the active player's). A template not cached yet reads as no weapon.
fn worn_weapons(
    vctx: &ViewCtx,
    d: &wrathplus_formats::SpellDisplay,
    player_view: bool,
    deps: &mut Deps,
) -> super::cast_cells::Weapons {
    const MAIN_HAND: u8 = 15;
    const RANGED: u8 = 17;
    let Some(store) = vctx.store else {
        return super::cast_cells::Weapons::default();
    };
    let mut worn = |slot: u8| {
        deps.worn |= 1 << slot;
        let guid = store.0.player_inv_slot(slot).filter(|&g| g != 0)?;
        let entry = vctx.objects.object(guid)?.object_entry()?;
        vctx.items.template(entry, guid, vctx.commands)
    };
    let ranged = (player_view && d.attributes & 0x2 != 0)
        .then(|| worn(RANGED).map(|t| (t.delay_ms, t.ranged_mod_range)))
        .flatten();
    let main_hand_delay = d
        .category_weapon_speed
        .then(|| worn(MAIN_HAND).map(|t| t.delay_ms))
        .flatten();
    super::cast_cells::Weapons {
        ranged,
        main_hand_delay,
    }
}

/// Build one spell's tooltip view, every cell resolved here where the catalogs live, and what it
/// read ([`Deps`]). The range, cast, cooldown, form, item and cost cells fill their
/// `GlobalStrings.lua` keys; the reagents and chance cells are composed in English, where the
/// reference fills `SPELL_REAGENTS` and `CHANCE_TO_*`.
pub(crate) fn build_view(
    spell_id: u32,
    spells: &Spells,
    vctx: &mut ViewCtx,
) -> Option<(wrathplus_ui::script::SpellTooltipView, Deps)> {
    let d = spells.catalog.get(spell_id)?;
    let deps = RefCell::new(Deps::default());
    // Every cell and the description's tokens modify through the spell's own family mask.
    deps.borrow_mut().mod_bits |= vctx.spell_mods.read_bits(d);
    let form = vctx.form;
    let caster = vctx.caster;
    // The `$`-tokens' level (`0x5075f0` hands the selector to `0x6e3130`, `507645`-`50764a`).
    let skill = |id| match caster {
        ViewCaster::Player => {
            let line = vctx.skill_lines.and_then(|c| c.spell_to_line(id));
            if let Some(line) = line {
                deps.borrow_mut().skill_line(line);
            }
            line.map_or(0, |line| crate::spell::skill_line_value(vctx.store, line))
        }
        // The pet's level is one of the pet's own inputs.
        ViewCaster::Pet(pet) => ViewCaster::pet_skill_value(pet),
    };
    // A cross-referenced spell's own family mask decides its modifiers.
    let lookup = |id| {
        let t = spells.catalog.get(id)?;
        deps.borrow_mut().mod_bits |= vctx.spell_mods.read_bits(t);
        Some(t)
    };
    let player_read = || deps.borrow_mut().unit(UnitField::Tokens);
    let home = vctx.home_area;
    let home_area = || {
        deps.borrow_mut().home = true;
        home
    };
    // `0x6e3130` for this spell: the level the cost's and the cast time's per-level terms read.
    let pet_level = |pet| d.skill_level(ViewCaster::pet_skill_value(pet));
    let ctx = wrathplus_formats::TokenContext {
        durations: &spells.durations,
        radii: &spells.radii,
        ranges: Some(&spells.ranges),
        skill: &skill,
        lookup: &lookup,
        mods: Some(vctx.spell_mods),
        unmodified_points: false,
        home_area: &home_area,
        global: vctx.get,
        printf: &crate::ui_script::token_printf,
        player: Some(&crate::ui_script::SelfTokens::new(vctx.store, None).watched(&player_read)),
    };
    // The aura tooltip `0x52f880` expands the aura text with the points' modifiers off (`52f940`).
    let aura_ctx = wrathplus_formats::TokenContext {
        unmodified_points: true,
        ..ctx
    };
    // The cost cell (`0x52e8ad`): the resolved `power_cost`, named by power type (keys at
    // `0x85416c`) and Health for a type outside 0..=4 (`0x52e8fc`: Bloodrage's -2), never a
    // percentage. Rage shows wire ÷ 10, a per-second column adds `_PER_TIME`, and no cost at all
    // leaves the cell empty. A pet with no unit costs -1 (`0x6e3233`), which shows as none.
    let resolved_cost = match caster {
        ViewCaster::Player => vctx
            .store
            .map_or(d.mana_cost, |s| usable::power_cost(d, s, vctx.spell_mods)),
        ViewCaster::Pet(pet) => pet.map_or(0, |p| {
            usable::power_cost_at(d, p, pet_level(Some(p)), vctx.spell_mods)
        }),
    };
    // The player's cost reads its own level and percentage field; the pet's reads the pet's.
    if matches!(caster, ViewCaster::Player) {
        let mut deps = deps.borrow_mut();
        if usable::cost_reads_level(d) {
            deps.unit(UnitField::Level);
        }
        match usable::cost_basis(d) {
            CostBasis::None | CostBasis::Fixed(_) => {}
            CostBasis::BaseMana => deps.unit(UnitField::BaseMana),
            CostBasis::BaseHealth => deps.unit(UnitField::BaseHealth),
        }
    }
    let cost = super::cost_cell::cost_cell(d, resolved_cost, &spells.catalog, vctx.get);
    // The range, cast and cooldown cells, 12340's (`super::cast_cells`). A melee row reads the
    // caster's and the auto-attack target's reaches; the pet views read the pet's own.
    let player_view = matches!(caster, ViewCaster::Player);
    let weapons = worn_weapons(vctx, d, player_view, &mut deps.borrow_mut());
    if player_view && super::cast_cells::range_reads_units(d, &spells.ranges, vctx.spell_mods) {
        deps.borrow_mut().range_units = true;
    }
    let range = super::cast_cells::range_cell(
        d,
        &spells.ranges,
        vctx.spell_mods,
        vctx.range_caster,
        vctx.attack_target,
        weapons
            .ranged
            .filter(|_| player_view)
            .map(|(_, scale)| scale),
        vctx.get,
    );
    let (cast_time, cooldown) = if super::cast_cells::has_cast_line(d) {
        // The level term is `0x6e3130`'s; the player's is read only where the row scales by one.
        if player_view
            && spells
                .cast_times
                .get(d.casting_time_index)
                .is_some_and(|row| row.reads_caster_level())
        {
            deps.borrow_mut().unit(UnitField::Level);
        }
        let (level, haste_unit) = match caster {
            ViewCaster::Player => {
                deps.borrow_mut().unit(UnitField::CastSpeed);
                let level = vctx.store.and_then(|s| s.0.unit_level()).unwrap_or(0);
                (level, vctx.store.map(|s| &s.0))
            }
            ViewCaster::Pet(pet) => (pet_level(pet), pet),
        };
        // The ranged attack time is the player's, a pet view's too (`007fef60`).
        let reads_ranged_time = d.ranged_speed_cooldown() || player_view && d.attributes & 0x2 != 0;
        let ranged_time = if reads_ranged_time {
            deps.borrow_mut().unit(UnitField::RangedTime);
            vctx.store
                .and_then(|s| s.0.unit_ranged_attack_time())
                .unwrap_or(0)
        } else {
            0
        };
        let player_ranged = weapons
            .ranged
            .filter(|_| player_view)
            .map(|(delay, _)| (delay, ranged_time));
        let mods = vctx.spell_mods;
        let ms = super::cast_cells::cast_time_ms(d, spells, mods, level, haste_unit, player_ranged);
        let cooldown_ms =
            super::cast_cells::cooldown_ms(d, mods, weapons.main_hand_delay, ranged_time);
        (
            super::cast_cells::cast_cell(d, ms, resolved_cost, vctx.get),
            super::cast_cells::cooldown_cell(d, cooldown_ms, vctx.get),
        )
    } else {
        (None, None)
    };
    // The required-form line (`0x52f10a`-`0x52f2ae`). With `AttributesEx2` bit 19 set the mask is
    // permissive (Inner Fire in Shadowform), and the line is skipped (`0x52f115`).
    let requires_form = (d.stances != 0 && !d.form_mask_is_permissive())
        .then(|| {
            let names: Vec<&str> = (0..32u32)
                .filter(|b| d.stances & (1 << b) != 0)
                .filter_map(|b| spells.forms.get(&(b + 1)).map(|f| f.name.as_str()))
                .filter(|n| !n.is_empty())
                .collect();
            (!names.is_empty())
                .then(|| {
                    keyed(
                        vctx.get,
                        "SPELL_REQUIRED_FORM",
                        &[Arg::S(&names.join(", "))],
                    )
                })
                .flatten()
        })
        .flatten();
    // No caster unit passes the form test (`0x52f1e8`). A spell with no stance mask fails it for
    // every form, so only a stance mask reads the player's form; a pet's is its own input.
    if matches!(caster, ViewCaster::Player) && d.stances != 0 {
        deps.borrow_mut().form = true;
    }
    let form_met = matches!(caster, ViewCaster::Pet(None))
        || (form != 0
            && 1u32
                .checked_shl(u32::from(form) - 1)
                .is_some_and(|bit| d.stances & bit != 0));
    // The required-item line (`0x52eea7`-`0x52f10a`): a mask is named whole by
    // `ItemSubClassMask.dbc` before its subclasses are joined; an empty mask or class < 0 skips it.
    let requires_item = (d.targets & TARGET_ITEM == 0 && d.equipped_item_class >= 0)
        .then(|| {
            vctx.sub_classes?
                .requirement_name(d.equipped_item_class as u32, d.equipped_item_subclass_mask)
        })
        .flatten()
        .and_then(|name| keyed(vctx.get, "SPELL_EQUIPPED_ITEM", &[Arg::S(&name)]));
    let chance = chance_line(d, vctx.store, &mut deps.borrow_mut());
    // The unit selector passes the item test outright (`0x52f07f`).
    if matches!(caster, ViewCaster::Player) {
        deps.borrow_mut().item_search(usable::worn_slots_read(d));
    }
    let item_met = matches!(caster, ViewCaster::Pet(_))
        || vctx.store.is_none_or(|s| {
            usable::equipped_item_fits(d, s, vctx.objects, vctx.items, vctx.commands)
        });
    // Reagents (`SPELL_REAGENTS`, `0x854e54`), red when short; one whose template has not
    // streamed is absent until `feed_spell_tooltips` re-pushes on its arrival.
    let reagents = {
        let mut parts: Vec<String> = Vec::new();
        for (entry, count) in d.reagents.iter().copied().filter(|&(e, _)| e != 0) {
            deps.borrow_mut().reagent(entry);
            let Some(name) = vctx
                .items
                .template(entry, 0, vctx.commands)
                .map(|t| t.name.clone())
            else {
                continue;
            };
            let text = if count > 1 {
                format!("{name} ({count})")
            } else {
                name
            };
            let short = vctx.store.is_some_and(|s| {
                count_of(&s.0, vctx.objects, entry, InventoryScope::CARRIED) < count
            });
            parts.push(if short {
                format!("|cffff2020{text}|r")
            } else {
                text
            });
        }
        (!parts.is_empty()).then(|| format!("Reagents: {}", parts.join(", ")))
    };
    let view = wrathplus_ui::script::SpellTooltipView {
        name: d.name.clone(),
        rank: d.rank.clone(),
        // The aura variant's right column (`0x52f8e5`), gated on `SpellDispelType.dbc` `[+0x28]`.
        dispel_type: spells.catalog.dispel_name(d).map(str::to_string),
        cost_range_lines: range.lines(cost.as_deref()),
        range: range.single(),
        cost,
        cast_time,
        cooldown,
        requires_item,
        item_met,
        requires_form,
        form_met,
        chance,
        reagents,
        description: d
            .description
            .as_deref()
            .map(|t| wrathplus_formats::substitute(t, d, &ctx))
            .unwrap_or_default(),
        aura_description: d
            .aura_description
            .as_deref()
            .map(|t| wrathplus_formats::substitute(t, d, &aura_ctx))
            .unwrap_or_default(),
    };
    Some((view, deps.into_inner()))
}

/// `Targets` bit `0x10` (name inferred): set, the required-item line is skipped (`0x52eeaa`).
const TARGET_ITEM: u32 = 0x10;

/// The four `Effect[0]` values that select a chance-to-X line (jump table `0x52f7c4`).
const EFFECT_DODGE: u32 = 20;
const EFFECT_PARRY: u32 = 22;
const EFFECT_BLOCK: u32 = 23;
const EFFECT_ATTACK: u32 = 78;

/// The chance-to-X line (`0x52f5b1`-`0x52f697`): ATTACK skips the passive gate that dodge, parry
/// and block need. The wire's values are already percents.
fn chance_line(
    d: &wrathplus_formats::SpellDisplay,
    store: Option<&ObjectStore>,
    deps: &mut Deps,
) -> Option<String> {
    let (which, label) = match d.effects[0] {
        EFFECT_ATTACK => (Chance::Crit, "crit"),
        EFFECT_DODGE if d.passive => (Chance::Dodge, "dodge"),
        EFFECT_PARRY if d.passive => (Chance::Parry, "parry"),
        EFFECT_BLOCK if d.passive => (Chance::Block, "block"),
        _ => return None,
    };
    deps.chance(which);
    // The enUS text of the reference's `CHANCE_TO_*` keys (`%.2f%% chance to crit`).
    Some(format!("{:.2}% chance to {label}", which.read(&store?.0)?))
}

/// What the feed knows of the views it has pushed: what each was built from and what the inputs
/// were when the last frame looked. The reference rebuilds on every hover and keeps no such
/// snapshot.
#[derive(Default)]
pub struct SpellFeedMemory {
    /// The views built against the player, by spell, with what each read.
    pub(super) pushed: HashMap<u32, Deps>,
    /// The spells whose view built against the pet is in the VM's pet store, and what each read
    /// of the player's side (the pet's own inputs are [`Self::pet`]).
    pub(super) pet_pushed: HashMap<u32, Deps>,
    /// What the pet views read off the pet unit.
    pet: Option<PetInputs>,
    /// The player-side inputs the pushed views were checked against last frame.
    pub(super) seen: Option<Seen>,
    /// The talent modifier tables as last frame's check found them.
    pub(super) mods: Option<crate::spell::SpellModifiers>,
    /// Per reagent on show, the state its views were built against.
    pub(super) reagents: Reagents,
}

impl SpellFeedMemory {
    /// Queue for a rebuild the pushed views, the player's and the pet's, that read what
    /// `changes` moved.
    pub(super) fn requeue(
        &mut self,
        changes: &Changes,
        wanted: &mut Vec<u32>,
        wanted_pet: &mut Vec<u32>,
    ) {
        if changes.is_empty() {
            return;
        }
        for (pushed, queue) in [
            (&mut self.pushed, wanted),
            (&mut self.pet_pushed, wanted_pet),
        ] {
            pushed.retain(|&id, deps| {
                let hit = deps.hits(changes);
                if hit {
                    queue.push(id);
                }
                !hit
            });
        }
    }
}

/// What a reagent's line renders for `entry`: the carried count and whether its name has streamed.
pub(super) fn reagent_state(
    entry: u32,
    store: Option<&ObjectStore>,
    objects: &Objects,
    items: &Items,
    commands: &NetCommands,
) -> (u32, bool) {
    let named = items.template(entry, 0, commands).is_some();
    let owned = store.map_or(0, |s| {
        count_of(&s.0, objects, entry, InventoryScope::CARRIED)
    });
    (owned, named)
}

/// The pet unit's fields a pet view reads (`0x52e610` with the unit selector set): its level, the
/// cost bases `0x612c50` reads, its form, and its and its melee target's reach and motion.
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) struct PetInputs {
    guid: Option<u64>,
    level: u32,
    base_mana: u32,
    base_health: u32,
    form: u8,
    range_units: RangeSeen,
}

impl PetInputs {
    pub(super) fn of(
        guid: Option<u64>,
        pet: Option<&wrathplus_protocol::ObjectFields>,
        range_units: RangeSeen,
    ) -> Self {
        let Some(p) = pet else {
            return Self {
                guid,
                ..Self::default()
            };
        };
        Self {
            guid,
            level: p.unit_level().unwrap_or(0),
            base_mana: p.unit_base_mana().unwrap_or(0),
            base_health: p.unit_base_health().unwrap_or(0),
            form: p.unit_shapeshift_form(),
            range_units,
        }
    }
}

/// The hoverable spell sources [`feed_spell_tooltips`] reads, one parameter under Bevy's 16.
#[derive(bevy::ecs::system::SystemParam)]
pub struct SpellTooltipSources<'w> {
    spells: Option<Res<'w, Spells>>,
    trainer_subjects: Option<Res<'w, TrainerTooltipSubjects>>,
    talents: Option<Res<'w, crate::ui_talent::Talents>>,
}

/// Push a view for every spell the UI can hover (the book, the class's talent ranks, the open
/// trainer's services, and what the VM holds: the pet's spells, the quest rewards, every unit's
/// auras and the tracking spell) before it is hovered, as the reference
/// reads them all locally; an ask for any other id too.
pub fn feed_spell_tooltips(
    script: Option<NonSendMut<UiScript>>,
    actions: Option<Res<PlayerActions>>,
    spell_sources: SpellTooltipSources,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    // The caster's and the pet's reach and motion and each one's auto-attack target, which the
    // melee range cell reads.
    range_units: crate::spell::RangeUnits,
    objects: Objects,
    home_bind: Option<Res<crate::net::HomeBind>>,
    area_names: Option<Res<crate::ui_quest_log::QuestHeaderNamesRes>>,
    items: Res<Items>,
    // One tuple param, under Bevy's 16-param ceiling.
    lookups: (
        Option<Res<crate::items::ItemSubClasses>>,
        Res<crate::spell::SpellModifiers>,
        Option<Res<crate::app_core::state::SkillLines>>,
    ),
    commands: Res<NetCommands>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<SpellFeedMemory>>,
) {
    let (sub_classes, spell_mods, skill_lines) = &lookups;
    let SpellTooltipSources {
        spells,
        trainer_subjects,
        talents,
    } = spell_sources;
    let Some(mut script) = script else {
        return;
    };
    let memory = memory.get(&script);
    let Some(spells) = spells.as_deref() else {
        return;
    };
    let mut wanted: Vec<u32> = script.take_spell_tooltip_asks();
    // What the VM holds for a hover: pet spells, quest rewards, auras, tracking.
    wanted.extend(
        script
            .spell_tooltip_subjects()
            .into_iter()
            .filter(|s| !memory.pushed.contains_key(s)),
    );
    if let Some(actions) = actions.as_deref() {
        wanted.extend(
            actions
                .spells
                .iter()
                .copied()
                .filter(|s| !memory.pushed.contains_key(s)),
        );
    }
    // The pet views: the pet's bar and book, and what their setters asked for.
    let mut wanted_pet: Vec<u32> = script.take_pet_spell_tooltip_asks();
    wanted_pet.extend(
        script
            .pet_spell_tooltip_subjects()
            .into_iter()
            .filter(|s| !memory.pet_pushed.contains_key(s)),
    );
    // The open trainer's services, which the detail icon's `SetTrainerService` renders, a
    // pet-learn service's against the pet.
    if let Some(trainer) = trainer_subjects.as_deref() {
        wanted.extend(
            trainer
                .player
                .iter()
                .copied()
                .filter(|s| !memory.pushed.contains_key(s)),
        );
        wanted_pet.extend(
            trainer
                .pet
                .iter()
                .copied()
                .filter(|s| !memory.pet_pushed.contains_key(s)),
        );
    }
    // Every rank of the class's talents: a talent tooltip reads the current and the next rank.
    if let (Some(talents), Ok(store)) = (talents.as_deref(), self_q.single()) {
        let race = store.0.unit_race().unwrap_or(0);
        let class = store.0.unit_class().unwrap_or(0);
        for tab in talents.catalog.tabs_for_class(race, class) {
            for t in talents.catalog.talents_in_tab(tab.id) {
                wanted.extend(
                    t.ranks
                        .iter()
                        .copied()
                        .filter(|s| *s != 0 && !memory.pushed.contains_key(s)),
                );
            }
        }
    }
    let home_area: Option<String> = home_bind
        .as_deref()
        .and_then(|b| b.0)
        .and_then(|id| area_names.as_deref()?.0.resolve(id as i32))
        .map(str::to_string);
    let self_store = self_q.single().ok();
    let caster_unit = range_units.caster();
    let attack_target = range_units.caster_attack_target();
    // What moved since the last frame, which each pushed view is checked against: the bind
    // point (Astral Recall's `$z`), the form, the skills, what the equipped-item search learns of
    // each equipment slot and the disarm flag, the level, cost bases and ranged attack time, the
    // block, dodge, parry and crit percentages, and the two units the melee range cell reads.
    let seen = Seen::of(
        self_store,
        home_area.as_deref(),
        RangeSeen::of(&caster_unit, attack_target.as_ref()),
        &objects,
        &items,
    );
    let mut changes = memory
        .seen
        .as_ref()
        .map(|prev| seen.changes_since(prev))
        .unwrap_or_default();
    memory.seen = Some(seen);
    // The talent tables: the cost, cast, cooldown and range cells and the description's tokens
    // resolve through them.
    if let Some(prev) = memory.mods.as_ref().filter(|_| spell_mods.is_changed()) {
        changes = changes.with_mods(spell_mods.diff(prev));
    }
    if memory.mods.is_none() || spell_mods.is_changed() {
        memory.mods = Some((**spell_mods).clone());
    }
    // And the reagents on show: their counts and whether their names have streamed.
    let reagent_state: Reagents = memory
        .reagents
        .keys()
        .map(|&entry| {
            (
                entry,
                reagent_state(entry, self_store, &objects, &items, &commands),
            )
        })
        .collect();
    changes = changes.with_reagents(&memory.reagents, &reagent_state);
    memory.reagents = reagent_state;
    memory.requeue(&changes, &mut wanted, &mut wanted_pet);
    // The pet's inputs: a new pet, or its level, bases, form, reach or motion moving, rebuilds the
    // pet views alone.
    let pet_guid = self_store.and_then(|s| s.0.unit_pet_guid());
    let pet = pet_guid.and_then(|g| objects.object(g));
    let pet_entity = pet_guid.and_then(|g| objects.entity(g));
    let pet_unit = pet_entity.and_then(|e| range_units.unit(e)).unwrap_or(
        wrathplus_formats::RangeUnit::still(crate::spell::DEFAULT_REACH),
    );
    let pet_target = pet_entity.and_then(|e| range_units.attack_target(e));
    let pet_inputs = PetInputs::of(pet_guid, pet, RangeSeen::of(&pet_unit, pet_target.as_ref()));
    if memory.pet != Some(pet_inputs) {
        memory.pet = Some(pet_inputs);
        wanted_pet.extend(memory.pet_pushed.drain().map(|(id, _)| id));
    }
    // The sources overlap (a self-buff is in the book and on the player): build each id once.
    wanted.sort_unstable();
    wanted.dedup();
    wanted_pet.sort_unstable();
    wanted_pet.dedup();
    // Build, then push: the build's borrow of the VM's strings ends before the store is written.
    let (built, built_pet) = {
        let get = |key: &str| wrathplus_ui::strings::global(script.lua(), key);
        let mut vctx = ViewCtx {
            home_area: home_area.as_deref(),
            form: self_store.map_or(0, |s| s.0.unit_shapeshift_form()),
            store: self_store,
            caster: ViewCaster::Player,
            range_caster: caster_unit,
            attack_target,
            objects: &objects,
            items: &items,
            commands: &commands,
            sub_classes: sub_classes.as_deref().map(|c| &c.0),
            skill_lines: skill_lines.as_deref().map(|s| &s.catalog),
            spell_mods,
            get: &get,
        };
        let built = build_views(wanted, spells, &mut vctx, memory, false);
        // The pet views read the pet's form and reach where the player's read the player's.
        vctx.caster = ViewCaster::Pet(pet);
        vctx.form = pet.map_or(0, |p| p.unit_shapeshift_form());
        vctx.range_caster = pet_unit;
        vctx.attack_target = pet_target;
        let built_pet = build_views(wanted_pet, spells, &mut vctx, memory, true);
        (built, built_pet)
    };
    for (id, view) in built {
        script.set_spell_tooltip(id, view);
    }
    for (id, view) in built_pet {
        script.set_pet_spell_tooltip(id, view);
    }
}

/// Build the views of `ids` against `vctx`'s caster, recording each with what it read in the
/// pushed set `pet` names and watching its reagents, seeded with the state the view was built
/// against.
fn build_views(
    ids: Vec<u32>,
    spells: &Spells,
    vctx: &mut ViewCtx,
    memory: &mut SpellFeedMemory,
    pet: bool,
) -> Vec<(u32, wrathplus_ui::script::SpellTooltipView)> {
    let mut built = Vec::new();
    for id in ids {
        let Some((view, deps)) = build_view(id, spells, vctx) else {
            continue;
        };
        for &entry in deps.reagents() {
            if let std::collections::btree_map::Entry::Vacant(slot) = memory.reagents.entry(entry) {
                slot.insert(reagent_state(
                    entry,
                    vctx.store,
                    vctx.objects,
                    vctx.items,
                    vctx.commands,
                ));
            }
        }
        built.push((id, view));
        let pushed = if pet {
            &mut memory.pet_pushed
        } else {
            &mut memory.pushed
        };
        pushed.insert(id, deps);
    }
    built
}
