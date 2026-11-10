//! The agent's Lua environment (#46): `require` for the agent's own modules and the built-in
//! library, kept across relogs and restarts as files in the agent's scripts folder.
//!
//! The UI VM is sandboxed (no `require`, `io`, `os`), as addons expect. This adds back exactly one
//! door: `require(name)` reads `<scripts>/<name with dots as slashes>.lua`, or a built-in module,
//! and nothing else. Built-ins preloaded as globals (each described where it loads): `Agent`,
//! `Revive`, `Upkeep`, `Survive`, `Rotation`, `Combat`, `Move`, `Quests`, `Avoid`, `Goals`,
//! `Services`; `wp.engine` and `wp.values` load on `require`. `AgentReload(name)` loads a module
//! again; `AgentModules()` lists the agent's files; an `autoexec.lua` runs in every new in-game VM.

use mlua::{Lua, Value};
use std::path::{Path, PathBuf};
use wrathplus_ui::script::UiScript;

/// Modules every agent has.
const BUILT_IN: &[(&str, &str)] = &[
    ("wp.engine", include_str!("../lua/wp/engine.lua")),
    ("wp.values", include_str!("../lua/wp/values.lua")),
    ("wp.agent", include_str!("../lua/wp/agent.lua")),
    ("wp.revive", include_str!("../lua/wp/revive.lua")),
    ("wp.upkeep", include_str!("../lua/wp/upkeep.lua")),
    ("wp.survive", include_str!("../lua/wp/survive.lua")),
    ("wp.rotation", include_str!("../lua/wp/rotation.lua")),
    ("wp.combat", include_str!("../lua/wp/combat.lua")),
    ("wp.move", include_str!("../lua/wp/move.lua")),
    ("wp.quest", include_str!("../lua/wp/quest.lua")),
    ("wp.avoid", include_str!("../lua/wp/avoid.lua")),
    ("wp.goals", include_str!("../lua/wp/goals.lua")),
    ("wp.services", include_str!("../lua/wp/services.lua")),
];

/// Where the agent's modules live: `WOW_AGENT_SCRIPTS`, else `<client home>/Agent/scripts`.
pub(crate) fn scripts_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WOW_AGENT_SCRIPTS") {
        return dir.into();
    }
    std::env::var_os("WRATHPLUS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Agent")
        .join("scripts")
}

