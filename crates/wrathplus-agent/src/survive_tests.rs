//! `wp.survive` and what Combat does with it, against a stubbed world: which pulls are safe, what a
//! death changes, potions, fleeing, and defence that nothing can switch off. The cases are the
//! deaths of the level 1-11 playtest (a warrior in Elwynn, Westfall and Redridge).

use wrathplus_ui::script::UiScript;

/// A level 11 warrior at full health, a two-hander in hand, Hamstring and Heroic Strike known.
const GAME: &str = r##"
calls, said = {}, {}
now = 1000
lvl, hp, hpmax, rage, combat, dead = 11, 250, 250, 30, false, false
pos = { x = 0, y = 0, z = 0, facing = 0, map = 0 }
units, walking, durability = {}, false, 25
bag = {}
BOOK = { "Hamstring", "Heroic Strike", "Charge" }
RANGE = { Charge = { 8, 25 } }
usable = { Hamstring = true, ["Heroic Strike"] = true, Charge = true }
function mob(guid, name, level, x, y, more)
  local u = { guid = guid, name = name, kind = "unit", reaction = "hostile", level = level, x = x, y = y, z = 0,
    health = 200, health_max = 200, dead = false, in_combat = false }
  for k, v in pairs(more or {}) do u[k] = v end
  return u
