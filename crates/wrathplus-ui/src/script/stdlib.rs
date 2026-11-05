//! The sandbox and the build 12340 stdlib layer on top of Lua 5.1.
//!
//! - [`sandbox`]: no filesystem, OS or native reach (`io`, `os`, `package`, `require`, `dofile`,
//!   `loadfile`, `load`, `module` and `debug` removed), the reference's `debugstack` and
//!   `debuglocals`, and a text-only `loadstring`.
//! - [`install`]: the bare globals (string, table and math aliases, the `debug*` family, the
//!   error-handler pair), `bit`, `time` and `date`, the clock and zone getters
//!   ([`super::clock`], [`super::zone_text`]), and a `string.format` that accepts positional
//!   `%N$` specs as the client's does, rewriting them into a sequential format. Mixing positional
//!   and sequential specs raises, as in the reference, whose single argument cursor a positional
//!   spec desyncs.

use mlua::{Lua, Value, Variadic};

#[path = "stdlib_guards.rs"]
mod guards;
#[path = "stdlib_strings.rs"]
mod strings;
#[path = "stdlib_time.rs"]
mod time;

pub(super) use guards::install_wrath_budget_guards;

/// An mlua error's bare Lua message, as `loadstring` returns it: without the `syntax error: ` or
/// `runtime error: ` word mlua's `Display` adds and the reference never does.
pub(super) fn lua_message(e: &mlua::Error) -> String {
    match e {
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        mlua::Error::RuntimeError(m) => m.clone(),
        other => other.to_string(),
    }
}

/// Sandbox the VM: no filesystem, OS or native reach, the reference's `debugstack`, and text-only
/// chunk loading.
pub(super) fn sandbox(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // Set to nil explicitly, whatever mlua's safe stdlib already leaves out. Build 12340 keeps
    // `newproxy`, which its restricted-environment handles are made of.
    for name in [
        "io", "os", "package", "require", "dofile", "loadfile", "load", "module", "debug",
    ] {
        g.set(name, Value::Nil)?;
    }

    {
        g.set(
            "debuglocals",
            lua.create_function(|lua, level: Option<usize>| {
                // Inspect primitive locals only; never call tostring/metamethods on addon objects.
                let level = level.unwrap_or(1).min(MAX_DEBUGSTACK_LEVELS);
                let mut lines = Vec::new();
                unsafe {
                    lua.exec_raw::<()>((), |state| {
                        let mut ar = std::mem::zeroed();
                        if mlua::ffi::lua_getstack(state, (level + 1) as i32, &mut ar) != 0 {
                            for index in 1..=128 {
                                let name = mlua::ffi::lua_getlocal(state, &ar, index);
                                if name.is_null() {
                                    break;
                                }
                                let name = std::ffi::CStr::from_ptr(name).to_string_lossy();
                                let text = match mlua::ffi::lua_type(state, -1) {
                                    mlua::ffi::LUA_TNIL => "nil".to_owned(),
                                    mlua::ffi::LUA_TBOOLEAN => {
                                        (mlua::ffi::lua_toboolean(state, -1) != 0).to_string()
                                    }
                                    mlua::ffi::LUA_TNUMBER => {
                                        mlua::ffi::lua_tonumber(state, -1).to_string()
                                    }
                                    mlua::ffi::LUA_TSTRING => {
                                        let mut len = 0;
                                        let ptr = mlua::ffi::lua_tolstring(state, -1, &mut len);
                                        String::from_utf8_lossy(std::slice::from_raw_parts(
                                            ptr as *const u8,
                                            len.min(1024),
                                        ))
                                        .into_owned()
                                    }
                                    _ => "<object>".to_owned(),
                                };
                                if !name.starts_with('(') {
                                    lines.push(format!("{name} = {text}"));
                                }
                                mlua::ffi::lua_pop(state, 1);
                            }
                        }
                    })?;
                }
                Ok(lines.join("\n"))
            })?,
        )?;
    }

    // debugstack([start [, count1 [, count2]]]): the reference's traceback, frames only, in Lua
    // 5.0's wording. There is no `stack traceback:` header and line 1 is the caller's frame:
    // `AceDB-2.0.lua:742` skips one line to read its caller's addon folder, and `AceLibrary.lua:70`
    // reads line 1 as a frame. `AceLibrary.lua:139` needs 5.0's ``in function `name'`` quoting,
    // not later Luas' `'name'`. `start` defaults to 1, the caller; `count1`/`count2` are 5.0's
    // `LEVELS1`/`LEVELS2`, which `FuBarPlugin-2.0.lua:752` relies on with `debugstack(6, 1, 0)`.
    g.set(
        "debugstack",
        lua.create_function(|lua, args: Variadic<Value>| {
            let num = |v: Option<&Value>, default: usize| match v {
                Some(Value::Integer(i)) => usize::try_from(*i).unwrap_or(0),
                Some(Value::Number(n)) if *n >= 0.0 => *n as usize,
                _ => default,
            };
            // Stack inspection is diagnostic only. Clamp hostile counts both to bound work/output
            // and to keep all level arithmetic representable on 32- and 64-bit hosts.
            let start = num(args.first(), 1).min(MAX_DEBUGSTACK_LEVELS);
            let count1 = num(args.get(1), 12).min(MAX_DEBUGSTACK_LEVELS);
            let count2 = num(args.get(2), 10).min(MAX_DEBUGSTACK_LEVELS);
            Ok(traceback_frames(lua, start, count1, count2))
        })?,
    )?;

    // Text-only `loadstring`: a binary chunk is rejected. The BOM strip and `#`-line skip apply,
    // as in the reference they live inside `luaL_loadbuffer`/`luaX_setinput`.
    let loadstring = lua.create_function(
        |lua, (src, chunkname): (mlua::String, Option<mlua::String>)| {
            let raw = src.as_bytes();
            let bytes = crate::source::chunk(&raw).to_vec();
            // The chunk name is the caller's string verbatim, defaulting to the source itself
            // (`luaB_loadstring` `0x703280`, `0x70329a`), which `luaO_chunkid` `0x6f5c40` renders
            // as `[string "…"]`. It comes from the raw bytes: `luaL_loadbuffer` `0x6f5690` strips
            // a BOM only after `0x703296` took the name.
            let name = match &chunkname {
                Some(n) => String::from_utf8_lossy(&n.as_bytes()).into_owned(),
                None => String::from_utf8_lossy(&raw).into_owned(),
            };
            let chunk = lua
                .load(bytes)
                .set_name(name)
                .set_mode(mlua::ChunkMode::Text);
            match chunk.into_function() {
                Ok(f) => Ok((Value::Function(f), Value::Nil)),
                // The failure leg returns nil and Lua's own message, unprefixed (`0x7032c6`,
                // `0x7032d2`).
                Err(e) => Ok((
                    Value::Nil,
                    Value::String(lua.create_string(lua_message(&e))?),
                )),
            }
        },
    )?;
    g.set("loadstring", loadstring)?;

    Ok(())
}

