//! The battleground queue, list and world-PvP verbs of build 12340's `PVPBattlegroundFrame.lua`,
//! `BattlefieldFrame.lua`, `MiniMapBattlefieldFrame` and `WorldStateFrame.lua`.
//!
//! The reference keeps two queue slots (`0xbea4b8`, `0x40` bytes each, written by the
//! `SMSG_BATTLEFIELD_STATUS` handler `0x54ae40`), the listed battleground type and its instance
//! list (`0xbea53c`, `0xbea5c4`, written by `SMSG_BATTLEFIELD_LIST` `0x54e390`), the selected
//! instance (`0xbea538`), the battleground-type list (`0xbea728`, the `BattlemasterList` rows of
//! instance type 3, sorted at start by `0x54a650` and again by `SortBGList`), and the world-PvP
//! queue (`0xbea4a8..b4`, written by the battlefield-manager handlers through `0x549500`). The app
//! pushes those, with every DBC and world-state value resolved; the VM owns the selection, the
//! type order and the requests.
//!
//! Every index argument is `lua_isnumber`-gated and rounded to nearest (`0x84e030` + `ROUND`).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{bool_or_default, flag};
use super::Model;

/// One queue slot as the getters read it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlefieldQueueSlot {
    /// `+0x18`: `0` none, `1` queued, `2` confirm, `3` active; anything else answers `"error"`.
    pub status: u32,
    /// `+0x10`, the `BattlemasterList` type.
    pub bg_type: u32,
    /// The name `GetBattlefieldStatus` answers: the map's for an invite or an active slot, else
    /// the type's (`0x54bc30`); `None` is a real `nil`.
    pub name: Option<String>,
    /// `+0x24`, the client instance id.
    pub instance_id: u32,
    /// `+0x1c`/`+0x20`.
    pub min_level: u32,
    pub max_level: u32,
    /// `+0x34`: the arena type's low seven bits, the team size; 0 for a battleground.
    pub team_size: u32,
    /// `+0x38`, the rated byte.
    pub rated: bool,
    /// `GetBattlefieldPortExpiration`: whole seconds left on the invite, 0 when unset or past.
    pub port_expiration_secs: u32,
    /// `GetBattlefieldEstimatedWaitTime`: the stored average wait, ms.
    pub estimated_wait_ms: u32,
    /// `GetBattlefieldTimeWaited`: ms since the queue stamp, 0 when unset.
    pub time_waited_ms: u32,
}

/// A `BattlemasterList` type as the type and list getters read it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlegroundTypeView {
    pub id: u32,
    pub name: String,
    /// 3 battleground, 4 arena.
    pub instance_type: u32,
    pub groups_allowed: bool,
    pub max_group_size: u32,
    pub min_level: u32,
    /// The player's level falls in one of the type's brackets (`0x54ba00`).
    pub can_enter: bool,
    /// The type's holiday world state is non-zero.
    pub is_holiday: bool,
    /// The type's first map's description for the player's side; `None` when the side resolves
    /// to neither (`0x6d6e90` answers -1).
    pub description: Option<String>,
    /// The type's first map has a `Map.dbc` row; `GetBattlefieldInfo` answers nothing without.
    pub has_map: bool,
}

/// One list's reward block (`0xbea548..` holiday, `0xbea549..` random).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BattlegroundRewardsView {
    pub has_win: bool,
    pub win_honor: u32,
    pub win_arena: u32,
    pub loss_honor: u32,
}

/// The last `SMSG_BATTLEFIELD_LIST`, as the app pushes it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlefieldListView {
    /// `0xbea53c`; 0 before any list.
    pub bg_type: u32,
    /// The instance ids in wire order (`0xbea5c4`).
    pub instances: Vec<u32>,
    pub holiday: BattlegroundRewardsView,
    pub random: BattlegroundRewardsView,
}

