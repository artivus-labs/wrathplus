//! The predicted form cancel, build 12340's `player+0x9f4`. The self-cancel `0x802f80`, which the
//! press-again cancel reaches for a lit spell (`CastSpell`, `CastShapeshiftForm`, `UseAction`),
//! scans a `MOD_SHAPESHIFT` (aura `0x24`) spell's effects while the player is in a form: a form
//! row that is missing or non-cancelable (`flags1 & 0x2`) ends the call with nothing sent, and the
//! effect whose form is the current one calls `0x721cf0`, which (for a player) shows the native
//! display if the unit wears one of the form's four `CreatureDisplayID`s, sets `+0x9f4` and fires
//! `UPDATE_SHAPESHIFT_FORM` (event `0x179`), before `CMSG_CANCEL_AURA` goes out.
//!
//! While it is set, the form byte reads as 0 (`0x71af70`) and a `MOD_SHAPESHIFT` spell is not
//! active (`0x802cb0`), so the stance bar, `GetShapeshiftForm` and the next press see no form. The
//! form byte's field callback `0x6da130` clears it when the server's update arrives, restores the
//! form display it swapped and fires `0x179` again. It is registered always-fire (`0x6e5393`,
//! `crate::net::FieldWritten`), so any player update whose mask carries the `UNIT_FIELD_BYTES_2`
//! word, or a re-create of the player, clears it even when the form byte is unchanged.
//!
//! `0x802f80` also returns before the prediction for a spell on the stance bar (`0xbe8e2c`) when
//! the Lua call is tainted (`0xd4139c`); this client has no taint, so that arm never runs.

use std::collections::HashMap;

use bevy::prelude::*;
use wrathplus_formats::{ShapeshiftForm, SpellDisplay};

use crate::app_core::session::{ObjectStore, SelfPlayer};

/// `SPELL_AURA_MOD_SHAPESHIFT` (`0x24`).
const AURA_MOD_SHAPESHIFT: u32 = 0x24;

/// `+0x9f4` and the display `0x721cf0` swapped in.
#[derive(Resource, Default, Debug)]
pub struct FormCancelPrediction {
    /// The raw form byte the prediction was taken over; `Some` is `+0x9f4` set.
    over: Option<u8>,
    /// The native display shown meanwhile, when the unit wore one of the form's displays.
    display: Option<u32>,
    /// `UPDATE_SHAPESHIFT_FORM` owed to the UI, by the start or the clear.
    announce: bool,
}

/// What `0x802f80`'s form arm made of a cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormCancel {
    /// Not a form cancel (no form, no `MOD_SHAPESHIFT`, another form): the cancel goes out.
    Plain,
    /// The current form's cancel, predicted: the cancel goes out.
    Predicted,
    /// A missing or non-cancelable form row: nothing is sent.
    Refused,
}

/// Whether `d` carries a `MOD_SHAPESHIFT` effect (`0x7fde50(spell, 0x24)`).
pub fn has_mod_shapeshift(d: &SpellDisplay) -> bool {
    d.effect_apply_aura.contains(&AURA_MOD_SHAPESHIFT)
}

impl FormCancelPrediction {
    /// The form byte as `0x71af70` reads it: 0 while a cancel is predicted.
    pub fn form_byte(&self, raw: u8) -> u8 {
        if self.over.is_some() {
            0
        } else {
            raw
        }
    }

    /// `+0x9f4`.
    pub fn predicted(&self) -> bool {
        self.over.is_some()
    }

    /// The native display shown over the form's while predicted.
    pub fn display(&self) -> Option<u32> {
        self.display.filter(|_| self.predicted())
    }

    /// Take the owed `UPDATE_SHAPESHIFT_FORM`.
    pub fn take_announce(&mut self) -> bool {
        std::mem::take(&mut self.announce)
    }

    /// `0x802f80`'s form arm, run where the cancel of `d` is about to go out, over the player's raw
    /// form byte, `UNIT_FIELD_DISPLAYID` and `UNIT_FIELD_NATIVEDISPLAYID`.
    pub fn on_cancel(
        &mut self,
        d: &SpellDisplay,
        me: &ObjectStore,
        forms: &HashMap<u32, ShapeshiftForm>,
    ) -> FormCancel {
        let raw = me.0.unit_shapeshift_form();
        let form = self.form_byte(raw);
        if form == 0 || !has_mod_shapeshift(d) {
            return FormCancel::Plain;
        }
        for (aura, misc) in d.effect_apply_aura.iter().zip(d.effect_misc_value) {
            if *aura != AURA_MOD_SHAPESHIFT {
                continue;
            }
            let row = u32::try_from(misc).ok().and_then(|m| forms.get(&m));
            let Some(row) = row.filter(|r| r.cancelable()) else {
                return FormCancel::Refused;
            };
            if misc == i32::from(form) {
                // `0x721cf0`.
                let display = me.0.unit_displayid().filter(|&v| v > 0).map(|v| v as u32);
                let native = me.0.unit_native_displayid().filter(|&v| v > 0);
                self.display = display
                    .filter(|v| row.creature_display.contains(v))
                    .and(native.map(|v| v as u32));
                self.over = Some(raw);
                self.announce = true;
                debug!(
                    "form cancel: predicted over form {raw}, display {:?}",
                    self.display
                );
                return FormCancel::Predicted;
            }
        }
        FormCancel::Plain
    }

    /// `0x6da130`, run by any update that wrote the `UNIT_FIELD_BYTES_2` word (`written`), moved
    /// or not, since it is registered always-fire: the prediction ends, owing the event. A move
    /// of the byte with no such write (a store with no create block) clears it too.
    pub fn observe(&mut self, raw: u8, written: bool) {
        if self.over.is_some_and(|over| written || over != raw) {
            debug!("form cancel: cleared by form byte {raw} (written: {written})");
            *self = Self {
                announce: true,
                ..Self::default()
            };
        }
    }
}

/// Clear the prediction on a write of the player's form word, or with no player to hold it.
pub(super) fn observe_form_byte(
    mut prediction: ResMut<FormCancelPrediction>,
    mut writes: MessageReader<crate::net::FieldWritten>,
    me: Query<(Entity, &ObjectStore), With<SelfPlayer>>,
) {
    let me = me.iter().next();
    let written = writes.read().any(|w| {
        w.field == wrathplus_protocol::types::Field::UnitBytes2
            && me.is_some_and(|(entity, _)| w.entity == entity)
    });
    match me {
        Some((_, store)) => prediction.observe(store.0.unit_shapeshift_form(), written),
        None if prediction.predicted() => *prediction = FormCancelPrediction::default(),
        None => {}
    }
}

#[cfg(test)]
#[path = "form_cancel_tests.rs"]
mod tests;
