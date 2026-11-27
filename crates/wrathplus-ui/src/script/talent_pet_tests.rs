use super::*;
use crate::script::UiScript;

/// The hunter pet's page answers every getter's `isPet`, and a pet `LearnTalent` queues as the
/// pet's; without a page the pet branch is the native miss.
#[test]
fn the_pet_page_answers_is_pet_and_queues_pet_learns() {
    let mut s = UiScript::new().unwrap();
    assert_eq!(
        s.eval::<i64>("return GetNumTalentGroups(false, true)")
            .unwrap(),
        0
    );
    let talent = |id, name: &str, rank| TalentView {
        id,
        name: name.into(),
        tier: 1,
        column: 1,
        rank,
        max_rank: 3,
        meets_prereq: true,
        ..Default::default()
    };
    s.set_pet_talents(Some(TalentUiState {
        tabs: vec![TalentTabView {
            name: "Ferocity".into(),
            background: "HunterPetFerocity".into(),
            icon: None,
            points_spent: 1,
        }],
        talents: vec![vec![
            talent(2107, "Cobra Reflexes", 1),
            talent(2109, "Dive", 0),
        ]],
        points: (4, 0),
    }));
    let read: String = s
        .eval(
            "local name, _, _, _, rank = GetTalentInfo(1, 1, false, true) \
             return GetNumTalentGroups(false, true)..' '..GetNumTalentTabs(false, true)..' '.. \
             GetTalentTabInfo(1, false, true)..' '..name..' '..rank..' '.. \
             GetUnspentTalentPoints(false, true)..' '..GetNumTalentTabs(false, false)",
        )
        .unwrap();
    assert_eq!(read, "1 1 Ferocity Cobra Reflexes 1 4 0");
    s.run("LearnTalent(1, 2, true) LearnTalent(2, 3)").unwrap();
    assert_eq!(s.take_talent_learns(), [(1, 2, true), (2, 3, false)]);
}
