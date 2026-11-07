//! The spell, the reference's `Spell_C`: the cast ladder and commit ([`cast_send`], `TryCast
//! 0x6e4b60` and `SendCast 0x6e54f0`), the target bind ([`cast_target`], `ArmCast 0x6e5250` and
//! `BindTarget 0x6e5b40`), the party and raid relations its arms ask ([`group_relation`],
//! `0x606c20`, `0x606d20`), the requirement validator ([`validator`], `0x6094f0`), the usable walk
//! ([`usable`], `IsSpellUsableNow 0x6e3d60`), the targeting cursor ([`targeting`], `0xcecac0`),
//! the in-flight slot, the cooldowns, the talent modifiers and their packet handlers ([`net`]).

use bevy::prelude::*;

use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::schedule::UnitFeed;
use crate::app_core::state::ClientState;
use wrathplus_world::schedule::WorldStage;

pub mod auto_ranged;
mod bind_gates;
mod cast_commit;
pub mod cast_fail_keys;
mod cast_send;
pub mod cast_target;
pub mod cooldowns;
pub mod form_cancel;
pub mod group_relation;
pub mod inflight;
mod mods;
mod native_control;
pub mod native_failure;
pub(crate) mod net;
pub use net::LastItemCast;
mod range_units;
pub mod reagents;
pub mod secure_toggle;
pub mod targeting;
pub(crate) mod unit_cast;
pub mod usable;
pub(crate) mod validator;

// The one cast path: every caster takes [`CastLadder`] and commits through [`CastCommit`];
// `send_spell_cast` is private to `cast_send`, so no second send path can exist.
pub use cast_send::{CastCommit, CastLadder, HeldCast, HeldForPick, TargetedBind};
pub use cast_target::AutoSelfCast;
pub(crate) use cooldowns::Cooldowns;
#[cfg(any(test, feature = "test-support"))]
pub use inflight::cast_test_support;
pub use inflight::{
    inflight, ActiveChannel, AutoRepeatActive, LocalMoveStart, PendingCast, QueuedMeleeSpell,
    SelfCancel, SPELL_INTERRUPT_MOVEMENT,
};
#[cfg(any(test, feature = "test-support"))]
pub use mods::OP_CAST_TIME;
pub use mods::{ModsDiff, SpellModifiers, SpellsCastTime, OP_COST, OP_GCD, OP_RADIUS};
pub use mods::{SpellMod, OP_COOLDOWN, OP_RANGE};
pub use range_units::{RangeUnits, DEFAULT_REACH};
// `TargetingWants` is exported for the ground reticle, which draws for the location word alone.
pub use targeting::{
    ground_cast_radius, CorpsePick, EnchantResume, PicksSelf, ScriptCursor, SpellTargeting,
    TargetingWants,
};

/// A script cast's inputs, the cast tail the action bar uses, for the spellbook's and the stance
/// bar's calls (the UI's `script_calls` pass).
#[derive(bevy::ecs::system::SystemParam)]
pub struct ScriptCast<'w, 's> {
    pub targeting: cast_target::CastTargeting<'w, 's>,
    pub ladder: CastLadder<'w, 's>,
    /// The `secureAbilityToggle` guard's cast requests ([`secure_toggle`]).
    pub recent: ResMut<'w, secure_toggle::RecentCastRequests>,
    /// The predicted form cancel ([`form_cancel`], `player+0x9f4`).
    pub form_cancel: ResMut<'w, form_cancel::FormCancelPrediction>,
}

/// The local self-cancel's set: a reader of the in-flight state orders `.after(LocalCancel)` so a
/// cast ended by a move or jump drops its cast bar the same frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalCancel;

pub struct SpellPlugin;

