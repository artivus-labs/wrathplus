//! The aura feed: the `UNIT_FIELD_AURA` blocks of the player and of every unit a token can name as
//! the ordered [`AuraState`] lists the aura bindings read, with the player's durations, and the
//! unit-token resolver's inputs the bindings reach them through ([`units`]). The player's list
//! keeps the reference cache's insertion order (`0xbc6040`, repacked by `PlayerAuras_Update
//! 0x4e4170`); any other unit's reads its descriptor by ascending slot, or, for a group member or
//! pet with no live object, its roster record.

mod native;
#[path = "ui_aura_tracking_order.rs"]
pub(crate) mod tracking_order;
mod units;

use bevy::prelude::*;

use wrathplus_formats::SpellCatalog;
use wrathplus_protocol::types::{UnitAuraSlot, AURA_FLAG_CANCELABLE, UNIT_AURA_POSITIVE_SLOTS};
use wrathplus_ui::script::{AuraState, ScriptValue, TrackingState, UiScript};

use crate::app_core::schedule::{UiInput, UnitFeed};
use crate::app_core::session::{ObjectStore, SelfPlayer};
use crate::app_core::state::aura_trace_period as trace_period;
use crate::app_core::state::{ClientState, PlayerActions, Selection, Spells};
use crate::net::{ClientCommand, Guid, GuidIndex, NetCommands};

pub(crate) use crate::app_core::state::{AuraDurations, DurationStamp};

/// A record of the player's cache (`0xbc6040`): the position persists, the fields refresh.
struct CachedAura {
    slot: u8,
    spell_id: u32,
    /// When the aura entered the cache, for the duration freshness gate.
    appeared_at: f64,
    flags: u8,
    level: u8,
    stacks: u8,
}

/// The player's aura cache: buffs and debuffs interleaved, in insertion order.
#[derive(Resource, Default)]
pub(crate) struct PlayerAuraCache {
    auras: Vec<CachedAura>,
}

pub(crate) use crate::app_core::schedule::AuraEvents;

/// Adds the aura feed and the `CancelPlayerBuff`/`CancelTrackingBuff` drain.
pub struct UiAuraPlugin;

impl Plugin for UiAuraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AuraDurations>()
            .init_resource::<PlayerAuraCache>()
            .init_resource::<AuraFeedMemory>()
            // Feed before the VM dispatch, drain after it.
            .add_systems(Update, feed_auras.in_set(UnitFeed).in_set(AuraEvents))
            .init_resource::<tracking_order::TrackingOrder>()
            // A reader of the book, after its writers.
            .add_systems(
                Update,
                tracking_order::feed_tracking_spells
                    .in_set(UnitFeed)
                    .after(crate::ui_action::ActionBook),
            )
            .add_systems(Update, drain_aura_cancels.after(UiInput))
            // Torn down only at the session's end, not with the avatar, which a worldport drops.
            .add_systems(OnExit(ClientState::InWorld), end_session_aura_state);
    }
}

/// `PlayerAuras_Update`'s passes: drop a record whose slot lost its spell, closing the gap
/// (`0x4e421b`), then append new slots in ascending order (`0x4e424e`-`0x4e4374`).
fn reconcile(cache: &mut Vec<CachedAura>, live: &[UnitAuraSlot], now: f64) {
    cache.retain(|c| {
        live.iter()
            .any(|a| a.slot == c.slot && a.spell_id == c.spell_id)
    });
    for a in live {
        if let Some(c) = cache.iter_mut().find(|c| c.slot == a.slot) {
            c.flags = a.flags;
            c.level = a.level;
            c.stacks = a.stacks;
        } else {
            if trace_period().is_some() {
                info!(
                    "aura trace: slot {} took spell {} (descriptor delta) @ {now:.2}",
                    a.slot, a.spell_id
                );
            }
            cache.push(CachedAura {
                slot: a.slot,
                spell_id: a.spell_id,
                appeared_at: now,
                flags: a.flags,
                level: a.level,
                stacks: a.stacks,
            });
        }
    }
}

/// What a frame repaints on; not the countdown, which the button polls.
type AuraProjection = (u32, u8, bool, Option<String>);

/// The feed's edge memory: per token, the projection last pushed. Behind a
/// [`crate::ui_script::VmMemo`], so a `/reload`'s new VM reads it fresh and `UNIT_AURA` re-fires.
#[derive(Resource, Default)]
struct AuraFeedMemory {
    /// What this VM was last told.
    vm: crate::app_core::vm_memo::VmMemo<AuraFeedMemo>,
    /// When [`trace_timers`] last printed: host state, outliving any one VM.
    traced_at: f64,
    /// The player's dispel types (`0xbe8e38`), host state as the book is ([`native::ListCtx`]).
    dispels: Vec<u32>,
}

/// The per-VM `UNIT_AURA` edge keys.
#[derive(Default)]
struct AuraFeedMemo {
    present: bool,
    last: Vec<AuraProjection>,
    /// The tracking aura's spell and receipt stamp: each landing is a setter call (`0x57ea30`).
    tracking_last: Option<(u32, u64)>,
    native_last: Option<Vec<crate::net::wrath_auras::StampedAura>>,
    /// Every other unit's list, by guid, the resolver's inputs and their pushes.
    units: units::UnitAuras,
}

