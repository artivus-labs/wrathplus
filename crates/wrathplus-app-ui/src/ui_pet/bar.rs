//! The pet action bar's feed: the ten packed words of the last `SMSG_PET_SPELLS`, which the
//! server owns whole, rendered as ten `PetActionView`s each frame.

use std::time::Instant;

use bevy::prelude::*;

use wrathplus_protocol::types::{
    PetActionEntry, PetSpells, PET_COMMAND_ATTACK, PET_COMMAND_DISMISS, PET_COMMAND_FOLLOW,
    PET_COMMAND_STAY, PET_REACT_AGGRESSIVE, PET_REACT_DEFENSIVE, PET_REACT_PASSIVE,
};
use wrathplus_ui::script::{PetActionView, UiScript};

use crate::app_core::session::ObjectStore;
use crate::app_core::state::Spells;
use crate::names::NameCache;
use crate::target::UNIT_FLAG_POSSESSED;

use super::{PetBar, PetUnit};

/// `(bar_signals, has_bar, hidden, attacking, usable, pickup_allowed, slots)` as last pushed.
type BarKey = (u32, bool, bool, bool, bool, bool, Vec<PetActionView>);

/// What the feed last pushed, so `PET_BAR_UPDATE` fires on a change; the key's leading `u32` is
/// [`PetBar::bar_signals`], so a press that changes nothing still repaints.
#[derive(Default)]
pub struct PetBarMemory {
    pushed: Option<BarKey>,
    /// The ten cooldown triples as last pushed, so a cooldown alone is pushed with no event: its
    /// events are [`fire_pet_cooldown_events`]'s.
    cooldowns: Vec<Option<(i64, u32, bool)>>,
}

/// A command token's `(name, texture)` pair, both global names (`PetActionBarFrame.lua:98-104`):
/// keys from `GlobalStrings.lua:3029-3032`, textures from `PetActionBarFrame.lua:6-12`.
pub(super) fn command_token(action: u32) -> Option<(&'static str, &'static str)> {
    Some(match action {
        PET_COMMAND_STAY => ("PET_ACTION_WAIT", "PET_WAIT_TEXTURE"),
        PET_COMMAND_FOLLOW => ("PET_ACTION_FOLLOW", "PET_FOLLOW_TEXTURE"),
        PET_COMMAND_ATTACK => ("PET_ACTION_ATTACK", "PET_ATTACK_TEXTURE"),
        PET_COMMAND_DISMISS => ("PET_ACTION_DISMISS", "PET_DISMISS_TEXTURE"),
        _ => return None,
    })
}

/// A reaction token's pair: the `PET_MODE_*` keys (`GlobalStrings.lua:3045-3047`), naming the
/// pet's state, not the menu's imperative `PET_AGGRESSIVE` family, though enUS reads them alike.
pub(super) fn reaction_token(action: u32) -> Option<(&'static str, &'static str)> {
    Some(match action {
        PET_REACT_PASSIVE => ("PET_MODE_PASSIVE", "PET_PASSIVE_TEXTURE"),
        PET_REACT_DEFENSIVE => ("PET_MODE_DEFENSIVE", "PET_DEFENSIVE_TEXTURE"),
        PET_REACT_AGGRESSIVE => ("PET_MODE_AGGRESSIVE", "PET_AGGRESSIVE_TEXTURE"),
        _ => return None,
    })
}

/// One packed slot word as the bar draws it. `showing_active` is [`active_aura_press`]'s answer,
/// the one call (`0x4bcea0`) that both `GetPetActionInfo` (`0x4bdd2f`) and `CastPetAction`
/// (`0x4bd24a`) make, so the button showing active art is the one whose press cancels.
pub(super) fn slot_view(
    entry: PetActionEntry,
    bar: &PetSpells,
    spell: Option<&wrathplus_formats::SpellDisplay>,
    cooldown: Option<(i64, u32, bool)>,
    pet_attacking: bool,
    showing_active: bool,
) -> PetActionView {
    PetActionView {
        // Every slot carries its raw word, empty ones too: the drag's relocation candidate is
        // vmangos's unused slot, `ACT_DISABLED` with spell 0 (type 1, low 16 bits zero).
        packed: entry.packed,
        passive: spell.is_some_and(|s| s.passive),
        ..slot_paint(entry, bar, spell, cooldown, pet_attacking, showing_active)
    }
}

