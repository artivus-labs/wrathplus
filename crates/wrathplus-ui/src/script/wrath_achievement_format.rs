//! Build 12340's one progress-text formatter (`0x5b0ea0`) and the tallies its two callers make:
//! `GetAchievementCriteriaInfo` (`0x5b58b0`) and `GetStatistic`/`GetComparisonStatistic`
//! (`0x5b5620`, `0x5b5e70`). Counters are 64-bit throughout; a money or date text reads the low
//! dword, as the formatter pushes it.

use super::wrath_achievement_progress::{State, WrathAchievementProgress, WrathCriteriaProgress};
use super::wrath_achievements::{
    money, WrathAchievement, WrathAchievementCriterion, WrathAchievementMetadata,
};
use mlua::{Lua, MultiValue, Value};

const STATISTIC: u32 = 0x1;
const SUM: u32 = 0x8;
const MAX: u32 = 0x10;
const MET: u32 = 0x20;
const PER_DAY: u32 = 0x40;
const BAR: u32 = 0x80;
/// A tied maximum names no criterion (`0x5b5b65`, `0x5b57fb`).
const TIE_UNNAMED: u32 = 0x400;
/// Criteria flags: shown as a bar with its required quantity, money, a packed date.
const CRIT_BAR: u32 = 0x1;
const CRIT_DATE: u32 = 0x10;
const CRIT_MONEY: u32 = 0x20;

/// The walk over an owner's criteria that have a counter (`0x5b5aa4`-`0x5b5b6e`,
/// `0x5b56d1`-`0x5b57ea`): the sum, the maximum and its criterion (`u32::MAX` for none), the met
/// count, the last counter in list order with its criterion's flags, and the earliest dated one.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tally {
    sum: u64,
    max: u64,
    max_id: u32,
    met: u64,
    last: u64,
    last_flags: u32,
    oldest: Option<(u32, WrathCriteriaProgress)>,
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            sum: 0,
            max: 0,
            max_id: u32::MAX,
            met: 0,
            last: 0,
            last_flags: 0,
            oldest: None,
        }
    }
}

/// The required quantity, sign-extended and compared unsigned (`cdq`, `0x5b5b33`).
fn required(c: &WrathAchievementCriterion) -> u64 {
    c.required_quantity as i32 as i64 as u64
}

/// `rule` is the row whose flag `0x400` unnames a tied maximum.
fn tally(
    meta: &WrathAchievementMetadata,
    progress: &WrathAchievementProgress,
    owner: u32,
    rule: u32,
) -> Tally {
    let mut t = Tally::default();
    let mut tie = false;
    let criteria = meta.linked_criteria.get(&owner).into_iter().flatten();
    for c in criteria.filter_map(|c| meta.criteria.get(c)) {
        let Some(n) = progress.criteria.get(&c.id) else {
            continue;
        };
        t.sum = t.sum.wrapping_add(n.counter);
        if n.counter > t.max {
            (t.max, t.max_id, tie) = (n.counter, c.id, false);
        } else if n.counter == t.max {
            tie = true;
        }
        t.met += u64::from(n.counter >= required(c));
        (t.last, t.last_flags) = (n.counter, c.flags);
        if t.oldest.is_none_or(|(_, o)| n.dated < o.dated) {
            t.oldest = Some((c.id, *n));
        }
    }
    if tie && rule & TIE_UNNAMED != 0 {
        t.max_id = 0;
    }
    t
}

/// What the formatter is shown besides the tally (its arguments `+0x2c`..`+0x4c`).
struct Shown<'a> {
    ach: &'a WrathAchievement,
    crit: Option<&'a WrathAchievementCriterion>,
    crit_flags: u32,
    earned: bool,
    /// The required quantity in place of the criterion's, 0 for none.
    required: u32,
    counter: u64,
    node: Option<(u32, WrathCriteriaProgress)>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Whole days since the node's date, unsigned (`mul 0xc22e4507`, `shr edx,0x10`: `/86400`).
fn days(node: Option<(u32, WrathCriteriaProgress)>) -> u32 {
    node.map_or(0, |(_, n)| {
        (now() as u32).wrapping_sub(n.dated as u32) / 86400
    })
}