/// One trace tick: each drawn aura's time left beside its button's text, the button polling
/// `GetPlayerBuffTimeLeft` every frame (`BuffFrame.lua:130`). Buffs fill `BuffButton0..15`,
/// debuffs `BuffButton16..23`.
fn trace_timers(script: &UiScript, cache: &[CachedAura], list: &[AuraState], bevy_now: f64) {
    let script_now = script.now();
    info!(
        "aura trace: {} aura(s), GetTime()={script_now:.2} app clock={bevy_now:.2} (skew {:+.2})",
        list.len(),
        script_now - bevy_now
    );
    let (mut helpful_n, mut harmful_n) = (0usize, 0usize);
    for (c, a) in cache.iter().zip(list) {
        let button = if a.helpful {
            helpful_n += 1;
            helpful_n - 1
        } else {
            harmful_n += 1;
            15 + harmful_n
        };
        let app_left = if a.expiration_time > 0.0 {
            format!("{:.1}s", a.expiration_time - script_now)
        } else {
            "permanent".to_string()
        };
        // The rendered duration text: the button keeps no expiry, so its text is the readback.
        let lua = script
            .eval::<String>(&format!(
                r#"local b = getglobal("BuffButton{button}")
                   local d = getglobal("BuffButton{button}Duration")
                   if not b then return "no such button" end
                   return string.format("shown=%s draws %q",
                       tostring(b:IsShown()), d and d:GetText() or "")"#
            ))
            .unwrap_or_else(|e| format!("<lua error: {e}>"));
        info!(
            "  slot {:>2}  spell {:>5}  {:<26}  BuffButton{button:<2}  app left {app_left:<10}  lua {lua}",
            c.slot,
            a.spell_id,
            a.name.as_deref().unwrap_or("<unknown spell>"),
        );
    }
}

fn projection_of(list: &[AuraState]) -> Vec<AuraProjection> {
    list.iter()
        .map(|a| (a.spell_id, a.count, a.cancelable, a.debuff_type.clone()))
        .collect()
}

/// One aura as `UnitBuff 0x519500` and `UnitDebuff 0x5198f0` return it for a unit not the player:
/// no duration, which the 1.12 wire carries for the player alone.
fn unit_aura_state(
    catalog: Option<&SpellCatalog>,
    slot: u8,
    spell_id: u32,
    count: u8,
    flags: u8,
) -> AuraState {
    let display = catalog.and_then(|cat| cat.get(spell_id));
    AuraState {
        spell_id,
        name: display.map(|d| d.name.clone()),
        icon: display.and_then(|d| d.icon.clone()),
        count,
        debuff_type: display
            .zip(catalog)
            .and_then(|(d, cat)| cat.dispel_name(d))
            .map(str::to_string),
        duration: 0.0,
        expiration_time: 0.0,
        helpful: slot < UNIT_AURA_POSITIVE_SLOTS,
        cancelable: flags & AURA_FLAG_CANCELABLE != 0,
        channeled: display.is_some_and(|d| d.attributes_ex & SPELL_ATTR_EX_IS_CHANNELED != 0),
        // `untilCancelled` is the player cache record's alone (`0xbc6040` `+0xc`).
        ..Default::default()
    }
}

/// What one aura of a unit not the player is listed from: its slot, spell, stack count and
/// `AURAFLAGS` nibble. A list is a pure function of these and the spell catalog.
type AuraInput = (u8, u32, u8, u8);

/// The list [`AuraInput`]s make.
fn aura_list(catalog: Option<&SpellCatalog>, inputs: &[AuraInput]) -> Vec<AuraState> {
    inputs
        .iter()
        .map(|&(slot, spell, count, flags)| unit_aura_state(catalog, slot, spell, count, flags))
        .collect()
}

/// Another unit's auras as `UnitBuff 0x519500` and `UnitDebuff 0x5198f0` read them: ascending
/// slot within each half, no durations, through the display filter. `buffs_visible` false drops
/// the helpful half, the unit-level gate's nil at every index ([`buffs_visible_on`]).
fn other_unit_inputs<'a>(
    _store: &'a ObjectStore,
    catalog: Option<&'a SpellCatalog>,
    buffs_visible: bool,
) -> impl Iterator<Item = AuraInput> + 'a {
    // 12340 descriptors carry no aura slots; native auras arrive by packet.
    std::iter::empty::<UnitAuraSlot>()
        .filter(move |a| buffs_visible || a.slot >= UNIT_AURA_POSITIVE_SLOTS)
        .filter(move |a| shown_in_aura_ui(catalog, a.spell_id))
        .map(|a| (a.slot, a.spell_id, a.stacks, a.flags))
}

