//! Tests for `env`: modules, the registry the fast tier reads, and the built-in library.

use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wp-agent-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn modules_load_once_reload_on_request_and_stay_in_their_folder() {
    let dir = scratch("require");
    std::fs::create_dir_all(dir.join("grind")).unwrap();
    std::fs::write(
        dir.join("grind/loop.lua"),
        "count = (count or 0) + 1 return { n = count }",
    )
    .unwrap();
    let script = UiScript::new().unwrap();
    assert!(!install(&script, dir.clone()).unwrap());
    let n: i64 = script.eval("return require('grind.loop').n").unwrap();
    assert_eq!(n, 1);
    let n: i64 = script.eval("return require('grind.loop').n").unwrap();
    assert_eq!(n, 1, "cached");
    let n: i64 = script.eval("return AgentReload('grind.loop').n").unwrap();
    assert_eq!(n, 2, "reloaded");
    let names: Vec<String> = script.eval("return AgentModules()").unwrap();
    assert_eq!(names, ["grind.loop"]);
    for bad in ["../x", "a..b", "a/b", ""] {
        assert!(script.run(&format!("require({bad:?})")).is_err(), "{bad}");
    }
    assert!(script.run("require('missing')").is_err());
    std::fs::write(dir.join("autoexec.lua"), "").unwrap();
    assert!(install(&script, dir.clone()).unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_shipped_example_module_loads() {
    let dir = scratch("example");
    std::fs::create_dir_all(dir.join("demo")).unwrap();
    std::fs::write(
        dir.join("demo/wander.lua"),
        include_str!("../lua/examples/wander.lua"),
    )
    .unwrap();
    let script = UiScript::new().unwrap();
    install(&script, dir).unwrap();
    let active: Vec<String> = script
        .eval("return require('demo.wander'):describe().active")
        .unwrap();
    assert_eq!(active, ["wander"]);
}

#[test]
fn modules_register_intents_and_decisions_for_the_fast_tier() {
    let script = UiScript::new().unwrap();
    install(&script, scratch("registry")).unwrap();
    let (names, ran, asked, resolved, after): (Vec<String>, bool, String, bool, usize) = script
        .eval(
            r#"
            local hits = 0
            Agent.intent("say_hi", { when = "Greet", run = function() hits = hits + 1 return hits end })
            Agent.intent("hidden", { when = "Never now", available = function() return false end, run = function() end })
            local names = {}
            for _, i in ipairs(Agent.available()) do names[#names + 1] = i.name end
            local ok = Agent.run("say_hi")
            local picked
            local id = Agent.decide("fight_or_flee", { question = "Fight?", options = { fight = "Stay", flee = "Run" }, on = function(c) picked = c end })
            local asked = Agent.questions()[1].question
            Agent.resolve(id, "flee")
            return names, ok and hits == 1, asked, picked == "flee", #Agent.questions()
            "#,
        )
        .unwrap();
    assert_eq!(names, ["say_hi"]);
    assert!(ran && resolved);
    assert_eq!((asked.as_str(), after), ("Fight?", 0));
    assert!(script
        .run("Agent.intent('Bad Name', { when = 'x', run = function() end })")
        .is_err());
}

#[test]
fn an_activity_runs_until_its_done_check_passes_or_it_times_out() {
    let mut script = UiScript::new().unwrap();
    install(&script, scratch("activity")).unwrap();
    script
        .run(
            r#"
            finished = false
            Agent.intent("walk_there", { when = "x", run = function() end, done = function() return finished end })
            Agent.intent("forever", { when = "y", run = function() end, done = function() return false end, timeout = 1 })
            Agent.run("walk_there")
            "#,
        )
        .unwrap();
    let busy: String = script.eval("return Agent.activity().name").unwrap();
    assert_eq!(busy, "walk_there");
    script.tick(0.3);
    assert!(script
        .eval::<bool>("return Agent.activity() ~= nil")
        .unwrap());
    script.run("finished = true").unwrap();
    script.tick(0.3);
    assert!(script
        .eval::<bool>("return Agent.activity() == nil")
        .unwrap());
    script.run("Agent.run('forever')").unwrap();
    for _ in 0..6 {
        script.tick(0.3);
    }
    assert!(
        script
            .eval::<bool>("return Agent.activity() == nil")
            .unwrap(),
        "timed out"
    );
    // Ends are announced for activities and failures, not for an instant intent that ran.
    let said: String = script
        .eval(
            r#"
            said = {}
            AgentSignal = function(name, intent, ok) said[#said + 1] = intent .. "=" .. tostring(ok) end
            Agent.intent("instant", { when = "z", run = function() end })
            Agent.intent("broken", { when = "z", run = function() error("no") end })
            Agent.run("instant")
            Agent.run("broken")
            Agent.run("walk_there")
            return table.concat(said, " ")
            "#,
        )
        .unwrap();
    assert_eq!(said, "broken=false");
    script.tick(0.3);
    let said: String = script.eval("return table.concat(said, ' ')").unwrap();
    assert_eq!(said, "broken=false walk_there=true");
}

#[test]
fn steps_run_one_after_another_as_one_activity() {
    let mut script = UiScript::new().unwrap();
    install(&script, scratch("steps")).unwrap();
    let ends: String = script
        .eval(
            r#"
            started, arrived, ends = {}, {}, {}
            AgentSignal = function(_, name, ok, why) ends[#ends + 1] = name .. "=" .. tostring(ok) .. ":" .. tostring(why) end
            local function leg(n) return {
              name = "leg " .. n,
              run = function() started[#started + 1] = n end,
              done = function() return arrived[n] == true end,
              timeout = 1,
            } end
            Agent.set_steps({ leg(1), leg(2) })
            Agent.run("follow_steps")
            return table.concat(started, ",") .. " / " .. Agent.available()[1].when
            "#,
        )
        .unwrap();
    assert_eq!(
        ends,
        "1 / Carry on with the planner's steps: step 1 of 2, leg 1."
    );
    // Leg 1 arrives: leg 2 starts within the same activity, nobody woken.
    script.run("arrived[1] = true").unwrap();
    script.tick(0.3);
    let (started, ends): (String, String) = script
        .eval("return table.concat(started, ','), table.concat(ends, ' ')")
        .unwrap();
    assert_eq!((started.as_str(), ends.as_str()), ("1,2", ""));
    script.run("arrived[2] = true").unwrap();
    script.tick(0.3);
    let ends: String = script.eval("return table.concat(ends, ' ')").unwrap();
    assert_eq!(ends, "follow_steps=true:all steps done");
    assert!(script
        .eval::<bool>("return Agent.intents.follow_steps == nil")
        .unwrap());
    // A step that never finishes times out, and the end names it.
    script
        .run("Agent.set_steps({ { name = 'stuck leg', run = function() end, done = function() return false end, timeout = 0.5 } }) Agent.run('follow_steps')")
        .unwrap();
    for _ in 0..4 {
        script.tick(0.3);
    }
    let ends: String = script.eval("return ends[#ends]").unwrap();
    assert_eq!(ends, "follow_steps=false:step stuck leg timed out");
    // Not offered again until the planner sets new steps: re-running it would just fail again.
    let offered = |script: &UiScript| -> bool {
        script
            .eval("for _, i in ipairs(Agent.available()) do if i.name == 'follow_steps' then return true end end return false")
            .unwrap()
    };
    assert!(!offered(&script));
    script
        .run("Agent.set_steps({ { name = 'another way', run = function() end } })")
        .unwrap();
    assert!(offered(&script));
}

#[test]
fn the_built_in_library_loads() {
    let script = UiScript::new().unwrap();
    install(&script, scratch("builtin")).unwrap();
    let ok: bool = script
        .eval("local E = require('wp.engine') local V = require('wp.values') return E.HIGH == 20 and type(V.health) == 'function'")
        .unwrap();
    assert!(ok);
}

#[test]
fn the_engine_runs_prerequisites_first_and_falls_back_to_alternatives() {
    let script = UiScript::new().unwrap();
    install(&script, scratch("engine")).unwrap();
    let ran: Vec<String> = script.eval(ENGINE_SCENARIO).unwrap();
    assert_eq!(ran, ["stance", "charge", "shoot", "idle-ok", "heal"]);
}

/// One tick per line of the expected run: a prerequisite before its action, an alternative after a
/// failure, defaults when nothing fires, and a higher relevance winning.
const ENGINE_SCENARIO: &str = r#"
local E = require("wp.engine")
local bot = E.new({ state = function() return "combat" end })
local ran = {}
local function act(name, ok) return { execute = function() ran[#ran + 1] = name return ok end } end
local fired = { pull = true }
bot:trigger("pull", { check = function() local f = fired.pull fired.pull = nil return f end })
bot:trigger("ranged", { check = function() local f = fired.ranged fired.ranged = nil return f end })
bot:trigger("hurt", { check = function() return fired.hurt end })
local stance = act("stance", true)
stance.useful = function() return not bot.memory.in_stance end
stance.execute = function() ran[#ran + 1] = "stance" bot.memory.in_stance = true return true end
bot:action("stance", stance)
local charge = act("charge", true)
charge.prerequisites = { "stance" }
bot:action("charge", charge)
local throw = act("throw", false)
throw.alternatives = { "shoot" }
bot:action("throw", throw)
bot:action("shoot", act("shoot", true))
bot:action("idle", { execute = function() ran[#ran + 1] = "idle-ok" return true end })
bot:action("heal", act("heal", true))
bot:strategy("fight", { state = "combat",
  triggers = { { "pull", { { "charge", E.NORMAL } } }, { "ranged", { { "throw", E.NORMAL } } },
           { "hurt", { { "heal", E.EMERGENCY } } } },
  defaults = { { "idle", E.IDLE + 1 } } })
bot:strategy("unused", { state = "combat", defaults = { { "heal", E.PASSTHROUGH } } })
bot:enable("+fight,+unused")
bot:enable("-unused")
bot:tick()            -- stance (charge's prerequisite)
bot:tick()            -- charge
fired.ranged = true
bot:tick()            -- throw fails, shoot is its alternative
bot.queue = {}
bot:tick()            -- nothing fired: the default
fired.hurt = true
bot:tick()            -- emergency beats the default
local out = {}
for _, n in ipairs(ran) do if n ~= "throw" then out[#out + 1] = n end end
return out
"#;
