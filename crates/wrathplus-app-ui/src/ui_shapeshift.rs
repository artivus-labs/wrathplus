//! The stance bar's feed and drain behind `GetNumShapeshiftForms` (`0x4b4590`),
//! `GetShapeshiftFormInfo` (`0x4b45c0`), `CastShapeshiftForm` (`0x4b4810`) and
//! `GetShapeshiftFormCooldown` (`0x4b49a0`). A known spell joins the bar (`0x4b25b0`) unless
//! `AttributesEx2 & 0x2`, when it has a `SPELL_AURA_MOD_SHAPESHIFT` effect or
//! `AttributesEx2 & 0x10`; the bar sorts by `StanceBarOrder`, negative last, then spell id
//! (`0x4b2bb0`).
//!
//! The edges are 12340's. Only a change of membership or order fires `UPDATE_SHAPESHIFT_FORMS`,
//! the learn, unlearn and sort signals (`0x54252d`, `0x53fe44`, `0x542647`), because the stock
//! `ShapeshiftBar_Update` rebuilds the shelf on every fire. A castability flip fires
//! `UPDATE_SHAPESHIFT_USABLE` then `SPELL_UPDATE_USABLE` (`0x53d17f`, `0x53d19c`). A move of the
//! active form fires `UPDATE_SHAPESHIFT_FORM` (`0x6da39a`, `0x6e806f`), as does a stance-bar aura
//! without `MOD_SHAPESHIFT` going up or down (`0x72213f`, `0x727830`), which lights a paladin aura.
//! A cooldown pushes silently: the cooldown flush fires `UPDATE_SHAPESHIFT_COOLDOWN` just ahead of
//! `SPELL_UPDATE_COOLDOWN` (`0x53bac0`), so the feed runs before it; an expiry fires nothing.

use std::time::Instant;

use bevy::prelude::*;

use wrathplus_ui::script::{ShapeshiftFormView, UiScript};

use crate::app_core::cooldowns::Cooldowns;
use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::PlayerActions;
use crate::app_core::state::Selection;
use crate::app_core::state::Spells;
use crate::items::Items;
use crate::net::wrath_auras::{PacketAuras, StampedAura};
use crate::net::{ClientCommand, Guid, NetCommands, Objects, Reputations};
use crate::spell::{usable, CastCommit};

/// Keeps a spell off the stance bar; Ghost Wolf 2645 carries it, so a shaman has no bar.
const ATTR_EX2_STANCE_BAR_EXCLUDE: u32 = 0x2;
/// Admits a spell with no `MOD_SHAPESHIFT` effect; in 1.12 data the live carriers, all with
/// `ActiveIconID` 122, are every rank of the paladin auras (Devotion 465 and the rest) and
/// Ironweave Battlesuit 27733.
const ATTR_EX2_STANCE_BAR_FORCE: u32 = 0x10;

/// `isActive`: 12340's `0x802cb0`, as `GetShapeshiftFormInfo` (`0x53de90`) calls it, forked on the
/// form id (the first `MOD_SHAPESHIFT` misc value, else 0). A form spell matches the form byte (the
/// spell-side arm, kept from 1.12's `4b46a0`). Any other stance-bar spell takes the press-again
/// predicate ([`crate::ui_action::toggle::active_action_toggle`]): a nonzero `ActiveIconID` and its
/// positive aura on the player; a paladin aura lights this way.
fn form_active(
    spell_id: u32,
    d: &wrathplus_formats::SpellDisplay,
    form_byte: u8,
    auras: &[StampedAura],
) -> bool {
    match d.shapeshift_form.unwrap_or(0) {
        0 => crate::ui_action::toggle::active_action_toggle(spell_id, d, auras),
        form => form == u32::from(form_byte),
    }
}

/// `SpellIconID`, or `ActiveIconID` when active and nonzero, under either arm of [`form_active`]:
/// both reach the one block that elects the icon (`4b4754`), so a lit paladin aura wears its
/// `ActiveIconID`. A form spell with the column at 0 is active yet keeps `SpellIconID`
/// (`4b4763 je`).
fn form_texture(d: &wrathplus_formats::SpellDisplay, active: bool) -> Option<String> {
    if active {
        d.active_icon.clone().or_else(|| d.icon.clone())
    } else {
        d.icon.clone()
    }
}

