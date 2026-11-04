//! Build 12340's instance lockout messages, the bookkeeping behind `CanShowResetInstances()`, and
//! `CMSG_RESET_INSTANCES`. The client composes most lines itself and shows them as
//! `CHAT_MSG_SYSTEM` (`00509dd0`); here the [`net`] handler queues them off
//! `SessionEvent::WrathInstanceControl` and [`feed_instance`] resolves them against the VM:
//!
//! - `SMSG_RAID_INSTANCE_MESSAGE` (`0x2fa`, `0050ca80`): kinds 1-3 the `RAID_INSTANCE_WARNING_*`
//!   lines (hours or minutes, truncated), 5 `RAID_INSTANCE_EXPIRED`, and 4 the Lua event
//!   `RAID_INSTANCE_WELCOME` (`0x212`, `"%s%d%d%d"`: name, seconds, locked, extended), which
//!   `UIParent.lua` formats. The name is `004fe070`'s, with the difficulty.
//! - `SMSG_INSTANCE_RESET` (`0x31e`, `0050ccd0`) and `_FAILED` (`0x31f`, `0050cda0`): the
//!   `Map.dbc` name, or the id with no row.
//! - `SMSG_INSTANCE_SAVE_CREATED` (`0052bb10`): `INSTANCE_SAVED`; flag 0 also ends the instance
//!   lock (`00513b20(0, 0, 0)`, [`crate::ui_static_popup`]).
//!
//! Labelled: the `CanShowResetInstances` bookkeeping (latch, window, ownership) is traced on 1.12
//! (`0x495c90`); 12340's is not. Deviation: walking out of a party dungeon ourselves also counts as
//! holding a bind ([`InstanceState::saw_own_dungeon`]), because the server reports only permanent
//! binds, so it alone would never offer the reset after a 5-man.

use bevy::prelude::*;
use wrathplus_protocol::wrath_instance_control::Packet;
use wrathplus_ui::script::{ScriptValue as V, UiScript};

use crate::app_core::chat::ChatEvent;
use crate::app_core::chat::ChatEventKind;
use crate::app_core::chat::ChatLog;
use crate::app_core::schedule::UiFeed;
use crate::app_core::schedule::UiInput;
use crate::net::{ClientCommand, NetCommands};

/// `CanShowResetInstances()`'s window, 25 hours: `cmp eax, 0x15f90` at `0x495ce6` against
/// `time(0)` minus when we left, unsigned, so the boundary is inclusive.
const RESET_OFFER_WINDOW_SECS: u64 = 90_000;

/// The lockout bookkeeping: three of the four globals `CanShowResetInstances` (`0x495c90`) reads;
/// the fourth, the map we stand on (`0xb4e378`), is [`wrathplus_world::world_map::CurrentMap`].
/// Session-scoped, like the reference's zero-initialized globals.
#[derive(Resource, Default)]
pub struct InstanceState {
    /// `0xb4e37c`, "do you hold any bind": `SMSG_UPDATE_INSTANCE_OWNERSHIP`, sent on every map
    /// change (`Player.cpp:2116`), where vmangos says yes only for a raid save.
    owns_saved: bool,
    /// WrathPlus's half of the same term: we watched the player walk out of a party dungeon. Raised
    /// by [`LatchWriter::WorldEntry`], cleared by a reset and at logout, and never by an
    /// `owns_saved = false`, which vmangos sends on the very teleport out of the dungeon.
    saw_own_dungeon: bool,
    /// `0xb4e374`: the `Map.dbc` id of the last dungeon left; only a party dungeon lands here
    /// (`0x495d33`).
    last_dungeon: Option<u32>,
    /// `0xb4e370`: when [`Self::last_dungeon`] was recorded, in seconds. Deviation: the app's
    /// monotonic clock, not `time(0)`, because it cannot run backwards; they differ only across a
    /// suspend.
    last_dungeon_at: u64,
    /// `SMSG_UPDATE_LAST_INSTANCE` map ids for [`track_instance_state`]: the packet tests against
    /// the current map, which [`wrathplus_world::world_map::CurrentMap`] reaches only after the
    /// drain's deferred insert.
    pending_last_instance: Vec<u32>,
    /// Lines and welcomes for [`feed_instance`], in wire order; the wire drain cannot reach the VM.
    lines: Vec<Pending>,
    /// The instance lock's changes in wire order, `Some((timeout_ms, encounter_mask, flag))` a
    /// set and `None` a clear, for [`crate::ui_static_popup`] to apply.
    lock_changes: Vec<Option<(u32, u32, u8)>>,
}