/// A roster record's aura block as the two bindings' roster walk reads it (`0x519763`-`0x5197d2`,
/// `0x519b53`-`0x519bc5`): ascending slot, the display filter alone (`0x51979f`, `0x519b8f`), no
/// unit gate and no `AURAFLAGS`, which the record does not hold. The stack count is always 1
/// (`0x51980f`, `0x519c2d`), and nothing is cancelable.
fn record_inputs<'a>(
    slots: impl Iterator<Item = (u8, u16)> + 'a,
    catalog: Option<&'a SpellCatalog>,
) -> impl Iterator<Item = AuraInput> + 'a {
    slots
        .map(|(slot, spell)| (slot, u32::from(spell)))
        .filter(move |&(_, spell)| spell != 0 && shown_in_aura_ui(catalog, spell))
        .map(|(slot, spell)| (slot, spell, 1, 0))
}

/// `AttributesEx & 0x4` (vmangos `SpellDefines.h:834`), the cancel gate's DBC arm (`0x4e4a10`).
const SPELL_ATTR_EX_IS_CHANNELED: u32 = 0x4;

/// `GetPlayerBuff`'s second return, cache record `+0xc`, set by `BuildBuffRecord 0x4e44b0` when
/// `DurationIndex` names an infinite `SpellDuration.dbc` row, base < 0 and per level <= 0
/// (`0x4e456e`-`0x4e457a`), or no row (`0x4e4580`); or when the aura is not cancelable and an
/// effect is an area aura, `0x23`, `0x77`, `0x80` or `0x81` (`0x4e4588`-`0x4e45be`).
/// `BuffFrame.lua:124` then shows no countdown. Deviation: on a catalog miss the reference keeps
/// the record's stale value (`0x4e4533`, `0x4e4550` skip the write); this answers from the joined
/// expiry, because a stale value is no mechanism to reproduce.
fn until_cancelled(
    display: Option<&wrathplus_formats::SpellDisplay>,
    duration_row: Option<&wrathplus_formats::SpellDuration>,
    cancelable: bool,
    expiration_time: f64,
) -> bool {
    let Some(d) = display else {
        return expiration_time == 0.0;
    };
    let permanent = match duration_row {
        Some(row) => row.base_ms < 0 && row.per_level_ms <= 0,
        // A negative, out-of-range or empty `DurationIndex`: all three reach `0x4e4580`.
        None => true,
    };
    permanent
        || (!cancelable
            && d.effects
                .iter()
                .any(|e| matches!(e, 0x23 | 0x77 | 0x80 | 0x81)))
}

/// The filter of every aura display, the player's bar and other units' rows alike: never-display
/// and tracking spells are hidden. The player's rebuild tests the attributes
/// (`0x4e42b6`-`0x4e42c8`), then diverts a tracking aura to the tracking global instead of the
/// cache (`0x4e42d6`-`0x4e4308`); `IsAuraDisplayable 0x519860` hides both from other units' rows.
/// Deviation: on other units' rows an id the catalog lacks stays visible, because every wire id is
/// a real spell and failing closed would blank every aura on a catalog failure; the reference
/// skips one there (`[0xc0d78c]`), though its player cache inserts one, as here.
fn shown_in_aura_ui(catalog: Option<&SpellCatalog>, spell_id: u32) -> bool {
    catalog
        .and_then(|c| c.get(spell_id))
        .is_none_or(|d| !d.hidden_from_aura_bar() && !d.tracking_aura())
}

/// The player's tracking aura, 12340's `[0xbeba68]`: an aura of a tracking spell becomes it as it
/// lands (`0x727760` → `0x57ea30`), with no attribute test, which matters because every 3.3.5
/// tracking spell carries the hidden-aura attributes. The last one wins; a catalog miss cannot be
/// told apart. Read by `GetTrackingTexture`, `CancelTrackingBuff` and
/// `GameTooltip:SetTrackingSpell` ([`UiScript::set_tracking`]).
fn tracking_state_of(
    catalog: Option<&SpellCatalog>,
    occupied: &[UnitAuraSlot],
) -> Option<TrackingState> {
    occupied.iter().rev().find_map(|a| {
        let d = catalog.and_then(|c| c.get(a.spell_id))?;
        d.tracking_aura().then(|| TrackingState {
            spell_id: a.spell_id,
            name: Some(d.name.clone()),
            icon: d.icon.clone(),
            cancelable: a.flags & AURA_FLAG_CANCELABLE != 0,
        })
    })
}

/// The units the resolver's guid globals name, from `UnitTokens`' own sources.
#[derive(bevy::ecs::system::SystemParam)]
struct AuraUnits<'w> {
    selection: Res<'w, Selection>,
    pet: Res<'w, crate::app_core::state::PetBar>,
    // `Option`: a UI-only harness lacks their plugins; without the party's no group token names
    // anyone.
    group: Option<Res<'w, crate::app_core::group::GroupState>>,
    hovered: Option<Res<'w, crate::target::Hovered>>,
    hovered_go: Option<Res<'w, crate::target::HoveredObject>>,
    interact: Option<Res<'w, crate::app_core::state::InteractNpc>>,
    focus: Option<Res<'w, crate::ui_unit::FocusUnit>>,
    packets: Option<Res<'w, crate::net::wrath_auras::PacketAuras>>,
    names: Option<Res<'w, crate::names::NameCache>>,
    commands: Res<'w, NetCommands>,
}