/// What the feed last pushed. The cooldown triple carries the absolute start, so a running
/// cooldown re-derives the same view every frame.
#[derive(Default)]
pub(crate) struct StanceMemory {
    pushed: Option<Vec<ShapeshiftFormView>>,
    /// The stance auras the last push saw ([`push_stance`]).
    stance_auras: Vec<u32>,
}

/// The edge one push crossed, as [`push_forms`] announces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormsEdge {
    Unchanged,
    /// Membership or order: `UPDATE_SHAPESHIFT_FORMS`.
    List,
    /// Castability: `UPDATE_SHAPESHIFT_USABLE` and `SPELL_UPDATE_USABLE`, then
    /// `UPDATE_SHAPESHIFT_FORM` when the active form moved too.
    Usable,
    /// The active form, and with it the texture: `UPDATE_SHAPESHIFT_FORM`.
    Form,
    /// Cooldown only: pushed, announced by the cooldown flush.
    Silent,
}

/// Push the rebuilt list and announce its edge (module doc), leaving the stance auras standing;
/// the tests' entry, callable without a Bevy world.
#[cfg(test)]
pub(crate) fn push_forms(
    script: &mut UiScript,
    memory: &mut StanceMemory,
    fresh: Vec<ShapeshiftFormView>,
) -> FormsEdge {
    push_stance(script, memory, fresh, None)
}

/// [`push_forms`] with the player's stance auras, sorted: the stance-bar spells without
/// `MOD_SHAPESHIFT` whose aura the player carries, or `None` to leave the last set standing.
pub(crate) fn push_stance(
    script: &mut UiScript,
    memory: &mut StanceMemory,
    fresh: Vec<ShapeshiftFormView>,
    stance_auras: Option<Vec<u32>>,
) -> FormsEdge {
    let old: &[ShapeshiftFormView] = memory.pushed.as_deref().unwrap_or(&[]);
    let edge = if old == fresh.as_slice() {
        FormsEdge::Unchanged
    } else if old.len() != fresh.len()
        || old
            .iter()
            .zip(&fresh)
            .any(|(o, n)| o.spell_id != n.spell_id)
    {
        FormsEdge::List
    } else if old
        .iter()
        .zip(&fresh)
        .any(|(o, n)| o.castable != n.castable)
    {
        FormsEdge::Usable
    } else if active_moved(old, &fresh) {
        FormsEdge::Form
    } else {
        FormsEdge::Silent
    };
    let form_too = edge == FormsEdge::Usable && active_moved(old, &fresh);
    if edge != FormsEdge::Unchanged {
        memory.pushed = Some(fresh.clone());
        script.set_shapeshift_forms(fresh);
    }
    match edge {
        FormsEdge::List => script.fire_event("UPDATE_SHAPESHIFT_FORMS", vec![]),
        FormsEdge::Usable => {
            script.fire_event("UPDATE_SHAPESHIFT_USABLE", vec![]);
            script.fire_event("SPELL_UPDATE_USABLE", vec![]);
        }
        FormsEdge::Form | FormsEdge::Silent | FormsEdge::Unchanged => {}
    }
    // The aura sites (`0x72213f` apply, `0x727830` remove) fire on their own edge, after the
    // list's: a stance-bar spell without `MOD_SHAPESHIFT` whose aura came or went on the player.
    let auras_moved = stance_auras.is_some_and(|a| {
        let moved = a != memory.stance_auras;
        memory.stance_auras = a;
        moved
    });
    if edge == FormsEdge::Form || form_too || auras_moved {
        script.fire_event("UPDATE_SHAPESHIFT_FORM", vec![]);
    }
    match edge {
        FormsEdge::Unchanged | FormsEdge::Silent if auras_moved => FormsEdge::Form,
        _ => edge,
    }
}

/// `SPELL_UPDATE_COOLDOWN` as 12340's `0x53bac0` fires it: `UPDATE_SHAPESHIFT_COOLDOWN` just
/// ahead while the stance bar has a form.
pub(crate) fn fire_spell_update_cooldown(script: &mut UiScript) {
    if script.num_shapeshift_forms() > 0 {
        script.fire_event("UPDATE_SHAPESHIFT_COOLDOWN", vec![]);
    }
    script.fire_event("SPELL_UPDATE_COOLDOWN", vec![]);
}

