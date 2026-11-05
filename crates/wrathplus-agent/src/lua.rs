//! Running an agent's Lua in the client's UI VM: a REPL-style call that returns the chunk's values
//! and what it printed, as JSON. Errors are values, never panics.

use mlua::{Lua, MultiValue, Value as LuaValue};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};
use wrathplus_ui::script::UiScript;

/// Tables nest at most this deep in a reply; deeper ones read `"<table>"`.
const MAX_DEPTH: usize = 8;
/// Entries per table in a reply; a longer table ends with a `"…"` marker.
const MAX_ENTRIES: usize = 2_000;

/// Run `code` and report `{ok, values, output}` or `{ok: false, error, output}`. A bare expression
/// (`UnitName("player")`) is returned as a REPL would; anything else runs as a chunk, whose
/// `return` values are reported. `budget` bounds the run in VM instruction units.
pub(crate) fn run(script: &UiScript, code: &str, budget: u64) -> Value {
    let lua = script.lua();
    let output = Arc::new(Mutex::new(Vec::<String>::new()));
    let globals = lua.globals();
    let saved_print: LuaValue = globals.get("print").unwrap_or(LuaValue::Nil);
    let sink = output.clone();
    let capture = lua.create_function(move |lua, args: MultiValue| {
        let tostring: mlua::Function = lua.globals().get("tostring")?;
        let mut parts = Vec::with_capacity(args.len());
        for arg in args {
            parts.push(tostring.call::<String>(arg).unwrap_or_else(|_| "?".into()));
        }
        sink.lock().expect("output lock").push(parts.join("\t"));
        Ok(())
    });
    if let Ok(capture) = capture {
        let _ = globals.set("print", capture);
    }
    script.set_instruction_budget(budget);
    let result = load(lua, code).and_then(|f| call_packed(lua, f));
    script.clear_instruction_budget();
    let _ = globals.set("print", saved_print);
    let output = std::mem::take(&mut *output.lock().expect("output lock"));
    match result {
        Ok(values) => json!({
            "ok": true,
            "values": values.iter().map(|v| to_json(v, 0)).collect::<Vec<_>>(),
            "output": output,
        }),
        Err(e) => json!({"ok": false, "error": e.to_string(), "output": output}),
    }
}

/// Call `f` and collect its results through a Lua-side pack, one table back to Rust. mlua 0.11's
/// `call::<MultiValue>` copies each string or table result with `lua_pushvalue` and no stack check
/// after a `LUA_MULTRET` call, which leaves no free slot: some 20+ such results abort a debug
/// build (`lapi.c` api check) and write past the frame in a release one.
fn call_packed(lua: &Lua, f: mlua::Function) -> mlua::Result<Vec<LuaValue>> {
    let pack: mlua::Function = lua
        .load("local f = ... local function pack(...) return { n = select('#', ...), ... } end return pack(f())")
        .set_name("=agent")
        .into_function()?;
    let packed: mlua::Table = pack.call(f)?;
    let n: usize = packed.raw_get("n")?;
    (1..=n).map(|i| packed.raw_get(i)).collect()
}

/// `return <code>` when that compiles (an expression), else the code as a chunk.
fn load(lua: &Lua, code: &str) -> mlua::Result<mlua::Function> {
    lua.load(format!("return {code}"))
        .set_name("=agent")
        .into_function()
        .or_else(|_| lua.load(code).set_name("=agent").into_function())
}