/// One queued message, in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Line(LockoutLine),
    /// `RAID_INSTANCE_WELCOME`'s arguments.
    Welcome {
        map: u32,
        difficulty: u32,
        seconds: i32,
        locked: bool,
        extended: bool,
    },
}

/// One lockout line as the reference holds it between `GetText` and `SStrPrintf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockoutLine {
    /// The `GlobalStrings.lua` token; 3.3.5's plurals are `|4` escapes in the template.
    token: &'static str,
    /// Whose name fills `%s`; `None` for `INSTANCE_SAVED`, which has none.
    name: Option<InstanceName>,
    /// The `%d` fills, after the `%s`.
    numbers: Vec<i64>,
    /// The `"(Debug-Only Lock Notice) %s"` wrapper, the save notice's flag-1 arm.
    debug_notice: bool,
}

/// The instance a line names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstanceName {
    /// `004fe070`: the `Map.dbc` name (empty with no row), as `DUNGEON_NAME_WITH_DIFFICULTY` with
    /// its `MapDifficulty` row's difficulty string when that is set.
    WithDifficulty { map: u32, difficulty: u32 },
    /// The reset handlers: the `Map.dbc` name, or the id in decimal (`"%d"`) with no row.
    Map(u32),
}

/// Which of the last-dungeon latch's two writers is speaking. The reference does not care; WrathPlus
/// does, as only the world entry is first-hand evidence of a bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LatchWriter {
    /// `0x464ff0`, the map-load path: we just walked out of this map.
    WorldEntry,
    /// `0x49e6ac`, `SMSG_UPDATE_LAST_INSTANCE`: the server's word, already given as `owns_saved`.
    Packet,
}

impl InstanceState {
    /// `SMSG_UPDATE_INSTANCE_OWNERSHIP` → `0x495d50`: one store, no line.
    pub(crate) fn set_ownership(&mut self, owns: bool) {
        self.owns_saved = owns;
    }

    /// The latch `0x495d10`: record `map` as the dungeon just left if it is a party dungeon and
    /// not the map we stand on, tested first (`cmp edx,esi; je`), as vmangos sends one packet per
    /// permanent bind on every map change, the entered instance included.
    pub(crate) fn note_last_instance(
        &mut self,
        map: u32,
        current_map: Option<u32>,
        party_dungeon: bool,
        now_secs: u64,
        writer: LatchWriter,
    ) {
        if current_map == Some(map) {
            return;
        }
        if party_dungeon {
            self.last_dungeon = Some(map);
            self.last_dungeon_at = now_secs;
            // Walking out of a 5-man proves the bind the server took on entry.
            if writer == LatchWriter::WorldEntry {
                self.saw_own_dungeon = true;
            }
        }
    }

    /// `SMSG_INSTANCE_RESET` → `0x495d00`, before the body is read: any reset clears the offer.
    pub(crate) fn clear_last_instance(&mut self) {
        self.last_dungeon = None;
        self.last_dungeon_at = 0;
        // The witness goes with the reset; the reference's bind term is the server's to retract.
        self.forget_witness();
    }

    /// Drop WrathPlus's half of the bind term, the one piece of state no server corrects.
    pub(crate) fn forget_witness(&mut self) {
        self.saw_own_dungeon = false;
    }

    /// `CanShowResetInstances()` (`0x495c90`), its four terms in the reference's order:
    ///
    /// 1. a bind held: `0xb4e37c`, or [`InstanceState::saw_own_dungeon`];
    /// 2. not standing in a party dungeon;
    /// 3. the last dungeon left is a party dungeon `Map.dbc` knows;
    /// 4. left no more than [`RESET_OFFER_WINDOW_SECS`] ago.
    ///
    /// A map with no `Map.dbc` row is not a party dungeon (the reference's null-record branch).
    pub(crate) fn can_reset(
        &self,
        current_map: Option<u32>,
        is_party_dungeon: &dyn Fn(u32) -> bool,
        now_secs: u64,
    ) -> bool {
        if !self.owns_saved && !self.saw_own_dungeon {
            return false;
        }
        if current_map.is_some_and(is_party_dungeon) {
            return false;
        }
        let Some(last) = self.last_dungeon else {
            return false;
        };
        if !is_party_dungeon(last) {
            return false;
        }
        now_secs.saturating_sub(self.last_dungeon_at) <= RESET_OFFER_WINDOW_SECS
    }

