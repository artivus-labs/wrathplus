//! Build 12340's gameplay-state readers over state the app pushes: `GetQuestResetTime`
//! (`0x5e6de0`), `GetMirrorTimerInfo` (`0x51cc10`) and `GetMirrorTimerProgress` (`0x517aa0`),
//! `IsPVPTimerRunning` (`0x51bd60`) and `GetPVPTimer` (`0x51bd00` → `0x6de7a0`), and
//! `GetInstanceInfo` (`0x51a8c0`). Evidence: `/tmp/azb-app-mig/ghidra/shim{1,2}.log`.
//!
//! Labelled: the writers of the daily reset (`0x5dff40`), of the mirror-timer table (`0x513ea0`)
//! and of the PvP timer's end (`player+0x1940`) are not decompiled; the app stamps them on the
//! packet or flag edge that carries them.

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript};

/// One mirror-timer slot (`0xbd0b80`, 7 words), the slot being the timer's type.
#[derive(Clone, Debug, PartialEq)]
pub struct WrathMirrorTimer {
    /// The value at the start, ms.
    pub value: i32,
    pub max_value: i32,
    /// ms of value per ms of time, signed.
    pub scale: i32,
    pub paused: bool,
    /// The owning spell's name, else `<NAME>_LABEL`.
    pub label: String,
    /// `GetTime()` at the start, seconds.
    pub started: f64,
}

/// The current map's `GetInstanceInfo` row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathInstanceInfo {
    /// `Map.dbc` name.
    pub name: String,
    /// `Map.dbc` `InstanceType`.
    pub instance_type: u32,
    /// The instance difficulty, 0-based (`0xbd0894`); Lua sees it plus one.
    pub difficulty: u32,
    /// The `MapDifficulty` row's `(difficulty string key, max players)` for the map and difficulty.
    pub map_difficulty: Option<(String, i32)>,
    /// `0xbd1980`, `SMSG_INSTANCE_DIFFICULTY`'s second word.
    pub dynamic_difficulty: u32,
    /// `Map.dbc` `Flags & 0x100`.
    pub is_dynamic: bool,
}

#[derive(Default)]
pub(crate) struct State {
    /// The daily reset, Unix seconds (`0xc23aec`).
    quest_reset_at: i64,
    mirror: [Option<WrathMirrorTimer>; 3],
    /// `None` without a player.
    pvp: Option<PvpTimer>,
    pub(crate) instance: Option<WrathInstanceInfo>,
    /// Each bag's `BagFamily`, `GetContainerNumFreeSlots`'s second value.
    bag_families: std::collections::HashMap<i64, u32>,
    /// The current area's `AreaTable` flag `0x8000000`.
    can_hearth_and_resurrect: bool,
    /// The non-zero `PLAYER_FIELD_DAILY_QUESTS` slots.
    daily_quests_completed: u32,
    /// The gossip NPC's creature `type_flags & 0x8000000`.
    force_gossip: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PvpTimer {
    /// `PLAYER_FLAGS_PVP_TIMER` (`0x40000`).
    running: bool,
    /// `GetTime()` when the timer ends; `None` for no end.
    ends: Option<f64>,
}

/// `0x513e00`: a slot's name.
fn mirror_name(slot: usize) -> &'static str {
    match slot {
        0 => "EXHAUSTION",
        1 => "BREATH",
        2 => "FEIGNDEATH",
        _ => "UNKNOWN",
    }
}

/// `0xac8074`: `GetInstanceInfo`'s type word, `"none"` from 5 up.
fn instance_type_word(ty: u32) -> &'static str {
    match ty {
        1 => "party",
        2 => "raid",
        3 => "pvp",
        4 => "arena",
        _ => "none",
    }
}

impl UiScript {
    /// The daily quest reset as Unix seconds, off `SMSG_QUERY_TIME_RESPONSE`.
    pub fn set_wrath_quest_reset_at(&mut self, unix_secs: i64) {
        self.model_mut().wrath_state.quest_reset_at = unix_secs;
    }

    /// A mirror-timer slot by wire type (0 fatigue, 1 breath, 2 feign death); `None` stops it.
    pub fn set_wrath_mirror_timer(&mut self, kind: usize, timer: Option<WrathMirrorTimer>) {
        if let Some(slot) = self.model_mut().wrath_state.mirror.get_mut(kind) {
            *slot = timer;
        }
    }

    /// `SMSG_PAUSE_MIRROR_TIMER` on a running slot.
    pub fn set_wrath_mirror_timer_paused(&mut self, kind: usize, paused: bool) {
        if let Some(Some(t)) = self.model_mut().wrath_state.mirror.get_mut(kind) {
            t.paused = paused;
        }
    }

