//! Press-again-to-cancel. Build 12340's cast entry (`0x80da80`, which `CastShapeshiftForm`
//! `0x53c0f0` and the cast bindings reach) tests the spell with `isActive` (`0x802cb0`) first: an
//! active spell is cancelled through `0x802f80` ([`send_press_again`]: `CMSG_CANCEL_AURA` `0x136`,
//! or `CMSG_DISMISS_CRITTER` `0x48d` for a companion) instead of being cast. The form-match recast
//! is [`form_recast_disposition`].
//!
//! The `secureAbilityToggle` guard holds a re-press inside 1500 ms of its cast request
//! ([`crate::spell::secure_toggle`], whose list writer is inferred). Not built: `isActive`'s
//! summoned-vehicle arm (`UNIT_FIELD_CREATEDBY`) and `0x802f80`'s mount-twin recursion, which
//! cancels another aura of the same mount creature.

use bevy::log::debug;
use wrathplus_formats::{ShapeshiftForm, SpellDisplay};
use wrathplus_protocol::types::Field;
use wrathplus_protocol::wrath_spell_control::Command;

use crate::app_core::session::ObjectStore;
use crate::net::wrath_auras::StampedAura;
use crate::net::{ClientCommand, Objects};
use crate::spell::form_cancel::{has_mod_shapeshift, FormCancel, FormCancelPrediction};
use crate::spell::secure_toggle::RecentCastRequests;
use crate::spell::{CastLadder, ScriptCast};
use std::time::Instant;

/// The packet aura flag `isActive` tests (`0x802e45`): `AFLAG_POSITIVE`.
pub(crate) const AFLAG_POSITIVE: u8 = 0x10;
/// `SPELL_AURA_MOUNTED`: `isActive`'s gate admits a first effect applying it (`0x8009b0`).
const AURA_MOUNTED: u32 = 78;
/// `SPELL_EFFECT_SUMMON`, the critter arm's effect (`0x802d0b`).
const EFFECT_SUMMON: u32 = 28;

/// `isActive` (`0x802cb0`) for a spell with no form: its gate, a nonzero `ActiveIconID` (record
/// `+0x218`) or a mount (first aura [`AURA_MOUNTED`]), then the spell's own aura on the unit with
/// [`AFLAG_POSITIVE`] (the aura loop, `0x802db0`-`0x802e49`). A lit paladin aura, hunter aspect or
/// the mount we ride passes; a warrior stance has `ActiveIconID` 0.
pub(crate) fn active_action_toggle(spell_id: u32, d: &SpellDisplay, auras: &[StampedAura]) -> bool {
    (d.active_icon_id != 0 || d.effect_apply_aura[0] == AURA_MOUNTED)
        && auras
            .iter()
            .any(|a| a.aura.spell_id == spell_id && a.aura.flags & AFLAG_POSITIVE != 0)
}

/// `isActive`'s critter arm: a SUMMON effect whose creature entry is the one in our
/// `UNIT_FIELD_CRITTER` guid (bits 24-47, `0x802d4e`-`0x802d69`). That field holds only a slot-5
/// summon, the `SummonProperties` test the arm makes (`0x8009b0`). Returns the critter's guid.
pub(crate) fn summoned_critter(d: &SpellDisplay, me: &ObjectStore) -> Option<u64> {
    let critter =
        me.0.get_field_guid(Field::UnitCritter, 0)
            .filter(|&g| g != 0)?;
    let entry = ((critter >> 24) & 0xff_ffff) as i32;
    let summons = (0..3).any(|i| d.effects[i] == EFFECT_SUMMON && d.effect_misc_value[i] == entry);
    summons.then_some(critter)
}

/// The press-again cancel (`0x80da80`): a lit spell is not cast; `0x802f80` dismisses its critter
/// (`CMSG_DISMISS_CRITTER`) or cancels its aura (`CMSG_CANCEL_AURA`), and sends nothing while we
/// are charmed (`UNIT_FIELD_CHARMEDBY`, `0x802fd0`) or the `secureAbilityToggle` guard holds the
/// press ([`RecentCastRequests::blocks`]). A `MOD_SHAPESHIFT` spell is not lit while its form's
/// cancel is predicted (`0x802cb0`), and its cancel runs the prediction ([`predict_form_cancel`]).
/// True when the press was taken; false records it as a cast request, which the caller goes on to
/// send.
pub(crate) fn send_press_again(
    ladder: &CastLadder,
    recent: &mut RecentCastRequests,
    form_cancel: &mut FormCancelPrediction,
    spell_id: u32,
    d: &SpellDisplay,
    me: &ObjectStore,
) -> bool {
    let now = Instant::now();
    let lit = active_action_toggle(spell_id, d, unit_auras(&ladder.objects, me))
        && !(form_cancel.predicted() && has_mod_shapeshift(d));
    let command = if let Some(critter) = summoned_critter(d, me) {
        ClientCommand::NativeSpellControl(Command::DismissCritter { critter })
    } else if lit {
        ClientCommand::CancelAura { spell_id }
    } else {
        recent.record(spell_id, now);
        return false;
    };
    if press_again_held(recent, spell_id, me, now) {
        return true;
    }
    let is_cancel = matches!(command, ClientCommand::CancelAura { .. });
    if is_cancel && predict_form_cancel(ladder, form_cancel, d, me) == FormCancel::Refused {
        return true;
    }
    debug!("press-again: spell {spell_id} re-pressed, {command:?}");
    let _ = ladder.commands.0.send(command);
    true
}