    /// Queue a line: one per packet, so two identical warnings print twice, as in the reference.
    fn push(&mut self, line: Pending) {
        self.lines.push(line);
    }

    fn take_lines(&mut self) -> Vec<Pending> {
        std::mem::take(&mut self.lines)
    }

    /// The bind term, the queued last-instance ids and the queued messages, for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot(&self) -> (bool, &[u32], &[Pending]) {
        (self.owns_saved, &self.pending_last_instance, &self.lines)
    }

    /// The lock changes since the last take, in wire order.
    pub(crate) fn take_lock_changes(&mut self) -> Vec<Option<(u32, u32, u8)>> {
        std::mem::take(&mut self.lock_changes)
    }

    fn take_pending_last_instance(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.pending_last_instance)
    }
}

/// The reference's `"(Debug-Only Lock Notice) %s"`, a literal in the binary (`0052bb10`).
const DEBUG_LOCK_NOTICE_PREFIX: &str = "(Debug-Only Lock Notice) ";

/// `GetInstanceInfo`'s row for the current map (`0x51a8c0`): `Map.dbc`'s name, type and
/// `Flags & 0x100`, the latest instance difficulty and its `MapDifficulty` row. `None` for a map
/// with no `Map.dbc` row. Labelled: the difficulty is the latest `SMSG_INSTANCE_DIFFICULTY`'s, 0
/// before one; its other writer (`0x52b550`) is not decompiled.
fn instance_info(
    map: u32,
    maps: Option<&wrathplus_assets::MapCatalogRes>,
    lock: Option<&crate::ui_static_popup::InstanceLock>,
    difficulties: Option<&crate::ui_calendar::CalendarCatalog>,
) -> Option<wrathplus_ui::script::WrathInstanceInfo> {
    let maps = &maps?.0;
    let instance_type = maps.instance_type(map)?;
    let difficulty = lock.and_then(|l| l.difficulty).unwrap_or(0);
    Some(wrathplus_ui::script::WrathInstanceInfo {
        name: maps.name(map).unwrap_or_default().to_string(),
        instance_type,
        difficulty,
        map_difficulty: difficulties.and_then(|d| d.difficulty(map, difficulty)),
        dynamic_difficulty: lock.map_or(0, |l| l.dynamic_difficulty),
        is_dynamic: maps.flags(map) & 0x100 != 0,
    })
}

fn map_name(map: u32, catalog: Option<&wrathplus_assets::MapCatalogRes>) -> String {
    catalog
        .and_then(|c| c.0.name(map))
        .filter(|n| !n.is_empty())
        .map_or_else(|| map.to_string(), str::to_string)
}

/// `004fe070`'s name: the `Map.dbc` name (empty with no row) and the `MapDifficulty` row's
/// difficulty string.
fn dungeon_name(
    map: u32,
    difficulty: u32,
    maps: Option<&wrathplus_assets::MapCatalogRes>,
    difficulties: Option<&crate::ui_calendar::CalendarCatalog>,
    get: &dyn Fn(&str) -> Option<String>,
) -> String {
    let Some(name) = maps.and_then(|c| c.0.name(map)) else {
        return String::new();
    };
    let key = difficulties
        .and_then(|d| d.difficulty(map, difficulty))
        .map(|(key, _)| key);
    name_with_difficulty(name, key.as_deref(), get)
}

/// `DUNGEON_NAME_WITH_DIFFICULTY` (`"%1$s (%2$s)"`) over a name and a difficulty string's text;
/// the bare name with no (or an empty) difficulty string.
fn name_with_difficulty(
    name: &str,
    key: Option<&str>,
    get: &dyn Fn(&str) -> Option<String>,
) -> String {
    use wrathplus_ui::strings::Arg;
    let key = key.filter(|k| !k.is_empty());
    let (Some(key), Some(template)) = (key, get("DUNGEON_NAME_WITH_DIFFICULTY")) else {
        return name.to_string();
    };
    let label = get(key).unwrap_or_else(|| key.to_string());
    wrathplus_ui::strings::fill(&template, &[Arg::S(name), Arg::S(&label)])
}

