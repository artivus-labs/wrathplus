//! `wp.combat` against a stubbed world: chasing a moving target, swinging in reach, the outcomes.

use wrathplus_ui::script::UiScript;

const GAME: &str = r#"
calls = {}
wolf = { guid = "0xW", name = "Mangy Wolf", kind = "unit", reaction = "hostile", level = 5, x = 20, y = 0, z = 0,
  distance = 20, dead = false }
UnitLevel = function() return 5 end
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
    // Dead, and nothing on it for us: the fight ends once the loot flag has had time to arrive,
    // signalled, and `engaged` says it was won.
    script.run("wolf.dead = true").unwrap();
    script.tick(0.25);
    let over: bool = script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(!over, "waits for the corpse to turn lootable");
    for _ in 0..7 {
        script.tick(0.25);
    }
    let (over, won): (bool, bool) = script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(over && won);
    let signals: String = script.eval("return table.concat(signals, ' | ')").unwrap();
    assert_eq!(signals, "COMBAT_DONE 0xW killed");
}

#[test]
fn a_kill_is_looted_before_the_fight_is_over() {
    let mut script = combat();
    script
        .run(
            "looted = {} closed = false open = false \
             LootFrame = { IsShown = function() return open end } \
             GetNumLootItems = function() return 2 end \
             LootSlot = function(i) looted[#looted + 1] = i end \
             CloseLoot = function() closed, open, wolf.lootable = true, false, false end \
             Combat.engage('0xW') wolf.dead, wolf.lootable, wolf.distance = true, true, 8",
        )
        .unwrap();
    // Out of reach: walk to the corpse first.
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20");
    let state: String = script.eval("return Combat.state").unwrap();
    assert_eq!(state, "walking to loot Mangy Wolf");
    // Beside it: open it, and take every slot as the window opens.
    script.run("wolf.distance, walking = 2, false").unwrap();
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20 | swing 0xW");
    script.run("open = true").unwrap();
    script.fire_event("LOOT_OPENED", vec![]);
    let taken: String = script.eval("return table.concat(looted, ',')").unwrap();
    assert_eq!(taken, "2,1");
    // The picks land on a later frame: no close straight away, and no second open meanwhile.
    script.tick(0.25);
    assert!(!script.eval::<bool>("return closed").unwrap());
    assert_eq!(calls(&script), "target 0xW | move 20 | swing 0xW");
    // A second on, still open: close it; emptied, the kill is over.
    for _ in 0..4 {
        script.tick(0.25);
    }
    assert!(script.eval::<bool>("return closed").unwrap());
    script.tick(0.25);
    let (over, won): (bool, bool) = script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(over && won);
}

#[test]
fn a_corpse_that_stays_lootable_is_left_after_three_opens() {
    let mut script = combat();
    script
        .run(
            "LootFrame = { IsShown = function() return false end } \
             Combat.engage('0xW') wolf.dead, wolf.lootable, wolf.distance = true, true, 2",
        )
        .unwrap();
    for _ in 0..30 {
        script.tick(0.25);
    }
    let (over, won): (bool, bool) = script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(over && won);
    assert_eq!(
        calls(&script),
        "target 0xW | swing 0xW | swing 0xW | swing 0xW | stop"
    );
}

