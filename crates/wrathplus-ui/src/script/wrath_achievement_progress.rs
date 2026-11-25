//! Build-12340 achievement progress: the player's earned list and criteria counters, a compared
//! player's, and the tracked list, over the static metadata in `wrath_achievements`. The bindings
//! follow the client's (`0x5b1390` `GetCategoryList` .. `0x5b48d0` `RemoveTrackedAchievement`,
//! table `0xacfc70`): the earned list is walked newest first, a compared player's earned list
//! (`0xacfddc`) joins the category listings, Feats of Strength (category 81) and flag `0x800`
//! rows count only once earned, a series shows its first unearned step, and Statistics' children
//! report the root as their parent. "Superseded" is a later step of the series in the player's
//! own earned list (`0x5b37c0`), the completed-count filter is "not a statistic" (`0x5b0d50`), and
//! `GetStatistic`'s text (`0x5b0ea0`) is the sum, maximum, met-count or last counter by the
//! achievement's flags, money-formatted for money criteria, `--` while zero.

use std::collections::BTreeMap;

use super::wrath_achievement_listing::{
    category_counts, faction_ok, in_category, listing, superseded,
};
use super::wrath_achievements::WrathAchievementMetadata;
use super::{binding_abi::number_arg, ScriptValue, UiScript};
use mlua::{Lua, MultiValue, Value};

pub(super) const FLAG_STATISTIC: u32 = 0x1;
pub(super) const FLAG_HIDDEN: u32 = 0x2;
const FLAG_SUM: u32 = 0x8;
const FLAG_MAX: u32 = 0x10;
const FLAG_MET_COUNT: u32 = 0x20;
/// Shown only once earned (`piVar[9] & 0x800`).
pub(super) const FLAG_EARNED_ONLY: u32 = 0x800;
pub(super) const FEATS_OF_STRENGTH: u32 = 81;
const STATISTICS_ROOT: u32 = 1;
/// `GetCategoryNumAchievements`/`GetAchievementInfo` pseudo-categories.
pub(super) const ALL_ACHIEVEMENTS: i64 = -1;
pub(super) const ALL_STATISTICS: i64 = -2;

/// One player's progress, as `SMSG_ALL_ACHIEVEMENT_DATA` / `SMSG_RESPOND_INSPECT_ACHIEVEMENTS`
/// and later updates leave it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathAchievementProgress {
    /// Earned achievements with their packed dates, in the client's list order (`0x5b28d0`).
    pub earned: Vec<(u32, u32)>,
    /// Criteria counters by criteria id.
    pub criteria: BTreeMap<u32, u64>,
    /// The player's achievement faction: 0 Horde, 1 Alliance, -1 unknown.
    pub faction: i32,
    /// The player's own data has arrived this session (`[0xc1f454]`, set by the all-data and
    /// criteria-update handlers at `0x5b32fc` and `0x5b316c`).
    pub received: bool,
}

impl WrathAchievementProgress {
    pub(super) fn earned_date(&self, id: u32) -> Option<u32> {
        self.earned.iter().find(|(e, _)| *e == id).map(|(_, d)| *d)
    }
}

/// A host request from the achievement window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathAchievementRequest {
    /// `SetAchievementComparisonUnit`: inspect this player's achievements.
    Compare { guid: u64 },
}

#[derive(Default)]
pub(super) struct State {
    pub(super) mine: WrathAchievementProgress,
    /// The list has arrived (`CanShowAchievementUI`, `0xc1f454`).
    received: bool,
    compared: Option<(u64, Option<WrathAchievementProgress>)>,
    /// Oldest first (`[0xacfde8]`).
    pub(super) tracked: Vec<u32>,
    requests: Vec<WrathAchievementRequest>,
}

