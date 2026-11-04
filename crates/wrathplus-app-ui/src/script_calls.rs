//! The one point in the frame where the script calls that touch the selection, the player's cast
//! or the targeting cursor take effect, in the order the script made them. In the reference each
//! such call is done when it returns (the macro runner `0x4f14e0` fires every line in one pass;
//! `TargetByName` `0x489d60` and `TargetUnit` `0x4899d0` commit through `SetSelection 0x493540`;
//! `CastSpellByName` `0x4b4ab0` casts at the selection as it stands then), so `/target Bob` then
//! `/cast Flash Heal` heals Bob, and `/cast` then `/target` casts at the old target. A fixed order
//! between per-kind drains cannot give both, so the VM keeps one queue ([`ScriptCall`]) and this
//! system applies it front to back, each call through its owner's applier.

use std::collections::VecDeque;

use bevy::prelude::*;

use wrathplus_ui::script::{ScriptCall, UiScript};

/// How deep calls made while applying a call may nest in one frame: a macro whose line presses a
/// macro, and so on. Past it (a macro that presses itself), what is left waits for the next frame,
/// so a frame always ends.
const MAX_NESTING: u8 = 8;

/// Apply this frame's script calls in call order, in the target chain after the world clicks.
#[allow(clippy::type_complexity)] // the appliers, one per owner of the state a call touches
pub fn apply_script_calls(
    script: Option<NonSendMut<UiScript>>,
    binds: Res<crate::app_core::state::BindingsState>,
    mut appliers: ParamSet<(
        crate::target::script::ScriptSelect,
        crate::spell::ScriptCursor,
        crate::spell::SelfCancel,
        crate::spell::ScriptCast,
        crate::ui_action::ActionPress,
        crate::ui_items::ScriptItemUse,
        crate::ui_pet::PetPress,
        crate::ui_action::AttackPress,
    )>,
    // The 12340 cast-order verbs' reads: our packet auras and talent groups, and our guid.
    // `Option` for harnesses without the net resources.
    wrath: (
        Option<Res<crate::net::wrath_auras::PacketAuras>>,
        Option<Res<crate::net::wrath_notices::WrathProgress>>,
        Option<Res<crate::app_core::session::SelfGuid>>,
    ),
    // `InteractUnit`'s object lookup and the world's interact on it.
    mut interact: (
        MessageWriter<crate::target::Interact>,
        Option<Res<crate::net::GuidIndex>>,
    ),
) {
    let (auras, progress, self_guid) = wrath;
    // Native Wrath defaults use the existing ordered application seam, not a second system
    // with duplicate state dependencies or an addon gameplay API.
    for (input, reverse) in [
        (crate::app_core::state::Input::TargetNearestEnemy, false),
        (crate::app_core::state::Input::TargetPreviousEnemy, true),
    ] {
        if binds.fired(input) {
            appliers
                .p0()
                .target_nearest(wrathplus_ui::script::NearestMode::Enemy, reverse);
        }
    }
    if binds.fired(crate::app_core::state::Input::AttackTarget) {
        appliers.p7().attack_target();
    }
    let Some(mut script) = script else {
        return;
    };
    in_call_order(&mut script, |script, call| {
        match call {
            ScriptCall::Select(request) => appliers.p0().select(request),
            ScriptCall::TargetByName { name, exact } => appliers.p0().target_by_name(&name, exact),
            ScriptCall::TargetNearest { mode, reverse } => {
                appliers.p0().target_nearest(mode, reverse);
            }
            ScriptCall::TargetLastTarget => appliers.p0().target_last_target(),
            ScriptCall::ClearTarget => appliers.p0().clear_target(),
            ScriptCall::AttackTarget => appliers.p7().attack_target(),
            ScriptCall::SpellTargetUnit(token) => appliers.p1().spell_target_unit(&token),
            ScriptCall::SpellStopTargeting => appliers.p1().stop_targeting(),
            ScriptCall::SpellStopCasting => appliers.p2().stop_casting(),
            ScriptCall::CastSpell { spell_id, on_self } => {
                crate::ui_spellbook::cast_spell(&mut appliers.p3(), spell_id, on_self);
            }
            ScriptCall::CastShapeshiftForm(spell_id) => {
                crate::ui_shapeshift::cast_form(&mut appliers.p3(), spell_id);
            }
            ScriptCall::UseAction(press) => {
                let used = crate::ui_action::use_action(&mut appliers.p4(), script, press);
                if used == crate::ui_action::UseOutcome::Attack {
                    appliers.p7().attack_target();
                }
            }
            ScriptCall::UseContainerItem { bag, slot } => {
                appliers.p5().use_container_item(script, bag, slot);
            }
            ScriptCall::UseInventoryItem(id) => appliers.p5().use_inventory_item(script, id),
            ScriptCall::CastPetSpell { spell_id, on_self } => {
                crate::ui_pet_book::cast_pet_spell(&mut appliers.p6(), spell_id, on_self);
            }
            ScriptCall::PetAction(slot) => appliers.p6().press_slot(slot),
            ScriptCall::PetOrder(packed) => appliers.p6().order(packed),
            // Build 12340's ChatFrame verbs ([`crate::ui_chat_verbs`]), in call order.
            ScriptCall::StartAttack { unit } => {
                // A token that names nothing streamed starts nothing (labelled: native's
                // by-name fallback `0x520190` is not built).
                let passed = match unit {
                    Some(token) => match appliers.p0().token_guid(&token) {
                        Some(guid) => Some(guid),
                        None => return,
                    },
                    None => None,
                };
                appliers.p7().start_attack(passed);
            }
            ScriptCall::StopAttack => appliers.p7().stop_attack(),
            ScriptCall::TargetLastFriend => appliers.p0().target_last_friend(),
            ScriptCall::CancelShapeshiftForm => {
                if let (Some(auras), Some(me)) =
                    (auras.as_deref(), self_guid.as_ref().and_then(|g| g.0))
                {
                    crate::ui_chat_verbs::cancel_form(&mut appliers.p3(), auras, me);
                }
            }
            ScriptCall::Dismount => crate::ui_chat_verbs::dismount(&mut appliers.p3()),
            ScriptCall::ActivateTalentGroup { group } => {
                if let Some(progress) = progress.as_deref() {
                    crate::ui_chat_verbs::activate_talent_group(
                        &mut appliers.p3(),
                        progress,
                        group,
                    );
                }
            }
            // `DismissCompanion("CRITTER")`: CMSG_DISMISS_CRITTER with the critter's raw guid
            // (AC `HandleDismissCritter`), Wire A's native spell-control writer.
            ScriptCall::DismissCritter { critter } => {
                let _ = appliers.p3().ladder.commands.0.send(
                    crate::net::ClientCommand::NativeSpellControl(
                        wrathplus_protocol::wrath_spell_control::Command::DismissCritter {
                            critter,
                        },
                    ),
                );
            }
            // `DestroyTotem(slot)`: CMSG_TOTEM_DESTROYED with the 0-based slot (AC
            // `HandleTotemDestroyed`).
            ScriptCall::DestroyTotem { slot } => {
                let _ = appliers.p3().ladder.commands.0.send(
                    crate::net::ClientCommand::NativeSpellControl(
                        wrathplus_protocol::wrath_spell_control::Command::TotemDestroyed { slot },
                    ),
                );
            }
            // `RunMacro`/`RunMacroText`: the lines run as a macro slot's do, in call order.
            ScriptCall::RunMacro { index, button } => {
                crate::ui_macro::run_macro(script, index, button);
            }
            ScriptCall::RunMacroText { text, button } => {
                crate::ui_macro::run_macro_text(script, &text, button)
            }
            // `InteractUnit(unit)` (`0x5277b0`): the object's own interact, as a right-click on it
            // runs it, read by the target chain's interact leg next frame. Labelled: the
            // reference interacts inside the call; a guid not streamed does nothing.
            ScriptCall::InteractUnit { guid } => {
                if let Some(&entity) = interact.1.as_ref().and_then(|index| index.0.get(&guid)) {
                    interact.0.write(crate::target::Interact(entity));
                }
            }
            // `CastSpellByID` at a unit token other than the selection or the player needs a
            // cast context bound to that unit, which the cast tail does not build: capture-only.
            other @ ScriptCall::CastSpellAt { .. } => {
                debug!("script call {other:?} not applied yet: capture-only");
            }
        }
        // TryCast's attack pick can move the selection mid-cast (`0x6e4efb`), so a press held
        // there is picked and resumed before the next call reads the selection.
        let Some(held) = appliers.p3().ladder.take_held() else {
            return;
        };
        if appliers.p7().pick_for_cast(held) {
            let mut cast = appliers.p3();
            let ctx = cast.targeting.context();
            cast.ladder.resume_after_pick(held, &ctx);
        }
    });
}

/// Apply every queued call front to back. Applying one may queue more: a macro's lines run
/// inside `UseAction` (`0x4e6098 call 0x4f1460`), so the calls they make go before the next one
/// queued.
pub fn in_call_order(script: &mut UiScript, mut apply: impl FnMut(&mut UiScript, ScriptCall)) {
    let mut queue: VecDeque<(ScriptCall, u8)> = script
        .take_script_calls()
        .into_iter()
        .map(|call| (call, 0))
        .collect();
    let mut next_frame = Vec::new();
    while let Some((call, depth)) = queue.pop_front() {
        apply(script, call);
        let nested = script.take_script_calls();
        if depth >= MAX_NESTING {
            next_frame.extend(nested);
            continue;
        }
        for call in nested.into_iter().rev() {
            queue.push_front((call, depth + 1));
        }
    }
    if !next_frame.is_empty() {
        script.requeue_script_calls(next_frame);
    }
}