/// Resolve template metadata through descriptor entry, including permanent pets whose
/// name resolution otherwise uses a pet cache. Context is reconsidered every feed.
fn native_visible(
    catalog: Option<&SpellCatalog>,
    auras: &[crate::net::wrath_auras::StampedAura],
    local_guid: u64,
    guid: u64,
    target: Option<&ObjectStore>,
    player: &ObjectStore,
    relations: &crate::target::RelationInputs,
    names: Option<&crate::names::NameCache>,
    commands: &NetCommands,
) -> bool {
    let type_flags = target
        .and_then(|s| s.0.object_entry())
        .filter(|&e| e != 0)
        .and_then(|entry| {
            names.map(|names| {
                names.resolve_creature(entry, guid, commands);
                names.creature_record(entry).map_or(0, |r| r.type_flags)
            })
        })
        .unwrap_or(0);
    native::all_visible(
        native::has_active_aura_295(catalog, auras),
        local_guid,
        relations.ring_reaction(target, Some(player)),
        type_flags,
    )
}

fn feed_auras(
    script: Option<NonSendMut<UiScript>>,
    self_q: Query<(&ObjectStore, &Guid), With<SelfPlayer>>,
    units: AuraUnits,
    stores: Query<&ObjectStore>,
    spells: Option<Res<Spells>>,
    actions: Option<Res<PlayerActions>>,
    index: Res<GuidIndex>,
    // The `UnitBuff` gate's inputs, the same reaction the selection ring and nameplates use, with
    // the group roster, forced reactions and owner lookup.
    relations: crate::target::RelationInputs,
    // Read-only: the net path writes the stamps and only the session's end drops them.
    durations: Res<AuraDurations>,
    mut cache: ResMut<PlayerAuraCache>,
    time: Res<Time<Real>>,
    mem: ResMut<AuraFeedMemory>,
) {
    let Some(mut script) = script else {
        return;
    };
    let Ok((store, self_guid)) = self_q.single() else {
        // No avatar does not end the aura state: a worldport re-streams the avatar mid-session, and
        // vmangos re-sends durations only on apply, refresh and `Map::ExistingPlayerLogin`. The
        // reference's `0xbc5f68` is zeroed once, by the startup init (`0x4e40c0` from `0x48f5a9`),
        // and touched otherwise only by its setter `0x4e43a6` and reader `0x4e4467`.
        return;
    };

    let bevy_now = time.elapsed_secs_f64();
    let catalog = spells.as_ref().map(|s| &s.catalog);
    let spell_durations = spells.as_ref().map(|s| &s.durations);
    // 12340 descriptors carry no aura slots; native auras arrive by packet.
    let occupied: Vec<UnitAuraSlot> = Vec::new();
    // Filtered before the cache, so a hidden aura takes no position (`0x4e42b6`-`0x4e4308`).
    let live: Vec<UnitAuraSlot> = occupied
        .iter()
        .filter(|a| shown_in_aura_ui(catalog, a.spell_id))
        .copied()
        .collect();
    reconcile(&mut cache.auras, &live, bevy_now);

    // No stamp is pruned for an empty slot: the packet arrives before its slot fills.

    let catalog_moved = spells.as_ref().is_some_and(|s| s.is_changed());
    let mem = mem.into_inner();
    native::dispel_types(&mut mem.dispels, catalog, actions.as_ref(), catalog_moved);
    let held = |guid: u64| index.0.get(&guid).and_then(|&e| stores.get(e).ok());
    let ctx = native::ListCtx::new(catalog, &held, actions.as_deref(), &mem.dispels);
    let script_now = script.now();

    let mut list: Vec<AuraState> = cache
        .auras
        .iter()
        .map(|c| {
            let display = catalog.and_then(|cat| cat.get(c.spell_id));
            let (duration, expiration_time) = join_duration(
                durations.by_slot.get(&c.slot),
                c.appeared_at,
                bevy_now,
                script_now,
            );
            AuraState {
                spell_id: c.spell_id,
                name: display.map(|d| d.name.clone()),
                icon: display.and_then(|d| d.icon.clone()),
                count: c.stacks,
                debuff_type: display
                    .zip(catalog)
                    .and_then(|(d, cat)| cat.dispel_name(d))
                    .map(str::to_string),
                duration,
                expiration_time,
                helpful: c.slot < UNIT_AURA_POSITIVE_SLOTS,
                cancelable: c.flags & AURA_FLAG_CANCELABLE != 0,
                until_cancelled: until_cancelled(
                    display,
                    display.and_then(|d| spell_durations.and_then(|c| c.get(d.duration_index))),
                    c.flags & AURA_FLAG_CANCELABLE != 0,
                    expiration_time,
                ),
                channeled: display
                    .is_some_and(|d| d.attributes_ex & SPELL_ATTR_EX_IS_CHANNELED != 0),
                ..Default::default()
            }
        })
        .collect();

    let packet_player = units
        .packets
        .as_deref()
        .and_then(|p| p.by_guid.get(&self_guid.0));
    if let Some(auras) = packet_player {
        let visible = native_visible(
            catalog,
            auras,
            self_guid.0,
            self_guid.0,
            Some(store),
            store,
            &relations,
            units.names.as_deref(),
            &units.commands,
        );
        list = native::list(&ctx, self_guid.0, auras, visible, bevy_now, script_now);
    }

    // Before the memory update, so a tracking change joins the edge key.
    let tracking = if let Some(auras) = packet_player {
        auras.iter().rev().find_map(|s| {
            let d = catalog.and_then(|c| c.get(s.aura.spell_id))?;
            d.tracking_aura().then(|| TrackingState {
                spell_id: s.aura.spell_id,
                name: Some(d.name.clone()),
                icon: d.icon.clone(),
                cancelable: s.aura.is_helpful(),
            })
        })
    } else {
        tracking_state_of(catalog, &occupied)
    };
    // A re-landed same aura is restamped (`received_at`), so it moves the key as well.
    let tracking_key = tracking.as_ref().map(|t| {
        let stamp = packet_player
            .and_then(|a| a.iter().rev().find(|s| s.aura.spell_id == t.spell_id))
            .map_or(0, |s| s.received_at.to_bits());
        (t.spell_id, stamp)
    });

    if let Some(period) = trace_period() {
        if bevy_now - mem.traced_at >= period {
            mem.traced_at = bevy_now;
            trace_timers(&script, &cache.auras, &list, bevy_now);
        }
    }

    // This VM's keys: a `/reload`'s new VM starts empty, so every edge re-fires.
    let memo = mem.vm.get(&script);

    // Fire on a discrete change, never on the countdown; a tracking switch counts.
    let projection = projection_of(&list);
    let native_changed = memo.native_last.as_deref() != packet_player.map(Vec::as_slice);
    memo.native_last = packet_player.cloned();
    let tracking_moved = !memo.present || tracking_key != memo.tracking_last;
    let changed = !memo.present || native_changed || projection != memo.last || tracking_moved;
    memo.last = projection;
    memo.tracking_last = tracking_key;
    memo.present = true;

    if changed && std::env::var_os("WRATHPLUS_AURA_DUMP").is_some() {
        info!("aura dump: {} aura(s) on the player bar", cache.auras.len());
        for c in &cache.auras {
            let name = catalog
                .and_then(|cat| cat.get(c.spell_id))
                .map(|d| d.name.as_str())
                .unwrap_or("<unknown spell>");
            info!(
                "  slot {:>2}  spell {:>5}  {:<26}  {:<6}  flags {:#06b}",
                c.slot,
                c.spell_id,
                name,
                if c.slot < UNIT_AURA_POSITIVE_SLOTS {
                    "buff"
                } else {
                    "debuff"
                },
                c.flags,
            );
        }
    }

    let pet_guid = units.pet.guid();
    // `UnitBuff`'s unit gate: the native packet and roster lists apply their own whole-unit gate,
    // and no native snapshot establishes type 295 here, so every live unit's list is shown.
    let buffs_visible = |_guid: u64, _unit: &ObjectStore| true;
    let target_guid = units.selection.guid.unwrap_or(0);
    let unit_changed = memo.units.feed(
        &mut script,
        units::Inputs {
            group: units.group.as_deref(),
            me: (self_guid.0, changed),
            pet: pet_guid,
            target: target_guid,
            // The world pick, as `UnitTokens` resolves `"mouseover"`.
            mouseover: units
                .hovered
                .as_deref()
                .and_then(|h| {
                    h.mouseover(&units.hovered_go.as_deref().copied().unwrap_or_default())
                })
                .map_or(0, |(_, guid)| guid),
            npc: units.interact.as_deref().and_then(|i| i.1).unwrap_or(0),
            focus: units.focus.as_deref().map_or(0, |f| f.guid),
            held: &held,
            buffs_visible,
            native_visible:
                |guid,
                 target: Option<&ObjectStore>,
                 auras: &[crate::net::wrath_auras::StampedAura]| {
                    native_visible(
                        catalog,
                        auras,
                        self_guid.0,
                        guid,
                        target,
                        store,
                        &relations,
                        units.names.as_deref(),
                        &units.commands,
                    )
                },
            ctx: &ctx,
            catalog_moved,
            packets: units.packets.as_deref(),
            clocks: (bevy_now, script_now),
        },
    );

    if unit_changed.contains(&"target") && std::env::var_os("WRATHPLUS_AURA_DUMP").is_some() {
        let l = if target_guid == self_guid.0 {
            Some(list.as_slice())
        } else {
            memo.units.list(target_guid)
        };
        let l = l.unwrap_or_default();
        info!("aura dump: {} aura(s) on the target rows", l.len());
        for a in l {
            info!(
                "  spell {:>5}  {:<26}  {:<6}  {}",
                a.spell_id,
                a.name.as_deref().unwrap_or("<unknown spell>"),
                if a.helpful { "buff" } else { "debuff" },
                a.debuff_type.as_deref().unwrap_or("-"),
            );
        }
    }

    script.set_player_auras(list);
    script.set_tracking(tracking);
    // `MINIMAP_UPDATE_TRACKING` (Minimap.xml:479-482), queued by the setter `0x57ea30` on every
    // call: each landing of the tracking aura, a re-landed same aura included.
    if tracking_moved {
        script.fire_event("MINIMAP_UPDATE_TRACKING", vec![]);
    }
    if changed {
        script.fire_event("UNIT_AURA", vec![ScriptValue::Str("player".into())]);
        // Labelled: 12340's event table keeps `PLAYER_AURAS_CHANGED` (`0x608af5`), but no stock
        // 3.3.5 FrameXML, GlueXML or Blizzard addon registers it (the buff bar reads `UNIT_AURA`);
        // its 12340 fire site is not traced. Kept for addons, with no arguments.
        script.fire_event("PLAYER_AURAS_CHANGED", vec![]);
    }
    // The reference fires these from the live descriptor's aura fields and the record's aura bits
    // (`0x5e57d8`, the pet's `0x5e58f1`), to every token naming the guid (`0x515e50`); a token
    // naming a new unit fires too, and a token clearing fires nothing, since the frames react to
    // their own events (`PLAYER_TARGET_CHANGED`, `UNIT_PET`, `PARTY_MEMBERS_CHANGED`).
    for token in unit_changed {
        script.fire_event("UNIT_AURA", vec![ScriptValue::Str(token.into())]);
    }
}