impl UiScript {
    /// The player's progress, full replacement; the packets' events are queued on their own.
    pub fn set_wrath_achievement_progress(&mut self, progress: WrathAchievementProgress) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            state.received = progress.received;
            state.mine = progress;
        }
    }

    /// `SMSG_ALL_ACHIEVEMENT_DATA` (`0x5b32f0`), after its progress is published: each timed
    /// criterion's `TRACKED_ACHIEVEMENT_UPDATE`, then `CRITERIA_UPDATE` and
    /// `RECEIVED_ACHIEVEMENT_LIST` (`0x5b3471`, `0x5b347d`). `criteria` is `(id, elapsed)`.
    pub fn wrath_achievement_list_arrived(&mut self, criteria: &[(u32, u32)]) {
        for &(id, elapsed) in criteria {
            super::wrath_achievement_tracked::criteria_progress(&self.lua, id, elapsed, false);
        }
        self.queue_event("CRITERIA_UPDATE", Vec::new());
        self.queue_event("RECEIVED_ACHIEVEMENT_LIST", Vec::new());
    }

    /// `SMSG_CRITERIA_UPDATE` (`0x5b3160`): `CRITERIA_UPDATE` for every packet, then the tracked
    /// update its criterion calls for.
    pub fn wrath_criteria_updated(&mut self, criteria: u32, elapsed: u32) {
        self.queue_event("CRITERIA_UPDATE", Vec::new());
        super::wrath_achievement_tracked::criteria_progress(&self.lua, criteria, elapsed, true);
    }

    /// `SMSG_CRITERIA_DELETED` that removed a counter: `CRITERIA_UPDATE` (`0x5b3764`).
    pub fn wrath_criteria_deleted(&mut self) {
        self.queue_event("CRITERIA_UPDATE", Vec::new());
    }

    /// The player's own earn, after its progress is published: off the tracked list, then
    /// `ACHIEVEMENT_EARNED(id)` (`0x5b3136`, `0x5b3148`).
    pub fn wrath_achievement_earned(&mut self, id: u32) {
        super::wrath_achievement_tracked::untrack(&self.lua, id);
        self.queue_event("ACHIEVEMENT_EARNED", vec![ScriptValue::Int(id.into())]);
    }

    /// `SMSG_ACHIEVEMENT_DELETED` that removed an earn fires `ACHIEVEMENT_EARNED(id)` too
    /// (`0x5b3687`).
    pub fn wrath_achievement_deleted(&mut self, id: u32) {
        self.queue_event("ACHIEVEMENT_EARNED", vec![ScriptValue::Int(id.into())]);
    }

    /// `SMSG_RESPOND_INSPECT_ACHIEVEMENTS` (`0x5b34a0`): kept for the compared player only
    /// (`0x5b2ab0`), and `INSPECT_ACHIEVEMENT_READY` fires for every response, with no argument
    /// (`0x5b35e8`).
    pub fn set_wrath_achievement_comparison(
        &mut self,
        guid: u64,
        progress: WrathAchievementProgress,
    ) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            if state.compared.as_ref().is_some_and(|(g, _)| *g == guid) {
                state.compared = Some((guid, Some(progress)));
            }
        }
        self.queue_event("INSPECT_ACHIEVEMENT_READY", Vec::new());
    }

    pub fn take_wrath_achievement_requests(&mut self) -> Vec<WrathAchievementRequest> {
        self.lua
            .app_data_mut::<State>()
            .map(|mut s| std::mem::take(&mut s.requests))
            .unwrap_or_default()
    }
}

/// `(quantity, completed)` for a criterion, from the player's counters.
pub(super) fn criterion_progress(lua: &Lua, criterion: u32, required: i32) -> (i32, bool) {
    let counter = lua
        .app_data_ref::<State>()
        .and_then(|s| s.mine.criteria.get(&criterion).copied())
        .unwrap_or(0);
    let quantity = i32::try_from(counter).unwrap_or(i32::MAX);
    (quantity, counter > 0 && quantity >= required)
}

