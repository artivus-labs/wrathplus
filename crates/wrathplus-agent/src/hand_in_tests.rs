//! The goal runner's `hand_in_quests` against a stubbed world: a finished quest whose hand-in is
//! somewhere else, as the quest's map area says, and what it reports when it cannot get there.

use super::goals_tests::{goals, text, ticks};

/// Elmore's Task, finished in Goldshire and handed in 500 yards away, as the quest's map area
/// says; the quest log names who takes it.
const ELMORE: &str = r#"
LOG = { { id = 1097, title = "Elmore's Task", level = 10, complete = true, objectives = {} } }
poi_map = 0
AgentQuestPOI = function(id)
  if id == 1097 then return { { objective = -1, map = poi_map, x = 400, y = 300 } } end
end
selected = 0
GetQuestLogSelection = function() return selected end
SelectQuestLogEntry = function(i) selected = i end
GetQuestLogQuestText = function() return "A long story.", "Speak with Grimand Elmore in Stormwind." end
Goals.on = true
"#;

#[test]
fn a_walk_that_ends_short_of_the_hand_in_says_where_it_is_and_what_the_quest_says() {
    let mut script = goals();
    script.run(ELMORE).unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "hand_in_quests"
    );
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:hand_in_quests");
    assert_eq!(text(&script, "return table.concat(calls, ' ')"), "move");
    // The route was partial: the walk "arrives" where the navmesh stops, 500 yards short.
    script.run("walking = false").unwrap();
    ticks(&mut script, 1.5);
    assert_eq!(
        text(&script, "return heard('ACTION_FAILED')"),
        "ACTION_FAILED progress hand_in_quests the walk to the hand-in of Elmore's Task at (400, 300) \
         ended 500 yards short (the route stops there): no route on foot from here, so go by another \
         way (a waypoint through the town gate, a road). The quest log says: \"Speak with Grimand \
         Elmore in Stormwind.\""
    );
    // Reading the quest's text left the log's selection as it was.
    assert_eq!(script.eval::<i64>("return selected").unwrap(), 0);
    // That quest backs off; the others, and the rest of progress, carry on.
    assert_ne!(
        text(&script, "return plan_of('progress')"),
        "hand_in_quests"
    );
}

#[test]
fn at_the_hand_in_with_nobody_marked_it_says_to_look_for_who_the_quest_names() {
    let mut script = goals();
    script.run(ELMORE).unwrap();
    ticks(&mut script, 0.75);
    script.run("walking = false pos = { x = 396, y = 300, z = 0, map = 0 }").unwrap();
    ticks(&mut script, 1.5);
    assert_eq!(
        text(&script, "return heard('ACTION_FAILED')"),
        "ACTION_FAILED progress hand_in_quests nobody in sight takes Elmore's Task at its hand-in \
         (400, 300): the point is the centre of a map area, so look within 60 yards for the NPC or \
         object the quest names. The quest log says: \"Speak with Grimand Elmore in Stormwind.\""
    );
}

#[test]
fn a_hand_in_may_be_an_object_and_one_on_another_continent_is_not_walked_to() {
    let mut script = goals();
    script.run(ELMORE).unwrap();
    ticks(&mut script, 0.75);
    // On the way a marked object comes into reach ("A half-eaten body" takes Find the Lost
    // Guards): the walk stops and the hand-in starts with it.
    script
        .run(
            "units = { { guid = '0xB', name = 'A half-eaten body', kind = 'gameobject', quest = 'reward', \
             distance = 12, x = 12, y = 0, z = 0 } }",
        )
        .unwrap();
    ticks(&mut script, 1.5);
    assert!(
        text(&script, "return table.concat(calls, ' ')").contains("interact 0xB"),
        "{}",
        text(&script, "return table.concat(calls, ' ')")
    );
    // A hand-in on another continent is no walk: the action does not plan for it.
    let script = goals();
    script.run(ELMORE).unwrap();
    script.run("poi_map = 1").unwrap();
    assert_ne!(
        text(&script, "return plan_of('progress')"),
        "hand_in_quests"
    );
    assert_eq!(
        script
            .eval::<i64>("return Quests.where(1097).hand_in.map")
            .unwrap(),
        1
    );
}
