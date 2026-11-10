//! `wp.goals` and staying alive, against the stubbed world of `goals_tests`: which mobs the runner
//! pulls, when it rests, repairs or buys food first, and the objectives it leaves for later.

use super::goals_tests::{goals, text, ticks};

fn hostile(guid: &str, name: &str, level: u32, x: i32, y: i32) -> String {
    format!(
        "{{ guid = '{guid}', name = '{name}', kind = 'unit', reaction = 'hostile', level = {level}, \
         x = {x}, y = {y}, z = 0, distance = math.sqrt(({x}) ^ 2 + ({y}) ^ 2), dead = false }}"
    )
}

#[test]
fn grinding_takes_the_lone_mob_not_the_nearer_one_in_a_pack() {
    let mut script = goals();
    // Two wolves side by side 20 yards off, a lone boar at 40: the boar.
    script
        .run(&format!(
            "units = {{ {}, {}, {} }} Goals.on = true",
            hostile("0xA", "Gray Forest Wolf", 5, 20, 0),
            hostile("0xB", "Gray Forest Wolf", 5, 20, 6),
            hostile("0xL", "Rockhide Boar", 5, -40, 0),
        ))
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    assert_eq!(text(&script, "return Combat.active"), "0xL");
    // The boar dies (nothing on it to loot); then only the pair is in sight: no fight is
    // started, and it walks on to look for a safe one.
    script.run("units[3].dead = true").unwrap();
    ticks(&mut script, 2.5);
    script.run("table.remove(units, 3) said = {}").unwrap();
    ticks(&mut script, 1.5);
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
    assert_eq!(
        text(&script, "return tostring(Goals.facts().mobs_for_xp)"),
        "false"
    );
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    // A low one beside a mob of our level is a fair pull (the pair weighs under the limit).
    script.run("units[1].level, units[2].level = 4, 1").unwrap();
    assert_eq!(
        text(&script, "return tostring(Goals.facts().mobs_for_xp)"),
        "true"
    );
}

#[test]
fn no_fight_is_started_hurt_with_a_broken_weapon_or_on_a_mob_above_the_band() {
    let mut script = goals();
    // 60% health: rest comes before the wolf (the playtest pulled at 145/227).
    script.run("units = { wolf } hp = 60").unwrap();
    assert_eq!(
        text(
            &script,
            "local f = Goals.facts() return tostring(f.hp_low) .. ' ' .. tostring(f.pull_ok) .. ' ' .. f.recent_deaths"
        ),
        "true false 0"
    );
    assert_eq!(text(&script, "return plan_of('progress')"), "rest grind");
    // A main hand at 0 durability: no grinding until it is repaired; the repairer is known.
    script
        .run(
            "hp = 100 GetInventoryItemDurability = function(slot) if slot == 16 then return 0, 25 end end \
             smith = { guid = '0xS', name = 'Kurran Steele', kind = 'unit', reaction = 'friendly', npc = { 'vendor', 'repair' }, x = 50, y = 0, z = 0, distance = 50, dead = false } \
             units = { wolf, smith }",
        )
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local f = Goals.facts() return tostring(f.gear_broken) .. ' ' .. tostring(f.needs_repair)"
        ),
        "true true"
    );
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "sell_and_repair"
    );
    // A death is a fact the plan can read, and the wolf of our level is no longer pulled.
    script
        .run("GetInventoryItemDurability = nil units = { wolf } pos = { x = 500, y = 0, z = 0, map = 0 }")
        .unwrap();
    script.fire_event("PLAYER_DEAD", vec![]);
    script
        .run("pos = { x = 0, y = 0, z = 0, map = 0 } wolf.level = 6")
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local f = Goals.facts() return f.recent_deaths .. ' ' .. tostring(f.mobs_for_xp)"
        ),
        "1 false"
    );
}

#[test]
fn an_objective_whose_mob_is_no_safe_fight_is_left_for_later_not_died_on() {
    let mut script = goals();
    // The playtest's People's Militia at level 11: the Trappers are level 13.
    script
        .run(&format!(
            "lvl = 11 LOG = {{ {{ id = 12, title = 'The People\\'s Militia', level = 12, objectives = {{ {{ 'Defias Trapper slain: 0/15', 'monster', false }} }} }} }} \
             units = {{ {} }} Goals.on = true",
            hostile("0xD", "Defias Trapper", 13, 25, 0),
        ))
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:work_objective");
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
    // It stands off for a while (a patrol moves on), then gives the objective up with the reason.
    script
        .run("local t = GetTime GetTime = function() return t() + 30 end")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(
        text(&script, "return heard('ACTION_FAILED')"),
        "ACTION_FAILED progress work_objective Defias Trapper is not a safe fight now: level 13, above the level 12 we pull now"
    );
    assert_eq!(text(&script, "return tostring(Combat.active)"), "nil");
}

