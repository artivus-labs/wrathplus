use super::*;

// ── quests_with_progressed_objectives ───────────────────────────────────────────────────────

/// A one-objective entry whose `cur`/`req` are parsed from its text's trailing `cur/req`, so
/// the two cannot disagree.
fn entry(quest_id: u32, objective_text: &str) -> QuestLogEntryView {
    let (cur, req) = objective_text
        .rsplit_once(' ')
        .and_then(|(_, tail)| tail.split_once('/'))
        .map(|(c, r)| (c.parse().unwrap(), r.parse().unwrap()))
        .expect("fixture line must end in `cur/req`");
    QuestLogEntryView {
        quest_id,
        objectives: vec![QuestLogObjectiveView {
            text: objective_text.into(),
            kind: "monster".into(),
            finished: cur >= req,
            cur,
            req,
            poi_bit: None,
        }],
        ..Default::default()
    }
}

fn state(entries: Vec<QuestLogEntryView>) -> QuestLogState {
    QuestLogState {
        num_quests: entries.iter().filter(|e| !e.is_header).count() as u32,
        entries,
    }
}

#[test]
fn no_progress_fires_nothing() {
    let old = state(vec![entry(1, "Kobold Vermin slain: 3/10")]);
    let same = state(vec![entry(1, "Kobold Vermin slain: 3/10")]);
    assert!(quests_with_progressed_objectives(&old, &same).is_empty());
}

#[test]
fn a_progressed_quest_fires_its_new_log_index_and_the_moved_lines() {
    let old = state(vec![
        entry(1, "Kobold Vermin slain: 3/10"),
        entry(2, "Tough Wolf Meat: 2/8"),
    ]);
    let progressed = state(vec![
        entry(1, "Kobold Vermin slain: 3/10"),
        entry(2, "Tough Wolf Meat: 3/8"),
    ]);
    let fired = quests_with_progressed_objectives(&old, &progressed);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].index, 2);
    assert_eq!(fired[0].changed_lines, ["Tough Wolf Meat: 3/8"]);
    // Both advancing fires both, each at its own index with its own line.
    let both = state(vec![
        entry(1, "Kobold Vermin slain: 4/10"),
        entry(2, "Tough Wolf Meat: 3/8"),
    ]);
    let fired = quests_with_progressed_objectives(&old, &both);
    assert_eq!(
        fired
            .iter()
            .map(|q| (q.index, q.changed_lines.clone()))
            .collect::<Vec<_>>(),
        [
            (1, vec!["Kobold Vermin slain: 4/10".to_string()]),
            (2, vec!["Tough Wolf Meat: 3/8".to_string()]),
        ]
    );
}

#[test]
fn appearing_or_leaving_the_log_is_not_progress() {
    let empty = state(vec![]);
    let with_entry = state(vec![entry(7, "Kobold Worker slain: 0/10")]);
    // A fresh accept is not progress, nor is a late template growing lines from none.
    assert!(quests_with_progressed_objectives(&empty, &with_entry).is_empty());
    let grown = state(vec![QuestLogEntryView {
        quest_id: 7,
        objectives: vec![],
        ..Default::default()
    }]);
    assert!(quests_with_progressed_objectives(&grown, &with_entry).is_empty());
    // Nor a turn-in or abandon removing the quest.
    assert!(quests_with_progressed_objectives(&with_entry, &empty).is_empty());
}

/// A turn-in destroys the items (an immediate `SMSG_DESTROY_OBJECT`) before the slot clears in
/// the batched `SMSG_UPDATE_OBJECT` (`Player::RewardQuest`), so the line dips to `0/req` for a
/// frame; the reference toasts only on `SMSG_QUESTUPDATE_ADD_ITEM`.
#[test]
fn a_regressing_objective_never_announces() {
    let held = state(vec![entry(313, "Rumbleshot's Ammo: 1/1")]);
    let emptied = state(vec![entry(313, "Rumbleshot's Ammo: 0/1")]);
    assert!(
        quests_with_progressed_objectives(&held, &emptied).is_empty(),
        "the bag emptying under a still-logged quest is not progress"
    );
    // The slot clearing next frame is silent too: the quest leaves the log.
    assert!(quests_with_progressed_objectives(&emptied, &state(vec![])).is_empty());
    // The control: the same quest actually being collected still announces.
    let looted = state(vec![entry(313, "Rumbleshot's Ammo: 1/1")]);
    assert_eq!(
        quests_with_progressed_objectives(&emptied, &looted)[0].changed_lines,
        ["Rumbleshot's Ammo: 1/1"]
    );
}

/// A name landing late rewrites the text with the count unmoved: no toast.
#[test]
fn a_name_landing_late_does_not_re_announce() {
    let placeholder = state(vec![entry(5, "...: 3/5")]);
    let named = state(vec![entry(5, "Small Barnacled Clam: 3/5")]);
    assert!(quests_with_progressed_objectives(&placeholder, &named).is_empty());
}

/// The COMPLETE flip announces once for the quest, not again per objective line.
#[test]
fn the_complete_flip_announces_once_not_per_objective() {
    let mut before = entry(9, "Kobold Vermin slain: 10/10");
    before.complete = 0;
    before.objectives[0].finished = false;
    let mut after = entry(9, "Kobold Vermin slain: 10/10");
    after.complete = 1;
    after.title = "A Threat Within".into();
    let fired = quests_with_progressed_objectives(&state(vec![before]), &state(vec![after]));
    assert_eq!(fired.len(), 1);
    assert!(fired[0].completed);
    assert!(
        fired[0].changed_lines.is_empty(),
        "the objective count did not move — only the quest's COMPLETE state did"
    );
}