/// `0x5b0ea0`: the text and the value it shows, by the achievement's flags.
fn format(
    lua: &Lua,
    meta: &WrathAchievementMetadata,
    t: &Tally,
    s: &Shown,
) -> mlua::Result<(String, u64)> {
    let flags = s.ach.flags;
    let zero = || if flags & STATISTIC != 0 { "--" } else { "0" }.to_string();
    let int = |v: u64| (v as i64).to_string();
    let cash = |v: u64| money(lua, v as u32 as i32);
    let (mut text, v) = if flags & SUM != 0 {
        let (mut v, mut avg) = (t.sum, 0.0);
        let d = days(s.node);
        if flags & PER_DAY != 0 && d != 0 {
            avg = v as f64 / f64::from(d);
            v = avg as i64 as u64;
        }
        let text = if s.crit_flags & CRIT_MONEY != 0 {
            cash(v)?
        } else if avg != 0.0 {
            format!("{avg:.2}")
        } else if v == 0 {
            zero()
        } else {
            int(v)
        };
        (text, v)
    } else if flags & MAX != 0 {
        let text = match meta.criteria.get(&t.max_id) {
            Some(c) if flags & STATISTIC != 0 => format!("{} ({})", t.max as i64, c.description),
            None if t.max == 0 => zero(),
            _ => int(t.max),
        };
        (text, t.max)
    } else if flags & MET != 0 {
        (if t.met == 0 { zero() } else { int(t.met) }, t.met)
    } else if s.counter == 0 {
        // An earn with no counter shows the criterion met when no count is asked (`0x5b1214`).
        match s
            .crit
            .filter(|_| s.earned && s.ach.required_criteria_count == 0)
        {
            Some(c) => {
                let v = required(c);
                let text = if v == 0 {
                    zero()
                } else if s.crit_flags & CRIT_MONEY != 0 {
                    cash(v)?
                } else {
                    int(v)
                };
                (text, v)
            }
            None if s.crit_flags & CRIT_MONEY != 0 && s.crit_flags & CRIT_BAR != 0 => (cash(0)?, 0),
            None => (zero(), 0),
        }
    } else {
        let mut v = s.counter;
        if let Some(c) = s.crit.filter(|_| flags & STATISTIC == 0) {
            v = v.min(required(c));
        }
        let mut avg = 0.0f32;
        let d = days(s.node);
        if flags & PER_DAY != 0 && d != 0 {
            let full = v as f64 / f64::from(d);
            (avg, v) = (full as f32, full as i64 as u64);
        }
        // The node's own criterion picks a date or money text (`0x5b1137`-`0x5b11b2`).
        let row = s.node.and_then(|(id, _)| meta.criteria.get(&id));
        let text = match row.map_or(0, |r| r.flags) {
            f if f & CRIT_DATE != 0 => {
                let (m, d, y) = super::wrath_achievement_progress::date(v as u32);
                format!("{m:02}/{d:02}/{}", y + 2000)
            }
            f if f & CRIT_MONEY != 0 => cash(v)?,
            _ if flags & STATISTIC != 0 && avg != 0.0 => format!("{:.2}", f64::from(avg)),
            _ if flags & STATISTIC != 0 && v == 0 => "--".to_string(),
            _ => int(v),
        };
        (text, v)
    };
    if let Some(c) = s.crit.filter(|_| s.crit_flags & CRIT_BAR != 0) {
        let req = if s.required != 0 {
            s.required
        } else {
            c.required_quantity
        };
        let req = if s.crit_flags & CRIT_MONEY != 0 {
            money(lua, req as i32)?
        } else {
            (req as i32).to_string()
        };
        text = format!("{text} / {req}");
    }
    Ok((text, v))
}

/// `GetStatistic`/`GetComparisonStatistic`: the formatter over the owner's tally, shown the last
/// counter with its criterion's flags and the earliest dated node with its criterion.
pub(super) fn statistic(
    lua: &Lua,
    meta: &WrathAchievementMetadata,
    progress: &WrathAchievementProgress,
    id: u32,
) -> mlua::Result<MultiValue> {
    let Some(row) = meta.achievements.get(&id) else {
        return Ok(MultiValue::new());
    };
    let owner = meta.criteria_owner(row);
    let rule = owner
        .and_then(|o| meta.achievements.get(&o))
        .map_or(0, |r| r.flags);
    let t = owner.map_or_else(Tally::default, |o| tally(meta, progress, o, rule));
    let shown = Shown {
        ach: row,
        crit: t.oldest.and_then(|(c, _)| meta.criteria.get(&c)),
        crit_flags: t.last_flags,
        earned: false,
        required: 0,
        counter: t.last,
        node: t.oldest,
    };
    let (text, _) = format(lua, meta, &t, &shown)?;
    Ok(MultiValue::from_vec(vec![Value::String(
        lua.create_string(text)?,
    )]))
}