/// The world-PvP (Wintergrasp) queue and the world states its getters read.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct WorldPvpView {
    /// `0xbea4b0`, the queue status.
    pub status: u32,
    /// `0xbea4ac`'s `AreaTable` name.
    pub zone_name: Option<String>,
    /// `0xbea4a8`, the battle id.
    pub battle_id: u32,
    /// `0xbea4b4`, the invite's server-time deadline (unix seconds).
    pub deadline: u32,
    /// World state `0x1117`: Wintergrasp takes queue requests.
    pub can_queue_wintergrasp: bool,
    /// World state `0xed9`: Wintergrasp's wait clock runs.
    pub wintergrasp_waiting: bool,
    /// World state `0x1102`: the next battle's server time.
    pub wintergrasp_next: i32,
    /// World states `0xc77` and `0xf3d`.
    pub arena_season: i32,
    pub previous_arena_season: i32,
    /// The player's subzone's `AreaTable` flags have `0x8000`.
    pub subzone_pvp_poi: bool,
}

/// The VM's half: the type order, the selection and the requests the app drains.
#[derive(Clone, Debug, Default)]
pub(crate) struct QueueState {
    pub(crate) slots: [BattlefieldQueueSlot; 2],
    /// Every `BattlemasterList` type, by id.
    pub(crate) types: Vec<BattlegroundTypeView>,
    /// The battleground types (instance type 3) in the client's order.
    pub(crate) order: Vec<u32>,
    pub(crate) list: BattlefieldListView,
    pub(crate) selected: u32,
    pub(crate) world_pvp: WorldPvpView,
    /// The active slot's map is an arena, and the active slot is rated (`0x549ad0`).
    pub(crate) active_arena: bool,
    pub(crate) active_rated: bool,
    pub(crate) instance_expiration_ms: u32,
    pub(crate) requests: Vec<BattlefieldRequest>,
}

/// What the bindings ask of the app, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BattlefieldRequest {
    /// `RequestBattlegroundInstanceInfo`: `CMSG_BATTLEFIELD_LIST` for the type (`0x54e5b0`).
    List { bg_type: u32 },
    /// `JoinBattlefield(index, asGroup, isRated)`, the raw 0-based index (`0x54cc70`).
    Join {
        index: i64,
        as_group: bool,
        rated: bool,
    },
    /// `AcceptBattlefieldPort(index, accept)`, 0-based slot.
    Port { slot: usize, accept: bool },
    /// `BattlefieldMgrQueueRequest(id)`.
    BfQueueRequest { battle_id: u32 },
    /// `BattlefieldMgrExitRequest(id)`.
    BfExit { battle_id: u32 },
    /// `CloseBattlefield()`: the battlemaster clears (`0x549b40`).
    Close,
}

/// The reference's status jump table with its `"error"` default.
fn status_text(status: u32) -> &'static str {
    match status {
        0 => "none",
        1 => "queued",
        2 => "confirm",
        3 => "active",
        _ => "error",
    }
}

/// `lua_isnumber` then the rounded value; `None` for a non-number.
fn number(lua: &Lua, v: &Value) -> mlua::Result<Option<i64>> {
    Ok(lua
        .coerce_number(v.clone())?
        .map(|n| n.round_ties_even() as i64))
}

fn index_arg(lua: &Lua, v: &Value, usage: &'static str) -> mlua::Result<i64> {
    number(lua, v)?.ok_or_else(|| mlua::Error::RuntimeError(usage.into()))
}

/// A 1-based index under `n`, unsigned as the reference compares it.
fn slot_index(index: i64, n: usize) -> Option<usize> {
    usize::try_from(index.checked_sub(1)?)
        .ok()
        .filter(|&i| i < n)
}

fn player_exists(model: &Model) -> bool {
    model.unit("player").is_some_and(|u| u.exists)
}

fn text(lua: &Lua, s: &Option<String>) -> mlua::Result<Value> {
    Ok(match s {
        Some(t) => Value::String(lua.create_string(t)?),
        None => Value::Nil,
    })
}

fn push(lua: &Lua, request: BattlefieldRequest) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .battlefield
        .requests
        .push(request);
}

impl QueueState {
    fn ty(&self, id: u32) -> Option<&BattlegroundTypeView> {
        self.types.iter().find(|t| t.id == id)
    }

