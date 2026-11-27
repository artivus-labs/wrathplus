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

use super::requests::Gate;
use super::{Model, UiScript};
use crate::script::binding_abi::{bool_or_default, number_arg, tointeger_arg};

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
    /// `0xacd883`: the selection or the comment changed, so the next join resends.
    pending: bool,
    /// `0xbea860`, written by `SetLFGBootVote` until a boot update replaces it.
    my_vote: Option<bool>,
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
        // Only a boot packet replaces `0xbea860`: a republish that resolves a name keeps it.
        let packet = |b: &WrathLfgBoot| WrathLfgBoot {
            victim_name: None,
            victim_realm: None,
            ..b.clone()
        };
        if queue.snapshot.boot.as_ref().map(packet) != snapshot.boot.as_ref().map(packet) {
            queue.my_vote = None;
        }
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

fn now() -> u32 {
    super::requests::now_ms()
}

/// The verb throttles (`0xacdd50`-`0xacdd6c`), each seeded 2 and on the OS clock: process globals,
/// as `requests.rs`'s lock gates are, so a `/reload` keeps them.
struct Throttles {
    /// `0xacdd6c`.
    join: Gate,
    /// `0xacdd50`, 2 s (`0xa4040c`).
    comment: Gate,
    /// `0xacdd54`.
    role_check: Gate,
    /// `0xacdd58`.
    answer: Gate,
    /// `0xacdd68`.
    teleport: Gate,
    /// `0xacdd5c`.
    boot: Gate,
}

static THROTTLES: std::sync::Mutex<Throttles> = std::sync::Mutex::new(Throttles::INITIAL);

impl Throttles {
    const INITIAL: Self = Self {
        join: Gate::INITIAL,
        comment: Gate::INITIAL,
        role_check: Gate::INITIAL,
        answer: Gate::INITIAL,
        teleport: Gate::INITIAL,
        boot: Gate::INITIAL,
    };
}

/// One call through the throttle `pick` names at OS time `t`, under `window` seconds.
fn throttle(pick: fn(&mut Throttles) -> &mut Gate, t: u32, window: f64) -> bool {
    let mut all = THROTTLES.lock().unwrap_or_else(|e| e.into_inner());
    pick(&mut all).request_within(t, window)
}

/// Hold the throttles for one test, seeded afresh: the tests share the statics in parallel.
#[cfg(test)]
fn throttle_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    edit_throttles(|t| *t = Throttles::INITIAL);
    held
}

