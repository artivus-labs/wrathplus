//! The death UI: the feed of death events into the Lua UI and the drain of its intents onto the
//! wire. Dead is health 0, ghost `PLAYER_FLAGS` 0x10, off the self descriptor each frame;
//! [`crate::death`] keeps the wire-fed stores and the ghost look.

use bevy::prelude::*;

use wrathplus_assets::coords::wow_to_bevy;
use wrathplus_ui::script::{DeathAction, DeathUiState, ScriptValue, UiScript};

use crate::app_core::death::DeathNet;
use crate::app_core::death::ResurrectOffer;
use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::app_core::session::ObjectStore;
use crate::app_core::session::SelfGuid;
use crate::app_core::session::SelfPlayer;
use crate::app_core::state::Spells;
use crate::net::{ClientCommand, NetCommands, Objects};

/// The feed's memory in two scopes: the body's state survives a `/reload`, as the reference's
/// engine-side death mirror (`[0xb4e340]`) does, while what the VM was told dies with the VM.
#[derive(Resource, Default)]
struct DeathFeedState {
    /// World-scoped: the last `(guid, dead, ghost)` the self body showed.
    mirror: Option<(u64, bool, bool)>,
    /// World-scoped: the release timer, armed at a death edge and never cleared by a res, as
    /// `0x6dc070` arms it. Labelled: 12340 zeroes it in its UI init (`0x52a9f6`); kept here.
    release: Option<ReleaseTimer>,
    /// VM-scoped: the announce latches.
    vm: crate::app_core::vm_memo::VmMemo<DeathAnnounced>,
}

/// What the live VM has been told; a first snapshot counts as an edge, so logging in or
/// reloading dead brings the popup up.
#[derive(Default)]
struct DeathAnnounced {
    /// The last `(guid, dead, ghost)` this VM was given an event for.
    last: Option<(u64, bool, bool)>,
    /// The last corpse-range verdict announced.
    corpse_range: Option<CorpseRange>,
    /// A fresh reclaim delay re-fires the range events, as the 1.12 client does (`0x4962d0`),
    /// so RECOVER_CORPSE re-shows with the new delay.
    reclaim_generation: u32,
    /// Held back while a player caster's name is still resolving.
    offer_generation: u32,
    /// A fresh ask re-fires `CONFIRM_XP_LOSS`.
    confirm_generation: u32,
}

/// The corpse-range verdicts the reference's events distinguish.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CorpseRange {
    Out,
    In,
    InInstance,
}

/// 12340's release timer (`0xbd0848`, no-timer byte `0xbd084c`) once armed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ReleaseTimer {
    Ends(f64),
    NoTimer,
}

/// `0x6dc070`, read once at the death edge: `NO_RELEASE_WINDOW` (`0x10`) arms 0 ms, else
/// `RELEASE_TIMER` (`0x08`) outside `PLAYER_FLAGS_IS_OUT_OF_BOUNDS` (`0x4000`) arms 360000 ms.
fn arm_release(bits: u8, player_flags: u32, now: f64) -> ReleaseTimer {
    if bits & 0x10 != 0 {
        ReleaseTimer::Ends(now)
    } else if bits & 0x08 != 0 && player_flags & 0x4000 == 0 {
        ReleaseTimer::Ends(now + 360.0)
    } else {
        ReleaseTimer::NoTimer
    }
}

/// `GetCorpseRecoveryDelay` (`0x516280`): whole seconds, rounded down, 0 once past.
fn recovery_seconds(reclaim_at: Option<f64>, now: f64) -> u32 {
    reclaim_at.map_or(0, |at| (at - now).max(0.0) as u32)
}

/// `GetReleaseTimeRemaining` (`0x516210`): whole seconds toward zero, -1 with no timer, 0 before
/// any death.
fn release_seconds(timer: Option<ReleaseTimer>, now: f64) -> i32 {
    match timer {
        None => 0,
        Some(ReleaseTimer::NoTimer) => -1,
        Some(ReleaseTimer::Ends(at)) => (at - now).max(0.0) as i32,
    }
}

/// The corpse-range dialog radius, 40 yd inclusive: the 1.12 client compares d² ≤ `[0xb4e2ac]`
/// (1600.0) against the corpse-query position. The server's reclaim gate,
/// `CORPSE_RECLAIM_RADIUS` 39 (`Corpse.h:40`), sits 1 yd inside it.
const CORPSE_RANGE_SQ: f32 = 40.0 * 40.0;

