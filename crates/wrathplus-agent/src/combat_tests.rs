//! `wp.combat` against a stubbed world: chasing a moving target, swinging in reach, the outcomes.

use wrathplus_ui::script::UiScript;

const GAME: &str = r#"
calls = {}
wolf = { guid = "0xW", name = "Mangy Wolf", x = 20, y = 0, z = 0, distance = 20, dead = false }
walking = false
AgentNearby = function() return wolf and { wolf } or {} end
AgentTarget = function(g) calls[#calls + 1] = "target " .. g end
AgentMoveTo = function(x) calls[#calls + 1] = "move " .. x walking = true end
AgentStop = function() calls[#calls + 1] = "stop" walking = false end
AgentWalk = function() return { status = walking and "walking" or "arrived" } end
AgentInteract = function(g) calls[#calls + 1] = "swing " .. g end
UnitIsDeadOrGhost = function() return false end
signals = {}
AgentSignal = function(name, guid, outcome) signals[#signals + 1] = name .. " " .. guid .. " " .. outcome end
"#;

fn combat() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-combat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn calls(script: &UiScript) -> String {
    script.eval("return table.concat(calls, ' | ')").unwrap()
}

#[test]
fn it_chases_a_moving_target_swings_in_reach_and_reports_the_kill() {
    let mut script = combat();
    assert!(script.eval::<bool>("return Combat.engage('0xW')").unwrap());
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20");
    // Still walking and the wolf has barely moved: no new route.
    script.run("wolf.x, wolf.distance = 21, 12").unwrap();
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20");
    // It runs on: route again to where it is now.
    script.run("wolf.x, wolf.distance = 26, 10").unwrap();
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20 | move 26");
    // In reach: stop and swing (the right-click attack faces it).
    script.run("wolf.distance = 2").unwrap();
    script.tick(0.25);
    assert_eq!(
        calls(&script),
        "target 0xW | move 20 | move 26 | stop | swing 0xW"
    );
    let state: String = script.eval("return Combat.state").unwrap();
    assert_eq!(state, "fighting Mangy Wolf");
    // Dead: the fight ends, signalled, and `engaged` says it was won.
    script.run("wolf.dead = true").unwrap();
    script.tick(0.25);
    let (over, won): (bool, bool) = script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(over && won);
    let signals: String = script.eval("return table.concat(signals, ' | ')").unwrap();
    assert_eq!(signals, "COMBAT_DONE 0xW killed");
}

#[test]
fn a_target_out_of_sight_is_lost() {
    let mut script = combat();
    script
        .run("Combat.lose_after = 0.5 Combat.engage('0xW') wolf = nil")
        .unwrap();
    for _ in 0..4 {
        script.tick(0.25);
    }
    let outcome: String = script.eval("return Combat.last.outcome").unwrap();
    assert_eq!(outcome, "lost");
    assert!(script.eval::<bool>("return Combat.active == nil").unwrap());
}