/// How long before its aura a duration packet may arrive and still join it. The measured lead is
/// one frame, about 50 ms, on a fresh apply; a recycled slot's stale stamp is seconds older.
const DURATION_SLACK: f64 = 1.0;

/// Joins a slot's stamp to the aura in that slot as `(duration, expirationTime)` on the script
/// clock, `(0, 0)` for none; only this rejects a stamp. A stamp older than the aura by more than
/// [`DURATION_SLACK`] is dropped as an earlier occupant's, where the reference's reader
/// (`0x4e4450`) has no freshness test.
fn join_duration(
    stamp: Option<&DurationStamp>,
    appeared_at: f64,
    bevy_now: f64,
    script_now: f64,
) -> (f64, f64) {
    stamp
        .filter(|d| d.received_at >= appeared_at - DURATION_SLACK)
        .map(|d| (d.total, script_now + (d.expires_at - bevy_now)))
        .unwrap_or((0.0, 0.0))
}

/// The session's end: drops the lists and stamps, so nothing reaches the next character's bar.
/// Deviation: the reference never clears its expiry array (`0xbc5f68`); this does, because vmangos
/// sends no durations at login to overwrite a kept stamp (`SMSG_UPDATE_AURA_DURATION` is an empty
/// placeholder in `Player::SendInitialPacketsBeforeAddToMap`, `Player.cpp:19248`).
fn end_session_aura_state(
    script: Option<NonSendMut<UiScript>>,
    mut durations: ResMut<AuraDurations>,
    mut cache: ResMut<PlayerAuraCache>,
) {
    if let Some(mut script) = script {
        script.clear_auras();
        script.set_unit_guids(&Default::default());
        script.set_tracking(None);
    }
    cache.auras.clear();
    durations.by_slot.clear();
    // No `AuraFeedMemory` reset: its edge keys die with the VM.
}