/// A module name: dot-separated words of letters, digits and underscores.
pub(crate) fn valid_module(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

/// The file a module lives in.
pub(crate) fn module_path(dir: &Path, name: &str) -> PathBuf {
    let mut path = dir.to_path_buf();
    for part in name.split('.') {
        path.push(part);
    }
    path.set_extension("lua");
    path
}

/// Every agent module on disk, as names.
pub(crate) fn list(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if path.extension().is_some_and(|e| e == "lua") {
                if let Ok(rel) = path.with_extension("").strip_prefix(base) {
                    let name = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(".");
                    if valid_module(&name) {
                        out.push(name);
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Install `require`, `AgentReload` and `AgentModules` into this VM. Returns whether an
/// `autoexec` module exists (the caller runs it once the VM is in game).
pub(crate) fn install(script: &UiScript, dir: PathBuf) -> mlua::Result<bool> {
    let lua = script.lua();
    let loaded = lua.create_table()?;
    lua.globals().set("__wp_loaded", loaded)?;
    let require_dir = dir.clone();
    let require_fn =
        lua.create_function(move |lua, name: String| load_module(lua, &require_dir, &name))?;
    lua.globals().set("require", require_fn)?;
    let reload_dir = dir.clone();
    let reload = lua.create_function(move |lua, name: String| {
        let loaded: mlua::Table = lua.globals().get("__wp_loaded")?;
        loaded.set(name.as_str(), Value::Nil)?;
        load_module(lua, &reload_dir, &name)
    })?;
    lua.globals().set("AgentReload", reload)?;
    let list_dir = dir.clone();
    let modules = lua.create_function(move |_, ()| Ok(list(&list_dir)))?;
    lua.globals().set("AgentModules", modules)?;
    // The intent and decision registry the fast tier reads, as the global `Agent`.
    let agent = load_module(lua, &dir, "wp.agent")?;
    lua.globals().set("Agent", agent)?;
    // Death handling every agent has: release, corpse run, reclaim (`Revive.on = false` to opt out).
    let revive = load_module(lua, &dir, "wp.revive")?;
    lua.globals().set("Revive", revive)?;
    // Chores: wear upgrades, sell junk and repair at merchants, train at class trainers.
    let upkeep = load_module(lua, &dir, "wp.upkeep")?;
    lua.globals().set("Upkeep", upkeep)?;
    // Staying alive: which pulls are safe, potions and emergency buttons, when to run (`Survive`).
    let survive = load_module(lua, &dir, "wp.survive")?;
    lua.globals().set("Survive", survive)?;
    // What to cast in a fight, as a priority list (`Rotation.list`), and fighting a target kept in
    // range (`Combat.engage(guid)`), which runs it.
    let rotation = load_module(lua, &dir, "wp.rotation")?;
    lua.globals().set("Rotation", rotation)?;
    let combat = load_module(lua, &dir, "wp.combat")?;
    lua.globals().set("Combat", combat)?;
    // Going to a unit, following one (with its fights), holding a spot.
    let movement = load_module(lua, &dir, "wp.move")?;
    lua.globals().set("Move", movement)?;
    // Quest givers' dialogs, and keeping in step with the party's quests.
    let quests = load_module(lua, &dir, "wp.quest")?;
    lua.globals().set("Quests", quests)?;
    // Walks kept clear of hostile mobs in sight, unless the fast tier says to fight through.
    let avoid = load_module(lua, &dir, "wp.avoid")?;
    lua.globals().set("Avoid", avoid)?;
    // Goals, the plan toward the top one, and the runner that keeps the agent busy (`Goals.on`).
    let goals = load_module(lua, &dir, "wp.goals")?;
    lua.globals().set("Goals", goals)?;
    // Where services are, from the realm, handed to the goal runner as known places.
    let services = load_module(lua, &dir, "wp.services")?;
    lua.globals().set("Services", services)?;
    Ok(module_path(&dir, "autoexec").is_file())
}

fn load_module(lua: &Lua, dir: &Path, name: &str) -> mlua::Result<Value> {
    if !valid_module(name) {
        return Err(mlua::Error::runtime(format!(
            "require: bad module name {name:?}"
        )));
    }
    let loaded: mlua::Table = lua.globals().get("__wp_loaded")?;
    let cached: Value = loaded.get(name)?;
    if !cached.is_nil() {
        return Ok(cached);
    }
    let source = match BUILT_IN.iter().find(|(n, _)| *n == name) {
        Some((_, source)) => source.to_string(),
        None => std::fs::read_to_string(module_path(dir, name))
            .map_err(|e| mlua::Error::runtime(format!("require: module {name} not found ({e})")))?,
    };
    let value: Value = lua
        .load(source)
        .set_name(format!("@agent/{name}.lua"))
        .call(name)?;
    // A module that returns nothing is loaded all the same, as in stock Lua.
    let value = if value.is_nil() {
        Value::Boolean(true)
    } else {
        value
    };
    loaded.set(name, value.clone())?;
    Ok(value)
}

#[cfg(test)]
#[path = "avoid_tests.rs"]
mod avoid_tests;
#[cfg(test)]
#[path = "combat_tests.rs"]
mod combat_tests;
#[cfg(test)]
#[path = "goals_runner_tests.rs"]
mod goals_runner_tests;
#[cfg(test)]
#[path = "goals_safety_tests.rs"]
mod goals_safety_tests;
#[cfg(test)]
#[path = "goals_tests.rs"]
mod goals_tests;
#[cfg(test)]
#[path = "hand_in_tests.rs"]
mod hand_in_tests;
#[cfg(test)]
#[path = "hot_loop_tests.rs"]
mod hot_loop_tests;
#[cfg(test)]
#[path = "move_tests.rs"]
mod move_tests;
#[cfg(test)]
#[path = "quest_skip_tests.rs"]
mod quest_skip_tests;
#[cfg(test)]
#[path = "quest_tests.rs"]
mod quest_tests;
#[cfg(test)]
#[path = "revive_tests.rs"]
mod revive_tests;
#[cfg(test)]
#[path = "rotation_tests.rs"]
mod rotation_tests;
#[cfg(test)]
#[path = "services_tests.rs"]
mod services_tests;
#[cfg(test)]
#[path = "survive_tests.rs"]
mod survive_tests;
#[cfg(test)]
#[path = "upkeep_tests.rs"]
mod upkeep_tests;
#[cfg(test)]
#[path = "zone_guard_tests.rs"]
mod zone_guard_tests;

#[cfg(test)]
mod tests {
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
}
