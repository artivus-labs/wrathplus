use super::*;
use crate::script::wrath_achievements::{
    WrathAchievement, WrathAchievementCategory, WrathAchievementCriterion, WrathAchievementMetadata,
};

/// 2009-10-17, packed as AzerothCore `secsToTimeBitFields` does (0-based month and day).
const OCT_17_2009: u32 = (9 << 24) | (9 << 20) | (16 << 14);

fn achievement(id: u32, category: u32, parent: u32, faction: i32, flags: u32) -> WrathAchievement {
    WrathAchievement {
        id,
        name: format!("A{id}"),
        category_id: category,
        flags,
        required_faction: faction,
        parent_achievement_id: parent,
        description: format!("D{id}"),
        points: id,
        icon: "Interface\\Icons\\Achievement_General".into(),
        reward: String::new(),
        ..Default::default()
    }
}

fn script() -> UiScript {
    let mut ui = UiScript::new().unwrap();
    let mut meta = WrathAchievementMetadata::default();
    // Out of id order, as Achievement.dbc is: the listing walks the file.
    for row in [
        achievement(10, 92, 0, -1, 0),
        achievement(21, 92, 0, 1, 0),
        achievement(11, 92, 10, -1, 0),
        achievement(12, 92, 11, -1, 0),
        achievement(13, 92, 12, -1, 0),
        achievement(20, 92, 0, 0, 0),
        achievement(30, 81, 0, -1, 0),
        achievement(40, 130, 0, -1, FLAG_STATISTIC | FLAG_SUM),
        WrathAchievement {
            reference_achievement_id: 40,
            ..achievement(41, 130, 0, -1, FLAG_STATISTIC)
        },
        achievement(42, 130, 0, -1, FLAG_STATISTIC),
        achievement(43, 130, 0, -1, FLAG_STATISTIC),
        achievement(50, 92, 0, -1, FLAG_HIDDEN),
    ] {
        meta.achievements.insert(row.id, row);
    }
    for (id, owner) in [(400, 40), (401, 40), (100, 10)] {
        meta.criteria.insert(
            id,
            WrathAchievementCriterion {
                id,
                achievement_id: owner,
                criteria_type: 1,
                asset_id: 0,
                required_quantity: 0,
                description: format!("C{id}"),
                flags: 0,
                time_limit: if id == 401 { 60 } else { 0 },
            },
        );
    }
    meta.linked_criteria.insert(40, vec![400, 401]);
    meta.linked_criteria.insert(10, vec![100]);
    for (id, parent) in [(92, -1), (1, -1), (130, 1), (81, -1)] {
        meta.categories.push(WrathAchievementCategory {
            id,
            parent_id: parent,
            name: format!("Cat{id}"),
        });
    }
    ui.set_wrath_achievement_metadata(meta);
    ui
}

fn progress() -> WrathAchievementProgress {
    WrathAchievementProgress {
        earned: vec![(10, OCT_17_2009)],
        criteria: BTreeMap::from([(400, 3.into()), (401, 4.into()), (100, 1.into())]),
        faction: 1,
        received: true,
    }
}

