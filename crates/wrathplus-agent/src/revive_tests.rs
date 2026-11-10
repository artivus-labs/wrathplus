//! `wp.revive` against a stubbed game: release, the corpse run, the reclaim, the spirit healer.

use wrathplus_ui::script::UiScript;

/// The game around a dead character, stubbed: state the test flips, calls it records.
const GAME: &str = r#"
calls = {}
dead, ghost, delay, hostiles = true, false, 0, {}
me = { x = 0, y = 0, z = 0 }
walking = false
healer = nil
UnitIsDead = function() return dead end
UnitIsGhost = function() return ghost end
GetNumPartyMembers = function() return 0 end
RepopMe = function() calls[#calls + 1] = "repop" end
RetrieveCorpse = function() calls[#calls + 1] = "reclaim" end
AcceptXPLoss = function() calls[#calls + 1] = "accept xp loss" end
GetCorpseRecoveryDelay = function() return delay end
AgentCorpse = function() return { x = 100, y = 0, z = 0 } end
AgentPosition = function() return me end
AgentNearby = function() return hostiles end
AgentFindNpc = function(service) if service == "spirithealer" then return healer end end
AgentInteract = function(guid) calls[#calls + 1] = "interact " .. guid end
AgentWalk = function() return { status = walking and "walking" or "idle" } end
AgentMoveTo = function(x, y) calls[#calls + 1] = string.format("move %d,%d", x, y) walking = true end
"#;

const WOLF_AT: &str = "hostiles = { { name = 'Mangy Wolf', reaction = 'hostile', x = WOLF_X, y = 0, kind = 'unit' } }";

fn revive(tag: &str) -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-revive-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

fn ticks(script: &mut UiScript, n: usize) {
    (0..n).for_each(|_| script.tick(0.6));
}

fn calls(script: &UiScript) -> String {
    script.eval("return table.concat(calls, ' ')").unwrap()
}

fn state(script: &UiScript) -> String {
    script.eval("return Revive.state or ''").unwrap()
}

fn wolf_at(script: &UiScript, x: i32) {
    script
        .run(&WOLF_AT.replace("WOLF_X", &x.to_string()))
        .unwrap();
}

#[test]
fn revive_releases_runs_back_and_reclaims_clear_of_hostiles() {
    // A solo character dead 100 yards from its corpse.
    let mut script = revive("reclaim");
    ticks(&mut script, 2);
    assert!(
        state(&script).starts_with("dead, releasing in"),
        "{}",
        state(&script)
    );
    ticks(&mut script, 5);
    assert_eq!(calls(&script), "repop");
    // A ghost far away runs to the corpse, which nothing hostile stands over.
    script.run("dead, ghost = false, true").unwrap();
    ticks(&mut script, 1);
    assert_eq!(calls(&script), "repop move 100,0");
    // At the corpse: the reclaim delay, then a wolf that wandered close, then clear.
    script
        .run("me = { x = 99, y = 0, z = 0 } walking = false delay = 12")
        .unwrap();
    ticks(&mut script, 1);
    assert_eq!(
        state(&script),
        "ghost at the corpse, waiting 12s for the reclaim delay"
    );
    script.run("delay = 0").unwrap();
    wolf_at(&script, 104);
    ticks(&mut script, 1);
    assert_eq!(
        state(&script),
        "ghost at the corpse, waiting for Mangy Wolf to move off"
    );
    // It moves away from the wolf, 30 yards out on the far side of the corpse, and stands up.
    ticks(&mut script, 1);
    assert!(calls(&script).ends_with("move 70,0"), "{}", calls(&script));
    script
        .run("me = { x = 70, y = 0, z = 0 } walking = false")
        .unwrap();
    ticks(&mut script, 1);
    assert!(calls(&script).ends_with("reclaim"), "{}", calls(&script));
    // Alive again: nothing more to do.
    script.run("ghost = false").unwrap();
    ticks(&mut script, 1);
    assert_eq!(state(&script), "");
}

#[test]
fn a_corpse_kept_unsafe_falls_back_to_the_spirit_healer() {
    let mut script = revive("healer");
    // A ghost at its corpse with a wolf standing on it, and a spirit healer seen at release.
    script
        .run(
            "dead, ghost = false, true \
             healer = { guid = '0xF1300018BB000001', name = 'Spirit Healer' } \
             me = { x = 99, y = 0, z = 0 } Revive.spirit_after = 2",
        )
        .unwrap();
    wolf_at(&script, 101);
    ticks(&mut script, 1);
    assert_eq!(
        state(&script),
        "ghost at the corpse, waiting for Mangy Wolf to move off"
    );
    ticks(&mut script, 4);
    assert!(
        calls(&script).contains("interact 0xF1300018BB000001"),
        "{}",
        calls(&script)
    );
    assert_eq!(
        state(&script),
        "ghost, resurrecting at the spirit healer (the corpse stayed unsafe)"
    );
    // The healer's confirmation is accepted, since Revive asked for it.
    script.fire_event("CONFIRM_XP_LOSS", vec![]);
    assert!(
        calls(&script).ends_with("accept xp loss"),
        "{}",
        calls(&script)
    );
}

#[test]
fn an_agents_own_spirit_healer_ask_is_left_to_it() {
    let mut script = revive("own-ask");
    script.run("dead, ghost = false, true").unwrap();
    ticks(&mut script, 1);
    script.fire_event("CONFIRM_XP_LOSS", vec![]);
    assert!(
        !calls(&script).contains("accept xp loss"),
        "{}",
        calls(&script)
    );
}

#[test]
fn the_spirit_healer_out_of_sight_from_the_corpse_is_run_back_to() {
    let mut script = revive("healer-far");
    // Released at the graveyard, the healer beside us: remembered, with where it stands. From the
    // corpse 400 yards on it is out of sight (in the playtest the ghost then stood at its corpse
    // for good, interacting with a unit the client no longer had).
    script
        .run(
            "dead, ghost = false, true \
             healer = { guid = '0xF1300018BB000001', name = 'Spirit Healer', x = -300, y = 0, z = 0 } \
             AgentFindNpc = function() if math.abs(me.x - healer.x) < 60 then return healer end end \
             me = { x = -295, y = 0, z = 0 } Revive.spirit_after = 2",
        )
        .unwrap();
    ticks(&mut script, 1);
    script
        .run("me = { x = 99, y = 0, z = 0 } walking = false calls = {}")
        .unwrap();
    wolf_at(&script, 101);
    ticks(&mut script, 5);
    assert_eq!(calls(&script), "move 70,0 move -300,0");
    assert_eq!(
        state(&script),
        "ghost, running back to the spirit healer (the corpse stayed unsafe)"
    );
    // Back in sight of it: ask.
    script
        .run("me = { x = -290, y = 0, z = 0 } walking = false")
        .unwrap();
    ticks(&mut script, 1);
    assert!(
        calls(&script).ends_with("interact 0xF1300018BB000001"),
        "{}",
        calls(&script)
    );
}