/// A Lua value as JSON: arrays for sequences `1..n`, objects otherwise (keys as strings);
/// functions, userdata and threads as a `"<kind>"` marker.
pub(crate) fn to_json(value: &LuaValue, depth: usize) -> Value {
    match value {
        LuaValue::Nil => Value::Null,
        LuaValue::Boolean(b) => json!(b),
        LuaValue::Integer(i) => json!(i),
        LuaValue::Number(n) if n.is_finite() => json!(n),
        LuaValue::Number(n) => json!(n.to_string()),
        LuaValue::String(s) => json!(s.to_string_lossy()),
        LuaValue::Table(t) if depth >= MAX_DEPTH => {
            let _ = t;
            json!("<table>")
        }
        LuaValue::Table(t) => {
            let len = t.raw_len();
            let pairs: Vec<(LuaValue, LuaValue)> = t
                .clone()
                .pairs::<LuaValue, LuaValue>()
                .filter_map(Result::ok)
                .take(MAX_ENTRIES + 1)
                .collect();
            if len > 0 && pairs.len() == len {
                let mut items: Vec<Value> = (1..=len.min(MAX_ENTRIES))
                    .map(|i| to_json(&t.raw_get(i).unwrap_or(LuaValue::Nil), depth + 1))
                    .collect();
                if len > MAX_ENTRIES {
                    items.push(json!("…"));
                }
                return Value::Array(items);
            }
            let mut map = Map::new();
            for (k, v) in pairs.iter().take(MAX_ENTRIES) {
                let key = match k {
                    LuaValue::String(s) => s.to_string_lossy().to_string(),
                    other => to_json(other, MAX_DEPTH).to_string(),
                };
                map.insert(key, to_json(v, depth + 1));
            }
            if pairs.len() > MAX_ENTRIES {
                map.insert("…".into(), json!("truncated"));
            }
            Value::Object(map)
        }
        LuaValue::Function(_) => json!("<function>"),
        LuaValue::UserData(_) | LuaValue::LightUserData(_) => json!("<userdata>"),
        LuaValue::Thread(_) => json!("<thread>"),
        _ => json!("<value>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: u64 = 200_000_000;

    #[test]
    fn expressions_return_their_values() {
        let script = UiScript::new().unwrap();
        let reply = run(&script, "1 + 2, 'x'", BUDGET);
        assert_eq!(reply, json!({"ok": true, "values": [3, "x"], "output": []}));
        // Many string results, trailing nils kept: mlua's multi-value read had no stack check.
        let reply = run(
            &script,
            "local t = {} for i = 1, 40 do t[i] = 's' .. i end return unpack(t, 1, 41)",
            BUDGET,
        );
        let values = reply["values"].as_array().unwrap();
        assert_eq!(
            (values.len(), &values[39], &values[40]),
            (41, &json!("s40"), &Value::Null)
        );
    }

    #[test]
    fn chunks_capture_print_and_restore_it() {
        let script = UiScript::new().unwrap();
        let reply = run(
            &script,
            "print('a', 1, nil) print(true) return {1, 2}",
            BUDGET,
        );
        assert_eq!(reply["values"], json!([[1, 2]]));
        assert_eq!(reply["output"], json!(["a\t1\tnil", "true"]));
        let print_is_ours: bool = script.eval("return print ~= nil").unwrap();
        assert!(print_is_ours);
        // The capture is gone: a later print does not land in an old reply.
        let reply = run(&script, "return 5", BUDGET);
        assert_eq!(reply["output"], json!([]));
    }

    #[test]
    fn errors_and_runaway_loops_come_back_as_values() {
        let script = UiScript::new().unwrap();
        let reply = run(&script, "error('boom')", BUDGET);
        assert_eq!(reply["ok"], false);
        assert!(reply["error"].as_str().unwrap().contains("boom"));
        let reply = run(&script, "while true do end", 1_000_000);
        assert_eq!(reply["ok"], false);
        // The VM is still usable afterwards.
        assert_eq!(run(&script, "return 7", BUDGET)["values"], json!([7]));
    }

    #[test]
    fn tables_become_arrays_or_objects() {
        let script = UiScript::new().unwrap();
        let reply = run(
            &script,
            "return {a = 1, b = {true, 'z'}}, {}, function() end",
            BUDGET,
        );
        assert_eq!(
            reply["values"],
            json!([{"a": 1, "b": [true, "z"]}, {}, "<function>"])
        );
    }
}