/// The spirit-healer dialog range: `CheckSpiritHealerDist` compares d² ≤ `[0xc4c28c]` (30.864).
const SPIRIT_HEALER_RANGE_SQ: f32 = 5.5556 * 5.5556;

/// Derive the death state from the self descriptor, push the snapshot, and fire the death events
/// on its edges.
fn feed_death(
    script: Option<NonSendMut<UiScript>>,
    self_q: Query<(&ObjectStore, &Transform), With<SelfPlayer>>,
    self_guid: Res<SelfGuid>,
    death_net: Res<DeathNet>,
    // Real time: `reclaim_at` is stamped on `Time<Real>` by the net path.
    time: Res<Time<Real>>,
    mut feed: ResMut<DeathFeedState>,
    names: Res<crate::names::NameCache>,
    net: Res<NetCommands>,
    objects: Objects,
    transforms: Query<&Transform>,
    map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
    status: Res<crate::net::NetStatus>,
    spells: Option<Res<Spells>>,
    items: Res<crate::items::Items>,
) {
    // A disconnect keeps the self avatar with its frozen descriptor; only a live session's
    // descriptor may drive the state machine.
    if !status.connected {
        return;
    }
    let Some(mut script) = script else {
        return;
    };
    let Some(guid) = self_guid.0 else {
        return;
    };
    let Ok((store, self_t)) = self_q.single() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let dead = store.0.unit_is_dead();
    let ghost = store.0.player_is_ghost();

    // ── The snapshot, set before the events fire so their handlers read current values ──
    let prev = feed.mirror.filter(|&(g, ..)| g == guid);
    // Alive to dead arms it (`0x6dc165`); logging in dead too, labelled: that edge is not read.
    if matches!(prev, Some((_, false, false)) | None) && dead && !ghost {
        let bits = store.0.player_release_bits();
        feed.release = Some(arm_release(bits, store.0.player_flags(), now));
    }
    let spirit_healer_in_range = death_net.spirit_healer.is_some_and(|npc| {
        objects
            .entity(npc)
            .and_then(|e| transforms.get(e).ok())
            .is_some_and(|t| {
                t.translation.distance_squared(self_t.translation) <= SPIRIT_HEALER_RANGE_SQ
            })
    });
    // Resolved first: the lookup borrows the VM that `set_death` needs mutably.
    let sickness = sickness_duration(store.0.unit_level().unwrap_or(0), &|key: &str| {
        script
            .lua()
            .globals()
            .get::<String>(key)
            .ok()
            .filter(|t| !t.is_empty())
    });
    script.set_death(DeathUiState {
        release_remaining: release_seconds(feed.release, now),
        recovery_delay: recovery_seconds(death_net.reclaim_at, now),
        resurrect_sickness: death_net.resurrect.as_ref().is_some_and(|o| o.sickness),
        resurrect_has_timer: death_net.resurrect.as_ref().is_some_and(|o| o.has_timer),
        spirit_healer_in_range,
        sickness_duration: sickness,
        // `HasSoulstone()`; the dead gate is first, so the inventory walk runs only while dead.
        self_res_label: resolve_self_res(&store.0, &objects, &items, spells.as_deref(), &net)
            .map(|r| r.label().to_owned()),
    });

    // ── The world edges: the release window and the corpse asks, which a `/reload` must not
    // repeat.
    feed.mirror = Some((guid, dead, ghost));
    match (prev, dead, ghost) {
        // Dead to ghost (released): ask where the corpse is.
        (Some((_, true, false)), _, true) => {
            let _ = net.0.send(ClientCommand::CorpseQuery);
        }
        // Ghost to alive: reclaim, spirit healer or an accepted res.
        (Some((_, _, true)), false, false) => {
            // The not-found answer drops the map markers. The reference sends nothing on this
            // edge: its ghost-bit watcher (`0x5ee990`) clears the corpse cache, and `0x491f50`
            // queries only while a ghost. The server's own push needs a looter
            // (`Map.cpp:3617-3629`).
            let _ = net.0.send(ClientCommand::CorpseQuery);
        }
        // Logging in a ghost: no edge, just the corpse ask.
        (None, _, true) => {
            let _ = net.0.send(ClientCommand::CorpseQuery);
        }
        _ => {}
    }

    // ── The VM edges: a fresh VM's memo is empty, so reloading dead re-fires PLAYER_DEAD.
    let memo = feed.vm.get(&script);
    let prev = memo.last.filter(|&(g, ..)| g == guid);
    memo.last = Some((guid, dead, ghost));
    match (prev, dead, ghost) {
        (Some((_, false, false)) | None, true, false) => {
            script.fire_event("PLAYER_DEAD", vec![]);
        }
        (Some((_, true, false)), d, g) if g || !d => {
            script.fire_event("PLAYER_ALIVE", vec![]);
        }
        (Some((_, _, true)), false, false) => {
            script.fire_event("PLAYER_UNGHOST", vec![]);
        }
        _ => {}
    }

    // ── The offer and confirm announcements ──
    if let Some(name) = offer_announcement(memo, &death_net, store.0.is_dead_or_ghost(), |offer| {
        // A player caster's name is empty on the wire; hold the popup until the cache resolves it.
        if offer.name.is_empty() {
            names.resolve(offer.caster, &net).map(str::to_owned)
        } else {
            Some(offer.name.clone())
        }
    }) {
        script.fire_event("RESURRECT_REQUEST", vec![ScriptValue::Str(name)]);
    }
    if death_net.spirit_healer.is_some() && memo.confirm_generation != death_net.confirm_generation
    {
        memo.confirm_generation = death_net.confirm_generation;
        // `arg1` is the XP the res costs: `PLAYER_NEXT_LEVEL_XP` × 0.05, truncated (`0x5df837`),
        // and 0 without the field, as `0x5df806` pushes 0 for a non-local object. No stock
        // FrameXML reads it; `UIParent.lua:399` asks `GetResSicknessDuration()`.
        let xp_cost = store
            .0
            .player_next_level_xp()
            .map_or(0, |next| (f64::from(next) * 0.05) as i64);
        script.fire_event("CONFIRM_XP_LOSS", vec![ScriptValue::Int(xp_cost)]);
    }

    // ── The corpse-run range gate: only a ghost, against the query's display position; the
    // coordinate transform is isometric, so the Bevy-space compare is in yards.
    let range = if ghost {
        death_net.corpse.map(|cp| {
            // Within range on the display map: IN_RANGE when it is the corpse's map, else
            // IN_INSTANCE, at a dungeon's entrance (`0x492130`, `0x4920f0`).
            let on_display_map = map
                .as_ref()
                .is_some_and(|m| m.0 == u32::try_from(cp.display_map).unwrap_or(u32::MAX));
            let near = on_display_map
                && wow_to_bevy(cp.position).distance_squared(self_t.translation) <= CORPSE_RANGE_SQ;
            let same_map = u32::try_from(cp.display_map) == Ok(cp.corpse_map);
            match (near, same_map) {
                (false, _) => CorpseRange::Out,
                (true, true) => CorpseRange::In,
                (true, false) => CorpseRange::InInstance,
            }
        })
    } else {
        None
    };
    if memo.reclaim_generation != death_net.reclaim_generation {
        memo.reclaim_generation = death_net.reclaim_generation;
        memo.corpse_range = None; // re-announce whatever holds now
    }
    if range != memo.corpse_range {
        match range {
            Some(CorpseRange::In) => script.fire_event("CORPSE_IN_RANGE", vec![]),
            Some(CorpseRange::InInstance) => script.fire_event("CORPSE_IN_INSTANCE", vec![]),
            // Leaving range and the ghost state ending both fire the one hide event.
            Some(CorpseRange::Out) | None if memo.corpse_range.is_some() => {
                script.fire_event("CORPSE_OUT_OF_RANGE", vec![]);
            }
            _ => {}
        }
        memo.corpse_range = range;
    }
}