/// `GetAchievementCriteriaInfo(achievementID, index)` or `(criteriaID)`'s ten values.
pub(super) fn criteria_info(lua: &Lua, first: u32, index: Option<i32>) -> mlua::Result<MultiValue> {
    let model = lua.app_data_ref::<super::Model>().expect("model");
    let state = lua.app_data_ref::<State>();
    let (Some(meta), Some(state)) = (model.wrath_achievements.as_ref(), state.as_ref()) else {
        return Ok(MultiValue::new());
    };
    let mine = &state.mine;
    let ((ach_id, c), bar_index) = match index {
        Some(index) => match meta.indexed_criterion(first, index) {
            // Each criterion the index form visits sets the bar flag (`0x5b5985`).
            Some((a, c)) => ((a.id, c), a.flags & 0xb0 != 0),
            None => return Ok(MultiValue::new()),
        },
        None => match meta.criteria.get(&first) {
            Some(c) => ((c.achievement_id, c), false),
            None => return Ok(MultiValue::new()),
        },
    };
    let node = mine.criteria.get(&c.id).copied();
    let Some(ach) = meta.achievements.get(&ach_id).filter(|a| a.flags & 2 == 0) else {
        return Ok(MultiValue::new());
    };
    let earned = mine.earned_date(ach_id).is_some();
    let mut t = meta
        .criteria_owner(ach)
        .map_or_else(Tally::default, |o| tally(meta, mine, o, ach.flags));
    let (mut crit_flags, mut override_) = (c.flags, 0);
    // `0x5b5b86`-`0x5b5ba7`: a bar achievement marks the criterion, an earned sum shows its
    // required quantity, and a met-count bar asks for the achievement's count.
    if ach.flags & BAR != 0 {
        crit_flags |= CRIT_BAR;
        if earned && ach.flags & SUM != 0 {
            t.sum = required(c);
        }
        if ach.flags & MET != 0 {
            override_ = ach.required_criteria_count;
        }
    }
    let shown = Shown {
        ach,
        crit: Some(c),
        crit_flags,
        earned,
        required: override_,
        counter: node.map_or(0, |n| n.counter),
        node: node.map(|n| (c.id, n)),
    };
    let (text, value) = format(lua, meta, &t, &shown)?;
    let required_out = if override_ != 0 {
        override_ as i32
    } else {
        c.required_quantity as i32
    };
    let met_by_earn = earned && ach.required_criteria_count == 0;
    let (completed, quantity, name) = match node.filter(|_| !bar_index) {
        Some(n) => (
            met_by_earn || n.counter >= required(c),
            value as u32 as i32,
            super::tooltip_links::known_name(&model, n.player),
        ),
        None if met_by_earn => (true, required_out, None),
        None => (false, value as u32 as i32, None),
    };
    let name = match name {
        Some(name) => Value::String(lua.create_string(name)?),
        None => Value::Nil,
    };
    Ok(MultiValue::from_vec(vec![
        Value::String(lua.create_string(&c.description)?),
        Value::Integer(i64::from(c.criteria_type)),
        Value::Boolean(completed),
        Value::Integer(quantity.into()),
        Value::Integer(required_out.into()),
        name,
        Value::Integer(i64::from(crit_flags)),
        Value::Integer(i64::from(c.asset_id)),
        Value::String(lua.create_string(text)?),
        Value::Integer(i64::from(c.id)),
    ]))
}

#[cfg(test)]
pub(super) mod probe {
    //! The formatter on a bare achievement and criterion, for the money and bar texts.
    use super::*;

    pub(in crate::script) fn text(
        lua: &Lua,
        ach_flags: u32,
        crit_flags: u32,
        counter: u64,
        required: i32,
    ) -> String {
        let meta = WrathAchievementMetadata::default();
        let ach = WrathAchievement {
            flags: ach_flags,
            ..Default::default()
        };
        let crit = WrathAchievementCriterion {
            required_quantity: required as u32,
            ..Default::default()
        };
        let shown = Shown {
            ach: &ach,
            crit: Some(&crit),
            crit_flags,
            earned: false,
            required: 0,
            counter,
            node: None,
        };
        format(lua, &meta, &Tally::default(), &shown).unwrap().0
    }
}
