//! The hot loop seen live: an action whose walk fails as it starts (fighting, dead, controlled),
//! restarted at once by the runner or the fast tier, or replacing itself.

use super::goals_tests::{goals, text, ticks};

fn offered(script: &wrathplus_ui::script::UiScript) -> String {
    text(
        script,
        "local o = {} for _, i in ipairs(Agent.available()) do o[#o + 1] = i.name end return table.concat(o, ' ')",
    )
}

/// How often seek_mobs has failed so far.
fn failures(script: &wrathplus_ui::script::UiScript) -> i64 {
    script
        .eval("local n = 0 for _ in heard('ACTION_FAILED'):gmatch('seek_mobs') do n = n + 1 end return n")
        .unwrap()
}

#[test]
fn nothing_that_walks_is_offered_while_fighting_or_dead() {
    let script = goals();
    script
        .run(
            "units = { wolf, { guid = '0xA', name = 'Smith Argus', kind = 'unit', reaction = 'friendly', \
             quest = 'available', distance = 8, x = 8, y = 0, z = 0 } } Goals.on = true",
        )
        .unwrap();
    script.run("Goals.list()").unwrap();
    let calm = offered(&script);
    assert!(
        calm.contains("goal:pick_up_quests") && calm.contains("pick_up_quests"),
        "{calm}"
    );
    script.run("combat = true").unwrap();
    assert_eq!(offered(&script), "", "in a fight");
    script
        .run("combat = false UnitIsDeadOrGhost = function() return true end")
        .unwrap();
    assert_eq!(offered(&script), "", "dead");
}

#[test]
fn an_action_that_fails_as_it_starts_backs_off_for_the_runner_and_the_fast_tier() {
    let mut script = goals();
    // Every walk is refused at once, as click-to-move refuses a controlled character.
    script
        .run(
            "AgentMoveTo = function() calls[#calls + 1] = 'move' return nil, 'controlled' end \
             Goals.on = true",
        )
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "seek_mobs grind"
    );
    ticks(&mut script, 0.5);
    assert_eq!(failures(&script), 1);
    assert!(text(&script, "return heard('ACTION_FAILED')").ends_with("no way on from here"));
    // Not again at once, though the standing goal has nothing else: and not offered to Jev either.
    ticks(&mut script, 15.0);
    assert_eq!(failures(&script), 1);
    assert!(!offered(&script).contains("goal:seek_mobs"));
    assert_eq!(text(&script, "return doing()"), "idle");
    // After `retry_after` one more try, then twice as long.
    ticks(&mut script, 6.0);
    assert_eq!(failures(&script), 2);
    ticks(&mut script, 30.0);
    assert_eq!(failures(&script), 2);
}

#[test]
fn choosing_the_running_action_again_carries_on_with_it() {
    let mut script = goals();
    script.run("units = { wolf } Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    let (ok, said): (bool, String) = script.eval("return Agent.run('goal:grind')").unwrap();
    assert!(ok);
    assert_eq!(said, "already doing goal:grind");
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    assert_eq!(text(&script, "return heard('INTENT_DONE')"), "");
    assert!(!text(&script, "return table.concat(Goals.log, ' | ')").contains("interrupted"));
}

/// Errands that looped or hung live: the sale that cannot empty the bags, the planner's walks.
#[path = "errand_tests.rs"]
mod errand_tests;
