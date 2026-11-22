use super::*;
use wrathplus_protocol::types::{QuestObjective, WrathQuestInfo};

/// AzerothCore 9305 "Spare Parts": four Emitter Spare Parts (22978), flags `0x80088`.
fn spare_parts() -> QuestTemplate {
    let mut required_items = [(0, 0); 6];
    required_items[0] = (22_978, 4);
    QuestTemplate {
        quest_id: 9_305,
        method: 2,
        level: 4,
        zone_or_sort: 3_526,
        quest_type: 0,
        flags: 0x80088,
        title: "Spare Parts".into(),
        objectives_text: String::new(),
        end_text: String::new(),
        wrath: Some(WrathQuestInfo {
            flags: 0x80088,
            required_items,
            ..Default::default()
        }),
        rep_objective_faction: 0,
        rep_objective_value: 0,
        next_quest_in_chain: 0,
        money: 0,
        money_max_level: 0,
        reward_spell: 0,
        src_item_id: 0,
        rewards: [(0, 0); 4],
        choices: [(0, 0); 6],
        point_map_id: 530,
        point_x: -4456.14,
        point_y: -13923.0,
        point_opt: 0,
        details: String::new(),
        objectives: std::array::from_fn(|i| QuestObjective {
            creature_or_go: 0,
            required_count: 0,
            item_id: if i == 0 { 22_978 } else { 0 },
            item_count: if i == 0 { 4 } else { 0 },
            text: String::new(),
        }),
    }
}

fn slot(state: u8, counters: [u16; 4]) -> QuestLogSlot {
    QuestLogSlot {
        quest_id: 9_305,
        counters,
        state,
        timer: 0,
    }
}

fn done_with(t: &QuestTemplate, s: &QuestLogSlot, strict: bool, bags: u32, money: u32) -> bool {
    let live = DoneInputs {
        bag_count: &|_| bags,
        standing: &|_| 100,
        money,
    };
    quest_done(t, s, strict, &live)
}

#[test]
fn an_item_quest_is_done_on_its_bag_count_without_the_complete_bit() {
    let t = spare_parts();
    for strict in [false, true] {
        assert!(!done_with(&t, &slot(0, [0; 4]), strict, 3, 0));
        assert!(done_with(&t, &slot(0, [0; 4]), strict, 4, 0), "{strict}");
        // The bit alone does not finish it, and a failed slot never is.
        assert!(!done_with(
            &t,
            &slot(quest_slot_state::COMPLETE, [0; 4]),
            strict,
            0,
            0
        ));
        assert!(!done_with(
            &t,
            &slot(quest_slot_state::FAIL, [0; 4]),
            strict,
            4,
            0
        ));
    }
}

#[test]
fn the_fifth_and_sixth_required_items_count_too() {
    let mut t = spare_parts();
    t.wrath.as_mut().unwrap().required_items[5] = (5_000, 2);
    let live = DoneInputs {
        bag_count: &|id| if id == 5_000 { 1 } else { 4 },
        standing: &|_| 0,
        money: 0,
    };
    assert!(!quest_done(&t, &slot(0, [0; 4]), true, &live));
}

#[test]
fn an_exploration_or_event_quest_waits_for_the_complete_bit() {
    let mut t = spare_parts();
    t.flags |= 4;
    assert!(!done_with(&t, &slot(0, [0; 4]), true, 4, 0));
    assert!(done_with(
        &t,
        &slot(quest_slot_state::COMPLETE, [0; 4]),
        true,
        4,
        0
    ));
}

#[test]
fn creature_counters_reputation_money_and_pvp_gate_it() {
    let mut t = spare_parts();
    t.objectives[1].creature_or_go = 16_483;
    t.objectives[1].required_count = 2;
    assert!(!done_with(&t, &slot(0, [0, 1, 0, 0]), true, 4, 0));
    assert!(done_with(&t, &slot(0, [0, 2, 0, 0]), true, 4, 0));
    // Standing 100 against a floor and a ceiling.
    t.rep_objective_faction = 930;
    t.rep_objective_value = 101;
    assert!(!done_with(&t, &slot(0, [0, 2, 0, 0]), true, 4, 0));
    t.rep_objective_value = 100;
    t.wrath.as_mut().unwrap().rep_objective_max = (930, 99);
    assert!(!done_with(&t, &slot(0, [0, 2, 0, 0]), true, 4, 0));
    t.wrath.as_mut().unwrap().rep_objective_max = (930, 100);
    assert!(done_with(&t, &slot(0, [0, 2, 0, 0]), true, 4, 0));
    // A negative money demand is met by the coinage.
    t.money = -500;
    assert!(!done_with(&t, &slot(0, [0, 2, 0, 0]), true, 4, 499));
    assert!(done_with(&t, &slot(0, [0, 2, 0, 0]), true, 4, 500));
    // Players slain count in slot counter 0.
    t.wrath.as_mut().unwrap().players_slain = 3;
    assert!(!done_with(&t, &slot(0, [2, 2, 0, 0]), true, 4, 500));
    assert!(done_with(&t, &slot(0, [3, 2, 0, 0]), true, 4, 500));
}

#[test]
fn only_the_lenient_test_passes_a_quest_with_nothing_to_do() {
    let mut t = spare_parts();
    t.objectives[0].item_id = 0;
    t.wrath.as_mut().unwrap().required_items = [(0, 0); 6];
    assert!(done_with(&t, &slot(0, [0; 4]), false, 0, 0));
    assert!(!done_with(&t, &slot(0, [0; 4]), true, 0, 0));
}