    /// `0x54a650`: the random type first, then a holiday type, then by minimum level, then id.
    fn sort_order(&mut self) {
        let mut order = std::mem::take(&mut self.order);
        order.sort_by(|&a, &b| {
            use std::cmp::Ordering;
            let random = random_first(a, b);
            if random != Ordering::Equal {
                return random;
            }
            let (ta, tb) = (self.ty(a), self.ty(b));
            let holiday = |t: Option<&BattlegroundTypeView>| t.is_some_and(|t| t.is_holiday);
            if holiday(ta) {
                return Ordering::Less;
            }
            if holiday(tb) {
                return Ordering::Greater;
            }
            let level = |t: Option<&BattlegroundTypeView>| t.map_or(0, |t| t.min_level);
            level(ta).cmp(&level(tb)).then(a.cmp(&b))
        });
        self.order = order;
    }
}

/// The random type sorts before every other (`cmp 0x20`), checked on the left first.
fn random_first(a: u32, b: u32) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    const RANDOM: u32 = 32;
    if a == RANDOM && b != RANDOM {
        Ordering::Less
    } else if b == RANDOM && a != RANDOM {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

impl super::UiScript {
    /// The two queue slots, the instance expiration (ms) and the active-arena pair, every frame.
    pub fn set_battlefield_queue(
        &mut self,
        slots: [BattlefieldQueueSlot; 2],
        instance_expiration_ms: u32,
        active_arena: bool,
        active_rated: bool,
    ) {
        let q = &mut self.model_mut().battlefield;
        q.slots = slots;
        q.instance_expiration_ms = instance_expiration_ms;
        q.active_arena = active_arena;
        q.active_rated = active_rated;
    }

    /// Every `BattlemasterList` type. The first push adopts the battleground types' table order
    /// and sorts it (`0x54e720`, at a start with no holiday states); later pushes keep the VM's
    /// order, appending any type new to it.
    pub fn set_battleground_types(&mut self, types: Vec<BattlegroundTypeView>) {
        let q = &mut self.model_mut().battlefield;
        let first = q.order.is_empty();
        for t in types.iter().filter(|t| t.instance_type == 3) {
            if !q.order.contains(&t.id) {
                q.order.push(t.id);
            }
        }
        q.order
            .retain(|id| types.iter().any(|t| t.id == *id && t.instance_type == 3));
        let holidays: Vec<bool> = types.iter().map(|t| t.is_holiday).collect();
        q.types = types;
        if first {
            // The start-up sort sees no world states, so no type is on holiday yet.
            for t in &mut q.types {
                t.is_holiday = false;
            }
            q.sort_order();
            for (t, h) in q.types.iter_mut().zip(holidays) {
                t.is_holiday = h;
            }
        }
    }

    /// The last list. A new list leaves the selection alone; it only decides whether
    /// `GetSelectedBattlefield` still finds it.
    pub fn set_battlefield_list(&mut self, list: BattlefieldListView) {
        self.model_mut().battlefield.list = list;
    }

    pub fn set_world_pvp(&mut self, view: WorldPvpView) {
        self.model_mut().battlefield.world_pvp = view;
    }

    /// The world-enter reset (`0x54ca50`): the listed type and the selection clear.
    pub fn reset_battlefield_selection(&mut self) {
        let q = &mut self.model_mut().battlefield;
        q.selected = 0;
        q.list.bg_type = 0;
    }

    /// The bindings' requests since the last drain, in call order.
    pub fn take_battlefield_requests(&mut self) -> Vec<BattlefieldRequest> {
        std::mem::take(&mut self.model_mut().battlefield.requests)
    }
}

/// The confirm countdown's server clock, `0x548700`; `None` until the first time reply.
fn server_now(model: &Model) -> Option<i64> {
    super::wrath_server_time::server_now(model)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetNumBattlefields",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.battlefield.list.instances.len() as i64)
        })?,
    )?;

    // name, description, maxGroup, canEnter, isHoliday, isRandom for the listed type; nothing
    // without a player, a type row or its map's row (`0x54d770`). Deviation: a side resolving to
    // neither answers a nil description where the reference pushes none and reads a stack slot.
    g.set(
        "GetBattlefieldInfo",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let q = &model.battlefield;
            let Some(t) = q
                .ty(q.list.bg_type)
                .filter(|t| t.has_map && player_exists(&model))
            else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&t.name)?),
                text(lua, &t.description)?,
                Value::Integer(i64::from(t.max_group_size)),
                flag(t.can_enter),
                flag(t.is_holiday),
                flag(t.id == 32),
            ]))
        })?,
    )?;

    // The instance id at that position; nothing without a player or out of range. The usage
    // string is the reference's own mislabel.
    g.set(
        "GetBattlefieldInstanceInfo",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetBattlefieldInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let list = &model.battlefield.list;
            if !player_exists(&model) {
                return Ok(MultiValue::new());
            }
            Ok(match slot_index(index, list.instances.len()) {
                Some(i) => {
                    MultiValue::from_vec(vec![Value::Integer(i64::from(list.instances[i] as i32))])
                }
                None => MultiValue::new(),
            })
        })?,
    )?;

    // JoinBattlefield(index[, asGroup[, isRated]]): the app resolves the instance or the arena
    // slot and the group-size refusals (`0x54cc70`).
    g.set(
        "JoinBattlefield",
        lua.create_function(|lua, args: MultiValue| {
            let index = index_arg(
                lua,
                args.front().unwrap_or(&Value::Nil),
                "Usage: JoinBattlefield(index)",
            )?;
            push(
                lua,
                BattlefieldRequest::Join {
                    index: index - 1,
                    as_group: bool_or_default(args.get(1), false),
                    rated: bool_or_default(args.get(2), false),
                },
            );
            Ok(())
        })?,
    )?;

    // Closes the list window: `BATTLEFIELDS_CLOSED` at once and the battlemaster cleared
    // (`0x549b40`), here on the next dispatch.
    g.set(
        "CloseBattlefield",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .pending_events
                .push(("BATTLEFIELDS_CLOSED".into(), Vec::new()));
            model.battlefield.requests.push(BattlefieldRequest::Close);
            Ok(())
        })?,
    )?;

    g.set(
        "SetSelectedBattlefield",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: SetSelectedBattlefield(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let q = &mut model.battlefield;
            q.selected =
                slot_index(index, q.list.instances.len()).map_or(0, |i| q.list.instances[i]);
            Ok(())
        })?,
    )?;

    g.set(
        "GetSelectedBattlefield",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let q = &model.battlefield;
            Ok(q.list
                .instances
                .iter()
                .position(|&id| id == q.selected)
                .map_or(0, |i| i as i64 + 1))
        })?,
    )?;

    // status, name, instanceID, minLevel, maxLevel, teamSize, registeredMatch: seven values on
    // every leg, `(nil, nil, 0, 0, 0, 0, nil)` off slots 1-2 (`0x54bc30`).
    g.set(
        "GetBattlefieldStatus",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetBattlefieldStatus(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let z = || Value::Integer(0);
            let Some(slot) = slot_index(index, 2).map(|i| &model.battlefield.slots[i]) else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    z(),
                    z(),
                    z(),
                    z(),
                    Value::Nil,
                ]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(status_text(slot.status))?),
                text(lua, &slot.name)?,
                Value::Integer(i64::from(slot.instance_id)),
                Value::Integer(i64::from(slot.min_level)),
                Value::Integer(i64::from(slot.max_level)),
                Value::Integer(i64::from(slot.team_size)),
                flag(slot.rated),
            ]))
        })?,
    )?;

    // The per-slot clocks (`0x549b80` seconds, `0x549d30` and `0x549dd0` ms); 0 off slots 1-2.
    for (name, usage, read) in [
        (
            "GetBattlefieldPortExpiration",
            "Usage: GetBattlefieldPortExpiration(index)",
            (|s: &BattlefieldQueueSlot| s.port_expiration_secs) as fn(&BattlefieldQueueSlot) -> u32,
        ),
        (
            "GetBattlefieldEstimatedWaitTime",
            "Usage: GetBattlefieldEstimatedWaitTime(index)",
            |s: &BattlefieldQueueSlot| s.estimated_wait_ms,
        ),
        (
            "GetBattlefieldTimeWaited",
            "Usage: GetBattlefieldTimeWaited(index)",
            |s: &BattlefieldQueueSlot| s.time_waited_ms,
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, index: Value| {
                let index = index_arg(lua, &index, usage)?;
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                Ok(i64::from(
                    slot_index(index, 2).map_or(0, |i| read(&model.battlefield.slots[i])),
                ))
            })?,
        )?;
    }

    g.set(
        "GetBattlefieldInstanceExpiration",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.battlefield.instance_expiration_ms))
        })?,
    )?;

    // AcceptBattlefieldPort(index, accept): nil off slots 1-2, else 1 and the app sends
    // `CMSG_BATTLEFIELD_PORT` with the slot's queue id. An accept with a player outside a
    // battleground (`0x54a700`: the map's `InstanceType` 3) first runs the port refusals, each a
    // catalog error and nil with nothing sent. A decline of the listed type also closes the list
    // window (`0x54dc3e`).
    g.set(
        "AcceptBattlefieldPort",
        lua.create_function(|lua, args: MultiValue| {
            let index = index_arg(
                lua,
                args.front().unwrap_or(&Value::Nil),
                "Usage: AcceptBattlefieldPort(index, accept)",
            )?;
            let Some(slot) = slot_index(index, 2) else {
                return Ok(Value::Nil);
            };
            let accept = bool_or_default(args.get(1), false);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let model = &mut *model;
            let in_bg = (model.wrath_state.instance.as_ref()).is_some_and(|i| i.instance_type == 3);
            if accept && player_exists(model) && !in_bg {
                let gates = super::wrath_port_gates::PortGates::of(model);
                if let Some(key) = gates.refusal(super::wrath_port_gates::CombatTest::Flag) {
                    model.ui_errors.push(key);
                    return Ok(Value::Nil);
                }
            }
            model
                .battlefield
                .requests
                .push(BattlefieldRequest::Port { slot, accept });
            if !accept && model.battlefield.slots[slot].bg_type == model.battlefield.list.bg_type {
                model
                    .pending_events
                    .push(("BATTLEFIELDS_CLOSED".into(), Vec::new()));
            }
            Ok(Value::Integer(1))
        })?,
    )?;

    // The listed type's `GroupsAllowed` (`0x54c6e0`).
    g.set(
        "CanJoinBattlefieldAsGroup",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let q = &model.battlefield;
            Ok(flag(q.ty(q.list.bg_type).is_some_and(|t| t.groups_allowed)))
        })?,
    )?;
    Ok(())
}