    /// The player's PvP timer: the flag and its end on the `GetTime()` clock; `None` without a
    /// player.
    pub fn set_wrath_pvp_timer(&mut self, timer: Option<(bool, Option<f64>)>) {
        self.model_mut().wrath_state.pvp = timer.map(|(running, ends)| PvpTimer { running, ends });
    }

    /// A bag's `BagFamily`.
    pub fn set_wrath_bag_family(&mut self, bag: i64, family: u32) {
        self.model_mut()
            .wrath_state
            .bag_families
            .insert(bag, family);
    }

    /// `CanHearthAndResurrectFromArea`'s answer for the current area.
    pub fn set_wrath_can_hearth_and_resurrect(&mut self, can: bool) {
        self.model_mut().wrath_state.can_hearth_and_resurrect = can;
    }

    /// `ForceGossip`'s answer for the open gossip NPC.
    pub fn set_wrath_force_gossip(&mut self, force: bool) {
        self.model_mut().wrath_state.force_gossip = force;
    }

    /// `GetDailyQuestsCompleted`'s count.
    pub fn set_wrath_daily_quests_completed(&mut self, n: u32) {
        self.model_mut().wrath_state.daily_quests_completed = n;
    }

    /// The current map's instance row; `None` for a map with no `Map.dbc` row.
    pub fn set_wrath_instance_info(&mut self, info: Option<WrathInstanceInfo>) {
        self.model_mut().wrath_state.instance = info;
    }
}

fn number(lua: &Lua, args: &MultiValue, i: usize) -> Option<f64> {
    args.get(i)
        .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
}

/// An item argument as the inventory finders take it (`0x754a20`/`0x754a50` by id, `0x754af0`/
/// `0x754b90` by name or link): a number is an item id, a string with `item:` its link's id, and
/// any other string a name.
enum ItemQuery {
    Id(u32),
    Name(String),
}

fn item_query(lua: &Lua, v: Value, usage: &str) -> mlua::Result<ItemQuery> {
    if let Value::String(s) = &v {
        let text = s.to_str()?.to_string();
        if let Some(rest) = text.split_once("item:").map(|(_, r)| r) {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            return Ok(ItemQuery::Id(digits.parse().unwrap_or(0)));
        }
        return Ok(ItemQuery::Name(text));
    }
    let n = lua
        .coerce_number(v)?
        .ok_or_else(|| mlua::Error::runtime(usage.to_string()))?;
    Ok(ItemQuery::Id(
        u32::try_from(n.round_ties_even() as i64).unwrap_or(0),
    ))
}

/// The first carried bag slot (backpack, then bags 1-4, each in slot order) holding the item.
/// Labelled: a name matches the template's name without case, and only the bags are searched,
/// where the native finders also reach worn items.
fn carried_item(model: &Model, query: &ItemQuery) -> Option<(i64, u32)> {
    (0..=4).find_map(|bag| {
        let container = model.containers.get(&bag)?;
        let mut slots: Vec<_> = container
            .slots
            .iter()
            .filter(|(_, s)| s.item_id != 0)
            .collect();
        slots.sort_by_key(|(slot, _)| **slot);
        slots.into_iter().find_map(|(&slot, s)| {
            let hit = match query {
                ItemQuery::Id(id) => s.item_id == *id,
                ItemQuery::Name(name) => model
                    .item_templates
                    .get(&s.item_id)
                    .is_some_and(|t| t.name.eq_ignore_ascii_case(name)),
            };
            hit.then_some((bag, slot))
        })
    })
}