#[test]
fn loot_off_leaves_the_corpse() {
    let mut script = combat();
    script
        .run("Combat.loot = false Combat.engage('0xW') wolf.dead, wolf.lootable = true, true")
        .unwrap();
    script.tick(0.25);
    let over: bool = script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(over);
    assert_eq!(calls(&script), "target 0xW | stop");
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

#[test]
fn attacked_with_no_fight_of_its_own_it_fights_back_and_turns_on_a_closer_attacker() {
    let mut script = combat();
    // Resting between pulls: a wolf attacks. Nothing engaged it before; now Combat does.
    script
        .run("UnitGUID = function() return '0xME' end wolf.in_combat, wolf.target, wolf.reaction = true, '0xME', 'hostile'")
        .unwrap();
    script.tick(0.25);
    assert_eq!(
        script.eval::<String>("return Combat.active").unwrap(),
        "0xW"
    );
    // Switched off (to run away): stopping the fight is not undone.
    script.run("Combat.defend = false Combat.stop()").unwrap();
    script.tick(0.25);
    assert!(script.eval::<bool>("return Combat.active == nil").unwrap());
    // Chasing a far target while a second wolf bites in melee: fight the biter.
    script
        .run(
            "Combat.defend = true \
             far = { guid = '0xF', name = 'Far Wolf', kind = 'unit', reaction = 'hostile', level = 5, x = 40, y = 0, z = 0, distance = 40 } \
             wolf.distance = 2 \
             AgentNearby = function() return { far, wolf } end \
             Combat.engage('0xF')",
        )
        .unwrap();
    script.tick(0.25);
    assert_eq!(
        script.eval::<String>("return Combat.active").unwrap(),
        "0xW"
    );
}

#[test]
fn a_walk_a_defensive_fight_broke_off_is_walked_on_after_it() {
    let mut script = combat();
    script
        .run(
            "UnitGUID = function() return '0xME' end \
             walking = true AgentWalk = function() return { status = walking and 'walking' or 'arrived', goal = { -8880, 600, 90 } } end \
             wolf.distance, wolf.in_combat, wolf.target, wolf.reaction = 2, true, '0xME', 'hostile'",
        )
        .unwrap();
    script.tick(0.25);
    script.tick(0.25);
    script.run("calls = {} wolf.dead = true").unwrap();
    // Nothing to loot on it: the fight ends once the loot flag has had time to arrive.
    for _ in 0..7 {
        script.tick(0.25);
    }
    assert_eq!(calls(&script), "stop | move -8880");
}

#[test]
fn an_attack_that_is_not_landing_is_started_again() {
    let mut script = combat();
    // In reach, but the wolf's health does not move (the live check: a level 5 character stood
    // at a spider for ten seconds without a hit landing, then ran from it at 94% health).
    script
        .run(
            "started = 0 StartAttack = function() started = started + 1 end \
             UnitGUID = function() return '0xME' end \
             wolf.distance, wolf.health, wolf.in_combat, wolf.target = 2, 100, true, '0xME' \
             Combat.engage('0xW')",
        )
        .unwrap();
    for _ in 0..12 {
        script.tick(0.25);
    }
    assert_eq!(script.eval::<i64>("return started").unwrap(), 0);
    // Four seconds without a scratch on it: the attack is started again, facing it.
    for _ in 0..8 {
        script.tick(0.25);
    }
    assert_eq!(script.eval::<i64>("return started").unwrap(), 1);
    // Once the hits land, nothing more is forced.
    for _ in 0..20 {
        script.run("wolf.health = wolf.health - 2").unwrap();
        script.tick(0.25);
    }
    assert_eq!(script.eval::<i64>("return started").unwrap(), 1);
    assert!(script.eval::<bool>("return Combat.fleeing == nil").unwrap());
}

/// The first outside session's "lost" rabbits (reproduced on the beta, 2026-11-10): a brand-new
/// character is in the world for some four seconds before its intro cinematic starts, so
/// `Combat.engage` finds its target and returns true; then the cinematic hides every unit and
/// stops all walking, and five seconds on the fight ended "lost". It waits instead, saying why.
#[test]
fn a_cinematic_hides_the_target_without_losing_it() {
    let mut script = combat();
    script
        .run(
            "cinematic = nil \
             AgentCinematic = function() return cinematic end \
             shown = wolf \
             AgentNearby = function() return (shown and not cinematic) and { shown } or {} end",
        )
        .unwrap();
    assert!(script.eval::<bool>("return Combat.engage('0xW')").unwrap());
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20");
    // The cinematic starts: nothing in sight for far longer than `lose_after`.
    script
        .run("cinematic = 'a cinematic is playing: StopCinematic() skips it' walking = false")
        .unwrap();
    for _ in 0..40 {
        script.tick(0.25);
    }
    assert_eq!(
        script.eval::<String>("return Combat.active").unwrap(),
        "0xW",
        "still this fight"
    );
    let state: String = script.eval("return Combat.state").unwrap();
    assert_eq!(
        state,
        "waiting: a cinematic is playing: StopCinematic() skips it"
    );
    // No walk asked for meanwhile (each would be refused), and nothing signalled.
    assert_eq!(calls(&script), "target 0xW | move 20");
    assert_eq!(
        script
            .eval::<String>("return table.concat(signals, ' | ')")
            .unwrap(),
        ""
    );
    // Over (skipped, or run out): the chase carries on and the fight is won.
    script.run("cinematic = nil").unwrap();
    script.tick(0.25);
    assert_eq!(calls(&script), "target 0xW | move 20 | move 20");
    script.run("wolf.distance = 2").unwrap();
    script.tick(0.25);
    script.run("wolf.dead = true").unwrap();
    for _ in 0..8 {
        script.tick(0.25);
    }
    assert_eq!(
        script
            .eval::<String>("return table.concat(signals, ' | ')")
            .unwrap(),
        "COMBAT_DONE 0xW killed"
    );
    // A target really gone after the cinematic is still lost, counted from its end.
    script
        .run("Combat.lose_after = 0.5 shown = { guid = '0xR', name = 'Rabbit', x = 9, y = 0, z = 0, distance = 9, dead = false } Combat.engage('0xR') cinematic = 'held'")
        .unwrap();
    for _ in 0..8 {
        script.tick(0.25);
    }
    assert!(script
        .eval::<bool>("return Combat.active == '0xR'")
        .unwrap());
    script.run("cinematic, shown = nil, nil").unwrap();
    for _ in 0..4 {
        script.tick(0.25);
    }
    assert_eq!(
        script.eval::<String>("return Combat.last.outcome").unwrap(),
        "lost"
    );
}

/// Under the cinematic nothing is in sight: `engage` says why it found nobody, and nothing that
/// "attacks" is fought.
#[test]
fn engaging_under_a_cinematic_says_why_nobody_is_in_sight() {
    let script = combat();
    script
        .run(
            "AgentCinematic = function() return 'a cinematic is playing' end \
             AgentNearby = function() return {} end",
        )
        .unwrap();
    let (ok, why): (bool, String) = script.eval("return Combat.engage('0xW')").unwrap();
    assert!(!ok);
    assert_eq!(why, "a cinematic is playing");
    // Out of a cinematic an unseen unit is a plain false.
    script
        .run("AgentCinematic = function() return nil end")
        .unwrap();
    let (ok, why): (bool, String) = script.eval("return Combat.engage('0xW')").unwrap();
    assert!(!ok);
    assert_eq!(why, "not in sight");
}