/// A `CMSG_CANCEL_AURA` per queued `CancelPlayerBuff` or `CancelTrackingBuff`, naming the spell,
/// not the slot (`0x6e7040`).
fn drain_aura_cancels(script: Option<NonSendMut<UiScript>>, net: Res<NetCommands>) {
    let Some(mut script) = script else {
        return;
    };
    for spell_id in script.take_cancel_aura_requests() {
        let _ = net.0.send(ClientCommand::CancelAura { spell_id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The plugin on a bare app: without a `UiScript` the systems that need one skip.
    fn aura_app() -> App {
        let (tx, _) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin))
            .insert_state(ClientState::InWorld)
            .init_resource::<Selection>()
            .init_resource::<crate::app_core::state::PetBar>()
            .init_resource::<GuidIndex>()
            .insert_resource(NetCommands(tx))
            .add_plugins(UiAuraPlugin);
        app
    }

    /// One live aura with a running timer: what a teleport must keep and a logout must drop.
    fn seed_one_timed_aura(app: &mut App) {
        app.world_mut()
            .resource_mut::<AuraDurations>()
            .set(32, 300_000, 10.0);
        app.world_mut()
            .resource_mut::<PlayerAuraCache>()
            .auras
            .push(CachedAura {
                slot: 32, // the first harmful slot, a debuff
                spell_id: 11976,
                appeared_at: 10.0,
                flags: 0x8,
                level: 60,
                stacks: 1,
            });
    }

    /// A worldport's avatar-less frames keep every aura live, and no duration is re-sent then.
    #[test]
    fn the_aura_state_survives_an_avatar_less_frame_and_dies_only_with_the_session() {
        let mut app = aura_app();
        seed_one_timed_aura(&mut app);

        // Frames with no avatar: the worldport gap.
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<PlayerAuraCache>().auras.len(),
            1,
            "the display cache survives the gap — its `appeared_at` is the freshness gate's anchor"
        );
        assert!(
            app.world()
                .resource::<AuraDurations>()
                .by_slot
                .contains_key(&32),
            "the duration stamp survives the gap — nothing will re-send it"
        );

        // The session ends: `/logout` to the glue screens.
        app.world_mut()
            .resource_mut::<NextState<ClientState>>()
            .set(ClientState::CharSelect);
        app.update();
        assert!(app.world().resource::<PlayerAuraCache>().auras.is_empty());
        assert!(app.world().resource::<AuraDurations>().by_slot.is_empty());
    }

    fn slot(slot: u8, spell_id: u32) -> UnitAuraSlot {
        UnitAuraSlot {
            slot,
            spell_id,
            flags: if slot < UNIT_AURA_POSITIVE_SLOTS {
                AURA_FLAG_CANCELABLE | 0x8
            } else {
                0x8
            },
            level: 60,
            stacks: 1,
        }
    }

    fn order(cache: &[CachedAura]) -> Vec<(u8, u32)> {
        cache.iter().map(|c| (c.slot, c.spell_id)).collect()
    }

    /// Against the real `Spell.dbc`; skips without client data.
    #[test]
    fn the_aura_display_filter_hides_a_real_battle_stance_but_keeps_battle_shout() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let catalog = wrathplus_formats::load_spell_catalog(&mut chain).expect("Spell.dbc");

        // Battle Stance 2457 and Battle Shout 6673.
        let slots = [slot(0, 2457), slot(1, 6673)];
        let shown: Vec<u32> = slots
            .iter()
            .filter(|a| shown_in_aura_ui(Some(&catalog), a.spell_id))
            .map(|a| a.spell_id)
            .collect();
        assert_eq!(shown, [6673], "the stance is filtered, the shout stays");

        // Fail-open: no catalog at all, or an id the catalog can't resolve, stays visible.
        assert!(shown_in_aura_ui(None, 2457));
        assert!(shown_in_aura_ui(Some(&catalog), 0xffff_fffe));
    }

    /// Against the real `Spell.dbc`; skips without client data. The bar drops both tracking auras,
    /// which carry the hidden-aura attributes, and the last one names the tracking state, as
    /// 12340's aura apply sets it with no attribute test (`0x727760` → `0x57ea30`).
    #[test]
    fn real_tracking_auras_are_hidden_from_the_bar_and_the_last_names_the_tracking_state() {
        let data = wrathplus_formats::wow_data_or_skip!();
        let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
        let catalog = wrathplus_formats::load_spell_catalog(&mut chain).expect("Spell.dbc");

        // Find Herbs 2383 (aura 45), Battle Shout 6673, Track Beasts 1494 (aura 44).
        let slots = [slot(0, 2383), slot(1, 6673), slot(2, 1494)];
        let shown: Vec<u32> = slots
            .iter()
            .filter(|a| shown_in_aura_ui(Some(&catalog), a.spell_id))
            .map(|a| a.spell_id)
            .collect();
        assert_eq!(
            shown,
            [6673],
            "both tracking auras are diverted, the shout stays"
        );

        assert_eq!(
            tracking_state_of(Some(&catalog), &slots).map(|t| t.spell_id),
            Some(1494),
            "Track Beasts, the last tracking aura"
        );
    }

    #[test]
    fn a_new_low_slot_aura_appends_at_the_end_not_sorted_by_slot() {
        let mut cache = Vec::new();
        // X lands in slot 5 first.
        reconcile(&mut cache, &[slot(5, 100)], 1.0);
        assert_eq!(order(&cache), [(5, 100)]);
        // Y lands in slot 2 later: the descriptor reads [2, 5], but Y is newer.
        reconcile(&mut cache, &[slot(2, 200), slot(5, 100)], 2.0);
        assert_eq!(
            order(&cache),
            [(5, 100), (2, 200)],
            "insertion order, not ascending slot"
        );
    }

    /// `PlayerAuras_Update`'s shift-down (`0x4e421b`), then a recycled slot's append.
    #[test]
    fn a_dropped_aura_repacks_and_a_recycled_slot_appends_fresh() {
        let mut cache = Vec::new();
        reconcile(&mut cache, &[slot(0, 10), slot(1, 20), slot(2, 30)], 1.0);
        assert_eq!(order(&cache), [(0, 10), (1, 20), (2, 30)]);

        // The middle aura (slot 1) drops.
        reconcile(&mut cache, &[slot(0, 10), slot(2, 30)], 2.0);
        assert_eq!(order(&cache), [(0, 10), (2, 30)], "the gap closes");

        // A new spell takes slot 1: it appends, not back into the old middle.
        reconcile(&mut cache, &[slot(0, 10), slot(1, 99), slot(2, 30)], 3.0);
        assert_eq!(
            order(&cache),
            [(0, 10), (2, 30), (1, 99)],
            "the recycled slot is the newest, so it is last"
        );
    }

    #[test]
    fn a_surviving_aura_refreshes_in_place() {
        let mut cache = Vec::new();
        reconcile(&mut cache, &[slot(0, 10), slot(1, 20)], 1.0);
        let mut restacked = slot(1, 20);
        restacked.stacks = 5;
        reconcile(&mut cache, &[slot(0, 10), restacked], 2.0);
        assert_eq!(order(&cache), [(0, 10), (1, 20)], "position unchanged");
        assert_eq!(cache[1].stacks, 5, "stack count refreshed");
        assert_eq!(cache[0].appeared_at, 1.0, "appeared_at is not disturbed");
    }

    #[test]
    fn a_duration_is_joined_only_when_it_is_no_older_than_the_aura() {
        // Received at t=100 for an aura that appeared at t=200: a previous occupant's.
        let stale = DurationStamp {
            total: 30.0,
            expires_at: 130.0,
            received_at: 100.0,
        };
        assert_eq!(
            join_duration(Some(&stale), 200.0, 200.0, 5000.0),
            (0.0, 0.0),
            "a stamp seconds older than the aura is rejected — no timer, not a wrong one"
        );
        // Received just before the aura: joined, and rebased onto the script clock.
        let fresh = DurationStamp {
            total: 30.0,
            expires_at: 230.0,
            received_at: 199.9,
        };
        assert_eq!(
            join_duration(Some(&fresh), 200.0, 200.0, 5000.0),
            (30.0, 5030.0)
        );
        // No stamp: a permanent aura, which gets no packet.
        assert_eq!(join_duration(None, 200.0, 200.0, 5000.0), (0.0, 0.0));
    }

    /// The duration packet arrives about a frame before the delta that fills its slot.
    #[test]
    fn a_stamp_that_arrives_one_frame_before_its_aura_still_joins_it() {
        let mut durations = AuraDurations::default();
        // t = 15.10: the packet lands while slot 0 is still empty.
        durations.set(0, 300_000, 15.10);
        let mut cache = Vec::new();
        assert!(cache.is_empty());
        // t = 15.15: the next frame's descriptor delta fills slot 0.
        reconcile(&mut cache, &[slot(0, 1126)], 15.15);

        let (total, expiry) = join_duration(
            durations.by_slot.get(&0),
            cache[0].appeared_at,
            15.15,
            900.0,
        );
        assert_eq!(total, 300.0, "the stamp survived the gap");
        // Script clock 900 plus the 300 s that began 50 ms ago, at the packet, not the delta.
        assert!(
            (expiry - 1199.95).abs() < 1e-9,
            "expiry rebased onto the script clock, got {expiry}"
        );
    }

    /// The two clauses of `0x4e452e`-`0x4e45c5`.
    #[test]
    fn until_cancelled_follows_the_duration_row_and_the_area_aura_override() {
        use wrathplus_formats::{SpellDisplay, SpellDuration};

        let row = |base_ms: i32, per_level_ms: i32| SpellDuration {
            base_ms,
            per_level_ms,
            max_ms: base_ms,
        };
        // Real `SpellDuration.dbc` rows: 21 `{-1, 0}` permanent, 30 `{1_800_000, 0}` 30 minutes,
        // 427 `{-600_000, 60_000}`, whose negative base alone is not permanence.
        let permanent_row = row(-1, 0);
        let timed_row = row(1_800_000, 0);
        let scaling_row = row(-600_000, 60_000);

        let spell = |effects: [u32; 3]| SpellDisplay {
            effects,
            ..SpellDisplay::default()
        };
        let ordinary = spell([6, 0, 0]);
        // 0x23 is `SPELL_EFFECT_APPLY_AREA_AURA_PARTY`, one of the four in the override set.
        let area = spell([0x23, 0, 0]);

        // Clause 1: the duration row decides, cancelable or not.
        assert!(!until_cancelled(
            Some(&ordinary),
            Some(&timed_row),
            true,
            0.0
        ));
        assert!(!until_cancelled(
            Some(&ordinary),
            Some(&timed_row),
            false,
            0.0
        ));
        assert!(until_cancelled(
            Some(&ordinary),
            Some(&permanent_row),
            true,
            0.0
        ));
        assert!(
            !until_cancelled(Some(&ordinary), Some(&scaling_row), true, 0.0),
            "a negative base with a positive per-level term is a SCALING duration, not permanence"
        );
        // No row: permanent (`0x4e4580`).
        assert!(until_cancelled(Some(&ordinary), None, true, 0.0));

        // Clause 2: the area-aura override applies only to a non-cancelable aura (`0x4e4588`).
        assert!(until_cancelled(Some(&area), Some(&timed_row), false, 0.0));
        assert!(
            !until_cancelled(Some(&area), Some(&timed_row), true, 0.0),
            "a CANCELABLE aura skips the effect scan entirely"
        );

        // A catalog miss answers from the joined expiry.
        assert!(until_cancelled(None, None, false, 0.0));
        assert!(!until_cancelled(None, None, false, 1234.0));
    }
}
