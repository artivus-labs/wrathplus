//! Build-12340 LFG L2 (https://github.com/artivus-labs/wrathplus/wiki/WRATH-LFG-L2-CONTRACT): the queue, role check, proposal, boot and
//! completion getters over App's [`WrathLfgQueueSnapshot`], the L0 selection, and the verbs'
//! outbox. One Wow.exe round: `/tmp/azb-ui-lfg-l2-native.log`, `/tmp/azb-ui-lfg-l2-inner.log`,
//! `/tmp/azb-ui-lfg-l2-LfgL2Rest.log`, `/tmp/azb-ui-lfg-l2-LfgL2Inner2.log`.
//!
//! The snapshot setter only stores: App fires the native events after publishing. The verbs
//! mutate only what native mutates locally (the L0 selection and comment, the sent flag, the
//! proposal answer), and leave every send to App as a [`WrathLfgQueueRequest`].
use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript};
use crate::script::binding_abi::{bool_or_default, number_arg};

/// `0x367`/`0x368`, as native stores them at `0xacd9a0` (player) and `0xacdac0` (party).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgUpdate {
    pub update_type: u8,
    /// `+0`.
    pub joined: bool,
    /// `+2`.
    pub queued: bool,
    /// `+4`.
    pub no_partial_clear: bool,
    /// `+5`.
    pub achievements: bool,
    /// `+6..+8`, the party update's three trailing bytes.
    pub unknown: [u8; 3],
    /// Full packed entries in wire order (`+0x14`).
    pub dungeons: Vec<u32>,
    /// `+0x1c`.
    pub comment: String,
}

/// `0x365` (`0xbeaa28..`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgQueueStatus {
    /// Full packed entry.
    pub dungeon: u32,
    pub avg_wait: i32,
    pub tank_wait: i32,
    pub healer_wait: i32,
    pub dps_wait: i32,
    pub my_wait: i32,
    /// As the handler stored them, its "three zeros read as one damager" already applied.
    pub tank_needs: u8,
    pub healer_needs: u8,
    pub dps_needs: u8,
    /// When the queue began, on the `GetTime` clock in milliseconds.
    pub queued_at_ms: u32,
}

/// A role-check member (`0xbeaa68`, `0x18` bytes each).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgRoleCheckMember {
    pub guid: u64,
    pub ready: bool,
    pub roles: u8,
    pub level: u8,
    /// From the name cache; `None` while unresolved.
    pub name: Option<String>,
}

/// `0x363` (`0xbeaa4c..`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgRoleCheck {
    pub state: u32,
    pub is_beginning: bool,
    /// Full packed entries.
    pub dungeons: Vec<u32>,
    pub members: Vec<WrathLfgRoleCheckMember>,
}

/// One anonymous proposal row (`0xbea994`, `0x18` bytes each, five of them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgProposalRow {
    /// Role bits: 1 leader, 2 tank, 4 healer, 8 damager.
    pub roles: u32,
    pub is_self: bool,
    pub in_dungeon: bool,
    pub same_group: bool,
    pub answered: bool,
    pub accepted: bool,
}

/// `0x361` (`0xbea980..`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgProposal {
    /// Full packed entry.
    pub dungeon: u32,
    pub state: u8,
    pub id: u32,
    /// Completed encounters, one bit per DungeonEncounter.dbc bit index.
    pub completed_mask: u32,
    pub silent: bool,
    pub rows: Vec<WrathLfgProposalRow>,
}

/// `0x36D` (`0xbea858..`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgBoot {
    pub in_progress: bool,
    pub did_vote: bool,
    pub my_vote: bool,
    pub victim: u64,
    pub victim_name: Option<String>,
    pub victim_realm: Option<String>,
    pub total_votes: u32,
    pub boot_votes: u32,
    pub time_left: i32,
    pub reason: String,
}

/// A completion reward item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgCompletionItem {
    pub item: u32,
    pub display: u32,
    pub count: u32,
}

/// `0x1FF` (`0xacdc1c..`), in native's slots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgCompletion {
    /// Full packed entry.
    pub dungeon: u32,
    pub strangers: u32,
    pub money_base: i32,
    pub xp_base: i32,
    pub money_var: i32,
    pub xp_var: i32,
    pub items: Vec<WrathLfgCompletionItem>,
}

/// App's one reducer output, published before each packet's events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgQueueSnapshot {
    pub player: Option<WrathLfgUpdate>,
    pub party: Option<WrathLfgUpdate>,
    pub queue: Option<WrathLfgQueueStatus>,
    pub role_check: Option<WrathLfgRoleCheck>,
    pub proposal: Option<WrathLfgProposal>,
    pub boot: Option<WrathLfgBoot>,
    pub completion: Option<WrathLfgCompletion>,
    pub disabled: bool,
}

/// A DungeonEncounter.dbc row (`0xad36cc`, `0x1c` bytes each).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathLfgEncounter {
    pub map: i32,
    pub difficulty: i32,
    pub bit: u32,
    pub name: String,
    /// The SpellIcon path, `None` without one.
    pub icon: Option<String>,
}

/// One verb for App to send, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathLfgQueueRequest {
    /// CMSG `0x35C` (`0x5572d0`): roles (masked by class), the two flag bytes, the selection,
    /// the three needs bytes and the comment. The flags and needs have no 3.3.5 Lua setter and
    /// stay zero.
    Join {
        roles: u8,
        no_partial_clear: bool,
        achievements: bool,
        dungeons: Vec<u32>,
        needs: [u8; 3],
        comment: String,
    },
    /// CMSG `0x35D` (`0x553ab0`).
    Leave,
    /// CMSG `0x36A` (`0x553e20`): the class-masked roles, 0 to decline.
    RoleCheckAnswer { roles: u8 },
    /// CMSG `0x362` (`0x554450`).
    Proposal { id: u32, accept: bool },
    /// CMSG `0x370` (`0x556bc0`).
    Teleport { out: bool },
    /// CMSG `0x366` (`0x5539a0`).
    SetComment { text: String },
}

/// The native verb throttle: three calls, then none until 10 s (`0x9e30cc`) pass, or 2 s
/// (`0xa4040c`) for the comment.
#[derive(Clone, Debug, Default)]
struct Throttle {
    count: u32,
    since: f64,
}

