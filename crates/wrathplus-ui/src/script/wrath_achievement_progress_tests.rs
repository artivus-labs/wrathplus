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
        achievement(41, 130, 0, -1, FLAG_STATISTIC),
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
        criteria: BTreeMap::from([(400, 3), (401, 4), (100, 1)]),
        faction: 1,
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
        for _, e in ipairs({'CRITERIA_UPDATE', 'ACHIEVEMENT_EARNED', 'TRACKED_ACHIEVEMENT_UPDATE'}) do f:RegisterEvent(e) end
        f:SetScript('OnEvent', function(_, e, a, b) seen[#seen+1] = {e, a, b} end)
    "#,
    )
    .unwrap();
    ui.set_wrath_achievement_progress(progress());
    ui.set_wrath_achievement_progress(progress());
    ui.wrath_achievement_earned(10);
    ui.tick(0.0);
    ui.run(
        r#"
        assert(#seen == 2 and seen[1][1] == 'CRITERIA_UPDATE' and seen[2][1] == 'ACHIEVEMENT_EARNED' and seen[2][2] == 10)
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
    ui.tick(0.0);
    ui.run(
        r#"
        assert(#seen == 17, #seen)
        for i = 3, 15 do assert(seen[i][1] == 'TRACKED_ACHIEVEMENT_UPDATE' and seen[i][3] == nil) end
        assert(seen[3][2] == 11 and seen[4][2] == 11 and seen[5][2] == 12 and seen[6][2] == 12)
        assert(seen[16][1] == 'TRACKED_ACHIEVEMENT_UPDATE' and seen[16][2] == 11, 'earned: untracked')
        assert(seen[17][1] == 'ACHIEVEMENT_EARNED' and seen[17][2] == 11)
        assert(GetNumTrackedAchievements() == 9 and not IsTrackedAchievement(11))
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
        f:SetScript('OnEvent', function() ready = ready + 1 end)
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
        criteria: BTreeMap::from([(400, 10)]),
        faction: 0,
    };
    ui.set_wrath_achievement_comparison(0x43, theirs.clone());
    ui.set_wrath_achievement_comparison(0x42, theirs);
    ui.tick(0.0);
    ui.run(
        r#"
        assert(ready == 1, 'only the asked player')
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
        local order = {}
        for i = 1, 4 do order[i] = GetAchievementInfo(92, i) or 0 end
        assert(table.concat(order, ',') == '11,21,12,0', table.concat(order, ','))
        ClearAchievementComparisonUnit()
        assert(GetComparisonAchievementPoints() == 0)
    "#,
    )
    .unwrap();
}