/// Install the stdlib layer: the bare globals, `time`/`date` and the positional `format`.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    install_error_handler(lua)?;
    time::install_time(lua)?;
    super::clock::install(lua)?;
    super::zone_text::install(lua)?;
    install_bit(lua)?;
    strings::install(lua)?;
    lua.load(WRATH_STDLIB)
        .set_name("=[WrathPlus wow stdlib]")
        .set_mode(mlua::ChunkMode::Text)
        .exec()?;
    let positional = POSITIONAL_FORMAT.replace(
        "local function reformat(fmt, ...)",
        "local function reformat(fmt, ...) local arg = {n = select('#', ...), ...}",
    );
    lua.load(&positional)
        .set_name("=[WrathPlus Wrath positional format]")
        .exec()?;
    Ok(())
}

/// The build-12340 client's global aliases over Lua 5.1's libraries, plus its `bit` library.
const WRATH_STDLIB: &str = r##"
-- math
abs = math.abs
ceil, deg, exp, floor = math.ceil, math.deg, math.exp, math.floor
frexp, ldexp, log, log10 = math.frexp, math.ldexp, math.log, math.log10
max, min, mod, rad, random = math.max, math.min, math.fmod, math.rad, math.random
sqrt = math.sqrt
-- The trig globals take and return DEGREES, unlike `math.*`: stock 3.3.5 FrameXML slides its
-- bars with `sin(fraction*90+90)` (`MainMenuBar.lua:14`) and pulses with `sin(elapsed*360)`
-- (`VehicleMenuBar.lua:924`). Labelled: `tan` and the inverses follow sin/cos by convention; no
-- stock caller pins them.
function sin(d) return math.sin(math.rad(d)) end
function cos(d) return math.cos(math.rad(d)) end
function tan(d) return math.tan(math.rad(d)) end
function asin(x) return math.deg(math.asin(x)) end
function acos(x) return math.deg(math.acos(x)) end
function atan(x) return math.deg(math.atan(x)) end
function atan2(y, x) return math.deg(math.atan2(y, x)) end
math.randomseed = nil -- 12340 has no "randomseed" string at all
PI = math.pi
-- string
format, gmatch, gsub = string.format, string.gmatch, string.gsub
strbyte, strchar, strfind, strlen = string.byte, string.char, string.find, string.len
strlower, strmatch, strrep, strrev = string.lower, string.match, string.rep, string.reverse
strsub, strupper = string.sub, string.upper
function tostringall(...)
    local n = select("#", ...)
    if n == 0 then return end
    local out = { ... }
    for i = 1, n do out[i] = tostring(out[i]) end
    return unpack(out, 1, n)
