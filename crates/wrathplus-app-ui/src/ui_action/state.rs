//! Each action slot's per-frame state behind `IsUsableAction`, `IsActionInRange`,
//! `IsCurrentAction` and `GetActionCooldown`, diff-pushed into the VM with the reference's events
//! on each change: the cooldown trio (`0x4b31b0`, `0x4f93d0`), the usable pair (`0x4b31c0`, on a
//! cache change only, `0x4e5c00`), the state pair (`0x4b3250`), combat enter and leave
//! (`0x6256ff`, `0x625778`), and autorepeat start and stop (`0x6e5952`, `0x6ea170`). Usability is
//! `IsSpellUsableNow 0x6e3d60` ([`crate::spell::usable`]), auto-repeat the `0xceac30` key, and
//! range the squared distance against `GetMinMaxRange 0x6e3480`.

use crate::inventory::carried_counts;
use std::collections::HashMap;
use std::time::Instant;

use bevy::prelude::*;

use wrathplus_formats::RangeTargets;
use wrathplus_protocol::types::{ACTION_KIND_ITEM, ACTION_KIND_SPELL};
use wrathplus_ui::script::{ActionState, UiScript};

use crate::app_core::cooldowns::Cooldowns;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Selection;
use crate::creature_anim::{Casting, Engaged};
use crate::items::Items;
use crate::net::NetCommands;

use super::{PlayerActions, Spells};
use crate::spell::usable;
use crate::spell::AutoRepeatActive;

/// The feed's memory: what was last pushed, and the edge detectors.
#[derive(Default)]
pub(super) struct StateMemory {
    pushed: HashMap<u32, ActionState>,
    last_generation: Option<u64>,
    engaged: bool,
    auto_repeat: Option<u32>,
    last_cd_trace: Option<Instant>,
}

/// A slot after the macro indirection (`0x4e5a50`), as the usable compute `0x4e5050` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotResolve {
    /// A spell or item slot, or a macro whose bound spell is live (`[rec+0x564] > 0`).
    Action(u8, u32),
    /// A macro that casts nothing (`[rec+0x564] == 0`): usable while it exists (`0x4e5030`),
    /// with no cooldown, range or check (`0x4e50f4`-`0x4e516f`).
    BareMacro,
    /// Grey with nothing to report: a `/cast` that did not resolve (`-1`, refused at
    /// `0x4e518b`), or a macro that no longer exists.
    Dead,
}

/// Resolves a slot through its macro: every `Is*Action`/`GetActionCooldown` binding goes through
/// `0x4e5a50`, so a macro bound to Fireball (or to an item) is that slot, its own icon aside.
fn resolve_through_macro(
    kind: u8,
    action: u32,
    bound: &crate::ui_macro::MacroBoundSpells,
) -> SlotResolve {
    use crate::ui_macro::BoundSpell;
    match wrathplus_ui::script::macro_action_id(kind, action) {
        Some(id) => match bound.0.get(&id) {
            Some(BoundSpell::Spell(s)) => SlotResolve::Action(ACTION_KIND_SPELL, *s),
            Some(BoundSpell::Item(i)) => SlotResolve::Action(ACTION_KIND_ITEM, *i),
            Some(BoundSpell::None) => SlotResolve::BareMacro,
            Some(BoundSpell::Unresolved) | None => SlotResolve::Dead,
        },
        None => SlotResolve::Action(kind, action),
    }
}