/// `0x802f80`'s form arm ahead of a self-cancel of `d` ([`crate::spell::form_cancel`]).
pub(crate) fn predict_form_cancel(
    ladder: &CastLadder,
    form_cancel: &mut FormCancelPrediction,
    d: &SpellDisplay,
    me: &ObjectStore,
) -> FormCancel {
    ladder.spells.as_deref().map_or(FormCancel::Plain, |s| {
        form_cancel.on_cancel(d, me, &s.forms)
    })
}

/// `CastSpell`'s two cancel forks, in order: the press-again cancel (12340 `isActive`
/// `0x802cb0`), then the form match over the form byte as `0x71af70` reads it, which `UseAction`
/// lacks, with its silent no-op. True when the press was taken.
pub(crate) fn cast_spell_cancels(cast: &mut ScriptCast, spell_id: u32) -> bool {
    let ScriptCast {
        targeting,
        ladder,
        recent,
        form_cancel,
    } = cast;
    let (Some(sp), Some(store)) = (ladder.spells.as_ref(), targeting.self_store.iter().next())
    else {
        return false;
    };
    let Some(d) = sp.catalog.get(spell_id) else {
        return false;
    };
    if send_press_again(ladder, recent, form_cancel, spell_id, d, store) {
        return true;
    }
    let form = form_cancel.form_byte(store.0.unit_shapeshift_form());
    match form_recast_disposition(d, form, sp.forms.get(&u32::from(form))) {
        Some(true) => {
            debug!("ui_spellbook: cast {spell_id} — the active form cancels");
            if predict_form_cancel(ladder, form_cancel, d, store) != FormCancel::Refused {
                let _ = ladder
                    .commands
                    .0
                    .send(ClientCommand::CancelAura { spell_id });
            }
            true
        }
        Some(false) => {
            debug!("ui_spellbook: cast {spell_id} — non-cancelable form, silent no-op");
            true
        }
        None => false,
    }
}

/// The lit press is taken but nothing is sent: the `secureAbilityToggle` window holds it
/// (`0x80daec`), or we are charmed (`UNIT_FIELD_CHARMEDBY`, the `0x802f80` guard).
pub(crate) fn press_again_held(
    recent: &RecentCastRequests,
    spell_id: u32,
    me: &ObjectStore,
    now: Instant,
) -> bool {
    if recent.blocks(spell_id, now) {
        debug!("press-again: spell {spell_id} re-pressed inside secureAbilityToggle's window");
        return true;
    }
    me.0.get_field_guid(Field::UnitCharmedby, 0).unwrap_or(0) != 0
}

/// A unit's packet auras (`SMSG_AURA_UPDATE`/`_ALL`), found by its descriptor guid.
pub(crate) fn unit_auras<'a>(objects: &'a Objects, store: &ObjectStore) -> &'a [StampedAura] {
    let guid = store.0.get_field_guid(Field::ObjectGuid, 0);
    guid.map_or(&[], |g| objects.auras_of(g))
}

/// The form-match recast: `None` casts (not the active form), `Some(true)` cancels the form,
/// `Some(false)` is a silent no-op when `SpellShapeshiftForm.flags1 & 0x2` is set (`0x4b35cf`,
/// twin of the stance bar's `0x4b4963`).
pub(crate) fn form_recast_disposition(
    d: &SpellDisplay,
    form_byte: u8,
    form_row: Option<&ShapeshiftForm>,
) -> Option<bool> {
    if form_byte == 0 || d.shapeshift_form != Some(u32::from(form_byte)) {
        return None;
    }
    Some(form_row.is_some_and(|r| r.cancelable()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_recast_forks_on_the_active_form_and_the_cancelable_guard() {
        let ghost_wolf = SpellDisplay {
            shapeshift_form: Some(16),
            ..Default::default()
        };
        let wolf_row = ShapeshiftForm {
            flags: 0x40,
            ..Default::default()
        };
        let stance_row = ShapeshiftForm {
            flags: 0x7,
            ..Default::default()
        };
        assert_eq!(
            form_recast_disposition(&ghost_wolf, 16, Some(&wolf_row)),
            Some(true)
        );
        assert_eq!(
            form_recast_disposition(&ghost_wolf, 16, Some(&stance_row)),
            Some(false),
            "the non-cancelable guard is a silent no-op, never a cast"
        );
        assert_eq!(
            form_recast_disposition(&ghost_wolf, 0, Some(&wolf_row)),
            None
        );
        assert_eq!(
            form_recast_disposition(&ghost_wolf, 1, Some(&wolf_row)),
            None
        );
        let plain = SpellDisplay::default();
        assert_eq!(form_recast_disposition(&plain, 16, Some(&wolf_row)), None);
    }
}