impl Plugin for SpellPlugin {
    fn build(&self, app: &mut App) {
        crate::cvars::RegisterCvars::register_cvars(app, secure_toggle::CVARS);
        crate::cvars::RegisterCvars::register_cvars(app, auto_ranged::CVARS);
        app.add_observer(auto_ranged::on_cvar)
            .add_systems(
                Update,
                // One native controller pass against this frame's packet state, modifier
                // family and transported positions, before UI feeds and command producers.
                auto_ranged::tick
                    .after(WorldStage::Net)
                    .after(mods::track_class_family)
                    .after(crate::swing_refusal::apply_swing_refusals)
                    .after(crate::transport::TransportPose)
                    .before(UiFeed)
                    .run_if(in_state(ClientState::InWorld)),
            )
            .add_systems(OnExit(ClientState::InWorld), auto_ranged::reset);
        app.init_resource::<PendingCast>()
            .init_resource::<QueuedMeleeSpell>()
            .init_resource::<ActiveChannel>()
            .init_resource::<LocalMoveStart>()
            .init_resource::<AutoRepeatActive>()
            .init_resource::<Cooldowns>()
            .init_resource::<SpellModifiers>()
            .init_resource::<crate::net::wrath_combo_points::NativeComboPoints>()
            .init_resource::<AutoSelfCast>()
            .init_resource::<secure_toggle::RecentCastRequests>()
            .init_resource::<form_cancel::FormCancelPrediction>()
            .add_message::<crate::net::FieldWritten>()
            .init_resource::<SpellTargeting>()
            .init_resource::<targeting::glyph::GlyphPlacements>()
            .add_systems(
                Startup,
                targeting::glyph::load.after(wrathplus_assets::AssetSet::Open),
            )
            .init_resource::<HeldForPick>()
            .init_resource::<targeting::EnchantConfirmItem>()
            .init_resource::<targeting::EnchantResume>()
            .init_resource::<targeting::PicksSelf>()
            .init_resource::<targeting::CorpsePick>()
            .init_resource::<targeting::UnitPick>()
            .init_resource::<group_relation::GroupRoster>()
            .add_observer(cast_target::on_cvar)
            .add_observer(secure_toggle::on_cvar)
            .add_systems(
                Update,
                (
                    inflight::local_self_cancel
                        .in_set(UnitFeed)
                        .in_set(LocalCancel),
                    mods::track_class_family
                        .after(WorldStage::Net)
                        .before(UnitFeed),
                    // `0x6da130`, before the stance feed reads the form byte through it.
                    form_cancel::observe_form_byte
                        .after(WorldStage::Net)
                        .before(UnitFeed),
                    (
                        targeting::publish_picks_self,
                        targeting::publish_corpse_pick,
                        targeting::publish_unit_pick,
                    )
                        .in_set(UiFeed),
                    // The party slots and raid roster the party and raid words read, before
                    // the feed's targeting push asks them.
                    group_relation::publish_group_roster
                        .in_set(UiFeed)
                        .before(UnitFeed),
                    // The item-target commit (`0x495d60`): after the input pass so a bag click
                    // binds the same frame; outside the target chain, as its clicks never reach
                    // the world.
                    targeting::commit_item_cast_on_pick.after(UiInput),
                ),
            )
            .add_systems(OnEnter(ClientState::InWorld), mods::clear_on_world_enter);
        net::register(app);
        native_failure::register(app);
        native_control::register(app);
        unit_cast::register(app);
    }
}

/// The player's skill in `spell_id`'s own line, the input of every per-level term: `0x5ea690`
/// hops spell to SkillLineAbility line (`0x6de040`), then `0x5ea520` reads that line's
/// `PLAYER_SKILL_INFO` slot. Every missing input reads 0, like the reference's null paths.
pub fn spell_skill_value(
    me: Option<&crate::app_core::session::ObjectStore>,
    skill_lines: Option<&wrathplus_formats::SkillLineCatalog>,
    spell_id: u32,
) -> u32 {
    let Some(line) = skill_lines.and_then(|c| c.spell_to_line(spell_id)) else {
        return 0;
    };
    skill_line_value(me, line)
}

/// The player's skill in `line`, [`spell_skill_value`]'s second half: 0 without a player or a
/// slot for it.
pub fn skill_line_value(me: Option<&crate::app_core::session::ObjectStore>, line: u32) -> u32 {
    let Some(store) = me else { return 0 };
    line_skill_value(
        (0..wrathplus_protocol::types::PLAYER_SKILL_SLOTS)
            .filter_map(|slot| store.0.player_skill(slot)),
        line,
    )
}

/// `0x5ea520`'s sum on the line's first slot (`0x5ea3f0`): the value, plus the bonus word's high
/// half read unsigned when the value is positive (`0x5ea571`-`0x5ea580`), plus its low half read
/// signed (`0x5ea5ac`-`0x5ea5ba`), floored at 0 (`0x5ea5bc`).
fn line_skill_value(
    slots: impl Iterator<Item = wrathplus_protocol::types::PlayerSkillSlot>,
    line: u32,
) -> u32 {
    for s in slots {
        if u32::from(s.skill_id) == line {
            let mut v = i32::from(s.value);
            if v > 0 {
                v += i32::from(s.perm_bonus as u16);
            }
            return (v + i32::from(s.temp_bonus)).max(0) as u32;
        }
    }
    0
}