/// [`slot_view`]'s painted half: everything but the raw `packed` and `passive`.
pub(super) fn slot_paint(
    entry: PetActionEntry,
    bar: &PetSpells,
    spell: Option<&wrathplus_formats::SpellDisplay>,
    cooldown: Option<(i64, u32, bool)>,
    pet_attacking: bool,
    showing_active: bool,
) -> PetActionView {
    let kind = entry.kind();
    let action = entry.action();

    if let Some((name, texture)) = (kind == wrathplus_protocol::types::PET_ACT_COMMAND)
        .then(|| command_token(action))
        .flatten()
    {
        // Lit when the unmasked `state >> 8` equals the action, so a disabled bar lights none, or
        // for Attack alone on the attack latch (`0x4bdf01`-`0x4bdf22`). `attack_active` is
        // `IsPetAttackActive`, that latch on this slot (`0x4be138`-`0x4be153`).
        let attacking = pet_attacking && action == PET_COMMAND_ATTACK;
        return PetActionView {
            name: Some(name.to_string()),
            texture: Some(texture.to_string()),
            is_token: true,
            active: bar.command_state() == action || attacking,
            attack_active: attacking,
            ..Default::default()
        };
    }

    if let Some((name, texture)) = (kind == wrathplus_protocol::types::PET_ACT_REACTION)
        .then(|| reaction_token(action))
        .flatten()
    {
        // A disabled bar reads as Passive (`0x4bde3c`).
        let showing = if bar.bar_disabled() {
            wrathplus_protocol::types::PET_REACT_PASSIVE
        } else {
            bar.react_state()
        };
        return PetActionView {
            name: Some(name.to_string()),
            texture: Some(texture.to_string()),
            is_token: true,
            active: showing == action,
            ..Default::default()
        };
    }

    // A zero word is empty (the client tests the dword); vmangos's unused slots are spell 0, which
    // has no record and takes the next exit, as in the client.
    if !entry.is_spell() || entry.is_empty() {
        return PetActionView::default();
    }
    let Some(spell) = spell else {
        // No `Spell.dbc` record, or no catalog: the reference returns nil and the button hides.
        return PetActionView::default();
    };
    PetActionView {
        name: Some(spell.name.clone()),
        subtext: spell.rank.clone(),
        // A running spell draws its `ActiveIconID` (`0x4bdd2f`/`0x4bdd38`/`0x4bdd77`), with no
        // fallback: the reference pushes nil when that icon does not resolve (`0x4bdd50`).
        texture: if showing_active {
            spell.active_icon.clone()
        } else {
            spell.icon.clone()
        },
        is_token: false,
        spell_id: Some(action),
        // Nil on every spell path (`0x4bdd5e`): `isActive` is a token's, `autoCast*` a spell's
        // (`0x4bdc50`).
        active: false,
        // Bits 31/30, not the type (`0x4bdd65`/`0x4bdda4`), gated on the record checked above.
        autocast_allowed: entry.autocast_allowed(),
        autocast_enabled: entry.autocast_on(),
        attack_active: false,
        cooldown,
        ..Default::default()
    }
}

/// The spell id when the slot shows active: `0x4bcea0`, the pet-side twin of `0x4e55f0`
/// ([`crate::ui_action::toggle::active_action_toggle`]). It tests `ActiveIconID != 0` first
/// (`0x4bcefd`), so a press on a spell without one re-casts rather than cancels.
pub(super) fn active_aura_press(
    entry: PetActionEntry,
    pet_auras: Option<&[crate::net::wrath_auras::StampedAura]>,
    spell: Option<&wrathplus_formats::SpellDisplay>,
) -> Option<u32> {
    if !entry.is_spell() || entry.is_empty() {
        return None;
    }
    let spell_id = entry.action();
    crate::ui_action::toggle::active_action_toggle(spell_id, spell?, pet_auras?).then_some(spell_id)
}

