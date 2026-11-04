//! Build 12340's combat binding mode: mode 2 of every key is switched on while the player is in
//! combat or auto-attacking ([`wrathplus_ui::script::UiScript::set_combat_binding_mode`]).

use bevy::prelude::*;

use wrathplus_ui::script::UiScript;

use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::creature_anim::Engaged;

/// `UNIT_FLAG_IN_COMBAT`, the bit `0x5206e0` tests (`[unit+0xd4] >> 0x13 & 1`).
const IN_COMBAT: u32 = 0x0008_0000;

/// `0x5206e0`, which the reference calls on every combat and attack edge (`0x524600`'s regen
/// events, the attack start and stop `0x6e4950`/`0x6e1660`, `SMSG_ATTACKSTART`/`STOP`): mode 2 on
/// while the player's unit is in combat or has an attack target (`0x71af90`, [`Engaged`]); the
/// local attack request (`[player+0x1858] & 2`) is read as [`Engaged`] too, as `StartAttack`
/// reads it. Run every frame before dispatch re-derives; the VM call fires `UPDATE_BINDINGS`
/// only on a flip.
pub(super) fn sync_combat_binding_mode(
    script: Option<NonSendMut<UiScript>>,
    me: Query<(&ObjectStore, Has<Engaged>), With<SelfPlayer>>,
) {
    let Some(mut script) = script else { return };
    let Some((store, engaged)) = me.iter().next() else {
        return;
    };
    script.set_combat_binding_mode(engaged || store.0.unit_flags() & IN_COMBAT != 0);
}
