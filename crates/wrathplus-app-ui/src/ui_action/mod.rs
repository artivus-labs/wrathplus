//! The app side of the action bar: the action table, the spell catalogs and the plugin. [`feed`]
//! pushes each slot's identity into the VM and [`state`] its per-frame cooldown, usability, range
//! and checked state; [`drain`] sends `UseAction` as a cast, swing, item use or macro, and
//! `PickupAction`/`PlaceAction` as `CMSG_SET_ACTION_BUTTON`.

use bevy::prelude::*;

use crate::app_core::schedule::UiInput;
use crate::app_core::schedule::UnitFeed;
use wrathplus_assets::{AssetSet, LockRecover, WorldAssets};

pub(crate) mod cast_fail;
mod drain;
// A `UseAction` press, applied in call order by `crate::script_calls`.
pub(crate) use drain::{attack_nearest_probe, use_action, ActionPress, AttackPress, UseOutcome};
mod cooldown_legs;
#[cfg(test)]
mod drain_tests;
pub(crate) mod drop_item;
mod errors;
mod feed;
#[cfg(test)]
mod feed_tests;
mod net;
mod range;
mod ranks;
pub(crate) mod spell_use;
mod state;
pub(crate) mod toggle;
mod weapon_icon;

/// Every feed that pushes cooldown triples runs `.before` this set: [`state::feed_action_state`]
/// fires the cooldown events synchronously, so a triple pushed later goes unread until the next
/// cooldown change. The pet list's flush, `ui_pet::bar::fire_pet_cooldown_events`, runs `.after`
/// it on the same rule.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CooldownEvents;

/// The rank pass and the identity feed, the writers of [`PlayerActions`]' book state; a reader of
/// the book in the unit feed orders after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ActionBook;

pub(crate) use errors::global_string;
pub(crate) use errors::{
    attack_actor_refusal, keyed_line, keyed_line_s, show_messages, ui_error_text,
    ui_error_text_for_script, CastErrors, MessageSink, MountErrors, PetTameFailures, Shown,
    UiError, UiErrorKeys, UiErrorTexts,
};
pub(crate) use weapon_icon::{melee_auto_attack_icon, ranged_weapon_icon};

pub(crate) use crate::app_core::state::SPELL_ATTACK;

pub(crate) use crate::app_core::state::{GoOpener, GoOpenerCasts, PlayerActions};
pub(crate) use crate::app_core::state::{LearnedAbilities, Spells};

/// The "Remove Insignia" effect, the second one `0x4b25e0` latches.
const SPELL_EFFECT_SKIN_PLAYER_CORPSE: u32 = 0x74;

/// The effect latched into `[0xcecad8]` at learn time (`0x5e9e42`); only Feed Pet 6991 has it.
const SPELL_EFFECT_FEED_PET: u32 = 0x65;

/// Re-derives [`LearnedAbilities`] whenever the book changes, standing in for learn-time writes.
fn track_learned_abilities(
    actions: Res<PlayerActions>,
    spells: Option<Res<Spells>>,
    mut learned: ResMut<LearnedAbilities>,
) {
    let Some(spells) = spells else { return };
    if !actions.is_changed() && !spells.is_changed() {
        return;
    }
    // The last match in id order: each learned rank overwrites the reference's latch, and a rank
    // chain ascends by id (Skinning 8613, 8617, 8618, 10768).
    let last_where = |keep: &dyn Fn(&wrathplus_formats::SpellDisplay) -> bool| {
        actions
            .spells
            .iter()
            .copied()
            .rfind(|&id| spells.catalog.get(id).is_some_and(keep))
    };
    let last_with = |effect: u32| last_where(&|d| d.effects[0] == effect);
    // `0x542030`'s skinning arm writes `[0xbe5d6c + 4×EffectMiscValue[0]]`, so each gather trade
    // keeps its own spell: learning Mining's corpse spell never replaces Skinning.
    let gather: [Option<u32>; 4] = std::array::from_fn(|kind| {
        last_where(&|d| {
            d.effects[0] == wrathplus_formats::SPELL_EFFECT_SKINNING
                && d.effect_misc_value[0] == kind as i32
        })
    });
    let next = LearnedAbilities {
        gather,
        skin_player_corpse: last_with(SPELL_EFFECT_SKIN_PLAYER_CORPSE),
        feed_pet: last_with(SPELL_EFFECT_FEED_PET),
    };
    if (next.gather, next.skin_player_corpse, next.feed_pet)
        != (learned.gather, learned.skin_player_corpse, learned.feed_pet)
    {
        *learned = next;
    }
}