impl Throttle {
    fn allow(&mut self, now: f64, window: f64) -> bool {
        self.count += 1;
        if self.count > 2 {
            if now - self.since < window {
                return false;
            }
            self.since = now;
            self.count = 0;
        }
        true
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Queue {
    snapshot: Arc<WrathLfgQueueSnapshot>,
    /// The proposal as the getters see it: the snapshot's, with native's local answer applied.
    proposal: Option<WrathLfgProposal>,
    encounters: Arc<Vec<WrathLfgEncounter>>,
    /// The current map and difficulty `IsInLFGDungeon` compares against.
    location: Option<(i32, i32)>,
    /// The L0 selection (`0xacd894`, count `0xacd898`), full packed entries.
    selection: Vec<u32>,
    /// `0xacd89c`.
    comment: String,
    /// `0xacd880`: a join was sent and not left.
    sent: bool,
    /// `0xacd883`: the comment changed before a join, so the next join resends.
    pending: bool,
    join: Throttle,
    comment_throttle: Throttle,
    role_check: Throttle,
    answer: Throttle,
    teleport: Throttle,
    requests: Vec<WrathLfgQueueRequest>,
}

impl Queue {
    /// DungeonEncounter.dbc, shared with the raid browser's encounter getters.
    pub(super) fn encounters(&self) -> &[WrathLfgEncounter] {
        &self.encounters
    }

    /// The current map and difficulty, `None` while unknown.
    pub(super) fn location(&self) -> Option<(i32, i32)> {
        self.location
    }
}

impl UiScript {
    /// Publish App's L2 snapshot. Stores only; App fires the events.
    pub fn set_wrath_lfg_queue(&mut self, snapshot: Arc<WrathLfgQueueSnapshot>) {
        let mut model = self.model_mut();
        let queue = &mut model.wrath_lfg.queue;
        queue.proposal = snapshot.proposal.clone();
        queue.snapshot = snapshot;
    }

    /// DungeonEncounter.dbc, for the proposal's encounter getters.
    pub fn set_wrath_lfg_encounters(&mut self, encounters: Arc<Vec<WrathLfgEncounter>>) {
        self.model_mut().wrath_lfg.queue.encounters = encounters;
    }

    /// The current map and difficulty, `None` while unknown.
    pub fn set_wrath_lfg_location(&mut self, location: Option<(i32, i32)>) {
        self.model_mut().wrath_lfg.queue.location = location;
    }

    /// Drain the L2 verbs in call order.
    pub fn take_wrath_lfg_queue_requests(&mut self) -> Vec<WrathLfgQueueRequest> {
        std::mem::take(&mut self.model_mut().wrath_lfg.queue.requests)
    }
}

fn grouped(m: &Model) -> bool {
    !m.party.members.is_empty() || !m.party.raid.is_empty()
}

/// The update record the getters read: the party's while grouped, else the player's.
fn record(m: &Model) -> Option<&WrathLfgUpdate> {
    let s = &m.wrath_lfg.queue.snapshot;
    if grouped(m) {
        s.party.as_ref()
    } else {
        s.player.as_ref()
    }
}

fn row(m: &Model, entry: u32) -> Option<&super::WrathLfgDungeon> {
    m.wrath_lfg.row(entry & 0xff_ffff)
}

fn int(n: i64) -> Value {
    Value::Integer(n)
}

fn text(lua: &Lua, s: &str) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(s)?))
}

fn role_name(roles: u32) -> &'static str {
    if roles & 2 != 0 {
        "TANK"
    } else if roles & 4 != 0 {
        "HEALER"
    } else if roles & 8 != 0 {
        "DAMAGER"
    } else {
        "UNKNOWN"
    }
}

/// `"Userlist"[class + 0xb]`: the roles each class may take, by ChrClasses id.
const CLASS_ROLES: [u8; 12] = [
    0, 0x0b, 0x0f, 0x09, 0x09, 0x0d, 0x0b, 0x0d, 0x09, 0x09, 0, 0x0f,
];

fn class_mask(m: &Model) -> u8 {
    let id = match m.unit("player").and_then(|p| p.class_file.as_deref()) {
        Some("WARRIOR") => 1,
        Some("PALADIN") => 2,
        Some("HUNTER") => 3,
        Some("ROGUE") => 4,
        Some("PRIEST") => 5,
        Some("DEATHKNIGHT") => 6,
        Some("SHAMAN") => 7,
        Some("MAGE") => 8,
        Some("WARLOCK") => 9,
        Some("DRUID") => 11,
        // Labelled: native reads the player's ChrClasses row; with no class known the roles pass
        // unmasked.
        _ => return 0x0f,
    };
    CLASS_ROLES[id]
}

/// The `lfgSelectedRoles` CVar (`0xbd0a28`, value at `+0x30`); `None` when unregistered.
fn roles_cvar(m: &Model) -> Option<u8> {
    m.cvars
        .get("lfgselectedroles")
        .map(|c| c.value.trim().parse::<i64>().unwrap_or(0) as u8)
}

fn error(lua: &Lua, key: &'static str) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .ui_errors
        .push(key);
}

fn fire(lua: &Lua, event: &str) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .pending_events
        .push((event.to_string(), Vec::new()));
}

fn push(lua: &Lua, r: WrathLfgQueueRequest) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .wrath_lfg
        .queue
        .requests
        .push(r);
}

