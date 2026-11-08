//! Build 12340's attack controller (`0x6e4950`, tick `0x6e2be0`), separate from the running shot.
use bevy::prelude::*;

use super::{cast_target, AutoRepeatActive, CastLadder};

pub const CVARS: &[crate::cvars::Registered] = &[crate::cvars::same("autoRangedCombat", "1")];

/// `player+0x1858`: registration, queue and the close-range-only attack request are distinct.
#[derive(Debug)]
pub struct AutoRanged {
    pub queued: Option<u32>,
    /// A supplied cast candidate, independent of the selection-following attack controller.
    pub(crate) explicit: Option<u64>,
    pub enabled: bool,
    pub attacking: bool,
    pub(crate) close_only: bool,
    pub(crate) attempting: bool,
    pub(crate) last_failure: Option<u8>,
}

impl Default for AutoRanged {
    fn default() -> Self {
        Self {
            queued: None,
            explicit: None,
            enabled: true,
            attacking: false,
            close_only: false,
            attempting: false,
            last_failure: None,
        }
    }
}

impl AutoRepeatActive {
    pub fn any(&self) -> bool {
        self.0.is_some() || self.1.queued.is_some()
    }

    /// `0x6cf2a0`: unregister and clear intention, not the independently running repeat.
    pub fn clear_intent(&mut self) {
        self.1.queued = None;
        self.1.explicit = None;
        self.1.attacking = false;
        self.1.close_only = false;
        self.1.last_failure = None;
    }

    /// `0x542030`: SpellRec+0x20 (AttributesEx4) bit24 latches Auto Shot or Shoot.
    pub(crate) fn learned(
        &self,
        spells: &crate::app_core::state::Spells,
        book: &crate::app_core::state::PlayerActions,
    ) -> Option<u32> {
        self.1
            .enabled
            .then(|| {
                book.spells.iter().rev().copied().find(|&id| {
                    spells
                        .catalog
                        .get(id)
                        .is_some_and(|d| d.attributes_ex4 & 0x0100_0000 != 0)
                })
            })
            .flatten()
    }
}

pub fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut active: ResMut<AutoRepeatActive>) {
    if ev.is("autoRangedCombat") {
        active.1.enabled = ev.flag();
        if !active.1.enabled {
            active.clear_intent();
        }
    }
}

/// Admission to `0x6e4950`: all attack surfaces share this request rather than sending a shot.
/// The tick chooses melee/ranged from the current selected unit, as the native callback does.
pub fn request(active: &mut AutoRepeatActive, known: Option<u32>, close_only: bool) -> bool {
    if !active.1.enabled || known.is_none() {
        return false;
    }
    active.1.explicit = None;
    active.1.attacking = true;
    active.1.close_only = close_only;
    true
}

pub fn tick(
    targeting: cast_target::CastTargeting,
    mut ladder: CastLadder,
    player: Option<Res<crate::app_core::player::Player>>,
    cvars: Option<Res<crate::cvars::Cvars>>,
) {
    if let Some(value) = cvars.as_ref().and_then(|c| c.num("autoRangedCombat")) {
        ladder.auto_repeat.1.enabled = value != 0.0;
        if !ladder.auto_repeat.1.enabled {
            ladder.auto_repeat.clear_intent();
        }
    }
    if !ladder.auto_repeat.1.attacking {
        return;
    }
    let Some(spells) = ladder.spells.as_deref() else {
        return;
    };
    let Some(book) = ladder.book.as_deref() else {
        return;
    };
    let Some(id) = ladder.auto_repeat.learned(spells, book) else {
        ladder.auto_repeat.clear_intent();
        return;
    };
    let ctx = targeting.context_at(ladder.auto_repeat.1.explicit.map(Some));
    let Some(target) = ctx.rel.target_store else {
        if ctx.selection_guid.is_none() {
            stop(&mut ladder);
        }
        return;
    };
    if !ctx.rel.attackable(false) || target.0.is_dead_or_ghost() {
        stop(&mut ladder);
        return;
    }
    let Some((a, b)) = ctx.range.self_pos.zip(ctx.range.target_pos) else {
        return;
    };
    let delta = b - a;
    let distance = delta.length_squared();
    let reach = (ctx.range.caster.reach
        + ctx.range.target.as_unit().map_or(1.5, |u| u.reach)
        + 1.333_333_4)
        .max(5.0);
    let Ok((entity, engaged)) = ladder.self_player.single() else {
        return;
    };
    // `0x71b820` uses a strict melee boundary; close targets select the normal attack leg.
    if distance < reach * reach {
        // `0x6e2610` clears the queue, but leaves controller registration intact.
        ladder.auto_repeat.1.queued = None;
        let close_only = ladder.auto_repeat.1.close_only;
        if !engaged {
            crate::creature_anim::start_attack_local(
                entity,
                ctx.selection_guid.unwrap_or(0),
                engaged,
                false,
                &mut ladder.auto_repeat,
                &mut ladder.sheath,
                &mut ladder.ecs,
                &ladder.commands,
            );
        }
        // The normal melee leg leaves the controller registered (`0x6e2c30`).
        ladder.auto_repeat.1.attacking = true;
        ladder.auto_repeat.1.close_only = close_only;
        return;
    }
    if ladder.auto_repeat.0.is_some() {
        return;
    }
    if ladder.auto_repeat.1.close_only {
        ladder.auto_repeat.clear_intent();
        return;
    }
    let Some(d) = spells.catalog.get(id) else {
        return;
    };
    let (min, max) = limits(d, spells, &ladder, &ctx);
    if engaged && distance <= min * min {
        return;
    }
    if engaged {
        crate::creature_anim::stop_attack_local(
            engaged,
            &mut ladder.queued_melee,
            &ladder.commands,
        );
    }
    ladder.auto_repeat.1.queued = Some(id);
    let facing = player.as_deref().is_some_and(|p| {
        let forward = Quat::from_rotation_y(p.face_yaw) * Vec3::NEG_Z;
        forward.dot(delta) > 0.0
    });
    // `0x6e2dc0`: movement mask 0x33, strict max, facing cone, then the ordinary cast entry.
    if facing && ctx.self_move_flags & 0x33 == 0 && distance < max * max {
        ladder.auto_repeat.1.attempting = true;
        let errors = ladder.cast_errors.0.len();
        ladder.send(id, &ctx, super::CastCommit::Spell);
        ladder.auto_repeat.1.attempting = false;
        // `0x808200`: retrying the queued spell suppresses identical successive failures.
        if let Some(fail) = ladder.cast_errors.0.get(errors).copied() {
            if ladder.auto_repeat.1.last_failure == Some(fail.reason) {
                ladder.cast_errors.0.truncate(errors);
            }
            ladder.auto_repeat.1.last_failure = Some(fail.reason);
        } else if ladder.auto_repeat.0 == Some(id) {
            ladder.auto_repeat.1.last_failure = None;
        }
    }
}