/// Whether any form's active ring or texture moved, over lists of one membership.
fn active_moved(old: &[ShapeshiftFormView], fresh: &[ShapeshiftFormView]) -> bool {
    old.iter()
        .zip(fresh)
        .any(|(o, n)| o.active != n.active || o.texture != n.texture)
}

pub struct UiShapeshiftPlugin;

impl Plugin for UiShapeshiftPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            // The feed precedes the two event sets whose handlers re-read this list. A click's
            // cast is a script call (`cast_form`).
            feed_shapeshift_bar
                .in_set(UnitFeed)
                .before(crate::ui_action::CooldownEvents)
                .before(crate::app_core::schedule::AuraEvents),
        );
    }
}

/// Build the bar from the known spells and the catalog (module doc), and diff-push it.
#[allow(clippy::type_complexity)] // a Bevy system's full input set
fn feed_shapeshift_bar(
    script: Option<NonSendMut<UiScript>>,
    actions: Res<PlayerActions>,
    spells: Option<Res<Spells>>,
    cooldowns: Res<Cooldowns>,
    self_q: Query<(&ObjectStore, &Guid), With<SelfPlayer>>,
    selection: Res<Selection>,
    // The selection's guid index, and the bag walk behind each form's reagent leg.
    objects: Objects,
    units: Query<&ObjectStore, Without<SelfPlayer>>,
    factions: Option<Res<crate::app_core::state::Factions>>,
    reputations: Res<Reputations>,
    items: Res<Items>,
    commands: Res<NetCommands>,
    clock: Res<crate::app_core::ui_flags::UiClock>,
    // One tuple for Bevy's 16-param ceiling: the talent modifiers, build 12340's combo points and
    // the packet auras behind a force-admitted form's `isActive`.
    (spell_mods, combo, packets): (
        Res<crate::spell::SpellModifiers>,
        Res<crate::net::wrath_combo_points::NativeComboPoints>,
        Option<Res<PacketAuras>>,
    ),
    relations: crate::target::RelationInputs,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<StanceMemory>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let memory = memory.get(&script);
    let Some(spells) = spells else {
        return;
    };
    let me = self_q.iter().next();
    let store = me.map(|(s, _)| s);
    let form_byte = store.map(|s| s.0.unit_shapeshift_form()).unwrap_or(0);
    script.set_shapeshift_form_byte(form_byte);
    let auras: &[StampedAura] = me
        .and_then(|(_, g)| packets.as_deref()?.by_guid.get(&g.0))
        .map_or(&[], Vec::as_slice);
    let now = Instant::now();
    // The frame's clock pair: a local `Instant::now()` would wobble the derived start.
    let (anchor, ui_now) = (clock.anchor, clock.ui_now);
    // The usable walk's target leg (Execute) reads the current target.
    let target_store = selection
        .guid
        .and_then(|g| objects.entity(g))
        .and_then(|e| units.get(e).ok());

    // Admission and order (module doc).
    let mut rows: Vec<(u32, &wrathplus_formats::SpellDisplay)> = actions
        .spells
        .iter()
        .filter_map(|&id| {
            let d = spells.catalog.get(id)?;
            let admitted = d.attributes_ex2 & ATTR_EX2_STANCE_BAR_EXCLUDE == 0
                && (d.shapeshift_form.is_some()
                    || d.attributes_ex2 & ATTR_EX2_STANCE_BAR_FORCE != 0);
            admitted.then_some((id, d))
        })
        .collect();
    rows.sort_by_key(|&(id, d)| {
        let order = if d.stance_bar_order < 0 {
            i64::MAX
        } else {
            i64::from(d.stance_bar_order)
        };
        (order, id)
    });
    // The aura sites' membership (`0x53bd40`) and `MOD_SHAPESHIFT` (aura 0x24, `0x7fde50`) tests.
    let mut stance_auras: Vec<u32> = rows
        .iter()
        .filter(|(id, d)| {
            d.shapeshift_form.is_none() && auras.iter().any(|a| a.aura.spell_id == *id)
        })
        .map(|&(id, _)| id)
        .collect();
    stance_auras.sort_unstable();

    // The bags, walked once for every form's reagent leg.
    let carried = store
        .map(|s| crate::inventory::carried_counts(&s.0, &objects))
        .unwrap_or_default();
    let fresh: Vec<ShapeshiftFormView> = rows
        .into_iter()
        .map(|(id, d)| {
            let active = form_active(id, d, form_byte, auras);
            // `isCastable`: true for the active form, else the `0x6e3d60` walk the action bar's
            // `IsUsableAction` runs.
            let castable = active
                || store.is_none_or(|s| {
                    let ctx = usable::UsableCtx {
                        store: s,
                        target_store,
                        factions: factions.as_deref(),
                        reputations: &reputations,
                        cooldowns: &cooldowns,
                        carried: &carried,
                        spell_mods: &spell_mods,
                        combo: &combo,
                        relations: relations.context(),
                    };
                    usable::spell_usable(id, d, &spells, &ctx, &objects, &items, &commands).0
                });
            let texture = form_texture(d, active);
            let cooldown = cooldowns
                .info(id, 0, Some(d), now)
                .ui_triple(anchor, ui_now);
            ShapeshiftFormView {
                spell_id: id,
                texture,
                name: d.name.clone(),
                active,
                castable,
                cooldown,
            }
        })
        .collect();

    let count = fresh.len();
    let edge = push_stance(&mut script, memory, fresh, Some(stance_auras));
    if edge != FormsEdge::Unchanged {
        debug!("ui_shapeshift: {count} form(s), active form byte {form_byte}, {edge:?} edge");
    }
}