/// `SPELL_EFFECT_SELF_RESURRECT`, the `Spell.dbc` effect the item leg scans for (`0x5ed650`).
const SPELL_EFFECT_SELF_RESURRECT: u32 = 94;

/// What `UseSoulstone()` (`0x48ad70`) would spend now; resolved app-side, where the spell catalog
/// and item cache live.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SelfRes {
    /// `PLAYER_SELF_RES_SPELL` is non-zero: send `CMSG_SELF_RES` and the server casts it.
    Spell {
        #[allow(dead_code)] // the server reads its own field
        spell: u32,
        label: String,
    },
    /// The field is zero but a carried item's on-use spell self-resurrects: use the item, named
    /// for the item.
    Item {
        bag_index: u8,
        slot: u8,
        guid: u64,
        entry: u32,
        label: String,
    },
}

impl SelfRes {
    /// What `HasSoulstone()` returns.
    fn label(&self) -> &str {
        match self {
            Self::Spell { label, .. } | Self::Item { label, .. } => label,
        }
    }
}

/// `HasSoulstone()`'s answer and `UseSoulstone()`'s routing, one predicate in the 1.12 client
/// (`0x48ac80`, `0x48ad70`), with its gates in its order:
///
/// 1. Dead: nil while `UNIT_FIELD_HEALTH` is positive; a released ghost has health 1.
/// 2. The field: non-zero is the spell leg, named from `Spell.dbc`, `"UNKNOWN"` when unresolved.
/// 3. On a zero field, the carried inventory at the walker's default mask (`0x47`) for an item
///    whose on-use spell carries [`SPELL_EFFECT_SELF_RESURRECT`].
///
/// An item template still in flight reads as no self-res until it lands.
fn resolve_self_res(
    store: &wrathplus_protocol::ObjectFields,
    objects: &Objects,
    items: &crate::items::Items,
    spells: Option<&Spells>,
    commands: &NetCommands,
) -> Option<SelfRes> {
    if !store.unit_is_dead() {
        return None;
    }
    if let Some(spell) = store.player_self_res_spell() {
        let label = spells
            .and_then(|s| s.catalog.get(spell))
            .map_or_else(|| "UNKNOWN".to_owned(), |d| d.name.clone());
        return Some(SelfRes::Spell { spell, label });
    }
    let slots = crate::inventory::collect_inventory(
        store,
        objects,
        crate::inventory::InventoryScope::DEFAULT,
    );
    slots.into_iter().find_map(|(bag_index, slot, guid)| {
        let entry = objects.object(guid)?.object_entry()?;
        let t = items.template(entry, guid, commands)?;
        let self_res = t.spells.iter().any(|sp| {
            sp.trigger == 0
                && spells.is_some_and(|c| {
                    c.catalog
                        .get(sp.spell_id)
                        .is_some_and(|d| d.effects.contains(&SPELL_EFFECT_SELF_RESURRECT))
                })
        });
        self_res.then(|| SelfRes::Item {
            bag_index,
            slot,
            guid,
            entry,
            label: t.name.clone(),
        })
    })
}