fn limits(
    d: &wrathplus_formats::SpellDisplay,
    spells: &crate::app_core::state::Spells,
    ladder: &CastLadder,
    ctx: &cast_target::CastContext,
) -> (f32, f32) {
    use wrathplus_ui::script::{RangeRow, RangeUnitFacts};
    let row = spells.ranges.get(d.range_index).map(|r| RangeRow {
        flags: r.flags,
        min: [r.min; 2],
        max: [r.max; 2],
    });
    let unit = |u: wrathplus_formats::RangeUnit| RangeUnitFacts {
        reach: u.reach,
        player: u.player,
        running: u.motion.moves_at_run_speed(),
        ..Default::default()
    };
    let ranged_mod = ctx
        .rel
        .self_store
        .and_then(|s| s.0.player_inv_slot(17))
        .and_then(|g| ladder.objects.object(g))
        .and_then(|o| o.object_entry())
        .and_then(|e| ladder.items.template_cached(e))
        .map(|t| t.ranged_mod_range);
    wrathplus_ui::script::range_limits(
        d.attributes,
        row,
        &unit(ctx.range.caster),
        ctx.range.target.as_unit().map(unit).as_ref(),
        false,
        ladder
            .spell_mods
            .modifiers(d, super::OP_RANGE)
            .map(|m| (m.flat, m.pct)),
        ranged_mod,
    )
}

fn stop(ladder: &mut CastLadder) {
    let queued = ladder.auto_repeat.1.queued;
    ladder.auto_repeat.clear_intent();
    // `0x6e1660`: only the controller's own running shot is cancelled on intent teardown.
    if queued.is_some() && queued == ladder.auto_repeat.0 {
        let entity = ladder.self_player.single().ok().map(|(e, _)| e);
        crate::creature_anim::cancel_auto_repeat_local(
            entity,
            &mut ladder.auto_repeat,
            &mut ladder.ecs,
            &ladder.commands,
        );
    }
}

pub(super) fn reset(mut active: ResMut<AutoRepeatActive>) {
    active.0 = None;
    active.clear_intent();
}

#[cfg(test)]
#[path = "auto_ranged_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "auto_ranged_result_tests.rs"]
mod result_tests;
#[cfg(test)]
#[path = "auto_ranged_seam_tests.rs"]
mod seam_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_intent_is_not_a_running_spell_and_clears_independently() {
        let mut state = AutoRepeatActive::default();
        assert!(request(&mut state, Some(75), false));
        state.1.queued = Some(75);
        assert!(state.any());
        assert_eq!(state.0, None);
        state.0 = Some(5019);
        state.clear_intent();
        assert_eq!(state.0, Some(5019));
        assert_eq!(state.1.queued, None);
        assert!(!state.1.attacking);
    }
    #[test]
    fn disabled_or_unlearned_never_registers_the_controller() {
        let mut state = AutoRepeatActive::default();
        assert!(!request(&mut state, None, false));
        state.1.enabled = false;
        assert!(!request(&mut state, Some(75), false));
        assert!(!state.any());
    }
}

#[cfg(test)]
#[path = "cast_range_tests.rs"]
mod cast_range_tests;
