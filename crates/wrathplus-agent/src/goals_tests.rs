//! `wp.goals` against a stubbed world: plans, the runner, holds, the planner's own goals and
//! actions, replanning on events, and failures that back off.

use wrathplus_ui::script::UiScript;

/// A level 5 warrior alone in a quiet spot: no quests, nobody in sight until a test adds them.
const GAME: &str = r##"
calls, said = {}, {}
lvl, hp, hpmax, combat, walking = 5, 100, 100, false, false
pos = { x = 0, y = 0, z = 0, map = 0 }
units = {}
LOG = {}
wolf = { guid = "0xW", name = "Mangy Wolf", kind = "unit", reaction = "hostile", level = 5,
  x = 20, y = 0, z = 0, distance = 20, dead = false }
AgentNearby = function(yards)
  local out = {}
  for _, u in ipairs(units) do if not yards or u.distance <= yards then out[#out + 1] = u end end
  return out
end
AgentPosition = function() return pos end
AgentInteract = function(g) calls[#calls + 1] = "interact " .. g end
AgentTarget = function(g) calls[#calls + 1] = "target " .. g end
AgentMoveTo = function(x, y) calls[#calls + 1] = "move" walking = true return {} end
AgentStop = function() walking = false end
AgentWalk = function() return { status = walking and "walking" or "arrived" } end
AgentFace = function() end
AgentAvoid = function() end
AgentSignal = function(name, ...)
  local parts = { name }
  for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
  said[#said + 1] = table.concat(parts, " ")
end
UnitLevel = function() return lvl end
UnitHealth = function() return hp end
UnitHealthMax = function() return hpmax end
UnitPowerType = function() return 1 end
UnitAffectingCombat = function() return combat end
UnitIsDeadOrGhost = function() return false end
UnitClass = function() return "Warrior", "WARRIOR" end
UnitGUID = function() return "0xME" end
UnitFactionGroup = function() return "Alliance" end
GetRealZoneText = function() return "Elwynn Forest" end
GetNumPartyMembers = function() return 0 end
GetContainerNumFreeSlots = function() return 10 end
GetNumQuestLogEntries = function() return #LOG end
GetQuestLogTitle = function(i)
  local q = LOG[i]
  return q.title, q.level, nil, 0, nil, nil, q.complete and 1 or nil, nil, q.id
end
GetNumQuestLeaderBoards = function(i) return #LOG[i].objectives end
GetQuestLogLeaderBoard = function(j, i) return unpack(LOG[i].objectives[j]) end
function heard(prefix)
  local out = {}
  for _, s in ipairs(said) do if s:find(prefix, 1, true) == 1 then out[#out + 1] = s end end
  return table.concat(out, " | ")
end
function plan_of(name)
  for _, g in ipairs(Goals.list()) do if g.name == name then return table.concat(g.plan, " ") end end
end
function doing() local a = Agent.activity() return a and a.name or "idle" end
"##;

pub(super) fn goals() -> UiScript {
    let dir = std::env::temp_dir().join(format!("wp-agent-goals-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let script = UiScript::new().unwrap();
    super::install(&script, dir).unwrap();
    script.run(GAME).unwrap();
    script
}

pub(super) fn text(script: &UiScript, lua: &str) -> String {
    script.eval(lua).unwrap()
}

pub(super) fn ticks(script: &mut UiScript, seconds: f32) {
    let mut left = seconds;
    while left > 0.0 {
        script.tick(0.25);
        left -= 0.25;
    }
}

/// An errand of the planner's: earning, buying and eating, against foraging.
const ERRAND: &str = r#"
Goals.fact("gold", function() return have_gold == true end)
Goals.fact("food", function() return false end)
Goals.push({ name = "fed", why = "Eat before the run", want = { fed = true } })
Goals.action({ name = "earn_gold", effects = { gold = true }, cost = 2, run = function() end })
Goals.action({ name = "buy_food", pre = { gold = true }, effects = { food = true }, cost = 3, run = function() end })
Goals.action({ name = "eat", pre = { food = true }, effects = { fed = true }, cost = 1, run = function() end })
Goals.action({ name = "forage", effects = { fed = true }, cost = function() return forage_cost end, run = function() end })
forage_cost = 10
"#;

#[test]
fn planning_picks_the_cheapest_chain() {
    let script = goals();
    script.run(ERRAND).unwrap();
    // Earn, buy and eat (6) beat foraging (10); with gold in hand the chain starts at buying.
    assert_eq!(
        text(&script, "return plan_of('fed')"),
        "earn_gold buy_food eat"
    );
    script.run("have_gold = true").unwrap();
    assert_eq!(text(&script, "return plan_of('fed')"), "buy_food eat");
    script.run("have_gold = false forage_cost = 5").unwrap();
    assert_eq!(text(&script, "return plan_of('fed')"), "forage");
    // The built-in progress: hurt with a wolf in sight, rest first, then fight it; a finished
    // quest's giver in sight comes before both.
    script.run("units = { wolf } hp = 40").unwrap();
    assert_eq!(text(&script, "return plan_of('progress')"), "rest grind");
    script
        .run("units = { wolf, { guid = '0xR', name = 'Remy', kind = 'unit', quest = 'reward', distance = 8 } }")
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "hand_in_quests"
    );
    // Highest priority first; the planner's goal above progress.
    assert_eq!(
        text(&script, "local n = {} for _, g in ipairs(Goals.list()) do n[#n + 1] = g.name .. '=' .. g.status end return table.concat(n, ' ')"),
        "fed=active progress=active"
    );
}

#[test]
fn the_runner_starts_the_next_action_when_idle_and_never_idles() {
    let mut script = goals();
    script.run("units = { wolf } Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    assert!(text(&script, "return table.concat(calls, ' ')").starts_with("target 0xW"));
    // The fast tier sees the runner's step, and may choose it itself.
    assert_eq!(
        text(&script, "local o = {} for _, i in ipairs(Agent.available()) do if i.name:find('goal:') then o[#o + 1] = i.name end end return table.concat(o, ' ')"),
        "goal:grind"
    );
    // The wolf dies (nothing on it to loot: the fight ends after Combat.loot_wait): the activity
    // ends well, and the runner looks for the next thing at once.
    script.run("wolf.dead = true").unwrap();
    ticks(&mut script, 2.0);
    assert_eq!(
        text(&script, "return heard('INTENT_DONE')"),
        "INTENT_DONE goal:grind true killed"
    );
    // Nothing in sight now, but the zone suits us: it walks on to look, not idling.
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:seek_mobs");
    // A second wolf shows up: the search is over and the fight starts.
    script
        .run("units = { { guid = '0xV', name = 'Young Wolf', kind = 'unit', reaction = 'hostile', level = 4, x = 30, y = 0, z = 0, distance = 30 } }")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
}

#[test]
fn a_quest_objective_is_worked_before_grinding() {
    let mut script = goals();
    script
        .run(
            "LOG = { { id = 7, title = 'Kobold Camp Cleanup', level = 4, objectives = { { 'Kobold Vermin slain: 0/10', 'monster', false } } } } \
             kobold = { guid = '0xK', name = 'Kobold Vermin', kind = 'unit', reaction = 'hostile', level = 3, x = 15, y = 0, z = 0, distance = 15 } \
             units = { wolf, kobold } Goals.on = true",
        )
        .unwrap();
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "work_objective"
    );
    ticks(&mut script, 0.75);
    assert_eq!(text(&script, "return doing()"), "goal:work_objective");
    assert_eq!(text(&script, "return Combat.active"), "0xK");
    // The kobold dies and the log counts it: one step done, and the next one starts.
    script
        .run("kobold.dead = true LOG[1].objectives[1][1] = 'Kobold Vermin slain: 1/10'")
        .unwrap();
    ticks(&mut script, 0.75);
    assert_eq!(
        text(&script, "return heard('INTENT_DONE')"),
        "INTENT_DONE goal:work_objective true nil"
    );
}

#[test]
fn holds_are_bounded_signal_on_expiry_and_keep_counting_while_renewed() {
    let mut script = goals();
    script.run("units = { wolf } Goals.on = true").unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    // Waiting for the party stops the step under way; an interruption is not a failure.
    script
        .run("Goals.hold({ reason = 'waiting for Pallytest', seconds = 10 })")
        .unwrap();
    assert_eq!(text(&script, "return doing()"), "idle");
    assert_eq!(
        text(&script, "return heard('INTENT_DONE')"),
        "INTENT_DONE goal:grind false hold: waiting for Pallytest"
    );
    ticks(&mut script, 4.0);
    let (reason, waited, remaining): (String, i64, i64) =
        script.eval("return Goals.holding()").unwrap();
    assert_eq!(
        (reason.as_str(), waited, remaining),
        ("waiting for Pallytest", 4, 6)
    );
    assert_eq!(text(&script, "return doing()"), "idle");
    // Over: signalled with the time waited, and a moment's grace for the fast tier to answer.
    ticks(&mut script, 6.25);
    assert_eq!(
        text(&script, "return heard('HOLD_EXPIRED')"),
        "HOLD_EXPIRED waiting for Pallytest 10"
    );
    assert_eq!(text(&script, "return doing()"), "idle");
    // It keeps waiting: the same reason counts on from the first hold.
    script
        .run("Goals.hold({ reason = 'waiting for Pallytest', seconds = 5 })")
        .unwrap();
    ticks(&mut script, 5.5);
    assert_eq!(
        text(&script, "return heard('HOLD_EXPIRED')"),
        "HOLD_EXPIRED waiting for Pallytest 10 | HOLD_EXPIRED waiting for Pallytest 15"
    );
    // No answer within the grace: the runner carries on.
    ticks(&mut script, 3.5);
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    assert_eq!(text(&script, "return heard('ACTION_FAILED')"), "");
    // No unlimited form.
    for bad in [
        "Goals.hold({ reason = 'x' })",
        "Goals.hold({ reason = 'x', seconds = math.huge })",
        "Goals.hold({ seconds = 5 })",
    ] {
        assert!(script.run(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_planners_own_goals_facts_and_actions_are_used() {
    let mut script = goals();
    script
        .run(
            r#"
            units = { wolf } Goals.on = true
            hogger_down, swings = false, 0
            Goals.fact("hogger_dead", function() return hogger_down end)
            Goals.action({ name = "fight_hogger", why = "Pull Hogger with the party",
              pre = { in_combat = false }, effects = { hogger_dead = true }, cost = 3,
              run = function() swings = swings + 1 end,
              done = function() return hogger_down, true end })
            Goals.push({ name = "kill_hogger", why = "The bounty", priority = 80,
              want = { hogger_dead = true }, done = "hogger_down" })
            "#,
        )
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:fight_hogger");
    script.run("hogger_down = true").unwrap();
    ticks(&mut script, 1.0);
    // Its `done`, sent as text, ends the goal; progress takes over.
    assert_eq!(
        text(&script, "return heard('GOAL_DONE')"),
        "GOAL_DONE kill_hogger true"
    );
    assert_eq!(text(&script, "return doing()"), "goal:grind");
    let (swings, status): (i64, String) = script
        .eval("return swings, Goals.list()[1].status")
        .unwrap();
    assert_eq!((swings, status.as_str()), (1, "done"));
    // A goal no action reaches is blocked, said once.
    script
        .run("Goals.push({ name = 'find_the_relic', why = 'Nobody knows how', priority = 90 })")
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(
        text(&script, "return heard('GOAL_BLOCKED')"),
        "GOAL_BLOCKED find_the_relic no plan reaches find_the_relic = true now"
    );
    assert!(script
        .eval::<bool>("return Goals.drop('progress') == false")
        .unwrap());
}

#[test]
fn it_plans_again_on_the_agents_signals_and_the_clients_events() {
    let mut script = goals();
    script
        .run(
            r#"
            Goals.on = true
            finished = false
            Goals.push({ name = "wait_for_word", priority = 70, done = function() return finished, true end })
            -- Something else has the agent: the runner starts nothing, only plans.
            Agent.intent("chat", { when = "x", run = function() end, done = function() return false end })
            Agent.run("chat")
            "#,
        )
        .unwrap();
    ticks(&mut script, 0.5);
    script.run("finished = true").unwrap();
    // Between passes nothing has changed as far as it knows.
    ticks(&mut script, 1.0);
    assert_eq!(text(&script, "return heard('GOAL_DONE')"), "");
    // A kill (an agent signal) makes it look again at once.
    script
        .run("Agent.signal('COMBAT_DONE', '0xW', 'killed')")
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(
        text(&script, "return heard('GOAL_DONE')"),
        "GOAL_DONE wait_for_word true"
    );
    // So does the quest log changing.
    script
        .run("finished = false Goals.push({ name = 'log_changed', priority = 70, done = function() return finished, true end })")
        .unwrap();
    ticks(&mut script, 0.5);
    script.run("finished = true").unwrap();
    ticks(&mut script, 1.0);
    assert!(!text(&script, "return heard('GOAL_DONE')").contains("log_changed"));
    script.fire_event("QUEST_LOG_UPDATE", vec![]);
    ticks(&mut script, 0.5);
    assert!(text(&script, "return heard('GOAL_DONE')").contains("GOAL_DONE log_changed true"));
}

#[test]
fn a_failing_action_is_reported_and_backs_off() {
    let mut script = goals();
    script
        .run(
            r#"
            Goals.on = true
            for name in pairs(Goals.actions) do Goals.remove_action(name) end
            runs = 0
            Goals.action({ name = "open_gate", why = "Pull the lever", effects = { gate_open = true },
              run = function() runs = runs + 1 end,
              done = function() return true, false, "the lever is stuck" end })
            Goals.push({ name = "gate_open", priority = 60 })
            "#,
        )
        .unwrap();
    ticks(&mut script, 1.0);
    assert_eq!(
        text(&script, "return heard('INTENT_DONE') .. ' / ' .. heard('ACTION_FAILED')"),
        "INTENT_DONE goal:open_gate false the lever is stuck / ACTION_FAILED gate_open open_gate the lever is stuck"
    );
    // Not again at once: the goal is blocked meanwhile, and said to be.
    ticks(&mut script, 15.0);
    assert_eq!(script.eval::<i64>("return runs").unwrap(), 1);
    assert!(text(&script, "return heard('GOAL_BLOCKED')").contains("GOAL_BLOCKED gate_open"));
    // After `retry_after` (20 s) one more try, then twice as long.
    ticks(&mut script, 6.0);
    assert_eq!(script.eval::<i64>("return runs").unwrap(), 2);
    ticks(&mut script, 30.0);
    assert_eq!(script.eval::<i64>("return runs").unwrap(), 2);
    ticks(&mut script, 12.0);
    assert_eq!(script.eval::<i64>("return runs").unwrap(), 3);
}

#[test]
fn zones_fit_by_level_and_travel_heads_for_the_nearest_that_fits() {
    let script = goals();
    script
        .run(
            r#"
            lvl = 12
            AgentZones = function() return {
              { id = 12, name = "Elwynn Forest", map = 0, faction = "Alliance", city = false, min = 5, max = 10, x = 0, y = 0 },
              { id = 40, name = "Westfall", map = 0, faction = "Alliance", city = false, min = 9, max = 18, x = 0, y = 1500 },
              { id = 38, name = "Loch Modan", map = 0, faction = "Alliance", city = false, min = 10, max = 18, x = 3000, y = 0 },
              { id = 1519, name = "Stormwind City", map = 0, faction = "Alliance", city = true, min = 10, max = 10, x = 0, y = 800 },
              { id = 14, name = "Durotar", map = 1, faction = "Horde", city = false, min = 5, max = 10, x = 0, y = 0 },
              { id = 33, name = "Stranglethorn Vale", map = 0, faction = "contested", city = false, min = 31, max = 43, x = -9000, y = 0 },
            } end
            "#,
        )
        .unwrap();
    let zones = text(
        &script,
        "local o = {} for _, z in ipairs(Goals.zones()) do o[#o + 1] = z.name .. '=' .. z.fit end return table.concat(o, ' ')",
    );
    assert_eq!(
        zones,
        "Elwynn Forest=low Westfall=ok Loch Modan=ok Stranglethorn Vale=high"
    );
    assert_eq!(
        text(&script, "return Goals.facts().zone_fit"),
        "low",
        "nothing in sight: the zone's band says we have outgrown it"
    );
    assert_eq!(
        text(&script, "return Goals.travel_target().name"),
        "Westfall"
    );
    assert_eq!(
        text(&script, "return plan_of('progress')"),
        "travel_to_zone grind"
    );
    // The planner's own choice wins: level around Loch Modan first.
    assert_eq!(
        text(&script, "return Goals.travel_target('Loch Modan').name"),
        "Loch Modan"
    );
}

#[test]
fn hurt_it_rests_eating_the_food_in_its_bags() {
    let mut script = goals();
    script
        .run(
            "hp = 20 \
             GetContainerNumSlots = function(bag) return bag == 0 and 2 or 0 end \
             GetContainerItemLink = function(bag, slot) if bag == 0 and slot == 2 then return 'bread' end end \
             GetItemSpell = function(link) if link == 'bread' then return 'Food' end end \
             UseContainerItem = function(bag, slot) calls[#calls + 1] = 'use ' .. tostring(bag) .. ',' .. tostring(slot) end \
             Goals.on = true",
        )
        .unwrap();
    ticks(&mut script, 0.5);
    assert_eq!(text(&script, "return doing()"), "goal:rest");
    assert_eq!(text(&script, "return table.concat(calls, ' ')"), "use 0,2");
}