end
function strlenutf8(s)
    local _, count = s:gsub("[^\128-\191]", "")
    return count
end
string.trim, string.split, string.join = strtrim, strsplit, strjoin
-- table
foreach, foreachi, getn, sort = table.foreach, table.foreachi, table.getn, table.sort
tinsert, tremove = table.insert, table.remove
function wipe(t)
    for k in pairs(t) do t[k] = nil end
    return t
end
table.wipe = wipe
-- globals
function getglobal(name) return _G[name] end
function setglobal(name, value) _G[name] = value end
function GetLocale() return "enUS" end
-- debug*: debuginfo, debugload, debugprint, debugdump, debugbreak and debugtimestamp all bind the
-- shared `return 0` stub `0x8e5250`, answering nothing (`/tmp/azb-app-mig/ghidra/debugfam*.log`);
-- 3.3.5's `_ERRORMESSAGE` calls debuginfo() unguarded.
do
    local function noop() end
    debuginfo, debugload, debugprint = noop, noop, noop
    debugdump, debugbreak, debugtimestamp = noop, noop, noop
end
"##;

/// LuaBitOp's `bit`, which build 12340 ships: 32-bit operations with signed results.
fn install_bit(lua: &Lua) -> mlua::Result<()> {
    fn arg(v: f64) -> u32 {
        (v.rem_euclid(4_294_967_296.0)) as u32
    }
    let bit = lua.create_table()?;
    let binary = |op: fn(u32, u32) -> u32| {
        lua.create_function(move |_, args: Variadic<f64>| {
            let mut it = args.iter().map(|&v| arg(v));
            let first = it.next().unwrap_or(0);
            Ok(it.fold(first, op) as i32)
        })
    };
    bit.set("band", binary(|a, b| a & b)?)?;
    bit.set("bor", binary(|a, b| a | b)?)?;
    bit.set("bxor", binary(|a, b| a ^ b)?)?;
    bit.set("bnot", lua.create_function(|_, a: f64| Ok(!arg(a) as i32))?)?;
    bit.set(
        "lshift",
        lua.create_function(|_, (a, n): (f64, f64)| Ok(arg(a).wrapping_shl(arg(n) & 31) as i32))?,
    )?;
    bit.set(
        "rshift",
        lua.create_function(|_, (a, n): (f64, f64)| Ok(arg(a).wrapping_shr(arg(n) & 31) as i32))?,
    )?;
    bit.set(
        "arshift",
        lua.create_function(|_, (a, n): (f64, f64)| Ok((arg(a) as i32).wrapping_shr(arg(n) & 31)))?,
    )?;
    bit.set(
        "mod",
        lua.create_function(|_, (a, b): (f64, f64)| {
            Ok((arg(a) as i32).wrapping_rem(arg(b) as i32))
        })?,
    )?;
    lua.globals().set("bit", bit)
}

/// The handler `seterrorhandler` holds, `None` until one is given.
pub(super) fn error_handler(lua: &Lua) -> Option<mlua::Function> {
    lua.named_registry_value::<Option<mlua::Function>>(super::REG_ERRORHANDLER)
        .ok()
        .flatten()
}