/// One line's text: the template, the `%s` and `%d` fills, then the debug wrapper; `None` for a
/// token the player's `GlobalStrings.lua` lacks or leaves empty.
fn lockout_text(
    line: &LockoutLine,
    name: Option<&str>,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    use wrathplus_ui::strings::Arg;
    let template = get(line.token)?;
    let mut args: Vec<Arg<'_>> = name.map(Arg::S).into_iter().collect();
    args.extend(line.numbers.iter().map(|&n| Arg::D(n)));
    let text = wrathplus_ui::strings::fill(&template, &args);
    if text.is_empty() {
        return None;
    }
    Some(if line.debug_notice {
        format!("{DEBUG_LOCK_NOTICE_PREFIX}{text}")
    } else {
        text
    })
}

/// `0050ca80`'s kinds 1, 2, 3 and 5; `None` for 4 (the welcome event) and the dropped kinds. The
/// seconds divide as a signed `int`, truncated.
fn raid_instance_line(kind: u32, map: u32, difficulty: u32, seconds: i32) -> Option<LockoutLine> {
    let (token, numbers) = match kind {
        1 => (
            "RAID_INSTANCE_WARNING_HOURS",
            vec![i64::from(seconds / 3_600)],
        ),
        2 => ("RAID_INSTANCE_WARNING_MIN", vec![i64::from(seconds / 60)]),
        3 => (
            "RAID_INSTANCE_WARNING_MIN_SOON",
            vec![i64::from(seconds / 60)],
        ),
        5 => ("RAID_INSTANCE_EXPIRED", Vec::new()),
        _ => return None,
    };
    Some(LockoutLine {
        token,
        name: Some(InstanceName::WithDifficulty { map, difficulty }),
        numbers,
        debug_notice: false,
    })
}

/// The lockout packets' handler; the feed, which has the VM, resolves what it queues.
pub mod net {
    use super::*;

    use crate::net::wrath_instance_control_capture::NativeInstanceControlPackets;

    /// The one consumer of the capture's lockout kinds, in wire order: lines, welcomes and the
    /// latch here, and the lock's set and clear (`00513b20`: a timed warning sets, a zero one or
    /// save flag 0 clears) queued for [`crate::ui_static_popup`]. Every other receipt stays.
    pub fn take_native(
        packets: Option<ResMut<NativeInstanceControlPackets>>,
        mut state: ResMut<InstanceState>,
    ) {
        let Some(mut packets) = packets else {
            return;
        };
        let state = &mut *state;
        packets.0.retain(|packet| {
            match *packet {
                Packet::RaidMessage {
                    kind,
                    map,
                    difficulty,
                    seconds,
                    welcome_flags,
                } => raid_instance_message(state, kind, map, difficulty, seconds, welcome_flags),
                Packet::SaveCreated { raw_value } => {
                    if raw_value == 0 {
                        state.lock_changes.push(None);
                    }
                    instance_save_created(state, raw_value);
                }
                Packet::LockWarning {
                    timeout_ms,
                    encounter_mask,
                    raw_flag,
                } => state.lock_changes.push((timeout_ms > 0).then_some((
                    timeout_ms,
                    encounter_mask,
                    raw_flag,
                ))),
                Packet::Reset { map } => instance_reset(state, map),
                Packet::ResetFailed { reason, map } => instance_reset_failed(state, reason, map),
                Packet::LastInstance { map } => state.pending_last_instance.push(map),
                // `SMSG_UPDATE_INSTANCE_OWNERSHIP`: one store, no line.
                Packet::Ownership { raw_value } => state.set_ownership(raw_value != 0),
                _ => return true,
            }
            false
        });
    }