/// The main bar's page, 0-based (`0xc1e598`), which build 12340 keeps engine-side.
#[derive(Default)]
struct ActionBarPage(u32);

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    lua.set_app_data(ActionBarPage::default());
    // GetActionBarPage() (`0x5a7fd0`): the page, 1-based. Labelled: the native answers 1 while
    // `0xc1e5a0` is set, which is not modelled.
    g.set(
        "GetActionBarPage",
        lua.create_function(|lua, ()| {
            Ok(lua.app_data_ref::<ActionBarPage>().map_or(0, |p| p.0) + 1)
        })?,
    )?;
    // ChangeActionBarPage(page) (`0x5a7f60`): pages 1-6, else an error; a new page fires
    // `ACTIONBAR_PAGE_CHANGED` (event `0xaf`). Labelled: the `0x5191c0(0xe)` gate.
    g.set(
        "ChangeActionBarPage",
        lua.create_function(|lua, page: Value| {
            let page = lua
                .coerce_number(page)?
                .map_or(0, |n| n.round_ties_even() as i64)
                - 1;
            if !(0..6).contains(&page) {
                return Err(mlua::Error::runtime(
                    "ChangeActionBarPage() needs a page in the range 1 to 6",
                ));
            }
            let moved = match lua.app_data_mut::<ActionBarPage>() {
                Some(mut current) if current.0 != page as u32 => {
                    current.0 = page as u32;
                    true
                }
                _ => false,
            };
            if moved {
                super::tick::fire_event_into(lua, "ACTIONBAR_PAGE_CHANGED", Vec::new());
            }
            Ok(())
        })?,
    )?;
    // IsPetAttackAction(index) (`0x5d5a10` → `0x5d41b0`): a boolean, the slot holds the pet's
    // Attack command. Labelled: read as the slot's `PET_ACTION_ATTACK` token, where the native
    // tests the action word.
    g.set(
        "IsPetAttackAction",
        lua.create_function(|lua, index: Value| {
            let index = lua
                .coerce_number(index)?
                .ok_or_else(|| mlua::Error::runtime("Usage: IsPetAttackAction(index)"))?
                .round_ties_even() as i64;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let attack = usize::try_from(index - 1)
                .ok()
                .and_then(|i| model.pet_bar.slots.get(i))
                .is_some_and(|s| {
                    s.view.is_token && s.view.name.as_deref() == Some("PET_ACTION_ATTACK")
                });
            Ok(attack)
        })?,
    )?;
    // GetCVarBool("cvar") (`0x5100c0`, the in-game table's): 1 when the CVar exists and its value
    // reads non-zero, else nil; a non-string raises. Labelled: its flag `0x40` (hidden from this
    // table) is not modelled.
    g.set(
        "GetCVarBool",
        lua.create_function(|lua, name: Value| {
            let Value::String(name) = name else {
                return Err(mlua::Error::runtime("Usage: GetCVarBool(\"cvar\")"));
            };
            let key = name.to_str()?.to_ascii_lowercase();
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let on = model
                .cvars
                .get(&key)
                .is_some_and(|slot| slot.value.trim().parse::<f64>().is_ok_and(|n| n != 0.0));
            Ok(on.then_some(1))
        })?,
    )?;
    // PickupItem(item) (`0x522fb0`): an item by id, link or cached name onto the cursor, mode 7,
    // as a dragged link; the cursor clears first, and an item naming nothing leaves it empty.
    g.set(
        "PickupItem",
        lua.create_function(|lua, item: Value| {
            // Neither a number nor a string names nothing: no usage error.
            let query = item_query(lua, item, "").unwrap_or(ItemQuery::Id(0));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let id = match query {
                ItemQuery::Id(id) => id,
                ItemQuery::Name(name) => model
                    .item_templates
                    .iter()
                    .find(|(_, t)| t.name.eq_ignore_ascii_case(&name))
                    .map_or(0, |(id, _)| *id),
            };
            super::cursor::clear_cursor(&mut model);
            if id != 0 {
                let template = model.item_templates.get(&id);
                let payload = super::CursorItemId {
                    item_id: id,
                    texture: template.and_then(|t| t.icon.clone()),
                    link: template.map(|t| super::item_stats::item_link(id, &t.name, t.quality)),
                };
                model.cursor = Some(super::CursorPayload::ItemId(payload));
                super::cursor::queue_cursor_update(&mut model);
            }
            Ok(())
        })?,
    )?;
    // SpellCanTargetItem() (`0x7fdd10`): 1 while the targeting spell takes an item (its word
    // `& 0x4010`, the app's `set_item_pick_armed`), else nil.
    g.set(
        "SpellCanTargetItem",
        lua.create_function(|lua, ()| {
            let armed = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .item_pick_armed;
            Ok(armed.then_some(1))
        })?,
    )?;
    // SpellTargetItem(item) (`0x8007e0`): while armed, the first carried slot holding the item
    // binds it as a bag click does (`0x5210d0`); nothing otherwise. Labelled: the `0x5191c0(2)`
    // gate is unread.
    g.set(
        "SpellTargetItem",
        lua.create_function(|lua, item: Value| {
            if !lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .item_pick_armed
            {
                return Ok(());
            }
            let query = item_query(
                lua,
                item,
                "Usage: SpellTargetItem(itemID|\"name\"|\"itemlink\")",
            )?;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if let Some(at) = carried_item(&model, &query) {
                model.item_picks.push(at);
            }
            Ok(())
        })?,
    )?;
    // SetMultiCastSpell(slot, spellID) (`0x5ab8a0`): the totem bar's slots 133-144 take a spell
    // action, or are cleared by spell 0 (`0x5aaa90`), through the `PlaceAction` queue. Labelled:
    // the per-slot totem check (`0x5a7bc0`, its refusal message `0x299`), the record check
    // (`0x4cfd20`) and the `0x5191c0(0xe)` gate are not modelled.
    g.set(
        "SetMultiCastSpell",
        lua.create_function(|lua, (slot, spell): (Value, Value)| {
            let number = |v: Value| -> mlua::Result<Option<i64>> {
                Ok(lua.coerce_number(v)?.map(|n| n.round_ties_even() as i64))
            };
            let (Some(slot), spell) = (number(slot)?, number(spell)?) else {
                return Err(mlua::Error::runtime(
                    "Usage: SetMultiCastSpell(slot, spellID)",
                ));
            };
            if !(133..=144).contains(&slot) {
                return Ok(());
            }
            let packed = u32::try_from(spell.unwrap_or(0)).unwrap_or(0) & 0x00ff_ffff;
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .action_sets
                .push((slot as u32, packed));
            Ok(())
        })?,
    )?;
    // EquipItemByName(item [, slot]) (`0x51cdb0`): the first carried, unlocked slot holding the
    // item auto-equips (`0x6dfee0`), or with a slot (1-19, or an `INVSLOT` name) moves there
    // (`0x6dff90`); a bad slot raises.
    g.set(
        "EquipItemByName",
        lua.create_function(|lua, (item, slot): (Value, Value)| {
            const BAD_SLOT: &str = "EquipItemByName(): Invalid inventory dstSlot";
            let dst = match &slot {
                Value::Nil => None,
                Value::String(name) => Some(
                    super::char_stats::slot_id_by_name(&name.to_str()?)
                        .filter(|id| (1..=19).contains(id))
                        .ok_or_else(|| mlua::Error::runtime(BAD_SLOT))?,
                ),
                other => Some(
                    lua.coerce_number(other.clone())?
                        .map(|n| n.round_ties_even() as i64)
                        .filter(|id| (1..=19).contains(id))
                        .ok_or_else(|| mlua::Error::runtime(BAD_SLOT))?,
                ),
            };
            let Ok(query) = item_query(lua, item, "") else {
                return Ok(());
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some((bag, at)) = carried_item(&model, &query) else {
                return Ok(());
            };
            let locked = model
                .containers
                .get(&bag)
                .and_then(|c| c.slots.get(&at))
                .is_some_and(|s| s.locked);
            if locked {
                return Ok(());
            }
            match dst {
                None => model.container_autoequips.push((bag, at)),
                Some(id) => model.container_moves.push(super::ContainerMove {
                    src_bag: bag,
                    src_slot: at,
                    dst_bag: super::cursor::EQUIPMENT_BAG,
                    dst_slot: id as u32,
                    count: None,
                }),
            }
            Ok(())
        })?,
    )?;
    // IsStackableAction(slot) (`0x5a8720` → `0x5a8470`): 1 when slot 1-144 holds an item whose
    // template stacks past one (`ActionButton.lua` then shows its count), else nil. Labelled: the
    // native compares template `+0x5c`, taken as the stack size (`ItemTemplateView::stackable`).
    g.set(
        "IsStackableAction",
        lua.create_function(|lua, slot: Value| {
            let slot = lua
                .coerce_number(slot)?
                .ok_or_else(|| mlua::Error::runtime("Usage: IsStackableAction(slot)"))?
                .round_ties_even() as i64;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let stacks = u32::try_from(slot)
                .ok()
                .filter(|s| (1..=144).contains(s))
                .and_then(|s| model.actions.get(&s))
                .filter(|a| a.kind == super::action::ACTION_KIND_ITEM)
                .and_then(|a| model.item_templates.get(&a.action))
                .is_some_and(|t| t.stackable > 1);
            Ok(stacks.then_some(1))
        })?,
    )?;
    // GetQuestResetTime(): the reset less `time(NULL)`, the local wall clock.
    g.set(
        "GetQuestResetTime",
        lua.create_function(|lua, ()| {
            let at = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_state
                .quest_reset_at;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            Ok((at - now) as f64)
        })?,
    )?;
    // GetMirrorTimerInfo(index): slot `index - 1`'s name, value, max, scale, paused and label.
    g.set(
        "GetMirrorTimerInfo",
        lua.create_function(|lua, args: MultiValue| {
            let slot = number(lua, &args, 0)
                .map(|n| n as i64 - 1)
                .and_then(|i| usize::try_from(i).ok())
                .filter(|&i| i < 3)
                .ok_or_else(|| mlua::Error::runtime("Usage: GetMirrorTimerInfo(\"timer\")"))?;
            let timer = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_state
                .mirror[slot]
                .clone();
            let s = |t: &str| lua.create_string(t).map(Value::String);
            Ok(MultiValue::from_vec(match timer {
                Some(t) => vec![
                    s(mirror_name(slot))?,
                    Value::Number(f64::from(t.value)),
                    Value::Number(f64::from(t.max_value)),
                    Value::Number(f64::from(t.scale)),
                    Value::Number(if t.paused { 1.0 } else { 0.0 }),
                    s(&t.label)?,
                ],
                // Labelled: an idle slot reads as `"UNKNOWN"` with zeroed words.
                None => vec![
                    s("UNKNOWN")?,
                    Value::Number(0.0),
                    Value::Number(0.0),
                    Value::Number(0.0),
                    Value::Number(0.0),
                    s("")?,
                ],
            }))
        })?,
    )?;
    // GetMirrorTimerProgress(name): `(now - start) * scale + value`, ms (`0x517aa0`).
    g.set(
        "GetMirrorTimerProgress",
        lua.create_function(|lua, name: Option<String>| {
            let slot = name
                .as_deref()
                .and_then(|n| (0..3).find(|&i| mirror_name(i) == n))
                .ok_or_else(|| mlua::Error::runtime("Usage: GetMirrorTimerProgress(\"timer\")"))?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match &model.wrath_state.mirror[slot] {
                Some(t) => {
                    let elapsed_ms = ((model.now - t.started) * 1000.0).round();
                    elapsed_ms * f64::from(t.scale) + f64::from(t.value)
                }
                None => 0.0,
            })
        })?,
    )?;
    // IsPVPTimerRunning(): 1 while `PLAYER_FLAGS & 0x40000`, else nil.
    g.set(
        "IsPVPTimerRunning",
        lua.create_function(|lua, ()| {
            let running = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_state
                .pvp
                .is_some_and(|p| p.running);
            Ok(running.then_some(1.0))
        })?,
    )?;
    // GetPVPTimer(): `0x6de7a0` plus 1000; 300000 with the flag clear, the ms left while it runs,
    // -1 past its end; 0 without a player.
    g.set(
        "GetPVPTimer",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match model.wrath_state.pvp {
                None => 0.0,
                Some(PvpTimer { running: false, .. }) => 301_000.0,
                Some(PvpTimer { ends, .. }) => match ends {
                    Some(end) if end > model.now => ((end - model.now) * 1000.0).round() + 1000.0,
                    _ => 999.0,
                },
            })
        })?,
    )?;
    // GetInstanceInfo(): name, type, difficulty + 1, difficulty name, max players, dynamic
    // difficulty, isDynamic; a map with no row reads nil, "none", 0, "", 0, 0, nil.
    g.set(
        "GetInstanceInfo",
        lua.create_function(|lua, ()| {
            let info = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_state
                .instance
                .clone();
            let s = |t: &str| lua.create_string(t).map(Value::String);
            let Some(info) = info else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    s("none")?,
                    Value::Number(0.0),
                    s("")?,
                    Value::Number(0.0),
                    Value::Number(0.0),
                    Value::Nil,
                ]));
            };
            // `0x819d40`: the row's string key resolved as a global string. Labelled: a key with no
            // global string reads empty.
            let (difficulty_name, max_players) = match &info.map_difficulty {
                Some((key, max)) => (
                    lua.globals()
                        .get::<Option<String>>(key.as_str())
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                    f64::from(*max),
                ),
                None => (String::new(), 0.0),
            };
            Ok(MultiValue::from_vec(vec![
                s(&info.name)?,
                s(instance_type_word(info.instance_type))?,
                Value::Number(f64::from(info.difficulty) + 1.0),
                s(&difficulty_name)?,
                Value::Number(max_players),
                Value::Number(f64::from(info.dynamic_difficulty)),
                Value::Boolean(info.is_dynamic),
            ]))
        })?,
    )?;
    // GetContainerNumFreeSlots(bag) (`0x5d7590`): the backpack's empty slots and 0, always; a bag
    // 1..11 that exists, its empty slots and `BagFamily`; the open bank's (-1) empty slots and 0;
    // anything else (an empty bag slot, the keyring, the closed bank) a lone 0. Labelled: the bank
    // is open when App has published container -1.
    g.set(
        "GetContainerNumFreeSlots",
        lua.create_function(|lua, args: MultiValue| {
            let bag = number(lua, &args, 0)
                .map(|n| n as i64)
                .ok_or_else(|| mlua::Error::runtime("Usage: GetContainerFreeSlots(index)"))?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let lone = || Ok(MultiValue::from_vec(vec![Value::Number(0.0)]));
            let c = model.containers.get(&bag);
            let free = match (bag, c) {
                (0, None) => 16,
                (-1..=11, Some(c)) => (1..=c.num_slots)
                    .filter(|s| !c.slots.contains_key(s))
                    .count(),
                _ => return lone(),
            };
            let family = match bag {
                1..=11 => model
                    .wrath_state
                    .bag_families
                    .get(&bag)
                    .copied()
                    .unwrap_or(0),
                _ => 0,
            };
            Ok(MultiValue::from_vec(vec![
                Value::Number(free as f64),
                Value::Number(f64::from(family)),
            ]))
        })?,
    )?;
    // IsEquippableItem(item) (`0x511150`): the template's `InventoryType` is not 0. Labelled: read
    // as `GetItemInfo`'s equip location, which is empty for type 0.
    g.set(
        "IsEquippableItem",
        lua.create_function(|lua, item: Value| {
            let Ok(info) = lua.globals().get::<mlua::Function>("GetItemInfo") else {
                return Ok(Value::Nil);
            };
            let v: MultiValue = info.call(item).unwrap_or_default();
            let equip = v.get(8).and_then(|x| match x {
                Value::String(s) => Some(s.to_string_lossy()),
                _ => None,
            });
            Ok(if equip.is_some_and(|e| !e.is_empty()) {
                Value::Number(1.0)
            } else {
                Value::Nil
            })
        })?,
    )?;
    g.set(
        "CanHearthAndResurrectFromArea",
        lua.create_function(|lua, ()| {
            let can = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_state
                .can_hearth_and_resurrect;
            Ok(can.then_some(1.0))
        })?,
    )?;
    g.set(
        "GetDailyQuestsCompleted",
        lua.create_function(|lua, ()| {
            Ok(f64::from(
                lua.app_data_ref::<Model>()
                    .expect("model app_data")
                    .wrath_state
                    .daily_quests_completed,
            ))
        })?,
    )?;
    g.set(
        "ForceGossip",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .wrath_state
                .force_gossip)
        })?,
    )?;
    // GetContainerItemQuestInfo(bag, slot) (`0x5d9400`): the item's template, `Bonding` 4 for a
    // quest item and its `StartQuest`, and whether that quest is in the log (`0x5e11d0`).
    // Labelled: `0x5e11d0` is read as the quest's presence in the quest log.
    g.set(
        "GetContainerItemQuestInfo",
        lua.create_function(|lua, args: MultiValue| {
            let (Some(bag), Some(slot)) = (number(lua, &args, 0), number(lua, &args, 1)) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetContainerItemQuestInfo(index, slot)",
                ));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let template = model
                .containers
                .get(&(bag as i64))
                .and_then(|c| c.slots.get(&(slot as u32)))
                .and_then(|s| model.item_templates.get(&s.item_id));
            let Some(t) = template else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ]));
            };
            let quest_item = if t.bonding == 4 {
                Value::Boolean(true)
            } else {
                Value::Nil
            };
            Ok(MultiValue::from_vec(if t.start_quest == 0 {
                vec![quest_item, Value::Nil, Value::Nil]
            } else {
                let active = model
                    .quest_log
                    .entries
                    .iter()
                    .any(|e| !e.is_header && e.quest_id == t.start_quest);
                vec![
                    quest_item,
                    Value::Number(f64::from(t.start_quest)),
                    Value::Boolean(active),
                ]
            }))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm() -> UiScript {
        UiScript::new().unwrap()
    }

    #[test]
    fn the_bar_page_is_engine_side_and_the_bool_cvar_reads_its_value() {
        let s = vm();
        s.run(
            "SEEN = 0 local f = CreateFrame('Frame') f:RegisterEvent('ACTIONBAR_PAGE_CHANGED') \
             f:SetScript('OnEvent', function() SEEN = SEEN + 1 end)",
        )
        .unwrap();
        assert_eq!(s.eval::<i64>("return GetActionBarPage()").unwrap(), 1);
        s.run("ChangeActionBarPage(3) ChangeActionBarPage(3)")
            .unwrap();
        assert_eq!(s.eval::<i64>("return GetActionBarPage()").unwrap(), 3);
        assert_eq!(
            s.eval::<i64>("return SEEN").unwrap(),
            1,
            "only a new page fires"
        );
        assert!(s.run("ChangeActionBarPage(7)").is_err());
        s.run("SetCVar('autoLootDefault', '1') SetCVar('scriptErrors', '0')")
            .unwrap();
        assert!(s
            .eval::<bool>(
                "return GetCVarBool('autolootdefault') == 1 and GetCVarBool('scriptErrors') == nil \
                 and GetCVarBool('nosuchcvar') == nil"
            )
            .unwrap());
        assert!(s.run("GetCVarBool(1)").is_err());
        assert!(s
            .eval::<bool>("return IsPetAttackAction(1) == false")
            .unwrap());
        assert!(s.run("IsPetAttackAction()").is_err());
    }

    #[test]
    fn pickup_item_holds_the_item_by_id_and_places_it_on_a_bar() {
        use crate::script::ItemTemplateView;
        let mut s = vm();
        s.set_item_template(
            2589,
            ItemTemplateView {
                name: "Linen Cloth".into(),
                quality: 1,
                icon: Some("Interface\\Icons\\INV_Fabric_Linen_01".into()),
                ..Default::default()
            },
        );
        s.run("PickupItem('linen cloth')").unwrap();
        assert!(s
            .eval::<bool>(
                "local k, id, link = GetCursorInfo() \
                 return k == 'item' and id == 2589 and link:find('Hitem:2589') ~= nil"
            )
            .unwrap());
        s.run("PlaceAction(5)").unwrap();
        assert_eq!(s.take_action_sets(), vec![(5, 0x8000_0000 | 2589)]);
        s.run("PickupItem(99999)").unwrap();
        assert!(
            s.eval::<bool>("return select('#', GetCursorInfo()) == 0")
                .unwrap(),
            "uncached: no report"
        );
        s.run("PickupItem({})").unwrap();
        assert!(
            s.eval::<bool>("return select('#', GetCursorInfo()) == 0")
                .unwrap(),
            "cleared"
        );
    }

    #[test]
    fn multi_cast_slots_take_a_spell_or_clear_through_the_place_queue() {
        let mut s = vm();
        s.run("SetMultiCastSpell(133, 8071) SetMultiCastSpell(144, 0) SetMultiCastSpell(5, 8071)")
            .unwrap();
        assert_eq!(s.take_action_sets(), vec![(133, 8071), (144, 0)]);
        assert!(s.run("SetMultiCastSpell()").is_err());
    }

    #[test]
    fn item_targeting_and_equip_by_name_find_the_first_carried_slot() {
        use crate::script::{ContainerSlot, ContainerState, ItemTemplateView};
        let mut s = vm();
        let mut bag = ContainerState {
            num_slots: 4,
            ..Default::default()
        };
        for (slot, id) in [(3, 1201), (1, 2589)] {
            bag.slots.insert(
                slot,
                ContainerSlot {
                    item_id: id,
                    ..Default::default()
                },
            );
        }
        s.model_mut().containers.insert(0, bag);
        s.set_item_template(
            1201,
            ItemTemplateView {
                name: "Dented Buckler".into(),
                ..Default::default()
            },
        );
        assert!(s
            .eval::<bool>("return SpellCanTargetItem() == nil")
            .unwrap());
        s.run("SpellTargetItem(2589)").unwrap();
        assert!(s.model_mut().item_picks.is_empty(), "not armed: nothing");
        s.model_mut().item_pick_armed = true;
        assert!(s.eval::<bool>("return SpellCanTargetItem() == 1").unwrap());
        s.run("SpellTargetItem('|cffffffff|Hitem:2589:0:0:0|h[Linen Cloth]|h|r')")
            .unwrap();
        assert_eq!(s.model_mut().item_picks, [(0, 1)]);
        s.run("EquipItemByName('dented buckler')").unwrap();
        assert_eq!(s.model_mut().container_autoequips, [(0, 3)]);
        s.run("EquipItemByName(1201, 'SecondaryHandSlot')").unwrap();
        let moved = s.model_mut().container_moves.pop().unwrap();
        assert_eq!((moved.src_bag, moved.src_slot, moved.dst_slot), (0, 3, 17));
        assert!(s.run("EquipItemByName(1201, 20)").is_err());
        assert!(s.run("SpellTargetItem({})").is_err());
    }

    #[test]
    fn a_stackable_action_is_an_item_slot_whose_template_stacks() {
        use crate::script::{ActionSlot, ItemTemplateView};
        let mut s = vm();
        let item = |id| ActionSlot {
            kind: super::super::action::ACTION_KIND_ITEM,
            action: id,
            ..Default::default()
        };
        s.set_action(1, Some(item(4540)));
        s.set_action(2, Some(item(2589)));
        let template = |stackable| ItemTemplateView {
            stackable,
            ..Default::default()
        };
        s.set_action(
            3,
            Some(ActionSlot {
                action: 133,
                ..Default::default()
            }),
        );
        s.set_item_template(4540, template(20));
        s.set_item_template(2589, template(1));
        for check in [
            "IsStackableAction(1) == 1",
            "IsStackableAction(2) == nil and IsStackableAction(3) == nil",
            "IsStackableAction(145) == nil and IsStackableAction(0) == nil",
        ] {
            assert!(
                s.eval::<bool>(&format!("return {check}")).unwrap(),
                "{check}"
            );
        }
        assert!(s.run("IsStackableAction('x')").is_err());
    }

    #[test]
    fn the_quest_reset_counts_down_on_the_wall_clock() {
        let mut s = vm();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        s.set_wrath_quest_reset_at(now + 3600);
        let left = s.eval::<f64>("return GetQuestResetTime()").unwrap();
        assert!((3598.0..=3600.0).contains(&left), "{left}");
    }

    #[test]
    fn a_mirror_timer_reads_its_slot_and_integrates_its_rate() {
        let mut s = vm();
        s.set_now(100.0);
        s.set_wrath_mirror_timer(
            1,
            Some(WrathMirrorTimer {
                value: 60_000,
                max_value: 60_000,
                scale: -1,
                paused: false,
                label: "Breath".into(),
                started: 100.0,
            }),
        );
        assert!(s
            .eval::<bool>(
                "local n, v, m, sc, p, l = GetMirrorTimerInfo(2) \
                 return n == 'BREATH' and v == 60000 and m == 60000 and sc == -1 and p == 0 \
                 and l == 'Breath'"
            )
            .unwrap());
        assert_eq!(
            s.eval::<String>("return (GetMirrorTimerInfo(1))").unwrap(),
            "UNKNOWN"
        );
        s.set_now(102.5);
        assert_eq!(
            s.eval::<f64>("return GetMirrorTimerProgress('BREATH')")
                .unwrap(),
            57_500.0
        );
        assert!(s.run("GetMirrorTimerInfo(4)").is_err());
        assert!(s.run("GetMirrorTimerProgress('UNKNOWN')").is_err());
    }

    #[test]
    fn the_pvp_timer_reads_the_flag_and_its_end() {
        let mut s = vm();
        assert_eq!(s.eval::<f64>("return GetPVPTimer()").unwrap(), 0.0);
        s.set_wrath_pvp_timer(Some((false, None)));
        assert_eq!(s.eval::<f64>("return GetPVPTimer()").unwrap(), 301_000.0);
        assert!(s.eval::<bool>("return IsPVPTimerRunning() == nil").unwrap());
        s.set_now(10.0);
        s.set_wrath_pvp_timer(Some((true, Some(310.0))));
        assert_eq!(s.eval::<f64>("return IsPVPTimerRunning()").unwrap(), 1.0);
        assert_eq!(s.eval::<f64>("return GetPVPTimer()").unwrap(), 301_000.0);
        s.set_now(400.0);
        assert_eq!(s.eval::<f64>("return GetPVPTimer()").unwrap(), 999.0);
    }

    #[test]
    fn instance_info_reads_the_map_and_its_difficulty_row() {
        let mut s = vm();
        assert!(s
            .eval::<bool>(
                "local n, t, d, dn, m, dy, isd = GetInstanceInfo() \
                 return n == nil and t == 'none' and d == 0 and dn == '' and m == 0 and isd == nil"
            )
            .unwrap());
        s.run("RAID_DIFFICULTY_25PLAYER = '25 Player'").unwrap();
        s.set_wrath_instance_info(Some(WrathInstanceInfo {
            name: "Naxxramas".into(),
            instance_type: 2,
            difficulty: 1,
            map_difficulty: Some(("RAID_DIFFICULTY_25PLAYER".into(), 25)),
            dynamic_difficulty: 0,
            is_dynamic: false,
        }));
        assert!(s
            .eval::<bool>(
                "local n, t, d, dn, m, dy, isd = GetInstanceInfo() \
                 return n == 'Naxxramas' and t == 'raid' and d == 2 and dn == '25 Player' \
                 and m == 25 and dy == 0 and isd == false"
            )
            .unwrap());
    }

    #[test]
    fn the_bag_area_and_daily_readers_read_the_pushed_state() {
        let mut s = vm();
        let mut slots = std::collections::HashMap::new();
        slots.insert(2, crate::script::ContainerSlot::default());
        s.set_container(
            1,
            Some(crate::script::ContainerState {
                name: None,
                num_slots: 4,
                slots,
            }),
        );
        s.set_wrath_bag_family(1, 8);
        s.set_wrath_can_hearth_and_resurrect(true);
        s.set_wrath_daily_quests_completed(3);
        assert!(s
            .eval::<bool>(
                "local f, fam = GetContainerNumFreeSlots(1) \
                 return f == 3 and fam == 8 and GetContainerNumFreeSlots(3) == 0 \
                 and CanHearthAndResurrectFromArea() == 1 and GetDailyQuestsCompleted() == 3"
            )
            .unwrap());
        assert!(s.run("GetContainerNumFreeSlots()").is_err());
    }

    /// `0x5d7590`: the backpack answers its empty slots and family 0 even before App publishes
    /// it; the keyring and the closed bank answer a lone 0, the open bank its slots and 0.
    #[test]
    fn free_slots_follow_the_native_container_cases() {
        let mut s = vm();
        let free = |s: &UiScript, bag: i32| {
            s.eval::<Vec<f64>>(&format!("return {{ GetContainerNumFreeSlots({bag}) }}"))
                .unwrap()
        };
        assert_eq!(free(&s, 0), vec![16.0, 0.0]);
        assert_eq!(free(&s, -2), vec![0.0]);
        assert_eq!(free(&s, -1), vec![0.0]);
        assert_eq!(free(&s, 4), vec![0.0]);
        let mut slots = std::collections::HashMap::new();
        slots.insert(1, crate::script::ContainerSlot::default());
        for bag in [-2, -1, 0] {
            s.set_container(
                bag,
                Some(crate::script::ContainerState {
                    name: None,
                    num_slots: 28,
                    slots: slots.clone(),
                }),
            );
        }
        s.set_wrath_bag_family(-1, 8);
        assert_eq!(free(&s, 0), vec![27.0, 0.0]);
        assert_eq!(free(&s, -1), vec![27.0, 0.0], "the bank's family is 0");
        assert_eq!(free(&s, -2), vec![0.0], "the keyring answers a lone 0");
    }
}