#[test]
fn published_progress_answers_the_achievement_window() {
    let mut ui = script();
    ui.run(
        r#"
        assert(CanShowAchievementUI() == false and HasCompletedAnyAchievement() == false)
        seen = {}
        local f = CreateFrame('Frame')
        for _, e in ipairs({'CRITERIA_UPDATE', 'ACHIEVEMENT_EARNED', 'TRACKED_ACHIEVEMENT_UPDATE', 'RECEIVED_ACHIEVEMENT_LIST'}) do f:RegisterEvent(e) end
        f:SetScript('OnEvent', function(_, e, ...) seen[#seen+1] = {e, ...} end)
    "#,
    )
    .unwrap();
    ui.set_wrath_achievement_progress(progress());
    ui.set_wrath_achievement_progress(progress());
    ui.wrath_achievement_list_arrived(&[(100, 0), (401, 7)]);
    ui.wrath_achievement_earned(10);
    ui.tick(0.0);
    ui.run(
        r#"
        local function is(i, ...) return table.concat(seen[i], ' ') == table.concat({...}, ' ') end
        assert(#seen == 4 and is(1, 'TRACKED_ACHIEVEMENT_UPDATE', 40, 401, 7, 60), 'a timed criterion')
        assert(is(2, 'CRITERIA_UPDATE') and is(3, 'RECEIVED_ACHIEVEMENT_LIST') and is(4, 'ACHIEVEMENT_EARNED', 10))
        assert(CanShowAchievementUI() == true and HasCompletedAnyAchievement() == true)
        local cats = GetCategoryList()
        assert(#cats == 2 and cats[1] == 92 and cats[2] == 81, #cats)
        local stats = GetStatisticsCategoryList()
        assert(#stats == 1 and stats[1] == 130)
        local name, parent, flags = GetCategoryInfo(130.9)
        assert(name == 'Cat130' and parent == -1 and flags == 0, 'truncated')
        assert(select(2, GetCategoryInfo(92)) == -1)
        local total, done = GetCategoryNumAchievements(92)
        assert(total == 3 and done == 1, total)
        assert(GetCategoryNumAchievements(92, true) == 5)
        assert(GetCategoryNumAchievements(81) == 0)
        local id, n, pts, completed, m, d, y, desc, fl, icon, reward = GetAchievementInfo(92, 1)
        assert(id == 10 and completed == true and m == 10 and d == 17 and y == 9 and desc == 'D10')
        assert(icon == 'Interface\\Icons\\Achievement_General' and reward == '')
        assert(GetAchievementInfo(92, 2) == 21 and GetAchievementInfo(92, '3') == 11)
        assert(select('#', GetAchievementInfo(92, 4)) == 0, 'the cursor skips 12 and 13')
        assert(select('#', GetAchievementInfo(81, 1)) == 0, 'no unearned Feats of Strength')
        assert(GetAchievementInfo(10, false) == 10 and select('#', GetAchievementInfo(10, {})) == 11)
        local id, _, _, completed, m = GetAchievementInfo(11)
        assert(id == 11 and completed == false and m == nil)
        assert(select('#', GetAchievementInfo(50)) == 0, 'hidden')
        local total, done = GetNumCompletedAchievements()
        assert(total == 5 and done == 1, total)
        assert(GetTotalAchievementPoints() == 10)
        assert(GetLatestCompletedAchievements() == 10 and select('#', GetLatestCompletedAchievements()) == 1)
        assert(GetNextAchievement(10) == 11 and select('#', GetNextAchievement(10)) == 1)
        assert(GetPreviousAchievement(11) == 10 and select('#', GetPreviousAchievement(10)) == 0)
        assert(GetStatistic(40) == '7')
        assert(select('#', GetAchievementInfoFromCriteria(400)) == 7 and GetAchievementInfoFromCriteria(400) == 40)
        local _, _, done, q = GetAchievementCriteriaInfo(10, 1)
        assert(done == true and q == 1)
        AddTrackedAchievement(11); AddTrackedAchievement(11); AddTrackedAchievement(12)
        RemoveTrackedAchievement(12)
        assert(IsTrackedAchievement(11) and not IsTrackedAchievement(12))
        assert(GetNumTrackedAchievements() == 1 and GetTrackedAchievements() == 11)
        assert(not pcall(AddTrackedAchievement, 999))
        for _, id in ipairs({10, 12, 13, 20, 21, 30, 40, 41, 42, 43}) do AddTrackedAchievement(id) end
        assert(GetNumTrackedAchievements() == 10 and not IsTrackedAchievement(43), 'ten at most')
    "#,
    )
    .unwrap();
    ui.tick(0.0);
    ui.wrath_achievement_earned(11);
    ui.wrath_criteria_updated(401, 5);
    ui.wrath_criteria_updated(400, 9);
    ui.wrath_criteria_updated(999, 0);
    ui.tick(0.0);
    ui.run(
        r#"
        local function is(i, ...) return table.concat(seen[i], ' ') == table.concat({...}, ' ') end
        assert(#seen == 25, #seen)
        for i = 5, 17 do assert(seen[i][1] == 'TRACKED_ACHIEVEMENT_UPDATE' and seen[i][3] == nil) end
        assert(seen[5][2] == 11 and seen[6][2] == 11 and seen[7][2] == 12 and seen[8][2] == 12)
        assert(is(18, 'TRACKED_ACHIEVEMENT_UPDATE', 11), 'earned: untracked')
        assert(is(19, 'ACHIEVEMENT_EARNED', 11))
        assert(GetNumTrackedAchievements() == 9 and not IsTrackedAchievement(11))
        assert(is(20, 'CRITERIA_UPDATE') and is(21, 'TRACKED_ACHIEVEMENT_UPDATE', 40, 401, 5, 60))
        assert(is(22, 'CRITERIA_UPDATE') and is(23, 'TRACKED_ACHIEVEMENT_UPDATE', 40))
        assert(is(24, 'TRACKED_ACHIEVEMENT_UPDATE', 41), 'tracked through its reference')
        assert(is(25, 'CRITERIA_UPDATE'), 'every packet')
    "#,
    )
    .unwrap();
}

#[test]
fn a_compared_player_joins_the_listing_and_answers_comparisons() {
    let mut ui = script();
    ui.set_wrath_achievement_progress(progress());
    ui.run(
        r#"
        UnitGUID = function() return '0x0000000000000042' end
        ready = 0
        local f = CreateFrame('Frame')
        f:RegisterEvent('INSPECT_ACHIEVEMENT_READY')
        f:SetScript('OnEvent', function(_, _, ...) ready = ready + 1 + select('#', ...) end)
        assert(SetAchievementComparisonUnit('target') == 1)
        assert(select('#', GetAchievementComparisonInfo(11)) == 0, 'not yet')
    "#,
    )
    .unwrap();
    assert_eq!(
        ui.take_wrath_achievement_requests(),
        [WrathAchievementRequest::Compare { guid: 0x42 }]
    );
    let theirs = WrathAchievementProgress {
        earned: vec![(11, OCT_17_2009)],
        criteria: BTreeMap::from([(400, 10.into())]),
        faction: 0,
        ..Default::default()
    };
    ui.set_wrath_achievement_comparison(0x43, theirs.clone());
    ui.set_wrath_achievement_comparison(0x42, theirs);
    ui.tick(0.0);
    ui.run(
        r#"
        assert(ready == 2, 'every response, no argument')
        local done, m, d, y = GetAchievementComparisonInfo(11)
        assert(done == true and m == 10 and d == 17 and y == 9)
        assert(GetComparisonAchievementPoints() == 11)
        assert(GetComparisonCategoryNumAchievements(92) == 1)
        assert(GetComparisonStatistic(40) == '10')
        local order = {}
        for i = 1, 5 do order[i] = GetAchievementInfo(92, i) or 0 end
        assert(table.concat(order, ',') == '10,11,21,12,0', table.concat(order, ','))
    "#,
    )
    .unwrap();
    // Their earn of a step whose successor I have is superseded by MY list (`0x5b37c0`).
    let mine = |id| WrathAchievementProgress {
        earned: vec![(id, OCT_17_2009)],
        faction: 1,
        ..Default::default()
    };
    ui.set_wrath_achievement_progress(mine(11));
    ui.set_wrath_achievement_comparison(0x42, mine(10));
    ui.run(
        r#"
        assert(CanShowAchievementUI() == false, 'the host holds the latch')
        local order = {}
        for i = 1, 4 do order[i] = GetAchievementInfo(92, i) or 0 end
        assert(table.concat(order, ',') == '11,21,12,0', table.concat(order, ','))
        ClearAchievementComparisonUnit()
        assert(GetComparisonAchievementPoints() == 0)
    "#,
    )
    .unwrap();
}