/// Each skill slot's line and effective value, all [`spell_skill_value`] reads: a text built
/// against one snapshot is rebuilt when the next differs.
pub type SkillSnapshot = [(u16, u32); wrathplus_protocol::types::PLAYER_SKILL_SLOTS as usize];

/// The [`SkillSnapshot`] of the player; all zeros without one.
pub fn skill_snapshot(me: Option<&crate::app_core::session::ObjectStore>) -> SkillSnapshot {
    std::array::from_fn(|slot| {
        me.and_then(|s| s.0.player_skill(slot as u8))
            .map_or((0, 0), |s| {
                (
                    s.skill_id,
                    line_skill_value(std::iter::once(s), s.skill_id.into()),
                )
            })
    })
}

/// The skill lines whose value differs between two snapshots: a line with no slot reads 0
/// ([`skill_line_value`]), and the first slot of a line is the one read.
pub fn changed_skill_lines(prev: &SkillSnapshot, now: &SkillSnapshot) -> Vec<u32> {
    let by_line = |snapshot: &SkillSnapshot| {
        let mut lines = std::collections::HashMap::new();
        for &(line, value) in snapshot.iter().filter(|(line, _)| *line != 0) {
            lines.entry(u32::from(line)).or_insert(value);
        }
        lines
    };
    let (prev, now) = (by_line(prev), by_line(now));
    let value =
        |lines: &std::collections::HashMap<u32, u32>, line| lines.get(&line).copied().unwrap_or(0);
    prev.keys()
        .chain(now.keys())
        .copied()
        .filter(|&line| value(&prev, line) != value(&now, line))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrathplus_protocol::types::PlayerSkillSlot;

    fn slot(value: u16, temp_bonus: i16, perm_bonus: i16) -> PlayerSkillSlot {
        PlayerSkillSlot {
            skill_id: 43,
            step: 0,
            value,
            max: 300,
            temp_bonus,
            perm_bonus,
        }
    }

    /// A line's change is its value's, wherever its slot sits: a moved slot is no change, a line
    /// gained or lost is one, and a slot at 0 is a line at 0.
    #[test]
    fn the_changed_lines_are_those_whose_value_moved() {
        let snapshot = |slots: &[(u16, u32)]| -> SkillSnapshot {
            let mut s = [(0, 0); wrathplus_protocol::types::PLAYER_SKILL_SLOTS as usize];
            s[..slots.len()].copy_from_slice(slots);
            s
        };
        let base = snapshot(&[(43, 100), (44, 50), (54, 0)]);
        assert!(changed_skill_lines(&base, &base).is_empty());
        assert!(
            changed_skill_lines(&base, &snapshot(&[(54, 0), (44, 50), (43, 100)])).is_empty(),
            "a reordered slot table changes no line"
        );
        assert_eq!(
            changed_skill_lines(&base, &snapshot(&[(43, 101), (44, 50)])),
            [43]
        );
        assert_eq!(
            changed_skill_lines(&base, &snapshot(&[(43, 100), (44, 50), (95, 1)])),
            [95],
            "a gained line, and the slot at 0 that went away is no change"
        );
        assert_eq!(
            changed_skill_lines(&base, &snapshot(&[(43, 100)])),
            [44],
            "a lost line reads 0"
        );
    }

    /// The high bonus half counts only over a positive value, and unsigned; the low half is
    /// signed and always counts; the sum floors at 0.
    #[test]
    fn the_skill_value_reads_its_bonus_halves_as_the_reference_does() {
        let value = |s: PlayerSkillSlot| super::line_skill_value(std::iter::once(s), 43);
        assert_eq!(value(slot(100, 0, 5)), 105);
        assert_eq!(value(slot(0, 0, 5)), 0);
        assert_eq!(value(slot(0, 7, 5)), 7);
        assert_eq!(value(slot(100, -20, 0)), 80);
        assert_eq!(value(slot(10, -20, 0)), 0);
        assert_eq!(value(slot(1, 0, -1)), 65536);
        assert_eq!(
            super::line_skill_value(std::iter::once(slot(100, 0, 0)), 44),
            0
        );
    }
}