/// The names build 12340 adds past 1.12, installed in the Wrath block.
pub(super) fn install_wrath(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // 1/nil each: the active slot's map is an arena; the active slot is rated (`0x549ad0`).
    g.set(
        "IsActiveBattlefieldArena",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let q = &model.battlefield;
            Ok(MultiValue::from_vec(vec![
                flag(q.active_arena),
                flag(q.active_rated),
            ]))
        })?,
    )?;

    // The listed type's `InstanceType == 4` (`0x54bae0`).
    g.set(
        "IsBattlefieldArena",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let q = &model.battlefield;
            Ok(flag(
                q.ty(q.list.bg_type).is_some_and(|t| t.instance_type == 4),
            ))
        })?,
    )?;

    // hasWin, winHonor, winArena, lossHonor, 0 (`0x54a370`, `0x54a300`).
    for (name, random) in [
        ("GetRandomBGHonorCurrencyBonuses", true),
        ("GetHolidayBGHonorCurrencyBonuses", false),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, ()| {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                let list = &model.battlefield.list;
                let r = if random { list.random } else { list.holiday };
                Ok(MultiValue::from_vec(vec![
                    Value::Boolean(r.has_win),
                    Value::Integer(i64::from(r.win_honor)),
                    Value::Integer(i64::from(r.win_arena)),
                    Value::Integer(i64::from(r.loss_honor)),
                    Value::Integer(0),
                ]))
            })?,
        )?;
    }

    g.set(
        "GetNumBattlegroundTypes",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.battlefield.order.len() as i64)
        })?,
    )?;

    // name, canEnter, isHoliday, isRandom, bgTypeID; nothing without a player or out of range
    // (`0x54e010`).
    g.set(
        "GetBattlegroundInfo",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetBattlegroundInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let q = &model.battlefield;
            let Some(t) = slot_index(index, q.order.len())
                .and_then(|i| q.ty(q.order[i]))
                .filter(|_| player_exists(&model))
            else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&t.name)?),
                flag(t.can_enter),
                flag(t.is_holiday),
                flag(t.id == 32),
                Value::Integer(i64::from(t.id)),
            ]))
        })?,
    )?;

    // Re-sort the type list with the current holiday states; a numeric argument is read and
    // ignored (`0x54c9f0`).
    g.set(
        "SortBGList",
        lua.create_function(|lua, _: MultiValue| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.battlefield.sort_order();
            Ok(())
        })?,
    )?;

    // `CMSG_BATTLEFIELD_LIST` for the type at that position, which becomes the listed type at
    // once (`0x54e5b0`); nothing without a player or out of range.
    g.set(
        "RequestBattlegroundInstanceInfo",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: RequestBattlegroundInstanceInfo(index)")?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if !player_exists(&model) {
                return Ok(());
            }
            let q = &mut model.battlefield;
            if let Some(&bg_type) = slot_index(index, q.order.len()).map(|i| &q.order[i]) {
                if q.ty(bg_type).is_some() {
                    q.list.bg_type = bg_type;
                    q.requests.push(BattlefieldRequest::List { bg_type });
                }
            }
            Ok(())
        })?,
    )?;

    // GetWorldPVPQueueStatus(1): status, zone name, battle id, and the invite's seconds left on
    // the server clock while confirming; nothing for any other index (`0x54c8a0`).
    g.set(
        "GetWorldPVPQueueStatus",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetWorldPVPQueueStatus(index)")?;
            if index != 1 {
                return Ok(MultiValue::new());
            }
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let w = &model.battlefield.world_pvp;
            let left = if w.status == 2 {
                i64::from(w.deadline) - server_now(&model).unwrap_or(0)
            } else {
                0
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(status_text(w.status))?),
                text(lua, &w.zone_name)?,
                Value::Integer(i64::from(w.battle_id)),
                Value::Integer(left),
            ]))
        })?,
    )?;

    g.set(
        "CanQueueForWintergrasp",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(model.battlefield.world_pvp.can_queue_wintergrasp))
        })?,
    )?;
    // Seconds to the next battle on the server clock, at least 0; nothing while the wait clock's
    // world state is 0 (`0x5435a0`).
    g.set(
        "GetWintergraspWaitTime",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let w = &model.battlefield.world_pvp;
            if !w.wintergrasp_waiting {
                return Ok(MultiValue::new());
            }
            let left = (i64::from(w.wintergrasp_next) - server_now(&model).unwrap_or(0)).max(0);
            Ok(MultiValue::from_vec(vec![Value::Integer(left)]))
        })?,
    )?;
    g.set(
        "GetCurrentArenaSeason",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.battlefield.world_pvp.arena_season))
        })?,
    )?;
    g.set(
        "GetPreviousArenaSeason",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.battlefield.world_pvp.previous_arena_season))
        })?,
    )?;
    g.set(
        "IsSubZonePVPPOI",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(model.battlefield.world_pvp.subzone_pvp_poi))
        })?,
    )?;

    // The battlefield-manager queue and exit verbs: a rounded id, no usage errors (`0x54e1a0`,
    // `0x54e200`). The two invite answers are the static-popup module's.
    g.set(
        "BattlefieldMgrQueueRequest",
        lua.create_function(|lua, id: Value| {
            let battle_id = number(lua, &id)?.unwrap_or(0) as u32;
            push(lua, BattlefieldRequest::BfQueueRequest { battle_id });
            Ok(())
        })?,
    )?;
    g.set(
        "BattlefieldMgrExitRequest",
        lua.create_function(|lua, id: Value| {
            let battle_id = number(lua, &id)?.unwrap_or(0) as u32;
            push(lua, BattlefieldRequest::BfExit { battle_id });
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests;
