//! The action bar's identity feed: each of the 120 slots' icon and count, pushed to the VM with
//! `ACTIONBAR_SLOT_CHANGED` per changed slot. [`feed_actions`] also drains the UI error queues
//! and feeds the stance page (`GetBonusBarOffset`: the form's `SpellShapeshiftForm.dbc`
//! BonusActionBar column, `0x4e4fc0`).

use crate::inventory::count_of;
use crate::inventory::InventoryScope;
use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use wrathplus_protocol::types::{ACTION_KIND_ITEM, ACTION_KIND_SPELL};
use wrathplus_ui::script::{ActionSlot, ScriptValue, UiScript};

use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfPlayer;
use crate::entities::ItemDisplays;
use crate::items::Items;
use crate::net::NetCommands;

use super::errors::{
    first_missing_totem, first_short_reagent, global_string, mount_result_key,
    ui_error_text_for_script,
};
use super::weapon_icon::{auto_attack_icon, substitutes_weapon_icon};
use super::{
    cast_fail, show_messages, CastErrors, MessageSink, MountErrors, PetTameFailures, PlayerActions,
    Shown, Spells, UiError, UiErrorKeys, UiErrorTexts,
};
use crate::spell::reagents::{has_reagents, reagent_casts};

/// An item action's icon when its template has not answered or its display id has no
/// `ItemDisplayInfo` row: the reference's literal `0x847fe4`, from the resolver's `0x5d8927`.
pub(super) const MISSING_ITEM_ICON: &str = "Interface\\Icons\\INV_Misc_QuestionMark";

/// The feed's memory of what it last pushed, for per-slot change events.
#[derive(Default)]
pub(super) struct FeedMemory {
    pushed: HashMap<u32, ActionSlot>,
    bonus_offset: u8,
    /// Whether the identity resolve has run against this VM: a new VM resets this memory, while
    /// every other input to the gate is host-side and may still match.
    resolved: bool,
    /// The [`Items::template_epoch`] of the last resolve; an advance re-resolves.
    template_epoch: u64,
    /// The macro-table generation of the last resolve: a macro edit changes a bar icon while
    /// touching neither the action table nor any item template.
    macro_generation: u64,
}

/// The DBC name tables the cast-fail argument arms read, as one parameter to stay within Bevy's
/// parameter limit; each is absent without game data. The shapeshift forms ride [`Spells`].
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct FailNameTables<'w> {
    /// `SpellFocusObject.dbc`, for `REQUIRES_SPELL_FOCUS`.
    focus: Option<Res<'w, crate::ui_tradeskill::SpellFocus>>,
    /// `AreaTable.dbc`, for `REQUIRES_AREA`.
    areas: Option<Res<'w, crate::area::AreaTableRes>>,
    /// `SpellMechanic.dbc`, for `PREVENTED_BY_MECHANIC`.
    mechanics: Option<Res<'w, super::SpellMechanics>>,
    /// `TotemCategory.dbc`, for `TOTEM_CATEGORY`.
    categories: Option<Res<'w, crate::ui_tradeskill::TotemCategories>>,
}

impl FailNameTables<'_> {
    fn args<'a>(&'a self, spells: Option<&'a Spells>) -> cast_fail::FailArgs<'a> {
        cast_fail::FailArgs {
            arg: None,
            power_type: None,
            focus: self.focus.as_deref().map(|f| &f.catalog),
            areas: self.areas.as_deref().map(|a| &a.0),
            mechanics: self.mechanics.as_deref().map(|m| &m.catalog),
            categories: self.categories.as_deref().map(|c| &c.0),
            forms: spells.map(|s| &s.forms),
        }
    }
}