/// `CastShapeshiftForm` (12340 `0x53c0f0`, into the cast entry `0x80da80`), forked on the form id
/// like the info call: the active form cancels unless `SpellShapeshiftForm.dbc` `flags1 & 0x2`
/// makes it a silent no-op (`0x4b4963`, warrior stances); a force-admitted spell that is lit (its
/// aura up) cancels, with no DBC guard since it has no form id. Anything else casts.
pub(crate) fn cast_form(cast: &mut crate::spell::ScriptCast, spell_id: u32) {
    let crate::spell::ScriptCast {
        targeting,
        ladder,
        recent,
    } = cast;
    let now = std::time::Instant::now();
    let store = targeting.self_store.iter().next();
    let form_byte = store.map(|s| s.0.unit_shapeshift_form()).unwrap_or(0);
    let d = ladder.spells.as_ref().and_then(|s| s.catalog.get(spell_id));
    // The active form's row, for the `0x4b4963` guard (shared with `crate::ui_action::toggle`).
    let row = ladder
        .spells
        .as_ref()
        .and_then(|s| s.forms.get(&u32::from(form_byte)));
    // The force-admit arm uses `form_active`'s predicate, so a lit button always cancels.
    let disposition = d.and_then(|d| {
        if d.shapeshift_form.unwrap_or(0) == 0 {
            let auras = store.map_or(&[][..], |s| {
                crate::ui_action::toggle::unit_auras(&ladder.objects, s)
            });
            form_active(spell_id, d, form_byte, auras).then_some(true)
        } else {
            crate::ui_action::toggle::form_recast_disposition(d, form_byte, row)
        }
    });
    match disposition {
        // The `secureAbilityToggle` window and the charm guard hold the cancel (`0x80daec`).
        Some(true)
            if store.is_some_and(|s| {
                crate::ui_action::toggle::press_again_held(recent, spell_id, s, now)
            }) => {}
        Some(true) => {
            debug!("ui_shapeshift: cancel form aura {spell_id}");
            let _ = ladder
                .commands
                .0
                .send(ClientCommand::CancelAura { spell_id });
        }
        Some(false) => {}
        None => {
            debug!("ui_shapeshift: cast form {spell_id}");
            recent.record(spell_id, now);
            ladder.send(spell_id, &targeting.context(), CastCommit::Spell);
        }
    }
}

#[cfg(test)]
#[path = "ui_shapeshift_tests.rs"]
mod tests;