/// The sickness a spirit-healer res applies at `level`, per vmangos `Player::ResurrectPlayer`:
/// none below 11, `level - 10` minutes through 19, 10 minutes from 20. Worded with `GENERIC_MIN`,
/// the minute arm of the reference's formatter (`0x51a3a0` into `0x52fa50`).
///
/// Deviation: the minutes come from the server's table, not from spell 15007's `Spell.dbc`
/// duration, because the aura the server applies is what the dialog must warn about.
fn sickness_duration(level: u32, get: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let minutes = match level {
        0..=10 => return None,
        11..=19 => level - 10,
        _ => 10,
    };
    // `GetText(token, nil, ordinal)`'s plural pick, through the shared primitive.
    let template = wrathplus_ui::strings::plural("GENERIC_MIN", Some(minutes), get)?;
    Some(wrathplus_ui::strings::fill(
        &template,
        &[wrathplus_ui::strings::Arg::D(i64::from(minutes))],
    ))
}

/// Map the queued Lua death intents onto the wire, after `UiInput` so a click drains that frame.
fn drain_death(
    script: Option<NonSendMut<UiScript>>,
    mut death_net: ResMut<DeathNet>,
    net: Res<NetCommands>,
    targeting: crate::spell::cast_target::CastTargeting,
    mut ladder: crate::spell::CastLadder,
    mut ui_errors: ResMut<crate::app_core::errors::UiErrorKeys>,
    // The soulstone's item leg is an item use (`0x5d8d00`), so it carries the bind gate.
    mut gate: crate::ui_bind_confirm::BindGate,
) {
    let Some(mut script) = script else {
        return;
    };
    for action in script.take_death_actions() {
        let _ = match action {
            DeathAction::Repop => net.0.send(ClientCommand::RepopRequest),
            // Else 0: vmangos resolves the corpse through the player and never reads the guid
            // (`MiscHandler.cpp:573-603`).
            DeathAction::RetrieveCorpse => net.0.send(ClientCommand::ReclaimCorpse {
                corpse: death_net.corpse_guid.unwrap_or(0),
            }),
            DeathAction::AcceptResurrect | DeathAction::DeclineResurrect => {
                let accept = action == DeathAction::AcceptResurrect;
                match death_net.resurrect.take() {
                    Some(offer) => net.0.send(ClientCommand::ResurrectResponse {
                        caster: offer.caster,
                        accept,
                    }),
                    None => Ok(()), // the offer timed out; nothing to answer
                }
            }
            DeathAction::AcceptXpLoss => match death_net.spirit_healer.take() {
                Some(npc) => net.0.send(ClientCommand::SpiritHealerActivate { npc }),
                None => Ok(()),
            },
            // Re-resolved at click time, as the reference's `OnCancel` calls `HasSoulstone()`
            // again; nothing is cleared here, the server zeroes `PLAYER_SELF_RES_SPELL`.
            DeathAction::UseSoulstone => {
                let store = targeting.self_store.single().ok().map(|s| s.0.clone());
                match store.as_ref().and_then(|store| {
                    resolve_self_res(
                        store,
                        &ladder.objects,
                        &ladder.items,
                        ladder.spells.as_deref(),
                        &ladder.commands,
                    )
                }) {
                    Some(SelfRes::Spell { .. }) => net.0.send(ClientCommand::SelfRes),
                    // An ordinary item use, `0x5d8d00`, like every bag click.
                    Some(SelfRes::Item {
                        bag_index,
                        slot,
                        guid,
                        entry,
                        ..
                    }) => {
                        let t = ladder
                            .items
                            .template(entry, guid, &ladder.commands)
                            .cloned();
                        let it = crate::app_core::state::ItemUse {
                            guid: Some(guid),
                            start_quest: t.as_ref().map_or(0, |t| t.start_quest),
                            bag_index,
                            slot,
                            entry,
                            spell_index: t.as_ref().and_then(|t| t.use_spell_index()).unwrap_or(0),
                            use_spell: t.as_ref().and_then(|t| t.use_spell).map(|u| u.spell_id),
                            on_object: None,
                            is_charter: false,
                        };
                        crate::ui_items::send_item_use(
                            it,
                            &targeting.context(),
                            &mut ladder,
                            &mut script,
                            &mut gate,
                            false,
                            &mut ui_errors,
                        );
                        Ok(())
                    }
                    // Nothing to spend: the reference returns silently.
                    None => Ok(()),
                }
            }
        };
    }
}

