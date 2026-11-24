//! `wp.combat` for a caster, and fights that go nowhere: the stall restart from range, the
//! "no_progress" outcome and what it reports, and turning on an attacker while chasing. The case is
//! AGENTSIX's death (2026-11-23): a level 8 gnome mage stood 3 yd from a Leper Gnome whose health
//! never moved while every attack and cast was refused ("Invalid target", "You cannot attack that
//! target"), the melee stall restart firing from range, until the mage died still "fighting".

use super::{calls, combat};
use wrathplus_ui::script::{ScriptValue, UiScript};

/// A mage (reach 25) at full health, the wolf 10 yd off and in combat with us, a second mob
/// (`gnome`) out of sight until a test brings it in. Survive's judgement is off: these are
/// Combat's own decisions.
const MAGE: &str = r##"
UnitClass = function() return "Mage", "MAGE" end
hp, tgt, started = 1000, nil, 0
UnitHealth = function(u) return u == "player" and hp or 100 end
UnitHealthMax = function(u) return u == "player" and 1000 or 100 end
UnitGUID = function(u) if u == "player" then return "0xME" end return tgt end
AgentTarget = function(g) tgt = g calls[#calls + 1] = "target " .. g end
AgentFace = function(g) calls[#calls + 1] = "face " .. g end
StartAttack = function() started = started + 1 end
AgentSignal = function(name, ...)
  local parts = { name }
  for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
  signals[#signals + 1] = table.concat(parts, " ")
end
Survive.on = false
Rotation.list = {}
wolf.distance, wolf.health, wolf.health_max = 10, 100, 100
wolf.in_combat, wolf.target = true, "0xME"
gnome = { guid = "0xG", name = "Leper Gnome", kind = "unit", reaction = "hostile", level = 8, x = 8, y = 0, z = 0,
  distance = 8, health = 156, health_max = 156, dead = false, in_combat = true, target = "0xME" }
shown = { wolf }
AgentNearby = function(yards)
  local out = {}
  for _, u in ipairs(shown) do if not yards or u.distance <= yards then out[#out + 1] = u end end
  return out
end
"##;

fn mage() -> UiScript {
    let script = combat();
    script.run(MAGE).unwrap();
    script
}

fn refused(script: &mut UiScript, message: &str) {
    script.fire_event("UI_ERROR_MESSAGE", vec![ScriptValue::Str(message.into())]);
}

/// `seconds` of a fight a quarter second at a time, running `each` before every tick.
fn fight(script: &mut UiScript, seconds: u32, each: &str) {
    for _ in 0..seconds * 4 {
        script.run(each).unwrap();
        script.tick(0.25);
    }
}

fn text(script: &UiScript, lua: &str) -> String {
    script.eval(lua).unwrap()
}

#[test]
fn a_caster_stall_restart_faces_and_retargets_and_never_swings() {
    let mut script = mage();
    assert!(script.eval::<bool>("return Combat.engage('0xW')").unwrap());
    // In reach (its range) and nothing landing for well past `stall`, the target dropped meanwhile.
    fight(&mut script, 3, "");
    script.run("tgt = nil").unwrap();
    fight(&mut script, 6, "");
    assert_eq!(script.eval::<i64>("return started").unwrap(), 0);
    let calls = calls(&script);
    assert!(!calls.contains("swing"), "{calls}");
    assert!(calls.contains("face 0xW"), "{calls}");
    assert_eq!(calls.matches("target 0xW").count(), 2, "{calls}");
    // No damage taken: still fighting, nothing given up.
    assert_eq!(text(&script, "return Combat.active"), "0xW");
}

#[test]
fn refused_and_losing_health_with_no_scratch_on_the_target_ends_as_no_progress() {
    let mut script = mage();
    script.run("shown = { wolf, gnome }").unwrap();
    assert!(script.eval::<bool>("return Combat.engage('0xW')").unwrap());
    fight(&mut script, 2, "hp = hp - 2");
    refused(&mut script, "Invalid target");
    refused(&mut script, "Another action is in progress");
    fight(&mut script, 8, "hp = hp - 2");
    assert_eq!(text(&script, "return Combat.active"), "0xW", "not yet");
    // Past `no_progress` seconds: the fight ends (the tick it does, before defence looks again).
    let mut waited = 10.0;
    while script
        .eval::<bool>("return Combat.active == '0xW'")
        .unwrap()
    {
        assert!(waited < 13.0, "still fighting after {waited} s");
        script.run("hp = hp - 2").unwrap();
        script.tick(0.25);
        waited += 0.25;
    }
    assert!(waited >= 12.0, "ended after {waited} s");
    assert!(script.eval::<bool>("return Combat.active == nil").unwrap());
    assert_eq!(text(&script, "return Combat.last.outcome"), "no_progress");
    let said = text(&script, "return table.concat(signals, ' | ')");
    assert!(
        said.starts_with("COMBAT_DONE 0xW no_progress Mangy Wolf's health has not dropped in 12s in reach (100) while we lost "),
        "{said}"
    );
    assert!(
        said.ends_with(
            "; refused: \"Invalid target\" x1; also attacking us: Leper Gnome (0xG, 8 yd)"
        ),
        "{said}"
    );
    assert_eq!(text(&script, "return Combat.last.attackers[1].guid"), "0xG");
    let (over, won, outcome, why): (bool, bool, String, String) =
        script.eval("return Combat.engaged('0xW')").unwrap();
    assert!(over && !won && outcome == "no_progress" && why.contains("Invalid target"));
    // Defence leaves the given-up wolf be (it is still on us) and turns to the one it can fight;
    // a planner that chooses the wolf again still gets it.
    script.tick(0.25);
    assert_eq!(text(&script, "return Combat.active"), "0xG");
    assert!(script.eval::<bool>("return Combat.gave_up('0xW')").unwrap());
    // An intent waiting on the wolf's fight still reads how it ended.
    let outcome: String = script
        .eval("return select(3, Combat.engaged('0xW'))")
        .unwrap();
    assert_eq!(outcome, "no_progress");
    assert!(script
        .eval::<bool>("return Combat.engage('0xW', { force = true })")
        .unwrap());
    assert_eq!(text(&script, "return Combat.active"), "0xW");
}

#[test]
fn no_progress_waits_while_the_target_loses_health() {
    let mut script = mage();
    assert!(script.eval::<bool>("return Combat.engage('0xW')").unwrap());
    refused(&mut script, "Invalid target");
    fight(
        &mut script,
        20,
        "hp = hp - 2 wolf.health = wolf.health - 0.25",
    );
    assert_eq!(text(&script, "return Combat.active"), "0xW");
    assert_eq!(text(&script, "return table.concat(signals, ' | ')"), "");
}

#[test]
fn no_progress_waits_while_we_take_no_damage() {
    let mut script = mage();
    assert!(script.eval::<bool>("return Combat.engage('0xW')").unwrap());
    fight(&mut script, 1, "");
    refused(&mut script, "Invalid target");
    fight(&mut script, 20, "");
    assert_eq!(text(&script, "return Combat.active"), "0xW");
    // Switched off, a losing fight with every action refused is fought on.
    script.run("Combat.no_progress = false").unwrap();
    refused(&mut script, "Invalid target");
    fight(&mut script, 20, "hp = hp - 2");
    assert_eq!(text(&script, "return Combat.active"), "0xW");
}

#[test]
fn a_caster_chasing_out_of_range_turns_on_an_attacker_in_range() {
    let mut script = mage();
    script
        .run("wolf.x, wolf.distance = 40, 40 shown = { gnome, wolf } Combat.engage('0xW')")
        .unwrap();
    script.tick(0.25);
    assert_eq!(text(&script, "return Combat.active"), "0xG");
    // Out of range the gnome only approaches: until it is within reach, the chase goes on.
    script
        .run("Combat.stop() gnome.distance = 30 Combat.engage('0xW')")
        .unwrap();
    script.tick(0.25);
    assert_eq!(text(&script, "return Combat.active"), "0xW");
}
