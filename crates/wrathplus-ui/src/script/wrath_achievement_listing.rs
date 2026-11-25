//! The category listings over the achievement table: `GetAchievementInfo(category, index)`'s order
//! and `GetCategoryNumAchievements`' counts, both walking Achievement.dbc in file order.

use super::wrath_achievement_progress::{
    compared, State, WrathAchievementProgress, ALL_ACHIEVEMENTS, ALL_STATISTICS, FEATS_OF_STRENGTH,
    FLAG_EARNED_ONLY, FLAG_HIDDEN, FLAG_STATISTIC,
};
use super::wrath_achievements::{WrathAchievement, WrathAchievementMetadata};

pub(super) fn in_category(row: &WrathAchievement, category: i64) -> bool {
    match category {
        ALL_ACHIEVEMENTS => row.flags & FLAG_STATISTIC == 0,
        ALL_STATISTICS => row.flags & FLAG_STATISTIC != 0,
        c => i64::from(row.category_id) == c,
    }
}

pub(super) fn faction_ok(row: &WrathAchievement, mine: &WrathAchievementProgress) -> bool {
    row.required_faction == -1 || mine.faction == -1 || row.required_faction == mine.faction
}

/// A later step of the row's series is in MY earned list, whichever list is being walked
/// (`0x5b37c0`).
pub(super) fn superseded(
    meta: &WrathAchievementMetadata,
    mine: &WrathAchievementProgress,
    id: u32,
) -> bool {
    meta.achievements
        .values()
        .any(|r| r.parent_achievement_id == id && mine.earned_date(r.id).is_some())
}

/// `GetAchievementInfo(category, index)`'s order (`0x5b3fc0`): earned by both, by the player, by
/// the compared player, then the table in file order, a series by its first unearned step.
pub(super) fn listing(meta: &WrathAchievementMetadata, state: &State, category: i64) -> Vec<u32> {
    let mine = &state.mine;
    let theirs = compared(state);
    let in_theirs = |id: u32| theirs.is_some_and(|p| p.earned_date(id).is_some());
    let pseudo = category == ALL_ACHIEVEMENTS || category == ALL_STATISTICS;
    let earned_pass = |id: u32| {
        meta.achievements.get(&id).is_some_and(|row| {
            in_category(row, category)
                && if pseudo {
                    row.flags & FLAG_HIDDEN == 0
                } else {
                    !superseded(meta, mine, id)
                }
        })
    };
    let mut out = Vec::new();
    for &(id, _) in &mine.earned {
        if in_theirs(id) && earned_pass(id) {
            out.push(id);
        }
    }
    for &(id, _) in &mine.earned {
        if !in_theirs(id) && earned_pass(id) {
            out.push(id);
        }
    }
    if let Some(theirs) = theirs {
        for &(id, _) in &theirs.earned {
            if mine.earned_date(id).is_none() && earned_pass(id) {
                out.push(id);
            }
        }
    }
    // The table pass lists nothing for Feats of Strength (`0x5b40d8`).
    if category == i64::from(FEATS_OF_STRENGTH) {
        return out;
    }
    let mut last = 0;
    for row in meta.achievements.values() {
        if !faction_ok(row, mine) || !in_category(row, category) {
            continue;
        }
        let earned = mine.earned_date(row.id).is_some() || in_theirs(row.id);
        if pseudo {
            // -2 does not skip earned rows (`0x5b4452`).
            let shown = row.flags & (FLAG_EARNED_ONLY | FLAG_HIDDEN) == 0;
            if shown
                && row.category_id != FEATS_OF_STRENGTH
                && (category == ALL_STATISTICS || !earned)
            {
                out.push(row.id);
            }
        } else if last != 0 && last == row.parent_achievement_id {
            // The next step of the series just listed or skipped moves the cursor (`0x5b4505`).
            last = row.id;
        } else if !earned && row.flags & FLAG_EARNED_ONLY == 0 {
            out.push(row.id);
            last = row.id;
        }
    }
    out
}

/// `GetCategoryNumAchievements` (`0x5b38e0`): `(total, completed)`.
pub(super) fn category_counts(
    meta: &WrathAchievementMetadata,
    state: &State,
    category: i64,
    all: bool,
) -> (i64, i64) {
    let mine = &state.mine;
    let theirs = compared(state);
    let all = all || category == ALL_ACHIEVEMENTS || category == ALL_STATISTICS;
    let (mut total, mut completed, mut theirs_only, mut last) = (0, 0, 0, 0);
    for row in meta.achievements.values() {
        if !in_category(row, category) || row.flags & FLAG_HIDDEN != 0 {
            continue;
        }
        let earned = mine.earned_date(row.id).is_some();
        let their = theirs.is_some_and(|p| p.earned_date(row.id).is_some());
        if !(faction_ok(row, mine) || their) {
            continue;
        }
        let listed = row.category_id != FEATS_OF_STRENGTH && row.flags & FLAG_EARNED_ONLY == 0;
        if !(listed || earned || their) {
            continue;
        }
        if last == 0 || last != row.parent_achievement_id {
            if earned {
                if !superseded(meta, mine, row.id) || all {
                    total += 1;
                    completed += 1;
                }
            } else if their {
                theirs_only += 1;
            } else {
                last = row.id;
                total += 1;
            }
        } else {
            last = row.id;
            if all {
                total += 1;
            } else if their {
                theirs_only += 1;
            }
        }
    }
    if category == i64::from(FEATS_OF_STRENGTH) {
        total = completed;
    }
    (total + theirs_only, completed)
}