    /// `0050ca80`: a warning or expiry line, or kind 4's welcome event (its two flag bytes read
    /// as `!= 0`).
    pub(crate) fn raid_instance_message(
        state: &mut InstanceState,
        kind: u32,
        map: u32,
        difficulty: u32,
        seconds: u32,
        welcome_flags: Option<[u8; 2]>,
    ) {
        let seconds = seconds as i32;
        if kind == 4 {
            let [locked, extended] = welcome_flags.unwrap_or_default();
            state.push(Pending::Welcome {
                map,
                difficulty,
                seconds,
                locked: locked != 0,
                extended: extended != 0,
            });
            return;
        }
        match raid_instance_line(kind, map, difficulty, seconds) {
            Some(line) => state.push(Pending::Line(line)),
            None => debug!("ui_instance: SMSG_RAID_INSTANCE_MESSAGE kind {kind}: no line"),
        }
    }

    /// `0050ccd0`: clear the latch (`005138a0`) before the body is read, then
    /// `INSTANCE_RESET_SUCCESS`.
    pub(crate) fn instance_reset(state: &mut InstanceState, map: u32) {
        state.clear_last_instance();
        state.push(Pending::Line(LockoutLine {
            token: "INSTANCE_RESET_SUCCESS",
            name: Some(InstanceName::Map(map)),
            numbers: Vec::new(),
            debug_notice: false,
        }));
    }

    /// `0050cda0`: one of three refusals by reason. Deviation: reason 3 and up prints nothing,
    /// where the reference prints an uninitialized stack buffer.
    pub(crate) fn instance_reset_failed(state: &mut InstanceState, reason: u32, map: u32) {
        let token = match reason {
            0 => "INSTANCE_RESET_FAILED",
            1 => "INSTANCE_RESET_FAILED_OFFLINE",
            2 => "INSTANCE_RESET_FAILED_ZONING",
            _ => return debug!("ui_instance: SMSG_INSTANCE_RESET_FAILED reason {reason}: no line"),
        };
        state.push(Pending::Line(LockoutLine {
            token,
            name: Some(InstanceName::Map(map)),
            numbers: Vec::new(),
            debug_notice: false,
        }));
    }

    /// `0052bb10`: `INSTANCE_SAVED`, bare on flag 0 and wrapped on flag 1. Deviation: flag 2 and up
    /// prints nothing, where the reference prints an uninitialized stack buffer.
    pub(crate) fn instance_save_created(state: &mut InstanceState, flag: u32) {
        let debug_notice = match flag {
            0 => false,
            1 => true,
            other => {
                return debug!("ui_instance: SMSG_INSTANCE_SAVE_CREATED flag {other}: no line")
            }
        };
        state.push(Pending::Line(LockoutLine {
            token: "INSTANCE_SAVED",
            name: None,
            numbers: Vec::new(),
            debug_notice,
        }));
    }
}

/// Runs both latch writers, VM-free so the latch survives a `/reload` as the reference's globals
/// do. The first observed map records nothing: there is no map left at login, as in the reference
/// (`0xb4e378` starts at 0, and map 0 is not a dungeon).
fn track_instance_state(
    mut state: ResMut<InstanceState>,
    maps: Option<Res<wrathplus_assets::MapCatalogRes>>,
    current_map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
    time: Res<Time<Real>>,
    mut last_map: Local<Option<u32>>,
) {
    let Some(here) = current_map.as_ref().map(|m| m.0) else {
        return;
    };
    let now = time.elapsed().as_secs();
    let party_dungeon = |m: u32| maps.as_ref().is_some_and(|c| c.0.is_party_dungeon(m));

    // Writer 1, the world entry: `prev` is the map we are leaving.
    if let Some(prev) = last_map.replace(here) {
        state.note_last_instance(
            prev,
            Some(here),
            party_dungeon(prev),
            now,
            LatchWriter::WorldEntry,
        );
    }
    // Writer 2, the packet, now that `CurrentMap` has caught up with the transfer.
    for map in state.take_pending_last_instance() {
        let is_dungeon = party_dungeon(map);
        state.note_last_instance(map, Some(here), is_dungeon, now, LatchWriter::Packet);
    }
}