/// `seterrorhandler(f)` / `geterrorhandler()`, engine globals in 1.12 (`0x702900`, `0x702950`)
/// over one registry reference, `[0x8722cc]`. It starts at -1 (`0x7039e0`), so `geterrorhandler()`
/// answers nil until something is set; in the running client FrameXML sets `_ERRORMESSAGE` first
/// (`BasicControls.xml:16`), so every addon finds that. `seterrorhandler` takes a function alone
/// and raises its `Usage:` on anything else, nil included, so the slot can never be cleared.
fn install_error_handler(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set(
        "seterrorhandler",
        lua.create_function(|lua, f: Value| match f {
            Value::Function(f) => lua.set_named_registry_value(super::REG_ERRORHANDLER, f),
            _ => Err(mlua::Error::RuntimeError(
                "Usage: seterrorhandler(errfunc)".into(),
            )),
        })?,
    )?;
    g.set(
        "geterrorhandler",
        lua.create_function(|lua, ()| Ok(error_handler(lua)))?,
    )?;
    Ok(())
}

// Blizzard's positional-format extension, over Lua 5.1 varargs.
const POSITIONAL_FORMAT: &str = r#"
-- ── Blizzard's positional string.format (%N$) ──────────────────────────────────────────────────
do
    -- `_find`/`_sub` are captured for the same reason `_format` is: this wrapper runs on every
    -- `format` call in the UI and must not follow an addon's later replacement of `string.*`.
    -- They are CALLS and not method syntax so an addon replacing the string metatable's
    -- `__index` cannot reach them either.
    local _format, _find, _sub, _len = string.format, string.find, string.sub, string.len
    local CONV = "diouxXeEfgGqscp"  -- Lua 5.1 conversion letters

    local function reformat(fmt, ...)
        if type(fmt) ~= "string" then
            return _format(fmt, unpack(arg, 1, arg.n))
        end
        -- fast path: no "%<digit>" at all ⇒ definitely no positional spec.
        if not _find(fmt, "%%%d") then
            return _format(fmt, unpack(arg, 1, arg.n))
        end

        local args = arg
        local pieces = {}      -- rebuilt (sequential) format fragments
        local order = {}       -- for each real conversion: the source arg index, or false=sequential
        local seen_pos, seen_seq = false, false
        local npieces, norder = 0, 0
        local i, len = 1, _len(fmt)

        while i <= len do
            local c = _sub(fmt, i, i)
            if c ~= "%" then
                npieces = npieces + 1; pieces[npieces] = c
                i = i + 1
            elseif _sub(fmt, i + 1, i + 1) == "%" then
                npieces = npieces + 1; pieces[npieces] = "%%"
                i = i + 2
            else
                local j = i + 1
                local ds, de = _find(fmt, "^%d+%$", j)  -- optional N$
                if ds then
                    seen_pos = true
                    norder = norder + 1; order[norder] = tonumber(_sub(fmt, ds, de - 1))
                    j = de + 1
                else
                    seen_seq = true
                    norder = norder + 1; order[norder] = false
                end
                -- copy flags/width/precision + the conversion letter
                local ce = _find(fmt, "[" .. CONV .. "]", j)
                if not ce then
                    error("invalid conversion in format string", 2)
                end
                npieces = npieces + 1; pieces[npieces] = "%" .. _sub(fmt, j, ce)
                i = ce + 1
            end
        end

        if seen_pos and seen_seq then
            error("cannot mix positional and sequential arguments in format string", 2)
        end

        local out, seq = {}, 0
        for k = 1, norder do
            local idx = order[k]
            if idx == false then
                seq = seq + 1
                out[k] = args[seq]
            else
                out[k] = args[idx]
            end
        end
        return _format(table.concat(pieces, "", 1, npieces), unpack(out, 1, norder))
    end

    string.format = reformat
    format = reformat
end
"#;

/// `luaO_chunkid` (`0x6f5c40`), for `debugstack`'s `short_src`. An `@` name longer than
/// [`CHUNKID_KEEP`] keeps only its tail behind `"..."`, so a long addon path loses the `\AddOns\`
/// that corpus libraries search for.
fn chunk_id(source: &str) -> String {
    if let Some(rest) = source.strip_prefix('=') {
        // A `=name` chunk is taken verbatim, clipped to the buffer from the front.
        return rest.chars().take(LUA_IDSIZE - 1).collect();
    }
    if let Some(rest) = source.strip_prefix('@') {
        let n = rest.chars().count();
        if n <= CHUNKID_KEEP {
            return rest.to_string();
        }
        let tail: String = rest.chars().skip(n - CHUNKID_KEEP).collect();
        return format!("...{tail}");
    }
    // A string chunk: `[string "<first line, clipped>"]`.
    let first = source.split('\n').next().unwrap_or_default();
    let budget = LUA_IDSIZE - STRING_CHUNK_OVERHEAD;
    if first.chars().count() > budget || first.len() < source.len() {
        let clipped: String = first.chars().take(budget).collect();
        format!("[string \"{clipped}...\"]")
    } else {
        format!("[string \"{first}\"]")
    }
}