/// `GetAchievementLink`'s fields after the guid (`0x5b6110`): the earned flag, the date (`0:0:-1`
/// unset, `0x76c190`), and a bit per visible criterion of the owner `owner` that my counter has
/// met, in list order (`0x5b61ee`-`0x5b62fa`); all bits are set for an earn whose owner needs no
/// count (`0x5b630f`).
pub(super) fn link_fields(
    lua: &Lua,
    meta: &WrathAchievementMetadata,
    id: u32,
    owner: u32,
) -> String {
    let state = lua.app_data_ref::<State>();
    let mine = state.as_ref().map(|s| &s.mine);
    let mut bits = [0u32; 4];
    let visible = meta
        .linked_criteria
        .get(&owner)
        .into_iter()
        .flatten()
        .filter_map(|c| meta.criteria.get(c))
        .filter(|c| c.flags & 2 == 0);
    for (i, c) in visible.enumerate().take(128) {
        // The required quantity is sign-extended and compared unsigned (`cdq`, `0x5b6256`).
        let required = c.required_quantity as i32 as i64 as u64;
        if mine
            .and_then(|m| m.criteria.get(&c.id))
            .is_some_and(|&n| n >= required)
        {
            bits[i / 32] |= 1 << (i % 32);
        }
    }
    let (earned, (m, d, y)) = match mine.and_then(|m| m.earned_date(id)) {
        Some(packed) => {
            if meta
                .achievements
                .get(&owner)
                .is_some_and(|r| r.required_criteria_count == 0)
            {
                bits = [u32::MAX; 4];
            }
            (1, date(packed))
        }
        None => (0, (0, 0, -1)),
    };
    let [b0, b1, b2, b3] = bits;
    format!("{earned}:{m}:{d}:{y}:{b0}:{b1}:{b2}:{b3}")
}

/// The packed date's `(month, day, year)` as the bindings push them: 1-based month and day, the
/// year since 2000 (AzerothCore `secsToTimeBitFields`).
fn date(packed: u32) -> (i64, i64, i64) {
    (
        i64::from((packed >> 20) & 0xf) + 1,
        i64::from((packed >> 14) & 0x3f) + 1,
        i64::from((packed >> 24) & 0x1f),
    )
}

fn with<R>(lua: &Lua, f: impl FnOnce(&WrathAchievementMetadata, &State) -> R) -> Option<R> {
    let model = lua.app_data_ref::<super::Model>()?;
    let meta = model.wrath_achievements.as_ref()?;
    let state = lua.app_data_ref::<State>()?;
    Some(f(meta, &state))
}

pub(super) fn compared(state: &State) -> Option<&WrathAchievementProgress> {
    state.compared.as_ref().and_then(|(_, p)| p.as_ref())
}

/// The 11 values of `GetAchievementInfo` (`0x5b3fc0`), or none for a hidden or unearned
/// earned-only row.
fn info(
    lua: &Lua,
    meta: &WrathAchievementMetadata,
    state: &State,
    id: u32,
) -> mlua::Result<MultiValue> {
    let Some(row) = meta.achievements.get(&id) else {
        return Ok(MultiValue::new());
    };
    let earned = state.mine.earned_date(id);
    if row.flags & FLAG_HIDDEN != 0 || (row.flags & FLAG_EARNED_ONLY != 0 && earned.is_none()) {
        return Ok(MultiValue::new());
    }
    let mut v = vec![
        Value::Integer(id.into()),
        Value::String(lua.create_string(&row.name)?),
        Value::Integer(row.points.into()),
    ];
    match earned {
        Some(packed) => {
            let (m, d, y) = date(packed);
            v.extend([
                Value::Boolean(true),
                Value::Integer(m),
                Value::Integer(d),
                Value::Integer(y),
            ]);
        }
        None => v.extend([Value::Boolean(false), Value::Nil, Value::Nil, Value::Nil]),
    }
    v.extend([
        Value::String(lua.create_string(&row.description)?),
        Value::Integer(row.flags.into()),
        Value::String(lua.create_string(&row.icon)?),
        Value::String(lua.create_string(&row.reward)?),
    ]);
    Ok(MultiValue::from_vec(v))
}

/// `GetStatistic`/`GetComparisonStatistic` (`0x5b5620`, `0x5b5e70`).
fn statistic(
    lua: &Lua,
    meta: &WrathAchievementMetadata,
    progress: &WrathAchievementProgress,
    id: u32,
) -> mlua::Result<MultiValue> {
    let Some(row) = meta.achievements.get(&id) else {
        return Ok(MultiValue::new());
    };
    let owner = meta.criteria_owner(row).unwrap_or(row.id);
    let mut sum = 0u64;
    let mut max = 0u64;
    let mut met = 0u64;
    let mut last = None;
    let mut money = false;
    for criterion in meta
        .linked_criteria
        .get(&owner)
        .into_iter()
        .flatten()
        .filter_map(|c| meta.criteria.get(c))
    {
        let Some(&counter) = progress.criteria.get(&criterion.id) else {
            continue;
        };
        sum = sum.saturating_add(counter);
        max = max.max(counter);
        if counter >= u64::from(criterion.required_quantity) {
            met += 1;
        }
        last.get_or_insert(counter);
        money |= criterion.flags & 0x20 != 0;
    }
    let value = if row.flags & FLAG_SUM != 0 {
        sum
    } else if row.flags & FLAG_MAX != 0 {
        max
    } else if row.flags & FLAG_MET_COUNT != 0 {
        met
    } else {
        last.unwrap_or(0)
    };
    let value = i32::try_from(value).unwrap_or(i32::MAX);
    let text = if value == 0 {
        "--".to_string()
    } else if money {
        super::wrath_achievements::money(lua, value)?
    } else {
        value.to_string()
    };
    Ok(MultiValue::from_vec(vec![Value::String(
        lua.create_string(text)?,
    )]))
}

