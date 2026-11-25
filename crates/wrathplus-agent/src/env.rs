//! The agent's Lua environment (#46): `require` for the agent's own modules and the built-in
//! library, kept across relogs and restarts as files in the agent's scripts folder.
//!
//! The UI VM is sandboxed (no `require`, `io`, `os`), as addons expect. This adds back exactly one
//! door: `require(name)` reads `<scripts>/<name with dots as slashes>.lua`, or a built-in module,
//! and nothing else. Built-ins preloaded as globals (each described in `BUILT_IN`): `Agent`,
//! `Revive`, `Upkeep`, `Survive`, `Rotation`, `Combat`, `Move`, `Quests`, `Avoid`, `Goals`,
//! `Notice`, `Services`, `Taxi`, `Professions`; `wp.engine` and `wp.values` load on `require`.
//! `AgentReload(name)` loads a module again; `AgentModules()` lists the agent's files; an
//! `autoexec.lua` runs in every new in-game VM.

use mlua::{Lua, Value};
use std::path::{Path, PathBuf};
use wrathplus_ui::script::UiScript;

/// A built-in module `wp.<file>` from `lua/wp/<file>.lua`, and the global it is preloaded as.
macro_rules! wp {
    ($f:literal $(, $g:literal)?) => {
        (concat!("wp.", $f), wp!(@g $($g)?), include_str!(concat!("../lua/wp/", $f, ".lua")))
    };
    (@g) => { None };
    (@g $g:literal) => { Some($g) };
}

/// Modules every agent has, in load order, with the global each is preloaded as (a later one may
/// use an earlier one's).
const BUILT_IN: &[(&str, Option<&str>, &str)] = &[
    wp!("engine"),
    wp!("values"),
    // The intent and decision registry the fast tier reads.
    wp!("agent", "Agent"),
    // Death handling every agent has: release, corpse run, reclaim (`Revive.on = false` to opt out).
    wp!("revive", "Revive"),
    // Chores: wear upgrades, sell junk and repair at merchants, train at class trainers.
    wp!("upkeep", "Upkeep"),
    // Staying alive: which pulls are safe, potions and emergency buttons, when to run.
    wp!("survive", "Survive"),
    // What to cast in a fight, as a priority list (`Rotation.list`), and fighting a target kept in
    // range (`Combat.engage(guid)`), which runs it.
    wp!("rotation", "Rotation"),
    wp!("combat", "Combat"),
    // Going to a unit, following one (with its fights), holding a spot.
    wp!("move", "Move"),
    // Quest givers' dialogs, and keeping in step with the party's quests.
    wp!("quest", "Quests"),
    // Walks kept clear of hostile mobs in sight, unless the fast tier says to fight through.
    wp!("avoid", "Avoid"),
    // Goals, the plan toward the top one, and the runner that keeps the agent busy (`Goals.on`).
    wp!("goals", "Goals"),
    // What the character passes that is new to it (quest givers, trainers, nodes, subzones),
    // signalled as AGENT_NOTICED, with mutes for what the planner does not want to hear of.
    wp!("notice", "Notice"),
    // Where services are, from the realm, handed to the goal runner as known places.
    wp!("services", "Services"),
    // Flight paths: the ones known, learning new ones, and flying where it beats the walk.
    wp!("taxi", "Taxi"),
    // Cooking and First Aid learnt, made and used; it teaches `Goals` facts and actions.
    wp!("professions", "Professions"),
];

/// The built-in modules: name, global, source (for `search`).
pub(crate) fn built_ins() -> &'static [(&'static str, Option<&'static str>, &'static str)] {
    BUILT_IN
}

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
    for (name, global, _) in BUILT_IN {
        if let Some(global) = global {
            let module = load_module(lua, &dir, name)?;
            lua.globals().set(*global, module)?;
        }
    }
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
    let source = match BUILT_IN.iter().find(|(n, _, _)| *n == name) {
        Some((_, _, source)) => source.to_string(),
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
#[path = "ask_tests.rs"]
mod ask_tests;
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
#[path = "notice_tests.rs"]
mod notice_tests;
#[cfg(test)]
#[path = "professions_tests.rs"]
mod professions_tests;
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
#[path = "taxi_tests.rs"]
mod taxi_tests;

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