/// Push the ten slot views on a change, and `PET_BAR_UPDATE` when the bar itself moved. A change
/// of the cooldowns alone is pushed without an event: the pet's cooldown events fire off the
/// list's generation, so a natural expiry fires none, as in the reference.
pub fn feed_pet_bar(
    script: Option<NonSendMut<UiScript>>,
    bar: Res<PetBar>,
    actions: Option<Res<crate::app_core::state::PlayerActions>>,
    spells: Option<Res<Spells>>,
    clock: Res<crate::app_core::ui_flags::UiClock>,
    pet: PetUnit,
    names: Res<NameCache>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<PetBarMemory>>,
) {
    let Some(mut script) = script else {
        return;
    };
    // Full learned sets, never the filtered visible spellbook. Re-push into a replacement VM
    // and on every pet/player mutation, including teardown; no packet receipt is replayed.
    script.set_wrath_known_spells(
        actions
            .as_ref()
            .into_iter()
            .flat_map(|a| a.spells.iter().copied()),
        bar.native
            .as_ref()
            .into_iter()
            .flat_map(|p| p.spells.iter().map(|a| a.id())),
        bar.native
            .as_ref()
            .into_iter()
            .flat_map(|p| p.actions.iter().map(|a| a.0)),
    );
    let memory = memory.get(&script);
    let now = Instant::now();
    let (anchor, ui_now) = (clock.anchor, clock.ui_now);
    let has_bar = bar.has_bar();
    // An unstreamed pet is not usable (`0x4bd034`), and no slot shows active.
    let pet_store = pet.store(bar.guid());
    let pet_auras = pet_store.map(|_| pet.auras_of(bar.guid()));
    let pet_flags = pet_store.map(|s| s.0.unit_flags());
    let usable = pet.actions_usable(&bar);
    let type_flags = pet_store
        .and_then(|s| s.0.object_entry())
        .and_then(|e| names.creature_record(e))
        .map(|r| r.type_flags);
    let hidden = has_bar && !pet_bar_shows(pet_store, type_flags);
    // `PickupPetAction`'s gate alone: the pet's object must resolve (`0x4be1f7`) and not be
    // possessed (`0x4be20a`). A possessed unit's bar cannot be rearranged but its buttons work,
    // so possession stays out of `usable`; an unstreamed pet has no flags to test and lifts nothing.
    let pickup_allowed = pet_flags.is_some_and(|flags| flags & UNIT_FLAG_POSSESSED == 0);
    let pet_attacking = bar.attacking;
    // `GetPetTimeRemaining`'s expiry on the VM clock, signed both ways around the anchor sample.
    script.set_pet_expiry(bar.expires.map(|t| match t.checked_duration_since(anchor) {
        Some(ahead) => ui_now + ahead.as_secs_f64(),
        None => ui_now - anchor.duration_since(t).as_secs_f64(),
    }));

    let fresh: Vec<PetActionView> = if let Some(native) = &bar.native {
        native
            .actions
            .iter()
            .map(|&entry| {
                let display = spells.as_ref().and_then(|s| s.catalog.get(entry.id()));
                let cooldown = display.and_then(|d| {
                    bar.cooldowns
                        .wrath_pet_info(entry.id(), Some(d), now)
                        .ui_triple(anchor, ui_now)
                });
                native_slot_view(
                    entry,
                    native,
                    display,
                    cooldown,
                    pet_attacking,
                    pet_store,
                    pet_auras,
                )
            })
            .collect()
    } else if has_bar {
        bar.spells
            .bar
            .iter()
            .map(|&entry| {
                let display = entry
                    .is_spell()
                    .then(|| spells.as_ref().and_then(|s| s.catalog.get(entry.action())))
                    .flatten();
                let cooldown = display.and_then(|d| {
                    bar.cooldowns
                        .info(entry.action(), 0, Some(d), now)
                        .ui_triple(anchor, ui_now)
                });
                slot_view(
                    entry,
                    &bar.spells,
                    display,
                    cooldown,
                    pet_attacking,
                    active_aura_press(entry, pet_auras, display).is_some(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    // `bar_signals` in the key repaints a press that moved nothing (`0x4bc940`/`0x4bc960`); the
    // cooldown triples are keyed apart, since they carry no `PET_BAR_UPDATE`.
    let cooldowns: Vec<Option<(i64, u32, bool)>> = fresh.iter().map(|s| s.cooldown).collect();
    let content: Vec<PetActionView> = fresh
        .iter()
        .cloned()
        .map(|mut s| {
            s.cooldown = None;
            s
        })
        .collect();
    let key = (
        bar.bar_signals,
        has_bar,
        hidden,
        pet_attacking,
        usable,
        pickup_allowed,
        content,
    );
    let bar_changed = memory.pushed.as_ref() != Some(&key);
    let cooldowns_changed = memory.cooldowns != cooldowns;
    if bar_changed || cooldowns_changed {
        script.set_pet_actions(has_bar, usable, pickup_allowed, fresh);
        script.set_pet_bar_hidden(hidden);
        script.set_pet_attacking(pet_attacking);
        memory.cooldowns = cooldowns;
    }
    if bar_changed {
        debug!(
            "ui_pet: bar {} ({} occupied slot(s), {}{})",
            if key.1 && !key.2 { "shown" } else { "hidden" },
            key.6.iter().filter(|s| s.name.is_some()).count(),
            if key.4 { "usable" } else { "disabled" },
            if key.5 { "" } else { ", possessed" },
        );
        memory.pushed = Some(key);
        script.fire_event("PET_BAR_UPDATE", vec![]);
    }
}

/// `CREATURE_TYPE_FLAG_NO_PET_BAR`, the template flag `0x7226b0` refuses (`0x7226c3`).
const CREATURE_TYPE_FLAG_NO_PET_BAR: u32 = 0x2000;

/// `PetHasActionBar`'s `0x5d35b0` past the count and guid: an unresolved pet passes (`0x5d35e4`);
/// a resolved one needs a template without `NO_PET_BAR` and health above 0 (`0x5d35f9`).
/// Labelled, not built: `[0xc1e5a0]` bit 0 (our mover is the pet, `0x5d4ad0`), whose bonus-bar
/// takeover (`0x5ab800`) is not built either; `0x7226b0`'s leg for a pet we ride as a vehicle
/// (`VehicleSeat` `VehicleAbilityDisplay`); and the possess latch (`0x5d35ff`-`0x5d3613`).
pub(crate) fn pet_bar_shows(pet: Option<&ObjectStore>, type_flags: Option<u32>) -> bool {
    let Some(pet) = pet else { return true };
    type_flags.unwrap_or(0) & CREATURE_TYPE_FLAG_NO_PET_BAR == 0
        && pet.0.unit_health().unwrap_or(0) > 0
}

/// The flush behind every mutation of the pet's cooldown list: `0x4b31b0` fires
/// `ACTIONBAR_UPDATE_COOLDOWN` (`0x4e5c60`, event `0xd8`) then `SPELL_UPDATE_COOLDOWN` (`0x106`),
/// and `0x4bce90` fires `PET_BAR_UPDATE_COOLDOWN` (`0x162`). Its callers are the pet's
/// `SMSG_SPELL_GO` leg (`0x6e85fc`), `StartGlobalCooldown` for the list (`0x6e2e77`, `0x6e2e8e`),
/// `SMSG_SPELL_COOLDOWN` for the pet's guid (`0x6e95b0`, `0x6e95b9`), `SMSG_COOLDOWN_EVENT` and
/// `SMSG_CLEAR_COOLDOWN` (`0x6e3071`, `0x6e3080`) and `SMSG_COOLDOWN_CHEAT` (`0x6e9712`,
/// `0x6e971c`); each is a change of [`Cooldowns::generation`](crate::spell::Cooldowns), and one
/// frame's changes fire it once. `SMSG_PET_SPELLS` seeds without it, so the events never fire for
/// a rebuild. Runs after [`CooldownEvents`](crate::ui_action::CooldownEvents): every feed that
/// pushes a pet cooldown, the bar's and the book's, has pushed by then, so the handlers read the
/// new list.
pub fn fire_pet_cooldown_events(
    script: Option<NonSendMut<UiScript>>,
    bar: Res<PetBar>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<crate::ui_script::gate::Watch>>,
) {
    let Some(mut script) = script else {
        return;
    };
    // A new VM starts from the list as it stands: the reference flushes on a mutation, not at load.
    let (watch, fresh) = memory.get_reset(&script);
    let moved = watch.moved(bar.cooldowns.generation);
    if !moved || fresh {
        return;
    }
    script.fire_event("ACTIONBAR_UPDATE_COOLDOWN", vec![]);
    script.fire_event("SPELL_UPDATE_COOLDOWN", vec![]);
    script.fire_event("PET_BAR_UPDATE_COOLDOWN", vec![]);
}

/// Native words retain all 24 ID bits. Do not route through `PetActionEntry::action()`.
fn native_slot_view(
    entry: wrathplus_protocol::wrath_pet::Action,
    pet: &wrathplus_protocol::wrath_pet::PetSpells,
    spell: Option<&wrathplus_formats::SpellDisplay>,
    cooldown: Option<(i64, u32, bool)>,
    attacking: bool,
    store: Option<&ObjectStore>,
    auras: Option<&[crate::net::wrath_auras::StampedAura]>,
) -> PetActionView {
    let kind = entry.raw_type() & 0x3f;
    let id = entry.id();
    let mut view = PetActionView {
        packed: entry.0,
        // `0x5d3fd0`: a token or an empty slot leaves the answer to the bar's usability.
        usable: true,
        ..Default::default()
    };
    if kind == 7 {
        if let Some((name, texture)) = command_token(id) {
            view.name = Some(name.into());
            view.texture = Some(texture.into());
            view.is_token = true;
            view.attack_active = attacking && id == PET_COMMAND_ATTACK;
            view.active =
                (pet.flags & 0x0800 == 0 && u32::from(pet.command) == id) || view.attack_active;
        }
        return view;
    }
    if kind == 6 {
        if let Some((name, texture)) = reaction_token(id) {
            view.name = Some(name.into());
            view.texture = Some(texture.into());
            view.is_token = true;
            let react = if pet.flags & 0x0800 != 0 {
                0
            } else {
                pet.react
            };
            view.active = u32::from(react) == id;
        }
        return view;
    }
    if id == 0 || !(kind == 1 || (8..=17).contains(&kind)) {
        return view;
    }
    let Some(spell) = spell else {
        return view;
    };
    let active =
        auras.is_some_and(|a| crate::ui_action::toggle::active_action_toggle(id, spell, a));
    view.name = Some(spell.name.clone());
    view.subtext = spell.rank.clone();
    view.texture = if active {
        spell.active_icon.clone()
    } else {
        spell.icon.clone()
    };
    view.spell_id = Some(id);
    view.passive = spell.passive;
    view.usable = store.is_none_or(|s| pet_may_cast(spell, s.0.unit_flags()));
    view.autocast_allowed = entry.0 & 0x8000_0000 != 0;
    view.autocast_enabled = entry.0 & 0x4000_0000 != 0;
    view.cooldown = cooldown;
    view
}

/// `0x5d3fd0`'s refusals for one pet-bar unit: stunned, fleeing or confused without the spell's
/// matching `AttributesEx5` usable-while bit (`0x8`, `0x20000`, `0x40000`). The native client
/// refuses only when every pet-bar unit refuses; the bar here drives one. Its further cast check
/// (`0x80b5d0`) is not built.
fn pet_may_cast(spell: &wrathplus_formats::SpellDisplay, unit_flags: u32) -> bool {
    const STUNNED: u32 = 0x0004_0000;
    const CONFUSED: u32 = 0x0040_0000;
    const FLEEING: u32 = 0x0080_0000;
    let ex5 = spell.attributes_ex5;
    !(unit_flags & STUNNED != 0 && ex5 & 0x8 == 0
        || unit_flags & FLEEING != 0 && ex5 & 0x2_0000 == 0
        || unit_flags & CONFUSED != 0 && ex5 & 0x4_0000 == 0)
}

#[cfg(test)]
mod native_view_tests {
    use super::*;
    use wrathplus_protocol::wrath_pet::{Action, PetSpells};

    #[test]
    fn a_stunned_feared_or_confused_pet_may_cast_only_spells_marked_for_it() {
        let plain = wrathplus_formats::SpellDisplay::default();
        assert!(pet_may_cast(&plain, 0));
        for (flag, bit) in [
            (0x0004_0000, 0x8),
            (0x0080_0000, 0x2_0000),
            (0x0040_0000, 0x4_0000),
        ] {
            assert!(!pet_may_cast(&plain, flag), "{flag:#x}");
            let marked = wrathplus_formats::SpellDisplay {
                attributes_ex5: bit,
                ..Default::default()
            };
            assert!(pet_may_cast(&marked, flag), "{flag:#x} with {bit:#x}");
        }
    }

    #[test]
    fn native_view_keeps_full_id_and_raw_word() {
        let word = 0xc112_abcd;
        let display = wrathplus_formats::SpellDisplay {
            name: "Native".into(),
            ..Default::default()
        };
        let view = native_slot_view(
            Action(word),
            &PetSpells::default(),
            Some(&display),
            None,
            false,
            None,
            None,
        );
        assert_eq!(view.packed, word);
        assert_eq!(view.spell_id, Some(0x12_abcd));
        assert!(view.autocast_allowed && view.autocast_enabled);
    }

    #[test]
    fn missing_catalog_and_unknown_types_keep_raw_word_without_fabricating_spell() {
        for word in [0x8112_abcd, 0x0512_abcd, 0x8100_0000] {
            let view = native_slot_view(
                Action(word),
                &PetSpells::default(),
                None,
                None,
                false,
                None,
                None,
            );
            assert_eq!(view.packed, word);
            assert!(view.name.is_none());
            assert!(view.spell_id.is_none());
        }
    }
}