fn ids(lua: &Lua, ids: impl IntoIterator<Item = u32>) -> MultiValue {
    let _ = lua;
    MultiValue::from_vec(
        ids.into_iter()
            .map(|id| Value::Integer(id.into()))
            .collect(),
    )
}

fn category_list(lua: &Lua, statistics: bool) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    let list = with(lua, |meta, _| {
        let parent = |id: u32| {
            meta.categories
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.parent_id)
        };
        meta.categories
            .iter()
            .filter(|c| {
                // Whether the category sits under Statistics, walking its parents.
                let mut at = c.parent_id;
                let mut under = false;
                for _ in 0..16 {
                    if at == STATISTICS_ROOT as i32 {
                        under = true;
                        break;
                    }
                    match u32::try_from(at).ok().and_then(parent) {
                        Some(p) => at = p,
                        None => break,
                    }
                }
                if statistics {
                    under
                } else {
                    c.id != STATISTICS_ROOT && !under
                }
            })
            .map(|c| c.id)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    for (i, id) in list.into_iter().enumerate() {
        table.raw_set(i + 1, id)?;
    }
    Ok(Value::Table(table))
}

fn guid_of(lua: &Lua, unit: &str) -> Option<u64> {
    let text: Option<String> = lua
        .globals()
        .get::<mlua::Function>("UnitGUID")
        .ok()?
        .call(unit)
        .ok()?;
    let text = text?;
    u64::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let g = lua.globals();
    g.set(
        "CanShowAchievementUI",
        lua.create_function(|lua, ()| Ok(lua.app_data_ref::<State>().is_some_and(|s| s.received)))?,
    )?;
    g.set(
        "HasCompletedAnyAchievement",
        lua.create_function(|lua, ()| {
            Ok(lua
                .app_data_ref::<State>()
                .is_some_and(|s| !s.mine.earned.is_empty()))
        })?,
    )?;
    g.set(
        "GetCategoryList",
        lua.create_function(|lua, ()| category_list(lua, false))?,
    )?;
    g.set(
        "GetStatisticsCategoryList",
        lua.create_function(|lua, ()| category_list(lua, true))?,
    )?;
    g.set(
        "GetCategoryInfo",
        lua.create_function(|lua, id: Value| {
            // Truncated: `0x5b15c0` sets the x87 control word to chop before `fistp`.
            let id = lua
                .coerce_number(id)?
                .ok_or_else(|| {
                    mlua::Error::RuntimeError("Usage: GetCategoryInfo(categoryID)".into())
                })?
                .trunc() as i64;
            let row = with(lua, |meta, _| {
                meta.categories
                    .iter()
                    .find(|c| i64::from(c.id) == id)
                    .cloned()
            })
            .flatten();
            Ok(match row {
                Some(c) => MultiValue::from_vec(vec![
                    Value::String(lua.create_string(&c.name)?),
                    // Statistics' children report a root parent.
                    Value::Integer(if c.parent_id == STATISTICS_ROOT as i32 {
                        -1
                    } else {
                        c.parent_id.into()
                    }),
                    Value::Integer(0),
                ]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    g.set(
        "GetCategoryNumAchievements",
        lua.create_function(|lua, (id, all): (Value, Value)| {
            let id = number_arg(lua, id, "Usage: GetCategoryNumAchievements(categoryID)")? as i64;
            let all = matches!(all, Value::Boolean(true));
            let (total, completed) =
                with(lua, |meta, state| category_counts(meta, state, id, all)).unwrap_or((0, 0));
            Ok((total, completed))
        })?,
    )?;
    g.set(
        "GetComparisonCategoryNumAchievements",
        lua.create_function(|lua, id: Value| {
            let id = number_arg(
                lua,
                id,
                "Usage: GetComparisonCategoryNumAchievements(categoryID)",
            )? as i64;
            let all = id == ALL_ACHIEVEMENTS || id == ALL_STATISTICS;
            Ok(with(lua, |meta, state| {
                let Some(theirs) = compared(state) else {
                    return 0;
                };
                meta.achievements
                    .values()
                    .filter(|r| in_category(r, id) && r.flags & FLAG_HIDDEN == 0)
                    .filter(|r| theirs.earned_date(r.id).is_some())
                    .filter(|r| all || !superseded(meta, theirs, r.id))
                    .count() as i64
            })
            .unwrap_or(0))
        })?,
    )?;
    g.set(
        "GetAchievementInfo",
        lua.create_function(|lua, (first, index): (Value, Value)| {
            let first = number_arg(lua, first, "Usage: GetAchievementInfo(achievementID)")?;
            // `lua_isnumber(2)` picks the `(category, index)` form (`0x5b3fe9`).
            let id = match lua.coerce_number(index)? {
                None => Some(first as u32),
                Some(index) => {
                    let index = index as i64;
                    with(lua, |meta, state| {
                        let list = listing(meta, state, first as i64);
                        usize::try_from(index - 1)
                            .ok()
                            .and_then(|i| list.get(i).copied())
                    })
                    .flatten()
                }
            };
            let Some(id) = id else {
                return Ok(MultiValue::new());
            };
            with(lua, |meta, state| info(lua, meta, state, id))
                .unwrap_or_else(|| Ok(MultiValue::new()))
        })?,
    )?;
    g.set(
        "GetAchievementInfoFromCriteria",
        lua.create_function(|lua, id: Value| {
            let id =
                number_arg(lua, id, "Usage: GetAchievementInfoFromCriteria(criteriaID)")? as u32;
            with(lua, |meta, _| -> mlua::Result<MultiValue> {
                let Some(row) = meta
                    .criteria
                    .get(&id)
                    .and_then(|c| meta.achievements.get(&c.achievement_id))
                else {
                    return Ok(MultiValue::new());
                };
                // Native pushes these seven, without the completion fields (`0x5b1720`).
                Ok(MultiValue::from_vec(vec![
                    Value::Integer(row.id.into()),
                    Value::String(lua.create_string(&row.name)?),
                    Value::Integer(row.points.into()),
                    Value::String(lua.create_string(&row.description)?),
                    Value::Integer(row.flags.into()),
                    Value::String(lua.create_string(&row.icon)?),
                    Value::String(lua.create_string(&row.reward)?),
                ]))
            })
            .unwrap_or_else(|| Ok(MultiValue::new()))
        })?,
    )?;
    g.set(
        "GetNumCompletedAchievements",
        lua.create_function(|lua, ()| {
            Ok(with(lua, |meta, state| {
                let (mut total, mut completed) = (0i64, 0i64);
                for row in meta.achievements.values() {
                    if row.flags & (FLAG_STATISTIC | FLAG_HIDDEN) != 0
                        || !faction_ok(row, &state.mine)
                        || row.category_id == FEATS_OF_STRENGTH
                    {
                        continue;
                    }
                    if state.mine.earned_date(row.id).is_some() {
                        completed += 1;
                    } else if row.flags & FLAG_EARNED_ONLY != 0 {
                        continue;
                    }
                    total += 1;
                }
                (total, completed)
            })
            .unwrap_or((0, 0)))
        })?,
    )?;
    for (name, theirs) in [
        ("GetTotalAchievementPoints", false),
        ("GetComparisonAchievementPoints", true),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, ()| {
                Ok(with(lua, |meta, state| {
                    let progress = if theirs {
                        compared(state)
                    } else {
                        Some(&state.mine)
                    };
                    progress.map_or(0, |p| {
                        p.earned
                            .iter()
                            .filter_map(|(id, _)| meta.achievements.get(id))
                            .map(|r| i64::from(r.points))
                            .sum::<i64>()
                    })
                })
                .unwrap_or(0))
            })?,
        )?;
    }
    g.set(
        "GetLatestCompletedAchievements",
        lua.create_function(|lua, ()| {
            let latest = with(lua, |meta, state| {
                state
                    .mine
                    .earned
                    .iter()
                    .map(|(id, _)| *id)
                    .filter(|id| {
                        meta.achievements
                            .get(id)
                            .is_some_and(|r| r.flags & FLAG_STATISTIC == 0)
                    })
                    .take(5)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
            Ok(ids(lua, latest))
        })?,
    )?;
    g.set(
        "GetNextAchievement",
        lua.create_function(|lua, id: Value| {
            let id = number_arg(lua, id, "Usage: GetNextAchievement(achievementID)")? as u32;
            Ok(with(lua, |meta, state| {
                let Some(next) = meta
                    .achievements
                    .values()
                    .find(|r| r.parent_achievement_id == id)
                else {
                    return MultiValue::new();
                };
                let mut v = vec![Value::Integer(next.id.into())];
                if state.mine.earned_date(next.id).is_some() {
                    v.push(Value::Boolean(true));
                }
                MultiValue::from_vec(v)
            })
            .unwrap_or_default())
        })?,
    )?;
    g.set(
        "GetPreviousAchievement",
        lua.create_function(|lua, id: Value| {
            let id = number_arg(lua, id, "Usage: GetPreviousAchievement(achievementID)")? as u32;
            let previous = with(lua, |meta, _| {
                meta.achievements
                    .get(&id)
                    .map(|r| r.parent_achievement_id)
                    .filter(|&p| p != 0)
            })
            .flatten();
            Ok(ids(lua, previous))
        })?,
    )?;
    for (name, theirs, usage) in [
        ("GetStatistic", false, "Usage: GetStatistic(achievementID)"),
        (
            "GetComparisonStatistic",
            true,
            "Usage: GetComparisonStatistic(achievementID)",
        ),
    ] {
        g.set(
            name,
            lua.create_function(move |lua, id: Value| {
                let id = number_arg(lua, id, usage)? as u32;
                with(lua, |meta, state| {
                    let empty = WrathAchievementProgress::default();
                    let progress = if theirs {
                        compared(state).unwrap_or(&empty)
                    } else {
                        &state.mine
                    };
                    statistic(lua, meta, progress, id)
                })
                .unwrap_or_else(|| Ok(MultiValue::new()))
            })?,
        )?;
    }
    g.set(
        "GetAchievementComparisonInfo",
        lua.create_function(|lua, id: Value| {
            let id = number_arg(
                lua,
                id,
                "Usage: GetAchievementComparisonInfo(achievementID)",
            )? as u32;
            Ok(with(lua, |meta, state| {
                let (Some(packed), Some(row)) = (
                    compared(state).and_then(|p| p.earned_date(id)),
                    meta.achievements.get(&id),
                ) else {
                    return MultiValue::new();
                };
                let (m, d, y) = date(packed);
                MultiValue::from_vec(vec![
                    Value::Boolean(true),
                    Value::Integer(m),
                    Value::Integer(d),
                    Value::Integer(y),
                    Value::Integer(row.flags.into()),
                ])
            })
            .unwrap_or_default())
        })?,
    )?;
    g.set(
        "SetAchievementComparisonUnit",
        lua.create_function(|lua, unit: Value| {
            let unit = super::binding_abi::string_arg(
                lua,
                unit,
                "Usage: AddAchievementComparisonUnit(unitToken)",
            )?;
            let Some(guid) = guid_of(lua, &unit).filter(|&g| g != 0) else {
                return Ok(MultiValue::new());
            };
            if let Some(mut state) = lua.app_data_mut::<State>() {
                state.compared = Some((guid, None));
                state
                    .requests
                    .push(WrathAchievementRequest::Compare { guid });
            }
            Ok(MultiValue::from_vec(vec![Value::Integer(1)]))
        })?,
    )?;
    g.set(
        "ClearAchievementComparisonUnit",
        lua.create_function(|lua, ()| {
            if let Some(mut state) = lua.app_data_mut::<State>() {
                state.compared = None;
            }
            Ok(())
        })?,
    )?;
    super::wrath_achievement_tracked::install(lua)
}

#[cfg(test)]
#[path = "wrath_achievement_progress_tests.rs"]
mod tests;