/// A skinnable creature's tooltip line key by its gather trade (12340 `0x620fc3` on `0x715e50`),
/// Skinning while its record is uncached, at difficulty 4 (red) until [`gather_difficulty`] shades
/// it; `None` when `UNIT_FIELD_FLAGS` bit 26 is clear.
pub(crate) fn gather_line_key(
    store: &crate::app_core::session::ObjectStore,
    names: Option<&crate::names::NameCache>,
) -> Option<(&'static str, u8)> {
    (store.0.unit_flags() & 0x0400_0000 != 0).then(|| {
        (
            crate::app_core::gather::LINE_KEYS[corpse_trade(store, names)],
            4,
        )
    })
}

/// The creature's gather trade off its template (`0x715e50`), 0 while uncached.
fn corpse_trade(
    store: &crate::app_core::session::ObjectStore,
    names: Option<&crate::names::NameCache>,
) -> usize {
    let record = (store.0.object_type() == Some(wrathplus_protocol::types::ObjectType::Unit))
        .then(|| store.0.object_entry())
        .flatten()
        .and_then(|entry| names?.creature_record(entry));
    crate::app_core::gather::gather_kind(record.map_or(0, |r| r.type_flags))
}

/// The gather line's difficulty, `0x620ee0`'s colour leg: with the trade's spell learned
/// (`0x53bce0`) and a positive skill in its line (`0x8016c0`), the creature level's required
/// skill (1 to level 10, then `10×level − 100` below 20, else `5×level`) against that skill
/// (`0x61a1f0`): 0 grey at +100, 1 green at +50, 2 yellow at +25, 3 orange at par, else 4 red.
pub(crate) fn gather_difficulty(
    store: &crate::app_core::session::ObjectStore,
    names: Option<&crate::names::NameCache>,
    me: Option<&crate::app_core::session::ObjectStore>,
    learned: &LearnedAbilities,
    skill_lines: Option<&wrathplus_formats::SkillLineCatalog>,
) -> u8 {
    let Some(spell) = learned.gather[corpse_trade(store, names)] else {
        return 4;
    };
    let skill = crate::spell::spell_skill_value(me, skill_lines, spell) as i64;
    if skill <= 0 {
        return 4;
    }
    let level = i64::from(store.0.unit_level().unwrap_or(0));
    let required = match level {
        ..=10 => 1,
        11..=19 => 10 * level - 100,
        _ => 5 * level,
    };
    match skill - required {
        100.. => 0,
        50.. => 1,
        25.. => 2,
        0.. => 3,
        _ => 4,
    }
}

/// `SpellMechanic.dbc`: the name in `SPELL_FAILED_PREVENTED_BY_MECHANIC`'s `%s` (reason `0x8d`).
#[derive(Resource)]
pub(crate) struct SpellMechanics {
    pub(crate) catalog: wrathplus_formats::SpellMechanicCatalog,
}

fn load_spell_mechanics(
    mut commands: Commands,
    assets: Option<Res<wrathplus_assets::WorldAssets>>,
) {
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = wrathplus_assets::LockRecover::lock_recover(&*assets.chain);
        wrathplus_formats::load_spell_mechanic_catalog(&mut chain)
    };
    match loaded {
        Ok(catalog) => {
            debug!("ui_action: {} spell-mechanic name(s)", catalog.len());
            commands.insert_resource(SpellMechanics { catalog });
        }
        Err(e) => warn!(
            "ui_action: SpellMechanic.dbc failed to load — the crowd-control refusal drops the \
             mechanic name: {e:#}"
        ),
    }
}

pub struct UiActionPlugin;