/// `LUA_IDSIZE`: the `short_src` buffer, 60 bytes with the terminator.
const LUA_IDSIZE: usize = 60;

/// Characters an `@` name keeps, tail-first, behind `"..."` (`0x6f5c40`).
const CHUNKID_KEEP: usize = 52;

/// What `[string "…"]` costs the budget in the third `luaO_chunkid` arm.
const STRING_CHUNK_OVERHEAD: usize = 17;

/// One `debugstack` frame, `short_src ":" [currentline ":"] DESC`, the caller pushing its `\n`.
/// DESC (`0x7038fa`) is `` in function `%s' `` for a named frame (`0x872c98`), else by `what`:
/// `" in main chunk"` (`0x872c88`), `" ?"` for C or a tail call (`0x872c6c`), or
/// `" in function <%s:%d>"` (`0x872c70`).
fn traceback_frame(d: &mlua::Debug) -> String {
    let src = d.source();
    let names = d.names();
    let short = src
        .source
        .as_deref()
        .map(chunk_id)
        .or_else(|| src.short_src.as_deref().map(str::to_owned))
        .unwrap_or_else(|| "?".to_string());
    let mut line = String::with_capacity(short.len() + 32);
    line.push_str(&short);
    line.push(':');
    if let Some(n) = d.current_line() {
        line.push_str(&n.to_string());
        line.push(':');
    }
    // A tail call has nothing to name it from and takes the `" ?"` arm, known by its `what` or as
    // the `(tail call)` pseudo-frame.
    let tail_call = src.what == "t" || short == "(tail call)";
    // Named only for a global, local, method or field (`0x817b1f`, `namewhat`'s first byte); an
    // upvalue call takes the `<file:line>` arm.
    let named = matches!(
        names.name_what.and_then(|w| w.bytes().next()),
        Some(b'f' | b'g' | b'l' | b'm')
    );
    match names.name.as_deref() {
        Some(name) if !name.is_empty() && named && !tail_call => {
            line.push_str(" in function `");
            line.push_str(name);
            line.push('\'');
        }
        _ if src.what == "main" => line.push_str(" in main chunk"),
        _ if src.what == "C" || tail_call => line.push_str(" ?"),
        _ => {
            line.push_str(" in function <");
            line.push_str(&short);
            line.push(':');
            line.push_str(&src.line_defined.unwrap_or(0).to_string());
            line.push('>');
        }
    }
    line
}

/// `debugstack`'s walk (`0x703760`): Lua 5.0's `db_errorfb` without its header, each frame
/// followed by `"\n"` (`0x703971`) rather than preceded by `"\n\t"`, so line 1 is a frame.
/// Frames print while the level is `<= start + count1` (`0x703857`, `jbe`); then a failed
/// `getstack(level + count2)` probe prints that level as an ordinary frame, else `"...\n"` and
/// the last `count2` follow. So `count1 = 1` gives two frames on a two-deep stack, not one.
fn traceback_frames(lua: &Lua, start: usize, count1: usize, count2: usize) -> String {
    let mut out = String::new();
    let mut level = start;
    let mut first_part = true;
    while lua.inspect_stack(level, |_| ()).is_some() {
        level = level.saturating_add(1);
        if level > start.saturating_add(count1) && first_part {
            first_part = false;
            if lua
                .inspect_stack(level.saturating_add(count2), |_| ())
                .is_none()
            {
                level = level.saturating_sub(1); // nothing to elide: print this level after all
            } else {
                out.push_str("...\n");
                while lua
                    .inspect_stack(level.saturating_add(count2), |_| ())
                    .is_some()
                {
                    level = level.saturating_add(1);
                }
            }
            continue;
        }
        if let Some(f) = lua.inspect_stack(level.saturating_sub(1), traceback_frame) {
            out.push_str(&f);
            out.push('\n');
        }
    }
    out
}

/// More than this is neither useful in an addon traceback nor safe to let hostile numeric input
/// turn into unbounded diagnostic work.
const MAX_DEBUGSTACK_LEVELS: usize = 256;
