//! The agent's Lua environment (#46): `require` for the agent's own modules and the built-in
//! library, kept across relogs and restarts as files in the agent's scripts folder.
//!
//! The UI VM is sandboxed (no `require`, `io`, `os`), as addons expect. This adds back exactly one
//! door: `require(name)` reads `<scripts>/<name with dots as slashes>.lua`, or a built-in module
//! (`wp.engine`, `wp.values`), and nothing else. `AgentReload(name)` forgets a module and loads it
//! again; `AgentModules()` lists the agent's files. If `autoexec.lua` exists, every new in-game
//! VM runs it, so an agent's logic comes back after `ReloadUI()` or a restart.

use mlua::{Lua, Value};
use std::path::{Path, PathBuf};
use wrathplus_ui::script::UiScript;

/// Modules every agent has.
const BUILT_IN: &[(&str, &str)] = &[
    ("wp.engine", include_str!("../lua/wp/engine.lua")),
    ("wp.values", include_str!("../lua/wp/values.lua")),
];

/// Where the agent's modules live: `WOW_AGENT_SCRIPTS`, else `<client home>/Agent/scripts`.
pub(crate) fn scripts_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WOW_AGENT_SCRIPTS") {
        return dir.into();
    }
    std::env::var_os("BENILLA_HOME")
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