#[cfg(test)]
fn edit_throttles(f: impl FnOnce(&mut Throttles)) {
    f(&mut THROTTLES.lock().unwrap_or_else(|e| e.into_inner()));
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

/// `0x559b70`: a dungeon or heroic (types 1, 5) keeps only dungeons and heroics, a raid (2) only
/// raids and only outside a party (`0x52bd10`), any other type nothing; a clear (`0x559340`) or a
/// new entry (`0x559c35`) marks the selection for the next join to resend.
fn select(q: &mut Queue, entry: u32, in_party: bool) {
    let all = |types: &[u32]| q.selection.iter().all(|e| types.contains(&(e >> 24)));
    let clear = match entry >> 24 {
        1 | 5 => !all(&[1, 5]),
        2 => in_party || !all(&[2]),
        _ => true,
    };
    if clear {
        q.selection.clear();
        q.pending = true;
    }
    if !q.selection.contains(&entry) {
        q.selection.push(entry);
        q.pending = true;
    }
}

/// The native join send (`0x5572d0`), behind its throttle; nothing without a role past leader.
fn send_join(lua: &Lua) {
    let t = now();
    let mut m = lua.app_data_mut::<Model>().expect("model app_data");
    let Some(roles) = roles_cvar(&m) else { return };
    if m.wrath_lfg.queue.sent && !m.wrath_lfg.queue.pending {
        return;
    }
    if !throttle(|g| &mut g.join, t, 10.0) {
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

/// The backfill gate (`0x553170`, `0x5556d0`, `0x55a0c0`): 1..3 party members (`0xbd1998`, the
/// count GetNumPartyMembers reads at `0x52c190`), an LFG dungeon, and the LFG byte without bit 2.
fn can_backfill(m: &Model) -> bool {
    let g = &m.wrath_group;
    (1..=3).contains(&m.party.members.len()) && g.lfg_dungeon_entry != 0 && g.lfg_flags & 2 == 0
}

/// `PartyLFGStartBackfill` (`0x55a0c0`): no role past leader raises ERR_LFG_NO_ROLES_SELECTED; past
/// the backfill gate the selection becomes the group's dungeon and the join send runs. True when it
/// ran, whatever the send did.
pub(crate) fn start_backfill(lua: &Lua) -> bool {
    {
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        // The role test reads the whole int (`& 0xfffffffe`); the join send masks a byte.
        let roles = m.cvars.get("lfgselectedroles");
        if roles.is_some_and(|c| c.value.trim().parse::<i64>().unwrap_or(0) & !1 != 0) {
            if !can_backfill(&m) {
                return false;
            }
            let entry = m.wrath_group.lfg_dungeon_entry;
            let in_party = !m.party.members.is_empty();
            let q = &mut m.wrath_lfg.queue;
            q.selection.clear();
            select(q, entry, in_party);
        } else {
            m.ui_errors.push("ERR_LFG_NO_ROLES_SELECTED");
            return false;
        }
    }
    send_join(lua);
    true
}

/// `SetLFGBootVote` (`0x554960`): nothing without a vote in progress (`0xbea858`), then the 10 s
/// throttle (`0xacdd5c`); a vote that passes is our answer at once (`0xbea860`). True sends it.
pub(crate) fn boot_vote(lua: &Lua, yes: bool) -> bool {
    let mut m = lua.app_data_mut::<Model>().expect("model app_data");
    let q = &mut m.wrath_lfg.queue;
    if !q.snapshot.boot.as_ref().is_some_and(|b| b.in_progress)
        || !throttle(|g| &mut g.boot, now(), 10.0)
    {
        return false;
    }
    q.my_vote = Some(yes);
    true
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
    // GetLFGQueuedList([table]) (`0x557520`) → the table (cleared, or new for nil or nothing,
    // `0x557538`-`0x557545`) keyed by the record's dungeon ids; the usage line names it GetLFD.
    set(lua, "GetLFGQueuedList", |lua, args| {
        let table = match arg(&args, 0) {
            Value::Table(t) => {
                t.clear()?;
                t
            }
            Value::Nil => lua.create_table()?,
            _ => return Err(mlua::Error::runtime("Usage: GetLFDQueuedList([table])")),
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
    // GetLFGRoleUpdateMember(index) (`0x554750`) → ready, level, name and class (nil, nil when the
    // name is not cached), then leader/tank/healer/damage for a ready member, four nils otherwise
    // (`0x5548b5`): eight values.
    set(lua, "GetLFGRoleUpdateMember", |lua, args| {
        let i = number_arg(lua, arg(&args, 0), "Usage: GetLFGRoleUpdateMember(index)")?;
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
                // The name-cache record's class (`0x67d770`, `0x72aab0` → `0x7159e0`), "" with no
                // row. Labelled: the record holds one class name, not ChrClasses' gendered pair,
                // and a guid it lacks (our own, never queried) reads the member's unit.
                let class = match m.player_names.get(&member.guid) {
                    Some(r) => r.class.as_ref().map(|c| c.0.as_str()),
                    None => m.unit_by_guid(member.guid).and_then(|u| u.class.as_deref()),
                };
                out.push(text(lua, class.unwrap_or_default())?);
            }
            None => out.extend([Value::Nil, Value::Nil]),
        }
        let roles = [1u8, 2, 4, 8].map(|b| Value::Boolean(member.roles & b != 0));
        out.extend(roles.map(|v| if member.ready { v } else { Value::Nil }));
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
        let q = &m.wrath_lfg.queue;
        let Some(b) = q.snapshot.boot.as_ref().filter(|b| b.in_progress) else {
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
            Value::Boolean(q.my_vote.unwrap_or(b.my_vote)),
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
    // GetLFGCompletionRewardItem(index) (`0x557f70`, rounding `0x557fa0`) → icon path and count.
    set(lua, "GetLFGCompletionRewardItem", |lua, args| {
        let i = tointeger_arg(
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
    // type), behind [`can_backfill`].
    fn backfill(m: &Model) -> Option<&super::WrathLfgDungeon> {
        can_backfill(m)
            .then(|| row(m, m.wrath_group.lfg_dungeon_entry))
            .flatten()
    }
    set(lua, "CanPartyLFGBackfill", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(MultiValue::from_vec(vec![Value::Boolean(can_backfill(&m))]))
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
    // entry, under `select`'s rules.
    set(lua, "SetLFGDungeon", |lua, args| {
        let id = number_arg(lua, arg(&args, 0), "Usage: SetLFGDungeon(type)")? as u32;
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        if let Some(entry) = m.wrath_lfg.row(id).map(super::l1::full_entry) {
            let in_party = !m.party.members.is_empty();
            select(&mut m.wrath_lfg.queue, entry, in_party);
        }
        Ok(MultiValue::new())
    })?;
    // ClearAllLFGDungeons() (`0x5595d0` → `0x559340`): empties the selection and marks it for
    // resend (`0x559388`).
    set(lua, "ClearAllLFGDungeons", |lua, _| {
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        let q = &mut m.wrath_lfg.queue;
        q.selection.clear();
        q.pending = true;
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
    // SetLFGComment([text]) (`0x553f50` → `0x5539a0`): sanitised, stored; sent only after a join,
    // else marked for the next join to resend.
    set(lua, "SetLFGComment", |lua, args| {
        let comment = match arg(&args, 0) {
            v @ (Value::String(_) | Value::Integer(_) | Value::Number(_)) => lua
                .coerce_string(v)?
                .map(|s| s.to_string_lossy())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let comment = super::super::wrath_chat_verbs::sanitised_line(&comment);
        let t = now();
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        let q = &mut m.wrath_lfg.queue;
        q.comment = comment.clone();
        if !q.sent {
            q.pending = true;
        } else if throttle(|g| &mut g.comment, t, 2.0) {
            q.requests.push(R::SetComment { text: comment });
        }
        Ok(MultiValue::new())
    })?;

    // ClearLFGDungeon(type, id) (`0x559500`): raises only when neither argument is a number; both
    // truncate, a type under 7 and a catalog id name an entry (`type << 24 | id`), and
    // `0x5593a0` drops it, marking the selection for resend.
    set(lua, "ClearLFGDungeon", |lua, args| {
        let num = |i| lua.coerce_number(arg(&args, i)).ok().flatten();
        let (kind, id) = (num(0), num(1));
        if kind.is_none() && id.is_none() {
            return Err(mlua::Error::runtime("Usage: ClearLFGDungeon(type, id)"));
        }
        let [kind, id] = [kind, id].map(|n| n.map_or(0, |n| n as i64 as i32) as u32);
        let mut m = lua.app_data_mut::<Model>().expect("model app_data");
        if kind < 7 && m.wrath_lfg.row(id).is_some() {
            let q = &mut m.wrath_lfg.queue;
            let entry = kind << 24 | id & 0xff_ffff;
            if let Some(at) = q.selection.iter().position(|&e| e == entry) {
                q.selection.remove(at);
                q.pending = true;
            }
        }
        Ok(MultiValue::new())
    })?;
    // GetLFGInfoLocal() (`0x552e90`) → join sent, the two flag bytes, the comment, the selection
    // count and the three needs bytes; the flags and needs have no 3.3.5 Lua setter.
    set(lua, "GetLFGInfoLocal", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let q = &m.wrath_lfg.queue;
        let mut out = vec![
            Value::Boolean(q.sent),
            Value::Boolean(false),
            Value::Boolean(false),
        ];
        out.extend([text(lua, &q.comment)?, int(q.selection.len() as i64)]);
        out.extend([int(0), int(0), int(0)]);
        Ok(MultiValue::from_vec(out))
    })?;
    // GetLFGTypes() (`0x552dd0`) → the seven type names, each key's global string (`0x819d40`).
    set(lua, "GetLFGTypes", |lua, _| {
        let keys = [
            "LFG_TYPE_NONE",
            "LFG_TYPE_DUNGEON",
            "LFG_TYPE_RAID",
            "LFG_TYPE_QUEST",
            "LFG_TYPE_ZONE",
            "LFG_TYPE_HEROIC_DUNGEON",
            "LFG_TYPE_RANDOM_DUNGEON",
        ];
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            out.push(match lua.globals().get::<Value>(key)? {
                v @ Value::String(_) => v,
                _ => text(lua, key)?,
            });
        }
        Ok(MultiValue::from_vec(out))
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
        let t = now();
        let accept = bool_or_default(args.front(), false);
        let roles = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            if !throttle(|g| &mut g.role_check, t, 10.0) {
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
            let t = now();
            {
                let mut m = lua.app_data_mut::<Model>().expect("model app_data");
                let q = &mut m.wrath_lfg.queue;
                let Some(id) = q.proposal.as_ref().filter(|p| p.dungeon != 0).map(|p| p.id) else {
                    return Ok(MultiValue::new());
                };
                if !throttle(|g| &mut g.answer, t, 10.0) {
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
    // `CombatTest::Flag` is `AcceptBattlefieldPort`'s (`0x54dafc`), not yet on these gates.
    const _: crate::script::wrath_port_gates::CombatTest =
        crate::script::wrath_port_gates::CombatTest::Flag;
    // LFGTeleport(out) (`0x556bc0`) → true when sent: a player, the throttle, then the port
    // refusals with `0x71afb0`'s combat test, each an error line and false.
    set(lua, "LFGTeleport", |lua, args| {
        use crate::script::wrath_port_gates::{CombatTest, PortGates};
        let t = now();
        let refusal = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            if m.unit("player").is_none_or(|p| !p.exists) {
                return Ok(MultiValue::new());
            }
            if !throttle(|g| &mut g.teleport, t, 10.0) {
                return Ok(MultiValue::from_vec(vec![Value::Boolean(false)]));
            }
            let mut gates = PortGates::of(&m);
            // Inferred: `[obj+0xa20]` is the swing target stored after CMSG_ATTACKSWING
            // (`0x6e2818`), so Attack (6603) current stands for it.
            gates.attacking = m.current_spells.contains(&6603);
            gates.refusal(CombatTest::FlagOrAttacking)
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

    // ── the reward tooltips (`0x630a80`/`0x630b90`: their numbers round through `lua_tointeger`,
    // `0x630ad8`, `0x630be7`; labelled: the rest is unread, they show the item
    // GetLFGDungeonRewardInfo / GetLFGCompletionRewardItem name through the id-keyed item render) ──
    let tooltip: mlua::Table =
        lua.named_registry_value(crate::script::tooltip::REG_TOOLTIP_METHODS)?;
    tooltip.set(
        "SetLFGDungeonReward",
        lua.create_function(|lua, (this, id, index): (mlua::Table, Value, Value)| {
            const USAGE: &str = "Usage: Tooltip:SetLFGDungeonReward(dungeonID, lootIndex)";
            let id = tointeger_arg(lua, id, USAGE)?;
            let index = tointeger_arg(lua, index, USAGE)?;
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
            let index = tointeger_arg(
                lua,
                index,
                "Usage: Tooltip:SetLFGCompletionReward(lootIndex)",
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
#[path = "queue_tests.rs"]
mod tests;