/// Clear [`DeathFeedState`]'s world scope on `DisconnectedMessage`, the edge [`DeathNet`] resets
/// on: a relog as a ghost must find no mirror, or the corpse query is never re-sent.
fn end_session_death_feed(
    mut msgs: MessageReader<crate::app_core::session::DisconnectedMessage>,
    mut feed: ResMut<DeathFeedState>,
) {
    if msgs.read().next().is_some() {
        *feed = DeathFeedState::default();
    }
}

/// The death feed into the UI and the intent drain.
pub struct UiDeathPlugin;

impl Plugin for UiDeathPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DeathFeedState>().add_systems(
            Update,
            (
                feed_death.in_set(UiFeed),
                drain_death.after(UiInput),
                // Before the feed, so the frame a session ends carries no memory of it.
                end_session_death_feed.before(feed_death),
            ),
        );
    }
}

/// The name `RESURRECT_REQUEST` fires with this frame, if any. Per message, and only while dead
/// or a ghost (`0x5ded50` gated on `0x605f30`); an unresolved name holds without consuming it.
fn offer_announcement(
    memo: &mut DeathAnnounced,
    death_net: &DeathNet,
    dead_or_ghost: bool,
    name_of: impl FnOnce(&ResurrectOffer) -> Option<String>,
) -> Option<String> {
    if !dead_or_ghost || memo.offer_generation == death_net.resurrect_generation {
        return None;
    }
    let name = name_of(death_net.resurrect.as_ref()?)?;
    memo.offer_generation = death_net.resurrect_generation;
    Some(name)
}

#[cfg(test)]
#[path = "ui_death/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ui_death/self_res_tests.rs"]
mod self_res_tests;
