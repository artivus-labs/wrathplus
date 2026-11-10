//! `wp.goals` against the stubbed world of `goals_tests`: what counts as an XP mob, the gap
//! between two actions, and a cinematic.

use super::goals_tests::{goals, text, ticks};

/// A rabbit is level 1 and neutral, so it used to pass for an XP mob: it gives none, and it says
/// nothing about the zone's level either.
#[test]
fn critters_are_no_xp_targets_and_do_not_skew_the_zone() {
    let mut script = goals();
    script
        .run(
            "function critter(i) return { guid = '0xC' .. i, name = 'Rabbit', kind = 'unit', reaction = 'neutral', \
               level = 1, creature_type = 'Critter', x = 10 + i, y = 0, z = 0, distance = 10 + i, dead = false } end \
             lvl = 1 wolf.level = 1 wolf.creature_type = 'Beast' \
             units = { critter(1), critter(2), critter(3) }",
        )
        .unwrap();
    assert!(!script.eval::<bool>("return Goals.fits(units[1])").unwrap());
    assert!(script.eval::<bool>("return Goals.fits(wolf)").unwrap());
    // A unit whose template has not come back yet (no type) is not taken for a critter.
    assert!(script
        .eval::<bool>("wolf.creature_type = nil local ok = Goals.fits(wolf) wolf.creature_type = 'Beast' return ok")
        .unwrap());
    assert!(!script
        .eval::<bool>("return Goals.facts().mobs_for_xp")
        .unwrap());
    // Three level 1 critters around a level 8 are no evidence the zone is below us.
    script.run("lvl = 8").unwrap();
    assert_eq!(text(&script, "return Goals.facts().zone_fit"), "ok");
    // The runner walks on to look for real mobs rather than fight the rabbits.
    script.run("lvl = 1 Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    assert!(!text(&script, "return table.concat(calls, ' ')").contains("target"));
    // With a wolf beside them, it is the wolf that is fought.
    script.run("units[#units + 1] = wolf").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    assert!(text(&script, "return table.concat(calls, ' ')").contains("target 0xW"));
}

/// `Agent.activity()` between two actions: nil only until the frame after the first one ends
/// well (the runner used to look again up to half a second later), and `Goals.state` says what
/// the runner is doing meanwhile.
#[test]
fn the_next_action_starts_on_the_frame_after_one_ends_well() {
    let mut script = goals();
    script.run("units = { wolf } Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    script.run("wolf.dead = true").unwrap();
    // Step frame by frame to the end of the fight.
    let mut frames = 0;
    while text(&script, "return heard('INTENT_DONE')").is_empty() {
        script.tick(0.05);
        frames += 1;
        assert!(frames < 100, "the fight never ended");
    }
    // The gap an outside reader can see is one frame at most (which frame scripts run first
    // decides whether there is one at all).
    if text(&script, "return doing()") == "idle" {
        script.tick(0.05);
    }
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    assert_eq!(text(&script, "return Goals.state"), "progress: seek_mobs");
}

/// An instant action (no `done`) is not hurried: it would run every frame while its plan stands.
#[test]
fn instant_actions_keep_the_runners_half_second_pace() {
    let mut script = goals();
    script
        .run(
            "runs = 0 \
             Goals.push({ name = 'busywork', priority = 90, want = { busy = true } }) \
             Goals.fact('busy', function() return false end) \
             Goals.action({ name = 'fiddle', effects = { busy = true }, cost = 1, run = function() runs = runs + 1 end }) \
             Goals.on = true",
        )
        .unwrap();
    for _ in 0..40 {
        script.tick(0.05);
    }
    let runs: i64 = script.eval("return runs").unwrap();
    assert!((3..=4).contains(&runs), "{runs} runs in two seconds");
}

/// A cinematic (a new character's intro): the runner waits and says why, rather than start
/// actions that can only fail and back off; it never skips the cinematic itself.
#[test]
fn the_runner_waits_out_a_cinematic() {
    let mut script = goals();
    script
        .run(
            "cinematic = 'a cinematic is playing: StopCinematic() skips it' stopped = 0 \
             AgentCinematic = function() return cinematic end \
             StopCinematic = function() stopped = stopped + 1 end \
             units = { wolf } Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 3.0);
    assert_eq!(text(&script, "return doing()"), "idle");
    assert_eq!(
        text(&script, "return Goals.state"),
        "waiting: a cinematic is playing: StopCinematic() skips it"
    );
    assert_eq!(text(&script, "return table.concat(calls, ' ')"), "");
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
    assert_eq!(script.eval::<i64>("return stopped").unwrap(), 0);
    script.run("cinematic = nil").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
}