pub(super) fn feed_actions(
    script: Option<NonSendMut<UiScript>>,
    mut actions: ResMut<PlayerActions>,
    mut cast_errors: ResMut<CastErrors>,
    mut mount_errors: ResMut<MountErrors>,
    mut pet_tame_failures: ResMut<PetTameFailures>,
    // The by-key and resolved-text queues as one parameter, at Bevy's 16-parameter limit.
    ui_errors: (ResMut<UiErrorKeys>, ResMut<UiErrorTexts>),
    spells: Option<Res<Spells>>,
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    objects: crate::net::Objects,
    items: Res<Items>,
    icons: Option<Res<ItemDisplays>>,
    sub_classes: Option<Res<crate::items::ItemSubClasses>>,
    name_tables: FailNameTables,
    commands: Res<NetCommands>,
    mut memory: Local<crate::app_core::vm_memo::VmMemo<FeedMemory>>,
    mut sink: MessageSink,
) {
    let Some(mut script) = script else {
        return;
    };
    let (mut ui_error_keys, mut ui_error_texts) = ui_errors;
    let memory = memory.get(&script);

    // Cast failures become error lines through [`cast_fail`], all resolved before any event
    // fires. A local `TOTEMS`/`REAGENTS` refusal carries no item, so the drain names the failing
    // one as the pre-send check finds it; an item still in flight requeues the entry, as 12340's
    // cache callback redisplays the line on the answer.
    let self_store = self_q.iter().next();
    let mut await_template: Vec<crate::app_core::errors::CastFail> = Vec::new();
    let mut fail_args = name_tables.args(spells.as_deref());
    fail_args.power_type = self_store.map(|s| s.0.unit_power_type());
    let get = |key: &str| global_string(&script, key);
    let item_name = |entry: u32| match items.template(entry, 0, &commands) {
        Some(t) => cast_fail::ItemName::Named(t.name.clone()),
        None if items.template_answered_unknown(entry) => cast_fail::ItemName::Unknown,
        None => cast_fail::ItemName::Pending,
    };
    let subclass = |class: u32, mask: u32| {
        sub_classes
            .as_deref()
            .and_then(|s| s.0.requirement_display_name(class, mask))
    };
    let lookups = cast_fail::Lookups {
        get: &get,
        item_name: &item_name,
        subclass: &subclass,
    };
    let texts: Vec<cast_fail::CastFailLine> = cast_errors
        .0
        .drain(..)
        .filter_map(|fail| {
            let d = spells.as_ref().and_then(|s| s.catalog.get(fail.spell_id));
            // With no store yet, the first one the spell names.
            let local_item = || {
                let d = d?;
                if fail.reason == cast_fail::reason::TOTEMS {
                    self_store
                        .and_then(|s| first_missing_totem(d, s, &objects))
                        .or_else(|| d.totems.iter().copied().find(|&t| t != 0))
                } else {
                    self_store
                        .and_then(|s| first_short_reagent(d, s, &objects))
                        .or_else(|| d.reagents.iter().map(|&(id, _)| id).find(|&id| id != 0))
                }
            };
            let arg = match fail.reason {
                cast_fail::reason::TOTEMS | cast_fail::reason::REAGENTS => {
                    fail.arg.or_else(local_item)
                }
                _ => fail.arg,
            };
            let args = cast_fail::FailArgs { arg, ..fail_args };
            let resolved = cast_fail::cast_fail_text(fail.reason, d, args, &lookups);
            debug!(
                "ui_action: cast fail — spell {} reason {:#04x} arg {:?} → {:?}",
                fail.spell_id, fail.reason, fail.arg, resolved
            );
            match resolved {
                cast_fail::Resolved::Line(line) => Some(line),
                cast_fail::Resolved::Silent => None,
                cast_fail::Resolved::Await => {
                    await_template.push(fail.requeued());
                    None
                }
            }
        })
        .collect();
    cast_errors.0.extend(await_template);
    show_messages(
        &mut script,
        &mut sink,
        "ui_action",
        texts.into_iter().map(|l| Shown::keyed(l.key, l.text)),
    );

    // (Dis)mount refusals by key ([`mount_result_key`]); none of these strings takes arguments.
    let mount_texts: Vec<Shown> = mount_errors
        .0
        .drain(..)
        .filter_map(|(mount, code)| {
            let key = mount_result_key(mount, code)?;
            super::keyed_line(&script, key)
        })
        .collect();
    show_messages(&mut script, &mut sink, "ui_action", mount_texts);

    // Taming refusals ([`PetTameFailures`]): the reason's `PETTAME_*` string fills
    // `ERR_TAME_FAILED` ("%s."), two lookups in that order.
    let tame_texts: Vec<Shown> = pet_tame_failures
        .0
        .drain(..)
        .filter_map(|reason| {
            let reason_key = wrathplus_protocol::types::pet_tame_failure_key(reason);
            let reason_text = global_string(&script, reason_key)?;
            super::keyed_line_s(&script, "ERR_TAME_FAILED", &[&reason_text])
        })
        .collect();
    show_messages(&mut script, &mut sink, "ui_action", tame_texts);

    // Client-local refusals by key ([`UiErrorKeys`]); the key also names the catalog record.
    let key_lines: Vec<Shown> = ui_error_keys
        .0
        .drain(..)
        .filter_map(|e| ui_error_text_for_script(&e, &script).map(|t| Shown::keyed(e.key, t)))
        .collect();
    show_messages(&mut script, &mut sink, "ui_action", key_lines);

    // Already-resolved lines ([`UiErrorTexts`]); the queued kind is the `0x4945b0` flag.
    let resolved: Vec<Shown> = ui_error_texts
        .0
        .drain(..)
        .map(|(text, kind)| Shown::unkeyed(kind, text))
        .collect();
    show_messages(&mut script, &mut sink, "ui_action", resolved);

    // The engine's own by-key refusals (`ERR_PASSIVE_ABILITY`): `wrathplus_ui` cannot reach
    // [`UiErrorKeys`], so it queues keys that show here, one frame late.
    let engine_keys = script.take_ui_errors();
    let engine_lines: Vec<Shown> = engine_keys
        .into_iter()
        .filter_map(|key| {
            let e = UiError::key(key);
            ui_error_text_for_script(&e, &script).map(|t| Shown::keyed(e.key, t))
        })
        .collect();
    show_messages(&mut script, &mut sink, "ui_action", engine_lines);

    let store = self_q.iter().next();

    // Stance page: our form's bonus bar offset, with `UPDATE_BONUS_ACTIONBAR` on change.
    let form = store.map(|s| s.0.unit_shapeshift_form()).unwrap_or(0);
    let offset = spells
        .as_ref()
        .and_then(|s| s.forms.get(&u32::from(form)))
        .map(|f| f.bonus_bar)
        .unwrap_or(0) as u8;
    if offset != memory.bonus_offset {
        debug!("ui_action: bonus bar offset {} (form {form})", offset);
        memory.bonus_offset = offset;
        script.set_bonus_bar_offset(offset);
        script.fire_event("UPDATE_BONUS_ACTIONBAR", vec![]);
    }

    // The identity resolve reruns on a new VM, a bar edit, a macro edit or a landed item
    // template: a template is fetched ask-once, so a cold slot's first resolve only issues the
    // query, and the epoch advance redisplays it, like the reference's cache callback.
    let template_epoch = items.template_epoch();
    let macro_generation = script.macros_generation();
    let macros_moved = macro_generation != memory.macro_generation;
    if !memory.resolved || actions.dirty || template_epoch != memory.template_epoch || macros_moved
    {
        actions.dirty = false;
        memory.resolved = true;
        memory.template_epoch = template_epoch;
        memory.macro_generation = macro_generation;
        let macros = script.macros();

        // Each occupied slot's display, diffed against what the VM holds; a changed slot is
        // pushed and fires `ACTIONBAR_SLOT_CHANGED` with its Lua action id.
        let mut fresh: HashMap<u32, ActionSlot> = HashMap::new();
        for (slot, button) in &actions.buttons {
            let (texture, count, consumable) = match button.kind {
                ACTION_KIND_SPELL => {
                    let def = spells.as_ref().and_then(|sp| sp.catalog.get(button.action));
                    let icon = spells.as_ref().and_then(|sp| {
                        let d = def?;
                        spell_action_icon(
                            button.action,
                            d,
                            sp,
                            store,
                            &objects,
                            &items,
                            icons.as_deref(),
                            &commands,
                        )
                    });
                    // A reagent spell counts its casts (`0x5a9360`, `0x7ffa30`).
                    let consumable = def.is_some_and(has_reagents);
                    let casts = def.zip(store).filter(|_| consumable).map_or(0, |(d, s)| {
                        reagent_casts(d, |id| {
                            count_of(&s.0, &objects, id, InventoryScope::CARRIED)
                        })
                    });
                    (icon, casts, consumable)
                }
                ACTION_KIND_ITEM => {
                    // Never nil: the reference's resolver `0x5d88b0` returns the question mark,
                    // and the stock `ActionButton.lua:158` hides the icon on a nil texture.
                    let template = items.template(button.action, 0, &commands).cloned();
                    let texture = template
                        .as_ref()
                        .and_then(|t| icons.as_ref()?.catalog.get(t.display_info_id)?.icon.clone())
                        .unwrap_or_else(|| MISSING_ITEM_ICON.to_string());
                    let count = store
                        .map(|s| count_of(&s.0, &objects, button.action, InventoryScope::CARRIED))
                        .unwrap_or(0);
                    // The Count gate (`0x4e5250`, [`ItemInfo::is_consumable`]) reads the icon's
                    // template, so it rides the same push.
                    let consumable = template.as_ref().is_some_and(|t| t.is_consumable());
                    (Some(texture), count, consumable)
                }
                // A macro slot shows the macro's own icon (`0x4e6bf9` calls `0x4f0fd0`), never its
                // bound spell's, though its state follows the bound spell.
                kind if wrathplus_ui::script::macro_action_id(kind, button.action).is_some() => (
                    wrathplus_ui::script::macro_action_id(kind, button.action)
                        .and_then(|id| macros.by_id(id))
                        .and_then(|m| m.texture.clone()),
                    0,
                    false,
                ),
                _ => (None, 0, false),
            };
            fresh.insert(
                u32::from(*slot) + 1,
                ActionSlot {
                    texture,
                    kind: button.kind,
                    action: button.action,
                    count,
                    consumable,
                },
            );
        }
        // A macro rename changes the name line (`GetActionText`) but not the slot value, so a
        // macro edit re-fires every macro slot.
        let changed: Vec<u32> = fresh
            .keys()
            .chain(memory.pushed.keys())
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|a| {
                fresh.get(a) != memory.pushed.get(a)
                    || (macros_moved
                        && fresh.get(a).is_some_and(|s| {
                            wrathplus_ui::script::macro_action_id(s.kind, s.action).is_some()
                        }))
            })
            .collect();
        for &action in &changed {
            script.set_action(action, fresh.get(&action).cloned());
        }
        memory.pushed = fresh;
        debug!(
            "ui_action: fed {} changed slot(s) ({} occupied)",
            changed.len(),
            memory.pushed.len()
        );
        for action in changed {
            script.fire_event(
                "ACTIONBAR_SLOT_CHANGED",
                vec![ScriptValue::Int(i64::from(action))],
            );
        }
    }

    // An item slot's count and a live icon change without an action-table edit (eating a stack
    // sends no `SMSG_ACTION_BUTTONS`), so the pushed slots refresh every frame.
    if let Some(store) = store {
        for (&action, slot) in memory.pushed.iter_mut() {
            let changed = match slot.kind {
                ACTION_KIND_ITEM => {
                    let fresh = count_of(&store.0, &objects, slot.action, InventoryScope::CARRIED);
                    let changed = fresh != slot.count;
                    if changed {
                        slot.count = fresh;
                    }
                    changed
                }
                // Live icons: a weapon face follows the equipped weapon and form (Attack's
                // `0x4e6870`), and a toggle's `ActiveIconID` follows its own aura (`0x4e6bbd`).
                ACTION_KIND_SPELL => {
                    let def = spells.as_ref().and_then(|s| s.catalog.get(slot.action));
                    // A reagent spell's casts follow its reagents' counts (`0x5aa5c0`).
                    let casts = def.filter(|_| slot.consumable).map_or(0, |d| {
                        reagent_casts(d, |id| {
                            count_of(&store.0, &objects, id, InventoryScope::CARRIED)
                        })
                    });
                    let recounted = std::mem::replace(&mut slot.count, casts) != casts;
                    let d = def.filter(|d| substitutes_weapon_icon(d) || d.active_icon_id != 0);
                    recounted
                        | match (d, spells.as_ref()) {
                            (Some(d), Some(sp)) => {
                                let fresh = spell_action_icon(
                                    slot.action,
                                    d,
                                    sp,
                                    Some(store),
                                    &objects,
                                    &items,
                                    icons.as_deref(),
                                    &commands,
                                );
                                let changed = fresh != slot.texture;
                                if changed {
                                    debug!(
                                        "ui_action: live icon swap slot {action} ({}) -> {fresh:?}",
                                        slot.action
                                    );
                                    slot.texture = fresh;
                                }
                                changed
                            }
                            _ => false,
                        }
                }
                _ => false,
            };
            if changed {
                script.set_action(action, Some(slot.clone()));
                script.fire_event(
                    "ACTIONBAR_SLOT_CHANGED",
                    vec![ScriptValue::Int(i64::from(action))],
                );
            }
        }
    }
}

/// A spell slot's icon, by the reference's `GetActionTexture` resolver `0x4e6a50` in order: the
/// auto-attack faces ([`auto_attack_icon`], `0x4e6870`, `0x4e6990`), then `ActiveIconID` while
/// the spell's own aura is live and cancelable (`0x4e6bbd`, the `0x4e55f0` predicate of
/// [`super::toggle::active_action_toggle`]), then the spell's icon. The spellbook's
/// `GetSpellTexture` (`0x4b3f50`) never serves `ActiveIconID`.
fn spell_action_icon(
    spell_id: u32,
    d: &wrathplus_formats::SpellDisplay,
    spells: &super::Spells,
    store: Option<&crate::app_core::session::ObjectStore>,
    objects: &crate::net::Objects,
    items: &Items,
    icons: Option<&ItemDisplays>,
    commands: &NetCommands,
) -> Option<String> {
    auto_attack_icon(d, store, &spells.forms, objects, items, icons, commands)
        .or_else(|| {
            store
                .filter(|s| {
                    super::toggle::active_action_toggle(
                        spell_id,
                        d,
                        super::toggle::unit_auras(objects, s),
                    )
                })
                .and_then(|_| d.active_icon.clone())
        })
        .or_else(|| d.icon.clone())
}