fn now(lua: &Lua) -> f64 {
    crate::script::clock::now(lua)
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// The native join send (`0x5572d0`), behind its throttle; nothing without a role past leader.
fn send_join(lua: &Lua) {
    let t = now(lua);
    let mut m = lua.app_data_mut::<Model>().expect("model app_data");
    let Some(roles) = roles_cvar(&m) else { return };
    if m.wrath_lfg.queue.sent && !m.wrath_lfg.queue.pending {
        return;
    }
    if !m.wrath_lfg.queue.join.allow(t, 10.0) {
        return;
    }
    let roles = roles & class_mask(&m);
    if roles & 0xfe == 0 {
        return;
    }
    let q = &mut m.wrath_lfg.queue;
    let request = WrathLfgQueueRequest::Join {
        roles,
        no_partial_clear: false,
        achievements: false,
        dungeons: q.selection.clone(),
        needs: [0; 3],
        comment: q.comment.clone(),
    };
    q.sent = true;
    q.pending = false;
    q.requests.push(request);
}

/// `PartyLFGStartBackfill` (`0x55a0c0`): the selection cleared, the group's LFG dungeon selected
/// when set, then the native join send.
pub(crate) fn start_backfill(lua: &Lua) {
    {
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        let entry = m.wrath_group.lfg_dungeon_entry;
        let selection = &mut m.wrath_lfg.queue.selection;
        selection.clear();
        if entry != 0 {
            selection.push(entry);
        }
    }
    send_join(lua);
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    use WrathLfgQueueRequest as R;

    // ── queue and server info ──
    // GetLFGQueueStats() (`0x553f90`) → 13 values while the record is joined and queued with a
    // queue dungeon: true, 0, the three needs, then the row's type and name, the four waits, my
    // wait and the queue start in seconds (eight nils without the row); nothing otherwise.
    set(lua, "GetLFGQueueStats", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(q) = m.wrath_lfg.queue.snapshot.queue.as_ref() else {
            return Ok(MultiValue::new());
        };
        if !record(&m).is_some_and(|r| r.joined && r.queued) || q.dungeon == 0 {
            return Ok(MultiValue::new());
        }
        let mut out = vec![
            Value::Boolean(true),
            int(0),
            int(q.tank_needs.into()),
            int(q.healer_needs.into()),
            int(q.dps_needs.into()),
        ];
        match row(&m, q.dungeon) {
            Some(r) => out.extend([
                int(r.dungeon_type.into()),
                text(lua, &r.name)?,
                int(q.avg_wait.into()),
                int(q.tank_wait.into()),
                int(q.healer_wait.into()),
                int(q.dps_wait.into()),
                int(q.my_wait.into()),
                Value::Number(f64::from(q.queued_at_ms) * 0.001),
            ]),
            None => out.extend(std::iter::repeat_n(Value::Nil, 8)),
        }
        Ok(MultiValue::from_vec(out))
    })?;
    // GetLFGInfoServer() (`0x552f30`) → inParty, joined, queued, noPartialClear, achievements,
    // comment, slot count and the three trailing bytes.
    set(lua, "GetLFGInfoServer", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let empty = WrathLfgUpdate::default();
        let r = record(&m).unwrap_or(&empty);
        let mut out = vec![
            Value::Boolean(grouped(&m)),
            Value::Boolean(r.joined),
            Value::Boolean(r.queued),
            Value::Boolean(r.no_partial_clear),
            Value::Boolean(r.achievements),
            text(lua, &r.comment)?,
            int(r.dungeons.len() as i64),
        ];
        out.extend(r.unknown.map(|b| int(b.into())));
        Ok(MultiValue::from_vec(out))
    })?;
    // GetLFGQueuedList([table]) (`0x557520`) → the table (cleared, or new) keyed by the
    // record's dungeon ids.
    set(lua, "GetLFGQueuedList", |lua, args| {
        let table = match arg(&args, 0) {
            Value::Table(t) => {
                t.clear()?;
                t
            }
            Value::Nil if args.is_empty() => lua.create_table()?,
            _ => return Err(mlua::Error::runtime("Usage: GetLFGQueuedList([table])")),
        };
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        if let Some(r) = record(&m) {
            for entry in &r.dungeons {
                table.raw_set(entry & 0xff_ffff, true)?;
            }
        }
        Ok(MultiValue::from_vec(vec![Value::Table(table)]))
    })?;
    // IsListedInLFR() (`0x5586d0` → `0x557260`): joined with a type-2 row in the list.
    set(lua, "IsListedInLFR", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let listed = record(&m).is_some_and(|r| {
            r.joined
                && r.dungeons
                    .iter()
                    .any(|&e| row(&m, e).is_some_and(|d| d.dungeon_type == 2))
        });
        Ok(MultiValue::from_vec(vec![Value::Boolean(listed)]))
    })?;

    // ── role check ──
    // GetLFGRoleUpdate() (`0x554610`) → state == 2, slot count, member count.
    set(lua, "GetLFGRoleUpdate", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let rc = m.wrath_lfg.queue.snapshot.role_check.as_ref();
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(rc.is_some_and(|r| r.state == 2)),
            int(rc.map_or(0, |r| r.dungeons.len() as i64)),
            int(rc.map_or(0, |r| r.members.len() as i64)),
        ]))
    })?;
    // GetLFGRoleUpdateSlot(index) (`0x554690`) → the type byte and id.
    set(lua, "GetLFGRoleUpdateSlot", |lua, args| {
        let i = number_arg(lua, arg(&args, 0), "Usage: GetLFGRoleUpdateSlot(index)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let entry = usize::try_from(i.wrapping_sub(1)).ok().and_then(|i| {
            m.wrath_lfg
                .queue
                .snapshot
                .role_check
                .as_ref()
                .and_then(|r| r.dungeons.get(i).copied())
        });
        Ok(match entry {
            Some(e) => {
                MultiValue::from_vec(vec![int((e >> 24).into()), int((e & 0xff_ffff).into())])
            }
            None => MultiValue::new(),
        })
    })?;
    // GetLFGRoleUpdateMember(index) (`0x554750`) → ready, level, name, class file ("" unknown),
    // then leader/tank/healer/damage for a ready member. An unready member pushes three nils
    // under a return count of 8, so native's first value is the index argument itself.
    set(lua, "GetLFGRoleUpdateMember", |lua, args| {
        let index = arg(&args, 0);
        let i = number_arg(lua, index.clone(), "Usage: GetLFGRoleUpdateMember(index)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(member) = usize::try_from(i.wrapping_sub(1)).ok().and_then(|i| {
            m.wrath_lfg
                .queue
                .snapshot
                .role_check
                .as_ref()
                .and_then(|r| r.members.get(i))
        }) else {
            return Ok(MultiValue::new());
        };
        let mut out = vec![Value::Boolean(member.ready), int(member.level.into())];
        match &member.name {
            Some(name) => {
                out.push(text(lua, name)?);
                // Labelled: `0x72aab0(0, name)` is unread; this answers the class file of the
                // streamed unit with the member's GUID, "" otherwise.
                let class = ["player", "party1", "party2", "party3", "party4"]
                    .iter()
                    .filter_map(|t| m.unit(t))
                    .find(|u| u.exists && u.guid == member.guid)
                    .and_then(|u| u.class_file.clone())
                    .unwrap_or_default();
                out.push(text(lua, &class)?);
            }
            None => out.extend([Value::Nil, Value::Nil]),
        }
        if member.ready {
            out.extend([1u8, 2, 4, 8].map(|b| Value::Boolean(member.roles & b != 0)));
        } else {
            out.insert(0, index);
            out.extend([Value::Nil, Value::Nil, Value::Nil]);
        }
        Ok(MultiValue::from_vec(out))
    })?;

    // ── proposal ──
    // GetLFGProposal() (`0x554170`) → 12 values for a proposal with a known row.
    set(lua, "GetLFGProposal", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let q = &m.wrath_lfg.queue;
        let Some(p) = q.proposal.as_ref().filter(|p| p.dungeon != 0) else {
            return Ok(MultiValue::new());
        };
        let Some(r) = row(&m, p.dungeon) else {
            return Ok(MultiValue::new());
        };
        let me = p.rows.iter().take(5).find(|r| r.is_self);
        let (roles, responded) = me.map_or((0, false), |r| (r.roles, r.answered));
        let (total, done) = if r.dungeon_type == 6 {
            (0, i64::from(p.completed_mask != 0))
        } else if r.flags & 4 != 0 {
            (1, i64::from(p.completed_mask != 0))
        } else {
            let rows = q
                .encounters
                .iter()
                .filter(|e| e.map == r.map_id && e.difficulty == r.difficulty);
            let (mut total, mut done) = (0, 0);
            for e in rows {
                total += 1;
                if p.completed_mask & (1 << (e.bit & 31)) != 0 {
                    done += 1;
                }
            }
            (total, done)
        };
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(true),
            int((p.dungeon >> 24).into()),
            int((p.dungeon & 0xff_ffff).into()),
            text(lua, &r.name)?,
            text(lua, &r.texture)?,
            text(lua, role_name(roles))?,
            Value::Boolean(responded),
            int(total),
            int(done),
            int(5),
            Value::Boolean(roles & 1 != 0),
            Value::Boolean(r.flags & 4 != 0),
        ]))
    })?;
    // GetLFGProposalMember(index) (`0x553000`) → leader, role, 0, responded, accepted, nil, nil:
    // the rows are anonymous.
    set(lua, "GetLFGProposalMember", |lua, args| {
        let i = number_arg(lua, arg(&args, 0), "Usage: GetLFGProposalMember(index)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(row) = m
            .wrath_lfg
            .queue
            .proposal
            .as_ref()
            .filter(|p| p.dungeon != 0 && (1..=5).contains(&i))
            .map(|p| p.rows.get(i as usize - 1).cloned().unwrap_or_default())
        else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(row.roles & 1 != 0),
            text(lua, role_name(row.roles))?,
            int(0),
            Value::Boolean(row.answered),
            Value::Boolean(row.accepted),
            Value::Nil,
            Value::Nil,
        ]))
    })?;
    // GetLFGProposalEncounter(index) (`0x554380` → `0x5538b0`) → the dungeon's index-th
    // encounter: name, icon, killed; three nils past the end.
    set(lua, "GetLFGProposalEncounter", |lua, args| {
        let i = number_arg(lua, arg(&args, 0), "Usage: GetLFGProposalEncounter(index)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let q = &m.wrath_lfg.queue;
        let (map, difficulty, mask) = q
            .proposal
            .as_ref()
            .filter(|p| p.dungeon != 0)
            .and_then(|p| row(&m, p.dungeon).map(|r| (r.map_id, r.difficulty, p.completed_mask)))
            .unwrap_or((0, 0, 0));
        let found = usize::try_from(i.wrapping_sub(1)).ok().and_then(|n| {
            q.encounters
                .iter()
                .filter(|e| e.map == map && e.difficulty == difficulty)
                .nth(n)
        });
        Ok(MultiValue::from_vec(match found {
            Some(e) => vec![
                text(lua, &e.name)?,
                match &e.icon {
                    Some(icon) => text(lua, icon)?,
                    None => Value::Nil,
                },
                Value::Boolean(mask & (1 << (e.bit & 31)) != 0),
            ],
            None => vec![Value::Nil, Value::Nil, Value::Nil],
        }))
    })?;

    // ── boot and completion ──
    // GetLFGBootProposal() (`0x554a60`) → false alone, or 8 values for a vote in progress.
    set(lua, "GetLFGBootProposal", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(b) = m
            .wrath_lfg
            .queue
            .snapshot
            .boot
            .as_ref()
            .filter(|b| b.in_progress)
        else {
            return Ok(MultiValue::from_vec(vec![Value::Boolean(false)]));
        };
        let name = match (&b.victim_name, &b.victim_realm) {
            (Some(n), Some(r)) if !r.is_empty() => text(lua, &format!("{n}-{r}"))?,
            (Some(n), _) => text(lua, n)?,
            (None, _) => Value::Nil,
        };
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(true),
            Value::Boolean(b.did_vote),
            Value::Boolean(b.my_vote),
            name,
            int(b.total_votes.into()),
            int(b.boot_votes.into()),
            int(b.time_left.into()),
            text(lua, &b.reason)?,
        ]))
    })?;
    // GetLFGCompletionReward() (`0x557e40`) → name, type, texture, money base and variance, XP
    // base and variance, strangers, item count.
    set(lua, "GetLFGCompletionReward", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(c) = m
            .wrath_lfg
            .queue
            .snapshot
            .completion
            .as_ref()
            .filter(|c| c.dungeon != 0)
        else {
            return Ok(MultiValue::new());
        };
        let Some(r) = row(&m, c.dungeon) else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            text(lua, &r.name)?,
            int(r.dungeon_type.into()),
            text(lua, &r.texture)?,
            int(c.money_base.into()),
            int(c.money_var.into()),
            int(c.xp_base.into()),
            int(c.xp_var.into()),
            int(c.strangers.into()),
            int(c.items.len() as i64),
        ]))
    })?;
    // GetLFGCompletionRewardItem(index) (`0x557f70`) → icon path and count.
    set(lua, "GetLFGCompletionRewardItem", |lua, args| {
        let i = number_arg(
            lua,
            arg(&args, 0),
            "Usage: GetLFGCompletionRewardItem(index)",
        )?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let item = usize::try_from(i.wrapping_sub(1)).ok().and_then(|i| {
            m.wrath_lfg
                .queue
                .snapshot
                .completion
                .as_ref()
                .and_then(|c| c.items.get(i))
        });
        let Some(item) = item else {
            return Ok(MultiValue::new());
        };
        let icon = m
            .wrath_lfg
            .presentation
            .display_icons
            .get(&item.display)
            .map_or(super::l1::FALLBACK_ICON, String::as_str);
        Ok(MultiValue::from_vec(vec![
            text(lua, icon)?,
            int(item.count.into()),
        ]))
    })?;

    // ── the group's LFG state ──
    // IsPartyLFG() (`0x52c310`): the group's LFG dungeon entry is set.
    set(lua, "IsPartyLFG", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![
            crate::script::binding_abi::flag(m.wrath_group.lfg_dungeon_entry != 0),
        ]))
    })?;
    // IsInLFGDungeon() (`0x555660`): that dungeon's map and difficulty are the current ones.
    set(lua, "IsInLFGDungeon", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let entry = m.wrath_group.lfg_dungeon_entry;
        let Some(r) = (entry != 0).then(|| row(&m, entry)).flatten() else {
            return Ok(MultiValue::new());
        };
        let here = m.wrath_lfg.queue.location == Some((r.map_id, r.difficulty));
        Ok(MultiValue::from_vec(vec![Value::Boolean(here)]))
    })?;
    // HasLFGRestrictions() (`0x52c350`).
    set(lua, "HasLFGRestrictions", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![Value::Boolean(
            m.wrath_group.lfg_restricted,
        )]))
    })?;
    // CanPartyLFGBackfill() (`0x553170`) / GetPartyLFGBackfillInfo() (`0x5556d0` → name, id,
    // type): an LFG group in status 1..3 without flag 2.
    fn backfill(m: &Model) -> Option<&super::WrathLfgDungeon> {
        let g = &m.wrath_group;
        (matches!(g.lfg_status, 1..=3) && g.lfg_dungeon_entry != 0 && g.lfg_flags & 2 == 0)
            .then(|| row(m, g.lfg_dungeon_entry))
            .flatten()
    }
    set(lua, "CanPartyLFGBackfill", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let g = &m.wrath_group;
        let can = matches!(g.lfg_status, 1..=3) && g.lfg_dungeon_entry != 0 && g.lfg_flags & 2 == 0;
        Ok(MultiValue::from_vec(vec![Value::Boolean(can)]))
    })?;
    set(lua, "GetPartyLFGBackfillInfo", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(r) = backfill(&m) else {
            return Ok(MultiValue::new());
        };
        Ok(MultiValue::from_vec(vec![
            text(lua, &r.name)?,
            int(r.id.into()),
            int(r.dungeon_type.into()),
        ]))
    })?;

    // ── L0 selection and roles ──
    // SetLFGDungeon(id) (`0x559f50` → `0x559b70`): a catalog row joins the selection as its full
    // entry. Labelled: `0x559b70` is unread; a repeat is not added twice.
    set(lua, "SetLFGDungeon", |lua, args| {
        let id = number_arg(lua, arg(&args, 0), "Usage: SetLFGDungeon(type)")? as u32;
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        if let Some(entry) = m.wrath_lfg.row(id).map(super::l1::full_entry) {
            let selection = &mut m.wrath_lfg.queue.selection;
            if !selection.contains(&entry) {
                selection.push(entry);
            }
        }
        Ok(MultiValue::new())
    })?;
    // ClearAllLFGDungeons() (`0x5595d0`, labelled: unread): empties the selection.
    set(lua, "ClearAllLFGDungeons", |lua, _| {
        lua.app_data_mut::<Model>()
            .expect("model app_data")
            .wrath_lfg
            .queue
            .selection
            .clear();
        Ok(MultiValue::new())
    })?;
    // GetLFGRoles() (`0x552e10`) → leader, tank, healer, damage from `lfgSelectedRoles`.
    set(lua, "GetLFGRoles", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(match roles_cvar(&m) {
            Some(r) => [1u8, 2, 4, 8].map(|b| Value::Boolean(r & b != 0)).to_vec(),
            None => vec![Value::Nil; 4],
        }))
    })?;
    // SetLFGRoles(leader, tank, healer, damage) (`0x553d60`): masked by the class, stored in
    // `lfgSelectedRoles`, then LFG_ROLE_UPDATE.
    set(lua, "SetLFGRoles", |lua, args| {
        let mut roles = 0u8;
        for (i, bit) in [1u8, 2, 4, 8].into_iter().enumerate() {
            if bool_or_default(args.get(i), false) {
                roles |= bit;
            }
        }
        {
            let mut m = lua.app_data_mut::<Model>().expect("model app_data");
            let value = (roles & class_mask(&m)).to_string();
            crate::script::cvars::set_from_engine(&mut m, "lfgSelectedRoles", value);
        }
        let event = "LFG_ROLE_UPDATE";
        fire(lua, event);
        Ok(MultiValue::new())
    })?;
    // SetLFGComment([text]) (`0x553f50` → `0x5539a0`): stored; sent only after a join, else
    // marked for the next join to resend.
    set(lua, "SetLFGComment", |lua, args| {
        let comment = match arg(&args, 0) {
            v @ (Value::String(_) | Value::Integer(_) | Value::Number(_)) => lua
                .coerce_string(v)?
                .map(|s| s.to_string_lossy())
                .unwrap_or_default(),
            _ => String::new(),
        };
        // Labelled: `0x7e1830` sanitises the text into 0x40 characters; this keeps the first 64.
        let comment: String = comment.chars().take(64).collect();
        let t = now(lua);
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        let q = &mut m.wrath_lfg.queue;
        q.comment = comment.clone();
        if !q.sent {
            q.pending = true;
        } else if q.comment_throttle.allow(t, 2.0) {
            q.requests.push(R::SetComment { text: comment });
        }
        Ok(MultiValue::new())
    })?;

    // ── verbs ──
    // JoinLFG() (`0x557660`): an empty selection or no role past leader refuses locally.
    set(lua, "JoinLFG", |lua, _| {
        let (empty, roles) = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            (m.wrath_lfg.queue.selection.is_empty(), roles_cvar(&m))
        };
        if empty {
            error(lua, "ERR_LFG_NO_SLOTS_SELECTED");
        } else if roles.is_some_and(|r| r & 0xfe != 0) {
            send_join(lua);
        } else {
            error(lua, "ERR_LFG_NO_ROLES_SELECTED");
        }
        Ok(MultiValue::new())
    })?;
    // LeaveLFG() (`0x554160` → `0x553ab0`): sent while a join is out or the record is joined.
    set(lua, "LeaveLFG", |lua, _| {
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        let joined = record(&m).is_some_and(|r| r.joined);
        let q = &mut m.wrath_lfg.queue;
        if q.sent || joined {
            q.sent = false;
            q.pending = false;
            q.requests.push(R::Leave);
        }
        Ok(MultiValue::new())
    })?;
    // CompleteLFGRoleCheck(accept) (`0x553e20`) → true when sent, false when refused.
    set(lua, "CompleteLFGRoleCheck", |lua, args| {
        let t = now(lua);
        let accept = bool_or_default(args.front(), false);
        let roles = {
            let mut m = lua.app_data_mut::<Model>().expect("model app_data");
            if !m.wrath_lfg.queue.role_check.allow(t, 10.0) {
                return Ok(MultiValue::new());
            }
            let Some(roles) = roles_cvar(&m) else {
                return Ok(MultiValue::new());
            };
            if accept {
                roles & class_mask(&m)
            } else {
                0
            }
        };
        if accept && roles & 0x0e == 0 {
            error(lua, "ERR_LFG_NO_ROLES_SELECTED");
            return Ok(MultiValue::from_vec(vec![Value::Boolean(false)]));
        }
        push(lua, R::RoleCheckAnswer { roles });
        Ok(MultiValue::from_vec(vec![Value::Boolean(true)]))
    })?;
    // AcceptProposal() / RejectProposal() (`0x5545e0`/`0x554600` → `0x554450`): behind the
    // throttle, the answer goes out with the stored id; declining drops the proposal, accepting
    // marks our row answered and accepted, then LFG_PROPOSAL_UPDATE. Labelled: accept's gate
    // `0x5191c0(9)` (an addon-action check) passes for the trusted UI.
    for (global, accept) in [("AcceptProposal", true), ("RejectProposal", false)] {
        set(lua, global, move |lua, _| {
            let t = now(lua);
            {
                let mut m = lua.app_data_mut::<Model>().expect("model app_data");
                let q = &mut m.wrath_lfg.queue;
                let Some(id) = q.proposal.as_ref().filter(|p| p.dungeon != 0).map(|p| p.id) else {
                    return Ok(MultiValue::new());
                };
                if !q.answer.allow(t, 10.0) {
                    return Ok(MultiValue::new());
                }
                q.requests.push(R::Proposal { id, accept });
                if accept {
                    if let Some(p) = q.proposal.as_mut() {
                        for row in p.rows.iter_mut().filter(|r| r.is_self) {
                            row.answered = true;
                            row.accepted = true;
                        }
                    }
                } else {
                    q.proposal = None;
                }
            }
            let event = "LFG_PROPOSAL_UPDATE";
            fire(lua, event);
            Ok(MultiValue::new())
        })?;
    }
    // LFGTeleport(out) (`0x556bc0`) → true when sent: a player, the throttle, then not in combat
    // (ERR_NOT_IN_COMBAT), alive (ERR_PLAYER_DEAD), not falling (ERR_NOT_WHILE_FALLING).
    // Labelled: the locked-out (`0x54a430`) and fatigue (`0x513f20`) refusals need host state.
    set(lua, "LFGTeleport", |lua, args| {
        let t = now(lua);
        let refusal = {
            let mut m = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(player) = m.unit("player").filter(|p| p.exists).cloned() else {
                return Ok(MultiValue::new());
            };
            if !m.wrath_lfg.queue.teleport.allow(t, 10.0) {
                return Ok(MultiValue::from_vec(vec![Value::Boolean(false)]));
            }
            if player.in_combat {
                Some("ERR_NOT_IN_COMBAT")
            } else if player.dead {
                Some("ERR_PLAYER_DEAD")
            } else if m.wrath_popup.falling() {
                Some("ERR_NOT_WHILE_FALLING")
            } else {
                None
            }
        };
        if let Some(key) = refusal {
            error(lua, key);
            return Ok(MultiValue::from_vec(vec![Value::Boolean(false)]));
        }
        push(
            lua,
            R::Teleport {
                out: bool_or_default(args.front(), false),
            },
        );
        Ok(MultiValue::from_vec(vec![Value::Boolean(true)]))
    })?;

    // ── the reward tooltips (labelled: `GameTooltip:SetLFGDungeonReward`/`SetLFGCompletionReward`
    // are unread; they show the item GetLFGDungeonRewardInfo / GetLFGCompletionRewardItem name,
    // through the id-keyed item render) ──
    let tooltip: mlua::Table =
        lua.named_registry_value(crate::script::tooltip::REG_TOOLTIP_METHODS)?;
    tooltip.set(
        "SetLFGDungeonReward",
        lua.create_function(|lua, (this, id, index): (mlua::Table, Value, Value)| {
            const USAGE: &str = "Usage: GameTooltip:SetLFGDungeonReward(dungeonID, lootIndex)";
            let id = number_arg(lua, id, USAGE)?;
            let index = number_arg(lua, index, USAGE)?;
            let item = {
                let m = lua.app_data_ref::<Model>().expect("model app_data");
                let state = &m.wrath_lfg;
                state
                    .row(id as u32)
                    .filter(|r| r.dungeon_type == 6 || r.flags & 4 != 0)
                    .map(super::l1::full_entry)
                    .and_then(|entry| super::l1::preview_item(state, entry, index.wrapping_sub(1)))
                    .map(|i| i.item)
            };
            match item {
                Some(item) => {
                    crate::script::tooltip_item::render_by_id(lua, &this, item, None, None)
                }
                None => Ok(()),
            }
        })?,
    )?;
    tooltip.set(
        "SetLFGCompletionReward",
        lua.create_function(|lua, (this, index): (mlua::Table, Value)| {
            let index = number_arg(
                lua,
                index,
                "Usage: GameTooltip:SetLFGCompletionReward(lootIndex)",
            )?;
            let item = {
                let m = lua.app_data_ref::<Model>().expect("model app_data");
                usize::try_from(index.wrapping_sub(1)).ok().and_then(|i| {
                    m.wrath_lfg
                        .queue
                        .snapshot
                        .completion
                        .as_ref()
                        .and_then(|c| c.items.get(i))
                        .map(|i| i.item)
                })
            };
            match item {
                Some(item) => {
                    crate::script::tooltip_item::render_by_id(lua, &this, item, None, None)
                }
                None => Ok(()),
            }
        })?,
    )?;

    // ── deserter and random cooldown: the unit's aura 71041 / 71328 (`0x555840`/`0x555760`) ──
    for (global, spell) in [
        ("UnitHasLFGDeserter", 71041u32),
        ("UnitHasLFGRandomCooldown", 71328),
    ] {
        set(lua, global, move |lua, args| {
            let Some(token) = (match arg(&args, 0) {
                v @ (Value::String(_) | Value::Integer(_) | Value::Number(_)) => {
                    lua.coerce_string(v)?.map(|s| s.to_string_lossy())
                }
                _ => None,
            }) else {
                return Err(mlua::Error::runtime(format!("Usage: {global}(\"unit\")")));
            };
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            if m.unit(&token).is_none_or(|u| !u.exists) {
                return Ok(MultiValue::new());
            }
            let has = crate::script::aura::auras_of(&m, &token)
                .ok()
                .flatten()
                .is_some_and(|list| list.iter().any(|a| a.spell_id == spell));
            Ok(MultiValue::from_vec(vec![Value::Boolean(has)]))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::WrathLfgQueueRequest as R;
    use super::*;
    use crate::script::{
        PartyMemberInfo, PartyState, UnitState, WrathGroup, WrathLfgCatalog, WrathLfgDungeon,
    };

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.register_cvars([("lfgSelectedRoles", "0")]);
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                exists: true,
                class_file: Some("WARRIOR".into()),
                ..Default::default()
            }),
        );
        s.model_mut().unit_guids.player = 1;
        s.set_wrath_lfg_catalog(
            Arc::new(WrathLfgCatalog {
                dungeons: vec![
                    WrathLfgDungeon {
                        id: 7,
                        name: "Halls".into(),
                        dungeon_type: 1,
                        map_id: 600,
                        difficulty: 1,
                        texture: "HALLS".into(),
                        ..Default::default()
                    },
                    WrathLfgDungeon {
                        id: 9,
                        name: "Raid".into(),
                        dungeon_type: 2,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            None,
        );
        s.set_wrath_lfg_encounters(Arc::new(vec![
            WrathLfgEncounter {
                map: 600,
                difficulty: 1,
                bit: 0,
                name: "Boss A".into(),
                icon: Some("IconA".into()),
            },
            WrathLfgEncounter {
                map: 600,
                difficulty: 1,
                bit: 3,
                name: "Boss B".into(),
                icon: None,
            },
        ]));
        s
    }

    fn snapshot() -> WrathLfgQueueSnapshot {
        WrathLfgQueueSnapshot {
            player: Some(WrathLfgUpdate {
                joined: true,
                queued: true,
                dungeons: vec![1 << 24 | 7, 2 << 24 | 9],
                comment: "hi".into(),
                ..Default::default()
            }),
            queue: Some(WrathLfgQueueStatus {
                dungeon: 1 << 24 | 7,
                avg_wait: 60,
                tank_wait: -1,
                healer_wait: 30,
                dps_wait: 90,
                my_wait: 45,
                tank_needs: 1,
                healer_needs: 0,
                dps_needs: 2,
                queued_at_ms: 5000,
            }),
            role_check: Some(WrathLfgRoleCheck {
                state: 2,
                dungeons: vec![1 << 24 | 7],
                members: vec![
                    WrathLfgRoleCheckMember {
                        guid: 1,
                        ready: true,
                        roles: 3,
                        level: 80,
                        name: Some("Me".into()),
                    },
                    WrathLfgRoleCheckMember {
                        guid: 2,
                        level: 79,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            proposal: Some(WrathLfgProposal {
                dungeon: 1 << 24 | 7,
                id: 44,
                completed_mask: 1 << 3,
                rows: vec![
                    WrathLfgProposalRow {
                        roles: 4,
                        ..Default::default()
                    },
                    WrathLfgProposalRow {
                        roles: 3,
                        is_self: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            boot: Some(WrathLfgBoot {
                in_progress: true,
                did_vote: true,
                my_vote: false,
                victim_name: Some("Bad".into()),
                victim_realm: Some("Elsewhere".into()),
                total_votes: 3,
                boot_votes: 2,
                time_left: 40,
                reason: "afk".into(),
                ..Default::default()
            }),
            completion: Some(WrathLfgCompletion {
                dungeon: 1 << 24 | 7,
                strangers: 2,
                money_base: 100,
                xp_base: 200,
                money_var: 10,
                xp_var: 20,
                items: vec![WrathLfgCompletionItem {
                    item: 5,
                    display: 6,
                    count: 3,
                }],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn getters_read_the_snapshot_in_native_shapes() {
        let mut s = vm();
        assert_eq!(
            s.eval::<u32>("return select('#', GetLFGQueueStats())")
                .unwrap(),
            0
        );
        assert!(s
            .eval::<bool>(
                "return GetLFGBootProposal()==false and select('#', GetLFGBootProposal())==1"
            )
            .unwrap());
        s.set_wrath_lfg_queue(Arc::new(snapshot()));
        for check in [
            "select('#', GetLFGQueueStats())==13",
            "select(1, GetLFGQueueStats())==true and select(2, GetLFGQueueStats())==0",
            "select(3, GetLFGQueueStats())==1 and select(5, GetLFGQueueStats())==2",
            "select(6, GetLFGQueueStats())==1 and select(7, GetLFGQueueStats())=='Halls'",
            "select(9, GetLFGQueueStats())==-1 and select(12, GetLFGQueueStats())==45",
            "select(13, GetLFGQueueStats())==5",
            "select('#', GetLFGInfoServer())==10 and select(1, GetLFGInfoServer())==false",
            "select(6, GetLFGInfoServer())=='hi' and select(7, GetLFGInfoServer())==2",
            "GetLFGQueuedList()[7]==true and GetLFGQueuedList()[9]==true",
            "IsListedInLFR()==true",
            "select(1, GetLFGRoleUpdate())==true and select(2, GetLFGRoleUpdate())==1 and select(3, GetLFGRoleUpdate())==2",
            "select(1, GetLFGRoleUpdateSlot(1))==1 and select(2, GetLFGRoleUpdateSlot(1))==7",
            "select('#', GetLFGRoleUpdateSlot(2))==0",
            "select('#', GetLFGRoleUpdateMember(1))==8 and select(3, GetLFGRoleUpdateMember(1))=='Me'",
            "select(5, GetLFGRoleUpdateMember(1))==true and select(6, GetLFGRoleUpdateMember(1))==true and select(7, GetLFGRoleUpdateMember(1))==false",
            "select(1, GetLFGRoleUpdateMember(2))==2 and select(2, GetLFGRoleUpdateMember(2))==false and select(3, GetLFGRoleUpdateMember(2))==79",
            "select('#', GetLFGProposal())==12 and select(3, GetLFGProposal())==7 and select(4, GetLFGProposal())=='Halls'",
            "select(6, GetLFGProposal())=='TANK' and select(8, GetLFGProposal())==2 and select(9, GetLFGProposal())==1",
            "select(10, GetLFGProposal())==5 and select(11, GetLFGProposal())==true",
            "select(1, GetLFGProposalMember(1))==false and select(2, GetLFGProposalMember(1))=='HEALER' and select('#', GetLFGProposalMember(1))==7",
            "select(2, GetLFGProposalMember(5))=='UNKNOWN' and select('#', GetLFGProposalMember(6))==0",
            "select(1, GetLFGProposalEncounter(1))=='Boss A' and select(3, GetLFGProposalEncounter(1))==false",
            "select(2, GetLFGProposalEncounter(2))==nil and select(3, GetLFGProposalEncounter(2))==true",
            "select('#', GetLFGProposalEncounter(3))==3 and GetLFGProposalEncounter(3)==nil",
            "select(4, GetLFGBootProposal())=='Bad-Elsewhere' and select(7, GetLFGBootProposal())==40 and select(8, GetLFGBootProposal())=='afk'",
            "select('#', GetLFGCompletionReward())==9 and select(4, GetLFGCompletionReward())==100 and select(5, GetLFGCompletionReward())==10",
            "select(6, GetLFGCompletionReward())==200 and select(8, GetLFGCompletionReward())==2 and select(9, GetLFGCompletionReward())==1",
            "select(2, GetLFGCompletionRewardItem(1))==3 and select('#', GetLFGCompletionRewardItem(2))==0",
        ] {
            assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
        }
        s.run("local p = CreateFrame('Frame') local t = CreateFrame('GameTooltip', 'LfgTip', p) t:SetOwner(p, 'ANCHOR_NONE') t:SetLFGCompletionReward(1) t:SetLFGCompletionReward(9) t:SetLFGDungeonReward(7, 1)").unwrap();
        assert!(
            s.run("LfgTip:SetLFGDungeonReward(7)").is_err()
                && s.run("LfgTip:SetLFGCompletionReward()").is_err()
        );
        for usage in [
            "GetLFGRoleUpdateSlot()",
            "GetLFGProposalEncounter('x')",
            "GetLFGCompletionRewardItem()",
            "GetLFGQueuedList(1)",
        ] {
            assert!(s.run(usage).is_err(), "{usage}");
        }
        // Grouped: the party record, which is absent.
        s.set_party(PartyState {
            members: vec![PartyMemberInfo {
                name: "Pal".into(),
                guid: 2,
            }],
            leader_guid: 1,
            own_guid: 1,
            ..Default::default()
        });
        assert!(s
            .eval::<bool>("return select('#', GetLFGQueueStats())==0 and select(1, GetLFGInfoServer())==true and IsListedInLFR()==false")
            .unwrap());
    }

    #[test]
    fn group_lfg_state_backfill_and_deserter() {
        let mut s = vm();
        assert!(s
            .eval::<bool>("return IsPartyLFG()==nil and select('#', IsInLFGDungeon())==0 and HasLFGRestrictions()==false and CanPartyLFGBackfill()==false")
            .unwrap());
        s.set_wrath_group(WrathGroup {
            lfg_dungeon_entry: 1 << 24 | 7,
            lfg_status: 2,
            lfg_restricted: true,
            ..Default::default()
        });
        s.set_wrath_lfg_location(Some((600, 1)));
        assert!(s
            .eval::<bool>("local n, id, t = GetPartyLFGBackfillInfo() return IsPartyLFG()==1 and IsInLFGDungeon()==true and HasLFGRestrictions()==true and CanPartyLFGBackfill()==true and n=='Halls' and id==7 and t==1")
            .unwrap());
        s.set_wrath_lfg_location(Some((601, 1)));
        assert!(s.eval::<bool>("return IsInLFGDungeon()==false").unwrap());
        s.model_mut().player_auras = vec![crate::script::AuraState {
            spell_id: 71041,
            ..Default::default()
        }];
        assert!(s
            .eval::<bool>("return UnitHasLFGDeserter('player')==true and UnitHasLFGRandomCooldown('player')==false and select('#', UnitHasLFGDeserter('party1'))==0")
            .unwrap());
        assert!(s.run("UnitHasLFGDeserter()").is_err());
    }

    #[test]
    fn selection_roles_and_verbs_follow_the_native_gates() {
        let mut s = vm();
        s.set_now(100.0);
        s.run("JoinLFG()").unwrap();
        s.run("SetLFGDungeon(7); SetLFGDungeon(7); SetLFGDungeon(99); JoinLFG()")
            .unwrap();
        assert_eq!(
            s.take_ui_errors(),
            vec!["ERR_LFG_NO_SLOTS_SELECTED", "ERR_LFG_NO_ROLES_SELECTED"]
        );
        // A warrior cannot heal: the healer bit is masked off.
        s.run("SetLFGRoles(true, true, true, false); local l,t,h,d = GetLFGRoles(); assert(l and t and not h and not d)").unwrap();
        s.run("SetLFGComment('lfm'); JoinLFG(); JoinLFG(); SetLFGComment('new'); LeaveLFG(); LeaveLFG()").unwrap();
        assert_eq!(
            s.take_wrath_lfg_queue_requests(),
            vec![
                R::Join {
                    roles: 3,
                    no_partial_clear: false,
                    achievements: false,
                    dungeons: vec![1 << 24 | 7],
                    needs: [0; 3],
                    comment: "lfm".into(),
                },
                R::SetComment { text: "new".into() },
                R::Leave,
            ]
        );
        s.run("ClearAllLFGDungeons(); JoinLFG()").unwrap();
        assert_eq!(s.take_ui_errors(), vec!["ERR_LFG_NO_SLOTS_SELECTED"]);

        // Role check: the third call opens a window (its stamp starts at 0); two more pass, then
        // nothing until ten seconds after that.
        s.run("assert(CompleteLFGRoleCheck(true)==true); assert(CompleteLFGRoleCheck(false)==true); for i = 1, 3 do assert(CompleteLFGRoleCheck(true)==true) end; assert(CompleteLFGRoleCheck(true)==nil)").unwrap();
        s.set_now(109.0);
        s.run("assert(CompleteLFGRoleCheck(true)==nil)").unwrap();
        s.set_now(110.0);
        s.run("assert(CompleteLFGRoleCheck(true)==true)").unwrap();
        let answers = s.take_wrath_lfg_queue_requests();
        assert_eq!(answers.len(), 6);
        assert_eq!(answers[1], R::RoleCheckAnswer { roles: 0 });
        assert_eq!(answers[5], R::RoleCheckAnswer { roles: 3 });

        // Proposal: accept marks our row; reject drops it.
        s.set_wrath_lfg_queue(Arc::new(snapshot()));
        s.run("AcceptProposal(); assert(select(7, GetLFGProposal())==true); RejectProposal(); assert(select('#', GetLFGProposal())==0); AcceptProposal()").unwrap();
        assert_eq!(
            s.take_wrath_lfg_queue_requests(),
            vec![
                R::Proposal {
                    id: 44,
                    accept: true
                },
                R::Proposal {
                    id: 44,
                    accept: false
                },
            ]
        );

        // Teleport: refused in combat with the native error.
        s.run("assert(LFGTeleport(true)==true)").unwrap();
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                exists: true,
                in_combat: true,
                ..Default::default()
            }),
        );
        s.run("assert(LFGTeleport(false)==false)").unwrap();
        assert_eq!(s.take_ui_errors(), vec!["ERR_NOT_IN_COMBAT"]);
        assert_eq!(
            s.take_wrath_lfg_queue_requests(),
            vec![R::Teleport { out: true }]
        );

        // Backfill: the group's dungeon becomes the selection and the join goes out again.
        s.set_wrath_group(WrathGroup {
            lfg_dungeon_entry: 1 << 24 | 7,
            ..Default::default()
        });
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                exists: true,
                class_file: Some("WARRIOR".into()),
                ..Default::default()
            }),
        );
        s.set_now(200.0);
        s.run("SetLFGDungeon(9) PartyLFGStartBackfill()").unwrap();
        match s.take_wrath_lfg_queue_requests().as_slice() {
            [R::Join {
                dungeons, roles, ..
            }] => assert_eq!((dungeons.clone(), *roles), (vec![1 << 24 | 7], 3)),
            other => panic!("{other:?}"),
        }
    }
}