#[test]
fn hurt_the_runner_rests_before_a_walk_and_food_is_bought_where_it_is_sold() {
    let mut script = goals();
    // A finished quest's giver in sight, at 50% health: rest first, then the hand-in.
    script
        .run(
            "hp = 50 units = { { guid = '0xR', name = 'Remy', kind = 'unit', quest = 'reward', x = 8, y = 0, z = 0, distance = 8 } } \
             Goals.on = true",
        )
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "hand_in_quests"
    );
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:rest");
    // Nothing to eat, money in hand, and an innkeeper seen: buying food is a step of its own.
    script
        .run(
            "Goals.on = false Agent.stop('test') hp = 100 money = 400 GetMoney = function() return money end \
             GetContainerNumSlots = function() return 0 end \
             units = { wolf, { guid = '0xI', name = 'Innkeeper Farley', kind = 'unit', reaction = 'friendly', npc = { 'innkeeper', 'vendor' }, x = 50, y = 0, z = 0, distance = 50, dead = false } }",
        )
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local f = Goals.facts() return tostring(f.needs_food) .. ' ' .. tostring(f.food_vendor_known) .. ' ' .. Goals.places.food[1].name"
        ),
        "true true Innkeeper Farley"
    );
    assert_eq!(text(&script, "return plan_of('progress')"), "buy_food");
}

#[test]
fn a_trainer_who_greets_with_gossip_is_asked_for_training() {
    let mut script = goals();
    // AzerothCore's class trainers open a gossip window first; nobody answering it left the
    // train step standing there for its whole timeout.
    script
        .run(
            "lvl = 6 picked = {} \
             GetGossipOptions = function() return 'Tell me about Stormwind.', 'gossip', 'I require warrior training.', 'trainer' end \
             SelectGossipOption = function(i) picked[#picked + 1] = i end \
             units = { { guid = '0xT', name = 'Llane Beshere', title = 'Warrior Trainer', kind = 'unit', reaction = 'friendly', npc = { 'trainer', 'classtrainer' }, x = 10, y = 0, z = 0, distance = 10, dead = false } }",
        )
        .unwrap();
    // No visit under way: somebody else's gossip window is left alone.
    script.fire_event("GOSSIP_SHOW", vec![]);
    assert_eq!(text(&script, "return tostring(#picked)"), "0");
    script.run("Goals.on = true").unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:train");
    script.fire_event("GOSSIP_SHOW", vec![]);
    assert_eq!(text(&script, "return table.concat(picked, ',')"), "2");
}

#[test]
fn a_zone_that_suits_us_is_not_left_because_nothing_safe_is_in_sight() {
    let script = goals();
    // Level 4 in Elwynn with only level 6 wolves about (the live check: after a death nothing in
    // sight was safe, looking around failed, and the runner set off on foot for Dun Morogh).
    script
        .run(&format!(
            r#"
            lvl = 4
            AgentZones = function() return {{
              {{ id = 12, name = "Elwynn Forest", map = 0, faction = "Alliance", city = false, min = 1, max = 10, x = 0, y = 0 }},
              {{ id = 1, name = "Dun Morogh", map = 0, faction = "Alliance", city = false, min = 5, max = 10, x = 3000, y = 0 }},
            }} end
            units = {{ {} }}
            "#,
            hostile("0xA", "Mangy Wolf", 6, 30, 0),
        ))
        .unwrap();
    assert_eq!(
        text(
            &script,
            "local f = Goals.facts() return f.zone_fit .. ' ' .. tostring(f.mobs_for_xp)"
        ),
        "ok false"
    );
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "seek_mobs grind"
    );
    // Even with the looking-around backed off, the plan is to look again, not to leave.
    script
        .run("Goals.failures.seek_mobs = { n = 1, until_ = GetTime() + 100, why = 'nothing' }")
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "seek_mobs grind"
    );
    // The planner's own choice of zone still goes.
    let may_travel = "return tostring(Goals.actions.travel_to_zone.pre(Goals.facts()))";
    assert_eq!(text(&script, may_travel), "false");
    script
        .run("Goals.push({ name = 'move_on', why = 'Dun Morogh next', priority = 60, data = { zone = 'Dun Morogh' } })")
        .unwrap();
    assert_eq!(text(&script, may_travel), "true");
}

#[test]
fn while_it_runs_from_a_fight_the_goal_runner_starts_nothing() {
    let mut script = goals();
    // Out of combat already (the mob gave up its swing) but still running from it: the runner
    // keeps its walks to itself, as it does in a fight, and the fast tier is offered none.
    script
        .run(
            "wolf.in_combat, wolf.target = true, '0xME' units = { wolf } \
             Combat.fleeing = { since = GetTime(), why = 'test' } Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return Goals.state"), "fleeing");
    assert_eq!(text(&script, "return doing()"), "idle");
    assert_eq!(
        text(&script, "local o = {} for _, i in ipairs(Agent.available()) do if i.name:find('goal:') then o[#o + 1] = i.name end end return table.concat(o, ' ')"),
        ""
    );
}