end
GetTime = function() return now end
AgentNearby = function(yards)
  local out = {}
  for _, u in ipairs(units) do
    u.distance = math.sqrt((u.x - pos.x) ^ 2 + (u.y - pos.y) ^ 2)
    if not yards or u.distance <= yards then out[#out + 1] = u end
  end
  table.sort(out, function(a, b) return a.distance < b.distance end)
  return out
end
AgentPosition = function() return pos end
AgentTarget = function(g) target = g calls[#calls + 1] = "target " .. g end
AgentInteract = function(g) calls[#calls + 1] = "swing " .. g end
AgentFace = function(g) calls[#calls + 1] = "face " .. g end
AgentMoveTo = function(x, y) calls[#calls + 1] = string.format("move %d,%d", x, y) walking = true end
AgentStop = function() walking = false end
AgentWalk = function() return { status = walking and "walking" or "arrived" } end
AgentSignal = function(name, ...)
  local parts = { name }
  for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
  said[#said + 1] = table.concat(parts, " ")
end
UnitLevel = function() return lvl end
UnitHealth = function(u) return u == "player" and hp or 100 end
UnitHealthMax = function(u) return u == "player" and hpmax or 100 end
UnitPower = function() return rage end
UnitPowerMax = function() return 100 end
UnitPowerType = function() return 1 end
UnitClass = function() return "Warrior", "WARRIOR" end
UnitGUID = function(u) if u == "player" then return "0xME" end return target end
UnitExists = function(u) return u == "player" or (u == "target" and target ~= nil) end
UnitAffectingCombat = function() return combat end
UnitIsDeadOrGhost = function() return dead end
UnitAura = function() return nil end
GetInventoryItemDurability = function(slot) if slot == 16 then return durability, 25 end end
GetSpellName = function(i) return BOOK[i] end
GetSpellInfo = function(s)
  local r = RANGE[s] or { 0, 0 }
  return s, "", "", 10, false, 1, 0, r[1], r[2]
end
GetSpellCooldown = function() return 0, 0, 1 end
IsUsableSpell = function(s) return usable[s] and rage >= 10 or nil end
CastSpellByName = function(s) calls[#calls + 1] = "cast " .. s end
GetContainerNumSlots = function(b) return b == 0 and #bag or 0 end
GetContainerItemLink = function(b, s) return b == 0 and bag[s] or nil end
GetContainerItemInfo = function() return nil, 1 end
GetItemInfo = function(link) return link, link, 1, 5, 1 end
GetItemSpell = function(link) return link == "Minor Healing Potion" and "Healing Potion" or nil end
UseContainerItem = function(b, s) calls[#calls + 1] = "use " .. bag[s] bag[s] = nil end
function heard(prefix)
  local out = {}
  for _, s in ipairs(said) do if s:find(prefix, 1, true) == 1 then out[#out + 1] = s end end
  return table.concat(out, " | ")
end
function did(prefix)
  local out = {}
  for _, s in ipairs(calls) do if s:find(prefix, 1, true) == 1 then out[#out + 1] = s end end
  return table.concat(out, " | ")
end
"##;

fn world() -> UiScript {
    let dir = std::env::temp_dir().join(format!(
        "wp-agent-survive-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

/// A fight second by second: we lose `taken` a second, each unit in `hit` loses `dealt`.
fn fight(script: &mut UiScript, seconds: u32, taken: f32, dealt: f32, hit: &str) {
    for _ in 0..seconds * 4 {
        script
            .run(&format!(
                "now = now + 0.25 hp = math.max(1, hp - {taken} / 4) \
                 for _, u in ipairs({{ {hit} }}) do u.health = math.max(1, u.health - {dealt} / 4) end"
            ))
            .unwrap();
        script.tick(0.25);
    }
}

fn text(script: &UiScript, lua: &str) -> String {
    script.eval(lua).unwrap()
}

fn verdict(script: &UiScript, unit: &str) -> String {
    text(
        script,
        &format!("local ok, why = Survive.pull_ok({unit}) return ok and 'ok' or why"),
    )
}

#[test]
fn a_pull_is_safe_inside_the_level_band_alone_and_no_elite_rare_critter_or_anothers_tap() {
    let script = world();
    script
        .run(
            "boar = mob('0xB', 'Rockhide Boar', 10, 30, 0) \
             thrasher = mob('0xT', 'Redridge Thrasher', 14, 30, 0) \
             trapper = mob('0xD', 'Defias Trapper', 13, 30, 0) \
             spider = mob('0xS', 'Forest Spider', 5, 30, 0) \
             hogger = mob('0xH', 'Hogger', 11, 30, 0, { classification = 'elite' }) \
             rare = mob('0xR', 'Mother Fang', 10, 30, 0, { classification = 'rare' }) \
             sheep = mob('0xC', 'Sheep', 10, 30, 0, { critter = true }) \
             taken = mob('0xX', 'Coyote', 10, 30, 0, { tapped = true }) \
             mine = mob('0xY', 'Coyote', 10, 300, 0, { tapped = true, tapped_by_me = true }) \
             units = { boar }",
        )
        .unwrap();
    assert_eq!(verdict(&script, "boar"), "ok");
    // Level 11 pulls 6 (above grey) to 12: the Thrasher (14) and the Trapper (13) of the playtest
    // are out, and so is a grey spider.
    assert_eq!(
        text(&script, "return table.concat({ Survive.band() }, '-')"),
        "6-12"
    );
    assert_eq!(
        verdict(&script, "thrasher"),
        "level 14, above the level 12 we pull now"
    );
    assert_eq!(
        verdict(&script, "trapper"),
        "level 13, above the level 12 we pull now"
    );
    assert_eq!(verdict(&script, "spider"), "grey: no XP at level 5");
    assert_eq!(verdict(&script, "hogger"), "an elite");
    assert_eq!(verdict(&script, "rare"), "a rare");
    assert_eq!(verdict(&script, "sheep"), "a critter");
    assert_eq!(verdict(&script, "taken"), "another player's");
    assert_eq!(verdict(&script, "mine"), "ok");
}

#[test]
fn hostiles_in_joining_distance_health_and_a_broken_weapon_stop_a_pull() {
    let script = world();
    // The bear of the first death: 12 yards from the boar, inside its own aggro radius of it.
    script
        .run(
            "boar = mob('0xB', 'Rockhide Boar', 10, 30, 0) \
             bear = mob('0xA', 'Young Forest Bear', 9, 30, 12) \
             units = { boar, bear }",
        )
        .unwrap();
    assert_eq!(
        verdict(&script, "boar"),
        "1 more hostile would join (Young Forest Bear)"
    );
    // A neutral (yellow) neighbour does not join, nor one far enough away.
    script.run("bear.reaction = 'neutral'").unwrap();
    assert_eq!(verdict(&script, "boar"), "ok");
    script.run("bear.reaction, bear.y = 'hostile', 40").unwrap();
    assert_eq!(verdict(&script, "boar"), "ok");
    // The fight started at 145/227 in the playtest, on a Coyote of our level: not below 70%.
    // A mob below us needs less (the level 10 boar 60%), one above us more (level 12: 88%).
    script.run("hp = 160").unwrap();
    assert_eq!(verdict(&script, "boar"), "ok");
    script.run("boar.level = 11").unwrap();
    assert_eq!(
        verdict(&script, "boar"),
        "health 64%: rest first (70% for this one)"
    );
    script.run("boar.level, hp = 12, 200").unwrap();
    assert_eq!(
        verdict(&script, "boar"),
        "health 80%: rest first (88% for this one)"
    );
    script.run("boar.level = 10").unwrap();
    // And the Coyote fought with a main hand at 0 durability.
    script.run("hp = 250 durability = 0").unwrap();
    assert_eq!(
        verdict(&script, "boar"),
        "the main hand is broken: repair first"
    );
    // A mob already on us is always fair: that is defence, not a pull.
    script
        .run("boar.in_combat, boar.target = true, '0xME'")
        .unwrap();
    assert_eq!(verdict(&script, "boar"), "ok");
}

#[test]
fn a_death_lowers_the_band_and_closes_its_spot_for_a_while() {
    let mut script = world();
    script
        .run(
            "units = { mob('0xG', 'Riverpaw Gnoll', 12, 5, 0, { in_combat = true, target = '0xME' }) } \
             near = mob('0xN', 'Coyote', 10, 30, 0) far_one = mob('0xF', 'Coyote', 10, 300, 0) \
             same = mob('0xL', 'Coyote', 11, 300, 0)",
        )
        .unwrap();
    assert_eq!(
        text(&script, "return tostring(Survive.recent_deaths())"),
        "0"
    );
    script.fire_event("PLAYER_DEAD", vec![]);
    assert_eq!(
        text(
            &script,
            "return Survive.recent_deaths() .. ' ' .. Survive.deaths[1].killer"
        ),
        "1 Riverpaw Gnoll"
    );
    // One death: two levels off the top (12 → 10), and nothing within 50 yards of the spot.
    script.run("units = {}").unwrap();
    assert_eq!(
        text(&script, "return table.concat({ Survive.band() }, '-')"),
        "6-10"
    );
    assert_eq!(
        verdict(&script, "near"),
        "close to where we died to Riverpaw Gnoll"
    );
    assert_eq!(verdict(&script, "far_one"), "ok");
    assert_eq!(
        verdict(&script, "same"),
        "level 11, above the level 10 we pull now"
    );
    // Ten minutes on it is forgotten.
    script.run("now = now + 601").unwrap();
    assert_eq!(verdict(&script, "near"), "ok");
    assert_eq!(verdict(&script, "same"), "ok");
}

#[test]
fn combat_refuses_an_unsafe_pull_unless_forced_and_says_why() {
    let script = world();
    script
        .run("units = { mob('0xT', 'Redridge Thrasher', 14, 30, 0) }")
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local ok, why = Combat.engage('0xT') return tostring(ok) .. ': ' .. why"
        ),
        "false: level 14, above the level 12 we pull now"
    );
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
    assert_eq!(
        text(&script, "return heard('PULL_REFUSED')"),
        "PULL_REFUSED Redridge Thrasher level 14, above the level 12 we pull now"
    );
    // Asked again at once: refused again, said once.
    script.run("Combat.engage('0xT')").unwrap();
    assert_eq!(
        text(&script, "return tostring(#said)"),
        "1",
        "one refusal a minute reaches the planner"
    );
    // The planner's call: forced, or the check switched off.
    assert!(script
        .eval::<bool>("return Combat.engage('0xT', { force = true })")
        .unwrap());
    script.run("Combat.stop() Combat.safe = false").unwrap();
    assert!(script.eval::<bool>("return Combat.engage('0xT')").unwrap());
}

#[test]
fn defence_fights_back_whatever_a_wrapper_round_engage_says() {
    let mut script = world();
    // The playtest's module wrapped Combat.engage to refuse grey mobs; a level 5 spider then
    // killed a level 11 that never swung back. Defence does not go through the wrapper, and a
    // grey mob on us is fought.
    script
        .run(
            "Combat.engage = function() return false end \
             units = { mob('0xS', 'Forest Spider', 5, 2, 0, { in_combat = true, target = '0xME' }) } \
             combat = true",
        )
        .unwrap();
    script.tick(0.25);
    assert_eq!(text(&script, "return tostring(Combat.active)"), "0xS");
    script.tick(0.25);
    assert_eq!(text(&script, "return did('swing')"), "swing 0xS");
}

#[test]
fn losing_and_low_it_drinks_the_potion_it_carries() {
    let mut script = world();
    script
        .run(
            "coyote = mob('0xC', 'Coyote', 11, 2, 0, { in_combat = true, target = '0xME' }) \
             units = { coyote } combat = true bag = { 'Minor Healing Potion' } \
             usable.Hamstring = false \
             Combat.engage('0xC')",
        )
        .unwrap();
    // 20 a second in, 5 out: we last 12 s, it lasts 40. Too much to outrun without a snare
    // (14 s at our heels), so it fights on, and at 40% drinks.
    fight(&mut script, 7, 20.0, 5.0, "coyote");
    assert_eq!(text(&script, "return did('use')"), "");
    assert_eq!(text(&script, "return tostring(Combat.fleeing)"), "nil");
    fight(&mut script, 2, 20.0, 5.0, "coyote");
    assert_eq!(
        text(&script, "return did('use')"),
        "use Minor Healing Potion"
    );
    assert_eq!(text(&script, "return tostring(Combat.active)"), "0xC");
}

#[test]
fn a_lost_fight_is_left_with_a_snare_and_defence_does_not_drag_it_back() {
    let mut script = world();
    script
        .run(
            "thrasher = mob('0xT', 'Redridge Thrasher', 14, 2, 0, { in_combat = true, target = '0xME', health = 285, health_max = 285 }) \
             pack = mob('0xP', 'Redridge Mongrel', 16, -40, 0) \
             units = { thrasher, pack } combat = true",
        )
        .unwrap();
    // It jumped us on a walk: defence takes it on.
    script.tick(0.25);
    assert_eq!(text(&script, "return tostring(Combat.active)"), "0xT");
    // 14 a second in, 4 out: decisively lost (we last 15 s, it 60), and with Hamstring the run
    // is survivable: snare, then run, away from the Thrasher and not toward the Mongrel.
    fight(&mut script, 6, 14.0, 4.0, "thrasher");
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
    assert_eq!(
        text(&script, "return did('cast Hamstring')"),
        "cast Hamstring"
    );
    let leg = text(&script, "return did('move')");
    let x: i32 = leg
        .trim_start_matches("move ")
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        x.abs() < 40,
        "runs sideways: the Thrasher is east, the Mongrel west ({leg})"
    );
    assert_eq!(
        text(
            &script,
            "return heard('COMBAT_DONE') .. ' / ' .. heard('FLEEING')"
        ),
        "COMBAT_DONE 0xT fled / FLEEING losing to Redridge Thrasher"
    );
    // A fight fled is a close call: for ten minutes nothing above our own level is pulled.
    assert_eq!(
        text(
            &script,
            "return Survive.recent_scares() .. ' ' .. table.concat({ Survive.band() }, '-')"
        ),
        "1 6-11"
    );
    // Still on us and hitting: defence stays out of it while we run.
    for _ in 0..8 {
        script.tick(0.25);
    }
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
    assert_eq!(text(&script, "return Combat.state"), "fleeing (Hamstring)");
    // It gives up: out of combat, the run is over and said so.
    script
        .run("thrasher.in_combat, thrasher.target, combat = false, nil, false now = now + 6")
        .unwrap();
    script.tick(0.25);
    assert_eq!(text(&script, "return tostring(Combat.fleeing)"), "nil");
    assert_eq!(text(&script, "return heard('FLED')"), "FLED 6");
}

#[test]
fn a_second_attacker_turns_a_won_fight_into_one_to_leave() {
    let mut script = world();
    script
        .run(
            "boar = mob('0xB', 'Rockhide Boar', 10, 2, 0, { in_combat = true, target = '0xME' }) \
             bear = mob('0xA', 'Young Forest Bear', 9, 60, 0, { health = 220, health_max = 220 }) \
             units = { boar, bear } combat = true \
             Combat.engage('0xB')",
        )
        .unwrap();
    // One boar: 8 in, 12 out. Winning.
    fight(&mut script, 6, 8.0, 12.0, "boar");
    assert_eq!(text(&script, "return tostring(Combat.active)"), "0xB");
    assert_eq!(
        text(&script, "return tostring(Survive.last.losing)"),
        "false"
    );
    // The bear joins: 22 in against the same 12 out, and 348 health of mobs to our 200.
    script
        .run("bear.x, bear.in_combat, bear.target = 3, true, '0xME'")
        .unwrap();
    fight(&mut script, 6, 22.0, 12.0, "boar");
    assert_eq!(
        text(&script, "return tostring(Survive.last.attackers)"),
        "2"
    );
    assert_eq!(
        text(&script, "return heard('FLEEING')"),
        "FLEEING losing to Rockhide Boar"
    );
}

#[test]
fn a_run_that_cannot_be_survived_is_not_started_and_one_that_fails_ends_in_a_fight() {
    let mut script = world();
    script
        .run(
            "mongrel = mob('0xM', 'Redridge Mongrel', 16, 2, 0, { in_combat = true, target = '0xME', health = 356, health_max = 356 }) \
             units = { mongrel } combat = true usable.Hamstring = false hp = 120",
        )
        .unwrap();
    script.tick(0.25);
    // 15 a second with 14 s of it at our heels is more than the 120 we have: fight it out.
    fight(&mut script, 6, 15.0, 3.0, "mongrel");
    assert_eq!(text(&script, "return tostring(Combat.active)"), "0xM");
    assert_eq!(text(&script, "return heard('FLEEING')"), "");
    // Told to run anyway (the planner's call): after `flee_for` seconds with it still on us, it
    // turns and fights, and does not run again for a while.
    script.run("hp = 250 Combat.flee('my call')").unwrap();
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
    for _ in 0..90 {
        script.run("now = now + 0.25").unwrap();
        script.tick(0.25);
    }
    assert_eq!(text(&script, "return tostring(Combat.fleeing)"), "nil");
    assert_eq!(text(&script, "return tostring(Combat.active)"), "0xM");
    fight(&mut script, 8, 8.0, 1.0, "mongrel");
    assert_eq!(
        text(&script, "return tostring(Combat.active)"),
        "0xM",
        "cornered: no second run"
    );
}

#[test]
fn it_turns_to_a_mob_that_steps_out_of_its_front_arc_and_charges_on_the_way_in() {
    let mut script = world();
    script
        .run(
            "boar = mob('0xB', 'Rockhide Boar', 10, 20, 0) units = { boar } rage = 0 \
             IsUsableSpell = function(s) return s == 'Charge' and not combat or nil end \
             Combat.engage('0xB')",
        )
        .unwrap();
    // 20 yards off, out of combat: Charge goes on the approach (its band is 8-25 yards); the melee
    // spells wait for reach.
    script.run("now = now + 0.25").unwrap();
    script.tick(0.25);
    assert_eq!(text(&script, "return did('cast')"), "cast Charge");
    // In reach, facing it (east, facing 0): a swing, no turn.
    script
        .run("boar.x, combat, now, calls = 2, true, now + 2, {}")
        .unwrap();
    script.tick(0.25);
    assert_eq!(
        text(&script, "return table.concat(calls, ' | ')"),
        "swing 0xB"
    );
    // It steps round behind us between swings: turn at once, not at the next swing.
    script.run("boar.x, now = -2, now + 0.5").unwrap();
    script.tick(0.25);
    assert_eq!(
        text(&script, "return table.concat(calls, ' | ')"),
        "swing 0xB | face 0xB"
    );
}