/// Show every queued line as `CHAT_MSG_SYSTEM` and fire every welcome, in wire order, then publish
/// `IsInInstance`, `GetInstanceInfo` and `CanShowResetInstances` every frame, as they move with
/// the map and the clock (the setters diff).
#[allow(clippy::too_many_arguments)]
fn feed_instance(
    script: Option<NonSendMut<UiScript>>,
    mut state: ResMut<InstanceState>,
    mut chat: ResMut<ChatLog>,
    maps: Option<Res<wrathplus_assets::MapCatalogRes>>,
    current_map: Option<Res<wrathplus_world::world_map::CurrentMap>>,
    time: Res<Time<Real>>,
    lock: Option<Res<crate::ui_static_popup::InstanceLock>>,
    difficulties: Option<Res<crate::ui_calendar::CalendarCatalog>>,
) {
    let Some(mut script) = script else {
        // No VM (an engine-only run): drop the queue rather than let it grow.
        let dropped = state.take_lines().len();
        if dropped > 0 {
            debug!("ui_instance: no VM — dropped {dropped} lockout message(s)");
        }
        return;
    };
    let here = current_map.as_ref().map(|m| m.0);
    let now = time.elapsed().as_secs();
    let (maps_ref, difficulties_ref) = (maps.as_deref(), difficulties.as_deref());
    for pending in state.take_lines() {
        let get = |key: &str| script.lua().globals().get::<String>(key).ok();
        let name = |n: InstanceName| match n {
            InstanceName::Map(map) => map_name(map, maps_ref),
            InstanceName::WithDifficulty { map, difficulty } => {
                dungeon_name(map, difficulty, maps_ref, difficulties_ref, &get)
            }
        };
        match pending {
            Pending::Line(line) => {
                match lockout_text(&line, line.name.map(name).as_deref(), &get) {
                    Some(text) => {
                        chat.push_event(ChatEvent::text_only(ChatEventKind::System, text))
                    }
                    None => debug!("ui_instance: {} resolves to nothing — no line", line.token),
                }
            }
            Pending::Welcome {
                map,
                difficulty,
                seconds,
                locked,
                extended,
            } => {
                let args = vec![
                    V::Str(name(InstanceName::WithDifficulty { map, difficulty })),
                    V::Int(i64::from(seconds)),
                    V::Int(i64::from(locked)),
                    V::Int(i64::from(extended)),
                ];
                script.fire_event("RAID_INSTANCE_WELCOME", args);
            }
        }
    }

    script.set_instance_type(here.and_then(|m| maps.as_ref().and_then(|c| c.0.instance_type(m))));
    script.set_wrath_instance_info(
        here.and_then(|m| {
            instance_info(m, maps.as_deref(), lock.as_deref(), difficulties.as_deref())
        }),
    );
    let party_dungeon = |m: u32| maps.as_ref().is_some_and(|c| c.0.is_party_dungeon(m));
    script.set_can_reset_instances(state.can_reset(here, &party_dungeon, now));
}

/// Turn the `ResetInstances()` calls the dialog made into `CMSG_RESET_INSTANCES` sends.
fn drain_instance(script: Option<NonSendMut<UiScript>>, commands: Res<NetCommands>) {
    let Some(mut script) = script else {
        return;
    };
    for _ in 0..script.take_reset_instance_asks() {
        let _ = commands.0.send(ClientCommand::ResetInstances);
    }
}

/// Drop the witness at logout, as the next login may be another character. The three globals stay,
/// as the reference keeps them (`0x495d00` runs only on `SMSG_INSTANCE_RESET`): its bind term is
/// re-sent at the next world entry, where ours has no server to correct it.
fn clear_witness_on_logout(
    mut state: ResMut<InstanceState>,
    mut logged_out: MessageReader<crate::net::LoggedOutMessage>,
) {
    if logged_out.read().next().is_some() {
        state.forget_witness();
    }
}

pub struct UiInstancePlugin;

impl Plugin for UiInstancePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InstanceState>().add_systems(
            Update,
            (
                // The latch before the feed, so a map change and its answer share a frame.
                net::take_native
                    .after(wrathplus_world::schedule::WorldStage::Net)
                    .before(track_instance_state),
                clear_witness_on_logout.before(track_instance_state),
                track_instance_state.before(feed_instance),
                // The welcome is a one-shot event: it waits for the interface.
                feed_instance
                    .in_set(UiFeed)
                    .run_if(crate::app_core::ui_state::ingame_ui_up),
                drain_instance.after(UiInput),
            ),
        );
    }
}

#[cfg(test)]
#[path = "ui_instance_tests.rs"]
mod tests;
