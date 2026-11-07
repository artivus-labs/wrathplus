//! The VM's casting flag, pushed each frame from `crate::spell`'s cast lifecycle. The cast bar's
//! `UNIT_SPELLCAST_*` events are `spell::unit_cast`'s.

use std::time::Instant;

use bevy::prelude::*;

use wrathplus_ui::script::UiScript;

use crate::app_core::schedule::UnitFeed;
use crate::app_core::session::SelfGuid;
use crate::creature_anim::Casting;
use crate::net::GuidIndex;
use crate::spell::{inflight, PendingCast, QueuedMeleeSpell};

/// Push the flag `SpellStopCasting` (`0x6e6e80`) answers from: auto-repeat or the in-flight slot,
/// never a channel, taken after the local cancel. A queued strike counts, so the first Esc stops
/// it and `ToggleGameMenu` skips `ClearTarget()` (`UIParent.lua:1489-1492`).
pub(crate) fn push_casting_flag(
    script: Option<NonSendMut<UiScript>>,
    pending: Res<PendingCast>,
    queued_melee: Res<QueuedMeleeSpell>,
    auto_repeat: Res<crate::spell::AutoRepeatActive>,
    self_guid: Res<SelfGuid>,
    index: Res<GuidIndex>,
    casting: Query<&Casting>,
) {
    let Some(mut script) = script else {
        return;
    };
    // The auto-repeat key (`0xceac30`), which the reference reads first, then the in-flight slot
    // the cancel resolves, so the flag and the cancel agree.
    let started = self_guid
        .0
        .as_ref()
        .and_then(|g| index.0.get(g))
        .and_then(|&e| casting.get(e).ok())
        .map(|c| c.spell_id);
    script.set_casting(
        auto_repeat.any() || inflight(&pending, &queued_melee, started, Instant::now()).is_some(),
    );
}

/// The casting flag, pushed before the VM ticks and after the local cancel, so a move or Esc
/// clears it on the next tick.
pub struct UiCastPlugin;

impl Plugin for UiCastPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            push_casting_flag
                .in_set(UnitFeed)
                .after(crate::spell::LocalCancel),
        );
    }
}

#[path = "ui_spell_targeting.rs"]
pub mod spell_targeting;

#[cfg(test)]
#[path = "ui_cast_esc_tests.rs"]
mod esc_tests;