impl Plugin for UiActionPlugin {
    fn build(&self, app: &mut App) {
        net::register(app);
        // The target chain's slots: the script calls in the order the script made them, then
        // `DropItemOnUnit`'s pet leg; and the melee probe's press, the player's Attack (the TAB keys
        // and ATTACKTARGET run their bodies' `TargetNearestEnemy` and `AttackTarget` as script calls).
        app.add_systems(
            Update,
            (
                (
                    crate::script_calls::apply_script_calls,
                    drop_item::drop_item_on_unit,
                )
                    .chain()
                    .in_set(crate::target::TargetSlot::ScriptCalls),
                attack_nearest_probe.in_set(crate::target::TargetSlot::MeleeProbe),
            ),
        );
        app.init_resource::<PlayerActions>()
            .init_resource::<LearnedAbilities>()
            .init_resource::<CastErrors>()
            .init_resource::<MountErrors>()
            .init_resource::<PetTameFailures>()
            .init_resource::<UiErrorKeys>()
            .init_resource::<UiErrorTexts>()
            .init_resource::<GoOpenerCasts>()
            .add_systems(Startup, load_spells.after(AssetSet::Open))
            .add_systems(Startup, load_spell_mechanics.after(AssetSet::Open))
            .add_systems(
                Update,
                (
                    // The feeds ride with the unit feed, before the VM ticks: the rank pass before
                    // the identity feed that consumes its `dirty`, the state feed after it, so a
                    // slot's rank fix and first state land the same frame. The drains follow the
                    // input pass, in any order: a gesture fills only one of their queues.
                    ranks::normalize_action_ranks
                        .in_set(UnitFeed)
                        .in_set(ActionBook)
                        .before(feed::feed_actions),
                    feed::feed_actions.in_set(UnitFeed).in_set(ActionBook),
                    state::feed_action_state
                        .in_set(UnitFeed)
                        .in_set(CooldownEvents)
                        .after(feed::feed_actions),
                    // The use queries' inputs, beside the bar's state from the same walks: after
                    // this frame's local cancel, vehicle state and transport poses it reads.
                    spell_use::feed_spell_use
                        .in_set(UnitFeed)
                        .after(state::feed_action_state)
                        .after(crate::spell::LocalCancel)
                        .after(crate::net::wrath_vehicle::derive_vehicle_state)
                        .after(crate::transport::TransportPose),
                    drain::drain_action_sets.after(UiInput),
                    // The target chain queues the openers earlier in the frame.
                    drain::drain_go_openers.after(UiInput),
                    // The latches must be current before the target chain's cursor reads them.
                    track_learned_abilities
                        .in_set(UnitFeed)
                        .after(feed::feed_actions),
                ),
            );
    }
}

fn load_spells(mut commands: Commands, assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let loaded = {
        let mut chain = assets.chain.lock_recover();
        wrathplus_formats::load_spell_catalog(&mut chain)
    };
    match loaded {
        Ok(catalog) => {
            let forms = {
                let mut chain = assets.chain.lock_recover();
                wrathplus_formats::load_shapeshift_forms(&mut chain).unwrap_or_else(|e| {
                    warn!("ui_action: SpellShapeshiftForm.dbc failed — stance paging off: {e:#}");
                    Default::default()
                })
            };
            let ranges = {
                let mut chain = assets.chain.lock_recover();
                wrathplus_formats::load_spell_ranges(&mut chain).unwrap_or_else(|e| {
                    warn!("ui_action: SpellRange.dbc failed — range indicator off: {e:#}");
                    wrathplus_formats::SpellRangeCatalog::default()
                })
            };
            let cast_times = {
                let mut chain = assets.chain.lock_recover();
                wrathplus_formats::load_spell_cast_times(&mut chain).unwrap_or_else(|e| {
                    warn!("ui_action: SpellCastTimes.dbc failed — cast-time cell off: {e:#}");
                    Default::default()
                })
            };
            let durations = {
                let mut chain = assets.chain.lock_recover();
                wrathplus_formats::load_spell_durations(&mut chain).unwrap_or_else(|e| {
                    warn!("ui_action: SpellDuration.dbc failed — $d/$o tokens off: {e:#}");
                    Default::default()
                })
            };
            let radii = {
                let mut chain = assets.chain.lock_recover();
                wrathplus_formats::load_spell_radii(&mut chain).unwrap_or_else(|e| {
                    warn!("ui_action: SpellRadius.dbc failed — $a token off: {e:#}");
                    Default::default()
                })
            };
            info!(
                "ui_action: {} spells in the display catalog, {} shapeshift forms, {} range rows, \
                 {} cast times, {} durations, {} radii",
                catalog.len(),
                forms.len(),
                ranges.len(),
                cast_times.len(),
                durations.len(),
                radii.len()
            );
            commands.insert_resource(Spells {
                catalog,
                forms,
                ranges,
                cast_times,
                durations,
                radii,
            });
        }
        Err(e) => warn!("ui_action: Spell.dbc failed to load — bar icons disabled: {e:#}"),
    }
}

#[cfg(test)]
mod gather_tests;