/// Computes and diff-pushes every occupied slot's state, and fires the reference's events.
#[allow(clippy::type_complexity)] // a Bevy system's full input set
pub(super) fn feed_action_state(
    script: Option<NonSendMut<UiScript>>,
    actions: Res<PlayerActions>,
    spells: Option<Res<Spells>>,
    mut cooldowns: ResMut<Cooldowns>,
    clock: Res<crate::app_core::ui_flags::UiClock>,
    auto_repeat: Res<AutoRepeatActive>,
    // One tuple for Bevy's 16-param ceiling: cast tracking, macro bindings, spell mods and the
    // range units.
    cast_state: (
        Res<crate::spell::PendingCast>,
        Res<crate::spell::QueuedMeleeSpell>,
        Res<crate::spell::ActiveChannel>,
        Res<crate::spell::SpellTargeting>,
        Res<crate::ui_macro::MacroBoundSpells>,
        Res<crate::spell::SpellModifiers>,
        Res<crate::net::wrath_combo_points::NativeComboPoints>,
        // The caster's and the target's reach and motion, which the range compare reads.
        crate::spell::RangeUnits,
        // The usable walk's `CanAttack`/`CanAssist` inputs.
        crate::target::RelationInputs,
    ),
    self_q: Query<(&ObjectStore, &Transform, Has<Engaged>, Option<&Casting>), With<SelfPlayer>>,
    selection: Res<Selection>,
    // One param for the guid index and the item store, for the same ceiling.
    objects: crate::net::Objects,
    units: Query<(&ObjectStore, &Transform), Without<SelfPlayer>>,
    factions: Option<Res<crate::app_core::state::Factions>>,
    reputations: Res<crate::net::Reputations>,
    items: Res<Items>,
    commands: Res<NetCommands>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<StateMemory>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let memory = memory.get(&script);
    let now = Instant::now();
    // Every cooldown pushes its absolute start on the `GetTime` clock, derived from this one
    // pair, so a running cooldown derives the same start every frame.
    let (anchor, ui_now) = (clock.anchor, clock.ui_now);
    cooldowns.prune(now);
    let gen_changed = memory.last_generation != Some(cooldowns.generation);
    memory.last_generation = Some(cooldowns.generation);
    // The cooldown-clock trace (`WOW_MOVE_TRACE` sink, tag "cd"): once per second.
    let trace_cd = wrathplus_assets::trace::enabled()
        && memory
            .last_cd_trace
            .is_none_or(|t| now.duration_since(t).as_secs_f32() >= 1.0);
    if trace_cd {
        memory.last_cd_trace = Some(now);
    }

    let (
        pending,
        queued_melee,
        channel,
        targeting,
        bound,
        spell_mods,
        combo,
        range_units,
        relations,
    ) = &cast_state;
    let me = self_q.iter().next();
    // The bags, walked once per frame for every reagent, totem and item count below.
    let carried = me
        .map(|(s, _, _, _)| carried_counts(&s.0, &objects))
        .unwrap_or_default();
    let engaged = me.is_some_and(|(_, _, e, _)| e);
    let form_byte = me
        .map(|(s, _, _, _)| s.0.unit_shapeshift_form())
        .unwrap_or(0);
    let casting_spell = me.and_then(|(_, _, _, c)| c.map(|c| c.spell_id));
    let current_cast = pending.current(now).or(casting_spell);
    // Build 12340's `IsCurrentSpell` (`0x806030`): the same in-flight arms as `IsCurrentAction`
    // without the form arm, plus the running auto-repeat and Attack (6603, the one
    // `SPELL_EFFECT_ATTACK` spell a player knows) while engaged.
    let current_spells: Vec<u32> = [
        current_cast,
        queued_melee.current(),
        channel.current(now),
        targeting.spell(),
        auto_repeat.0,
        engaged.then_some(6603),
    ]
    .into_iter()
    .flatten()
    .collect();
    let caster = range_units.caster();
    let self_pos = me.map(|(_, t, _, _)| t.translation);
    // The target's reach, motion and squared distance (the reference tests dx²+dy²+dz²,
    // `0x6e47b0`).
    let target_entity = selection.guid.and_then(|g| objects.entity(g));
    let target = target_entity.and_then(|e| units.get(e).ok());
    let target_unit = target_entity
        .filter(|_| target.is_some())
        .and_then(|e| range_units.unit(e));
    let dist_sq = match (self_pos, target) {
        (Some(a), Some((_, t))) => Some(a.distance_squared(t.translation)),
        _ => None,
    };

    let mut fresh: HashMap<u32, ActionState> = HashMap::new();
    for (&slot, button) in &actions.buttons {
        let action = u32::from(slot) + 1;
        // `0x5a8e40` leaves enable zero until a spell/item cooldown reader resolves.
        let mut st = ActionState {
            cooldown: Some((0, 0, false)),
            ..Default::default()
        };
        let (kind, id) = match resolve_through_macro(button.kind, button.action, bound) {
            SlotResolve::Action(kind, id) => (kind, id),
            SlotResolve::BareMacro => {
                // `0x4e5050`'s spell-less leg: usable. It also greys every spell-less macro and
                // item while the player-control flag `[0xb4b3e4]` is clear (taxi, fear, charm,
                // `0x4958e0`; set at boot, `0x48f626`); that gate is not built.
                st.usable = true;
                fresh.insert(action, st);
                continue;
            }
            SlotResolve::Dead => {
                fresh.insert(action, st);
                continue;
            }
        };
        let button = &wrathplus_protocol::types::ActionButton {
            slot,
            action: id,
            kind,
        };
        match button.kind {
            ACTION_KIND_SPELL => {
                let d = spells.as_ref().and_then(|s| s.catalog.get(button.action));
                let Some(d) = d else {
                    fresh.insert(action, st);
                    continue;
                };
                st.is_attack = d.is_melee_auto_attack();
                // `IsCurrentAction 0x4e53a0`: Attack while engaged; a spell while in flight
                // (`0xceca88`, which a queued on-swing strike holds), channelled (`0xceac58`), or
                // when its form is ours (`0x4e5556`). The form arm is not the icon's aura scan:
                // a form granted by another spell lights the check but not the icon.
                st.current = if st.is_attack {
                    engaged
                } else {
                    current_cast == Some(button.action)
                        || queued_melee.current() == Some(button.action)
                        || channel.current(now) == Some(button.action)
                        // Checked while its ground click is pending (`0x4e54d0`, `0x6e48e0`).
                        || targeting.spell() == Some(button.action)
                        || (form_byte != 0 && d.shapeshift_form == Some(u32::from(form_byte)))
                };
                st.auto_repeat = auto_repeat.0 == Some(button.action);
                // The usable walk (`0x6e3d60`); only its power gate sets `notEnoughMana`.
                if let (Some((store, _, _, _)), Some(sp)) = (me, spells.as_deref()) {
                    let ctx = usable::UsableCtx {
                        store,
                        target_store: target.map(|(s, _)| s),
                        factions: factions.as_deref(),
                        reputations: &reputations,
                        cooldowns: &cooldowns,
                        carried: &carried,
                        spell_mods,
                        combo,
                        relations: relations.context(),
                    };
                    let (u, oom) = usable::spell_usable(
                        button.action,
                        d,
                        sp,
                        &ctx,
                        &objects,
                        &items,
                        &commands,
                    );
                    st.usable = u;
                    st.not_enough_mana = oom;
                } else {
                    st.usable = true;
                }
                let row = spells.as_ref().and_then(|s| s.ranges.get(d.range_index));
                // `ActionHasRange 0x4e5810` asks `GetMinMaxRange` with a null target (`0x4e5852`)
                // and tests `|min|` and `|max|` against `FLT_EPSILON`. The auto-attack target the
                // melee arm falls back to cannot move that: its max is at least the 5.0 floor.
                st.has_range = spell_mods
                    .min_max_range(d, row, caster, RangeTargets::default())
                    .is_some_and(|(min, max)| min.abs() > f32::EPSILON || max.abs() > f32::EPSILON);
                // `IsActionInRange 0x4e56f0` runs `CanTargetUnit 0x6e4440`, which binds the selected
                // target, then `IsTargetInRange 0x6e47b0`, which hands `GetMinMaxRange` that target
                // (`0x6e47ca`): its reach pads a ranged row, and its motion with the caster's earns
                // the moving bonus. Nil without a target.
                st.in_range = match dist_sq {
                    Some(d2) if st.has_range => {
                        let targets = RangeTargets {
                            target: target_unit,
                            attack_target: None,
                        };
                        spell_mods
                            .min_max_range(d, row, caster, targets)
                            .map(|(min, max)| d2 >= min * min && d2 <= max * max)
                    }
                    _ => None,
                };
                let info = cooldowns.info(button.action, 0, Some(d), now);
                st.cooldown = info.ui_triple(anchor, ui_now);
                if st.cooldown.is_some() && trace_cd {
                    // The store's clock against the widget's; the sink stamps wall time.
                    wrathplus_assets::trace::line(
                        "cd",
                        &format!(
                            "tick action={} rem={}ms dur={}ms engine_now={ui_now:.3}",
                            button.action, info.remaining_ms, info.duration_ms,
                        ),
                    );
                }
            }
            ACTION_KIND_ITEM => {
                // Only the `Copy` on-use spell, not a clone of the template per slot per frame.
                let template = items.template(button.action, 0, &commands);
                if template.is_some() {
                    st.cooldown = None;
                }
                let use_spell = template.and_then(|t| t.use_spell);
                // `IsConsumableAction` (`0x4e5250`) reads only the template, so the identity
                // feed pushes it with the count it gates, not this one.
                let count = carried.get(&button.action).copied().unwrap_or(0);
                // `IsEquippedAction`, the green border: worn in any equipment slot (0..18).
                st.equipped = me.is_some_and(|(s, _, _, _)| {
                    (0..19).any(|i| {
                        s.0.player_inv_slot(i)
                            .and_then(|g| objects.object(g))
                            .and_then(|o| o.object_entry())
                            == Some(button.action)
                    })
                });
                // `0x4e5050`'s item arm runs the count, `IsItemOnCooldown` and the on-use spell
                // through the same `0x6e3d60` walk. With no player the reference answers (0, 0)
                // (`0x4e5080`), the default state.
                if let Some((store, _, _, _)) = me {
                    let ctx = usable::UsableCtx {
                        store,
                        target_store: target.map(|(s, _)| s),
                        factions: factions.as_deref(),
                        reputations: &reputations,
                        cooldowns: &cooldowns,
                        carried: &carried,
                        spell_mods,
                        combo,
                        relations: relations.context(),
                    };
                    let (u, oom) = usable::item_usable(
                        button.action,
                        use_spell.as_ref(),
                        count > 0 || st.equipped,
                        &ctx,
                        spells.as_deref(),
                        &objects,
                        &items,
                        &commands,
                    );
                    st.usable = u;
                    st.not_enough_mana = oom;
                }
                if let Some(u) = use_spell {
                    let d = spells.as_ref().and_then(|s| s.catalog.get(u.spell_id));
                    let info = cooldowns.info(u.spell_id, button.action, d, now);
                    st.cooldown = info.ui_triple(anchor, ui_now);
                }
            }
            _ => {}
        }
        // The triple holds the absolute start, so a running cooldown diffs equal every frame and
        // a re-arm, even within one frame gap, restarts the sweep.
        fresh.insert(action, st);
    }

    let mut usable_changed = false;
    let mut state_changed = false;
    let keys: Vec<u32> = fresh
        .keys()
        .chain(memory.pushed.keys())
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    for action in keys {
        let (new, old) = (fresh.get(&action), memory.pushed.get(&action));
        if new == old {
            continue;
        }
        let d = ActionState::default();
        let (n, o) = (new.unwrap_or(&d), old.unwrap_or(&d));
        if (n.usable, n.not_enough_mana) != (o.usable, o.not_enough_mana) {
            usable_changed = true;
        }
        if (n.current, n.auto_repeat) != (o.current, o.auto_repeat) {
            state_changed = true;
        }
        if n.cooldown != o.cooldown && wrathplus_assets::trace::enabled() {
            wrathplus_assets::trace::line(
                "cd",
                &format!(
                    "push action={action} cooldown={:?} engine_now={:.3}",
                    n.cooldown,
                    script.now()
                ),
            );
        }
        script.set_action_state(action, new.copied());
    }
    memory.pushed = fresh;
    script.set_current_spells(current_spells);

    // The events in the reference's flush order, the `ACTIONBAR_*` one first.
    if gen_changed {
        script.fire_event("ACTIONBAR_UPDATE_COOLDOWN", vec![]);
        crate::ui_shapeshift::fire_spell_update_cooldown(&mut script);
        script.fire_event("BAG_UPDATE_COOLDOWN", vec![]);
    }
    if usable_changed {
        script.fire_event("ACTIONBAR_UPDATE_USABLE", vec![]);
        script.fire_event("SPELL_UPDATE_USABLE", vec![]);
    }
    if state_changed {
        script.fire_event("ACTIONBAR_UPDATE_STATE", vec![]);
        script.fire_event("CURRENT_SPELL_CAST_CHANGED", vec![]);
    }
    if engaged != memory.engaged {
        memory.engaged = engaged;
        script.fire_event(
            if engaged {
                "PLAYER_ENTER_COMBAT"
            } else {
                "PLAYER_LEAVE_COMBAT"
            },
            vec![],
        );
    }
    if auto_repeat.0 != memory.auto_repeat {
        memory.auto_repeat = auto_repeat.0;
        script.fire_event(
            if auto_repeat.0.is_some() {
                "START_AUTOREPEAT_SPELL"
            } else {
                "STOP_AUTOREPEAT_SPELL"
            },
            vec![],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_formats::SpellDisplay;
    use wrathplus_protocol::types::Field;

    /// `[rec+0x564]`'s three values (`0x4e5a50`): a live spell, a bare macro, and grey.
    #[test]
    fn a_macro_slot_resolves_through_its_bound_spell() {
        use crate::ui_macro::BoundSpell;
        use wrathplus_protocol::types::ACTION_KIND_MACRO;

        let mut bound = crate::ui_macro::MacroBoundSpells::default();
        bound.0.insert(3, BoundSpell::Spell(133)); // macro 3 casts Fireball
        bound.0.insert(4, BoundSpell::None); // macro 4 is `.spawn 16032`
        bound.0.insert(5, BoundSpell::Unresolved); // macro 5 is `/cast Pyroblast`, unknown

        assert_eq!(
            resolve_through_macro(ACTION_KIND_MACRO, 3, &bound),
            SlotResolve::Action(ACTION_KIND_SPELL, 133),
            "from here down the macro IS the Fireball slot"
        );
        assert_eq!(
            resolve_through_macro(ACTION_KIND_MACRO, 4, &bound),
            SlotResolve::BareMacro,
            "a macro that casts nothing is usable, with no cooldown, no range"
        );
        assert_eq!(
            resolve_through_macro(ACTION_KIND_MACRO, 5, &bound),
            SlotResolve::Dead,
            "a /cast of an unknown spell is the reference's -1: grey"
        );
        assert_eq!(
            resolve_through_macro(ACTION_KIND_MACRO, 6, &bound),
            SlotResolve::Dead,
            "a slot whose macro no longer exists: grey"
        );
        assert_eq!(
            resolve_through_macro(ACTION_KIND_SPELL, 133, &bound),
            SlotResolve::Action(ACTION_KIND_SPELL, 133)
        );
        assert_eq!(
            resolve_through_macro(ACTION_KIND_ITEM, 117, &bound),
            SlotResolve::Action(ACTION_KIND_ITEM, 117)
        );
    }

    /// `IsActionInRange` (`0x4e56f0`) hands `GetMinMaxRange` the selected unit, so a ranged row
    /// pads by that unit's reach and the player's alone: a Fireball (0-35) button reads in range
    /// out to 38 yards from a 1.5-reach target, whoever the player swings at, and `ActionHasRange`
    /// (`0x4e5810`, a null target) says the button has a range.
    #[test]
    fn a_ranged_button_pads_its_range_by_the_selected_target_alone() {
        use wrathplus_protocol::types::ActionButton;
        use wrathplus_protocol::ObjectFields;

        const FIREBALL: u32 = 133;
        const SWUNG_AT: u64 = 0xA1;
        const SELECTED: u64 = 0xB2;
        /// `UNIT_FIELD_COMBATREACH`.
        const REACH: u16 = Field::UnitCombatreach.index();

        let verdict = |distance: f32| {
            let (tx, _rx) = crossbeam_channel::unbounded();
            let mut app = App::new();
            let mut actions = PlayerActions::default();
            actions.buttons.insert(
                0,
                ActionButton {
                    slot: 0,
                    action: FIREBALL,
                    kind: ACTION_KIND_SPELL,
                },
            );
            let fireball = SpellDisplay {
                range_index: 4,
                ..Default::default()
            };
            let row = wrathplus_formats::SpellRange {
                min: 0.0,
                max: 35.0,
                flags: 0,
            };
            app.insert_resource(actions)
                .insert_resource(crate::ui_macro::MacroBoundSpells::default())
                .insert_resource(Spells {
                    catalog: wrathplus_formats::SpellCatalog::from_displays(
                        [(FIREBALL, fireball)].into_iter().collect(),
                    ),
                    forms: Default::default(),
                    ranges: wrathplus_formats::SpellRangeCatalog::from_rows(
                        [(4, row)].into_iter().collect(),
                    ),
                    cast_times: Default::default(),
                    durations: Default::default(),
                    radii: Default::default(),
                })
                .init_resource::<Cooldowns>()
                .init_resource::<crate::spell::SpellModifiers>()
                .init_resource::<crate::net::wrath_combo_points::NativeComboPoints>()
                .init_resource::<crate::app_core::ui_flags::UiClock>()
                .init_resource::<AutoRepeatActive>()
                .init_resource::<crate::spell::PendingCast>()
                .init_resource::<crate::spell::QueuedMeleeSpell>()
                .init_resource::<crate::spell::ActiveChannel>()
                .init_resource::<crate::spell::SpellTargeting>()
                .init_resource::<crate::spell::HeldForPick>()
                .init_resource::<crate::net::GuidIndex>()
                .init_resource::<crate::net::Reputations>()
                .init_resource::<Items>()
                .insert_resource(NetCommands(tx));
            // The player swings at a 4.0-reach unit, close by.
            let swung_at = app
                .world_mut()
                .spawn((
                    Transform::from_xyz(2.0, 0.0, 0.0),
                    ObjectStore(ObjectFields::from_pairs(&[
                        (22, 100),
                        (REACH, 4.0f32.to_bits()),
                    ])),
                ))
                .id();
            app.world_mut().spawn((
                SelfPlayer,
                Engaged(SWUNG_AT),
                Transform::default(),
                ObjectStore(ObjectFields::from_fields(&[
                    (Field::UnitHealth, 100),
                    (Field::UnitPower, 500),
                ])),
            ));
            // The selected unit keeps the descriptor's default 1.5 reach.
            let selected = app
                .world_mut()
                .spawn((
                    Transform::from_xyz(distance, 0.0, 0.0),
                    ObjectStore(ObjectFields::from_fields(&[(Field::UnitHealth, 100)])),
                ))
                .id();
            let mut index = app.world_mut().resource_mut::<crate::net::GuidIndex>();
            index.0.insert(SWUNG_AT, swung_at);
            index.0.insert(SELECTED, selected);
            app.insert_resource(Selection {
                target: Some(selected),
                guid: Some(SELECTED),
                last: None,
            });
            app.insert_non_send_resource(UiScript::new().unwrap());
            app.add_systems(Update, feed_action_state);
            app.update();
            let script = app.world().non_send_resource::<UiScript>();
            assert!(script
                .eval::<bool>("return ActionHasRange(1) == 1")
                .unwrap());
            script
                .eval::<i64>("return IsActionInRange(1) or -1")
                .unwrap()
        };

        // 35 + the caster's 1.5 + the selected unit's 1.5 = 38.
        assert_eq!(verdict(37.5), 1, "inside the padded max");
        assert_eq!(
            verdict(38.5),
            0,
            "past it: the swing's 4.0 reach pads nothing"
        );
    }

    /// `IsActionInRange` hands `GetMinMaxRange` the selected unit, so a melee button reads in
    /// range out to 7.667 yards while the player and the selected mob both run, and to the 5.0
    /// floor otherwise (`0x6e3648`-`0x6e36a2`).
    #[test]
    fn a_melee_button_reaches_the_moving_bonus_while_both_units_run() {
        use wrathplus_protocol::types::ActionButton;
        use wrathplus_protocol::ObjectFields;

        const HAMSTRING: u32 = 1715;
        const SELECTED: u64 = 0xB2;

        let verdict = |caster_runs: bool, mob_runs: bool, distance: f32| {
            let (tx, _rx) = crossbeam_channel::unbounded();
            let mut app = App::new();
            let mut actions = PlayerActions::default();
            actions.buttons.insert(
                0,
                ActionButton {
                    slot: 0,
                    action: HAMSTRING,
                    kind: ACTION_KIND_SPELL,
                },
            );
            let melee = wrathplus_formats::SpellRange {
                min: 0.0,
                max: 5.0,
                flags: 1,
            };
            app.insert_resource(actions)
                .insert_resource(crate::ui_macro::MacroBoundSpells::default())
                .insert_resource(Spells {
                    catalog: wrathplus_formats::SpellCatalog::from_displays(
                        [(
                            HAMSTRING,
                            SpellDisplay {
                                range_index: 2,
                                ..Default::default()
                            },
                        )]
                        .into_iter()
                        .collect(),
                    ),
                    forms: Default::default(),
                    ranges: wrathplus_formats::SpellRangeCatalog::from_rows(
                        [(2, melee)].into_iter().collect(),
                    ),
                    cast_times: Default::default(),
                    durations: Default::default(),
                    radii: Default::default(),
                })
                .init_resource::<Cooldowns>()
                .init_resource::<crate::spell::SpellModifiers>()
                .init_resource::<crate::net::wrath_combo_points::NativeComboPoints>()
                .init_resource::<crate::app_core::ui_flags::UiClock>()
                .init_resource::<AutoRepeatActive>()
                .init_resource::<crate::spell::PendingCast>()
                .init_resource::<crate::spell::QueuedMeleeSpell>()
                .init_resource::<crate::spell::ActiveChannel>()
                .init_resource::<crate::spell::SpellTargeting>()
                .init_resource::<crate::spell::HeldForPick>()
                .init_resource::<crate::net::GuidIndex>()
                .init_resource::<crate::net::Reputations>()
                .init_resource::<Items>()
                .insert_resource(crate::app_core::player::Player::with_move_flags(
                    if caster_runs {
                        crate::app_core::movement::move_flags::FORWARD
                    } else {
                        0
                    },
                ))
                .insert_resource(NetCommands(tx));
            let speeds = crate::net::UnitSpeeds(wrathplus_protocol::MoveSpeeds {
                walk: 2.5,
                run: 7.0,
                run_back: 4.5,
                swim: 4.7,
                swim_back: 2.5,
                turn_rate: 3.1,
                flight: 7.0,
                flight_back: 4.5,
                pitch_rate: std::f32::consts::PI,
            });
            app.world_mut().spawn((
                SelfPlayer,
                crate::app_core::session::Embodied,
                speeds,
                Transform::default(),
                ObjectStore(ObjectFields::from_fields(&[
                    (Field::UnitHealth, 100),
                    (Field::UnitPower, 500),
                ])),
            ));
            // A mob at the descriptor's default 1.5 reach; a live spline at 8 yd/s is its run.
            let mob = app
                .world_mut()
                .spawn((
                    speeds,
                    Transform::from_xyz(distance, 0.0, 0.0),
                    ObjectStore(ObjectFields::from_fields(&[(Field::UnitHealth, 100)])),
                ))
                .id();
            if mob_runs {
                app.world_mut()
                    .entity_mut(mob)
                    .insert(crate::motion::Spline {
                        points: vec![[distance, 0.0, 0.0], [distance + 16.0, 0.0, 0.0]],
                        start: Instant::now(),
                        duration: std::time::Duration::from_secs(2),
                        id: 1,
                        grounded: true,
                        interpolation: wrathplus_protocol::SplineInterpolation::Linear,
                        run_mode: Some(true),
                        deck: None,
                    });
            }
            app.world_mut()
                .resource_mut::<crate::net::GuidIndex>()
                .0
                .insert(SELECTED, mob);
            app.insert_resource(Selection {
                target: Some(mob),
                guid: Some(SELECTED),
                last: None,
            });
            app.insert_non_send_resource(UiScript::new().unwrap());
            app.add_systems(Update, feed_action_state);
            app.update();
            let script = app.world().non_send_resource::<UiScript>();
            script
                .eval::<i64>("return IsActionInRange(1) or -1")
                .unwrap()
        };

        // 1.5 + 1.5 + 1.3333 is under the 5.0 floor; the bonus goes on the floor: 7.667.
        assert_eq!(verdict(true, true, 7.5), 1, "both running: inside 7.667");
        assert_eq!(verdict(true, true, 8.0), 0, "past it");
        assert_eq!(verdict(false, true, 7.5), 0, "the player stands");
        assert_eq!(verdict(true, false, 7.5), 0, "the mob stands");
        assert_eq!(verdict(false, false, 4.9), 1, "the floor");
    }

    /// An item slot's on-use spell takes the `0x6e3d60` walk (`0x4e5050`), and every shipped
    /// Food/Drink spell has `Attributes` bit 28, `ATTR_NOT_IN_COMBAT` (the walk's leg 8).
    #[test]
    fn food_on_the_bar_greys_while_the_player_is_in_combat() {
        use wrathplus_protocol::types::{ActionButton, ItemUseSpell};
        use wrathplus_protocol::ObjectFields;

        // One ON_USE block casting 433 "Food", whose shipped `Attributes` are `0x18000100`.
        const FOOD_ITEM: u32 = 1487;
        const FOOD_SPELL: u32 = 433;
        // `ITEM_FIELD_STACK_COUNT`, and `PLAYER_FIELD_PACK_SLOT_1`, backpack slot 1.
        const STACK: u16 = Field::ItemStackCount.index();
        const PACK_SLOT_1: u16 = Field::PlayerPackSlot1.index();

        let lit = |in_combat: bool| {
            let (tx, _rx) = crossbeam_channel::unbounded();
            let mut app = App::new();
            let mut actions = PlayerActions::default();
            actions.buttons.insert(
                0,
                ActionButton {
                    slot: 0,
                    action: FOOD_ITEM,
                    kind: ACTION_KIND_ITEM,
                },
            );
            let mut items = Items::default();
            items.insert_template(
                FOOD_ITEM,
                Some(wrathplus_protocol::types::ItemInfo {
                    use_spell: Some(ItemUseSpell {
                        spell_id: FOOD_SPELL,
                        cooldown_ms: -1,
                        category: 0,
                        category_cooldown_ms: -1,
                    }),
                    ..crate::items::test_template("Conjured Muffin")
                }),
            );
            let food = SpellDisplay {
                attributes: 0x1800_0100,
                ..Default::default()
            };
            app.insert_resource(actions)
                .insert_resource(crate::ui_macro::MacroBoundSpells::default())
                .insert_resource(Spells {
                    catalog: wrathplus_formats::SpellCatalog::from_displays(
                        [(FOOD_SPELL, food)].into_iter().collect(),
                    ),
                    forms: Default::default(),
                    ranges: Default::default(),
                    cast_times: Default::default(),
                    durations: Default::default(),
                    radii: Default::default(),
                })
                .init_resource::<Cooldowns>()
                .init_resource::<crate::spell::SpellModifiers>()
                .init_resource::<crate::net::wrath_combo_points::NativeComboPoints>()
                .init_resource::<crate::app_core::ui_flags::UiClock>()
                .init_resource::<AutoRepeatActive>()
                .init_resource::<crate::spell::PendingCast>()
                .init_resource::<crate::spell::QueuedMeleeSpell>()
                .init_resource::<crate::spell::ActiveChannel>()
                .init_resource::<crate::spell::SpellTargeting>()
                .init_resource::<crate::spell::HeldForPick>()
                .init_resource::<Selection>()
                .init_resource::<crate::net::GuidIndex>()
                .init_resource::<crate::net::Reputations>()
                .insert_resource(items)
                .insert_resource(NetCommands(tx));
            crate::items::test_spawn_item(
                app.world_mut(),
                0xF0,
                ObjectFields::from_pairs(&[(Field::ObjectEntry.index(), FOOD_ITEM), (STACK, 10)]),
                false,
            );
            // The player: alive, with the ten muffins in backpack slot 1.
            let flags = (1u32 << 3)
                | if in_combat {
                    crate::app_core::state::UNIT_FLAG_IN_COMBAT
                } else {
                    0
                };
            app.world_mut().spawn((
                SelfPlayer,
                Transform::default(),
                ObjectStore(ObjectFields::from_pairs(&[
                    (Field::UnitHealth.index(), 100),
                    (Field::UnitPower.index(), 500),
                    (Field::UnitFlags.index(), flags),
                    (PACK_SLOT_1, 0xF0),
                ])),
            ));
            app.insert_non_send_resource(UiScript::new().unwrap());
            app.add_systems(Update, feed_action_state);
            app.update();
            app.world()
                .non_send_resource::<UiScript>()
                .eval::<bool>("return (IsUsableAction(1)) and true or false")
                .unwrap()
        };

        assert!(lit(false), "out of combat the muffins are full-colour");
        assert!(
            !lit(true),
            "in combat the food's on-use spell fails leg 8 and the button greys"
        );
    }
}

#[cfg(test)]
#[path = "state_cooldown_tests.rs"]
mod cooldown_tests;
