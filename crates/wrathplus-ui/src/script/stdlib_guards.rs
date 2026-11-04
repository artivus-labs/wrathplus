//! The guarded protected calls: `pcall`, `xpcall` and `coroutine.resume` behind the sticky
//! instruction-budget check, handing Lua the error value build 12340 would.
//!
//! A Rust verb's `Err` reaches Lua as mlua's error userdata, where the reference raises a string
//! through `luaL_error` (`0x84f280`): `luaL_where(L, 1)` (`0x84f210`, `lua_getinfo "Sl"`) then the
//! message, so the text carries `src:line: ` when a Lua function called the verb and nothing when
//! C did (`pcall(Verb)`). The guards turn that userdata back into the reference's string wherever
//! Lua can catch it; the host's own error text is left as mlua writes it.

use mlua::{Lua, MultiValue, Value};

/// Put native protected operations behind a sticky host check. Lua 5.1 can catch a hook error, and
/// `coroutine.resume` reports one as a false result. Once the shared budget trips, these wrappers
/// raise a fresh Rust callback error after the protected operation returns, so nesting protected
/// calls only moves the same failure outward and cannot resume useful Lua execution.
pub(in crate::script) fn install_wrath_budget_guards(
    lua: &Lua,
    budget: std::sync::Arc<super::super::InstructionBudget>,
) -> mlua::Result<()> {
    let check = lua.create_function(move |lua, mut values: MultiValue| {
        use std::sync::atomic::Ordering::Relaxed;

        // The sentinel wins over any conversion.
        if budget.exhausted.load(Relaxed) {
            return Err(super::super::instruction_budget_error(
                budget.used.load(Relaxed),
            ));
        }
        if let (Some(Value::Boolean(false)), Some(Value::Error(e))) =
            (values.front(), values.get(1))
        {
            let text = Value::String(lua.create_string(stock_message(e))?);
            values[1] = text;
        }
        Ok(values)
    })?;
    // An `xpcall` handler runs at the raise, before `check` sees the value.
    let text = lua.create_function(|lua, value: Value| match value {
        Value::Error(e) => Ok(Value::String(lua.create_string(stock_message(&e))?)),
        other => Ok(other),
    })?;
    lua.load(WRATH_BUDGET_GUARDS)
        .set_name("=[WrathPlus wrath budget guards]")
        .set_mode(mlua::ChunkMode::Text)
        .call((check, text))
}

// The natives are held in locals named as the stock globals: `luaL_argerror` names a verb by its
// call site's variable (`getobjname`), so an argument error reads `'pcall'`, not a guard's own
// name. Each guard checks the one argument its native checks and raises at level 2, the caller,
// where `luaL_where(L, 1)` in the native would put it. The `xpcall` handler cache is weak in
// both halves: Lua 5.1 has no ephemerons, and each wrapper holds its own key.
const WRATH_BUDGET_GUARDS: &str = r##"
local check, text = ...
local G = _G
local pcall, xpcall, resume = pcall, xpcall, coroutine.resume
local select, type, error = select, type, error
local wrapped = setmetatable({}, { __mode = "kv" })

G.pcall = function(...)
    if ... == nil and select("#", ...) == 0 then
        error("bad argument #1 to 'pcall' (value expected)", 2)
    end
    return check(pcall(...))
end
G.xpcall = function(f, ...)
    local h = ...
    if type(h) == "function" then
        local w = wrapped[h]
        if not w then
            w = function(e) return h(text(e)) end
            wrapped[h] = w
        end
        return check(xpcall(f, w))
    end
    if h == nil and select("#", ...) == 0 then
        error("bad argument #2 to 'xpcall' (value expected)", 2)
    end
    return check(xpcall(f, ...))
end
G.coroutine.resume = function(co, ...)
    if type(co) ~= "thread" then
        error("bad argument #1 to 'resume' (coroutine expected)", 2)
    end
    return check(resume(co, ...))
end
"##;

/// The verbs whose raise is `lua_pushfstring` then `lua_error`, with no `luaL_where`, so no
/// position: `PlaySound` (`0x985930`), `PlayMusic` (`0x985966`), `PlaySoundFile` (`0x9859c6`).
const BARE_RAISES: [&str; 3] = [
    "Usage: PlaySound(",
    "Usage: PlayMusic(",
    "Usage: PlaySoundFile(",
];

/// An mlua error as the string the reference's raise would have left: the root cause's bare
/// message, positioned at the verb's caller as `luaL_where(L, 1)` would.
fn stock_message(e: &mlua::Error) -> String {
    // Nested callback errors: a verb's raise under a Lua call another verb made. The innermost
    // traceback is the raising verb's own.
    let (mut root, mut traceback) = (e, "");
    while let mlua::Error::CallbackError {
        cause,
        traceback: t,
    } = root
    {
        root = cause;
        traceback = t;
    }
    match root {
        // A Lua error that came back through a Rust verb already carries its position; mlua's
        // traceback tail is not part of the message.
        mlua::Error::RuntimeError(m) if m.contains("\nstack traceback:") => {
            m[..m.find("\nstack traceback:").unwrap_or(m.len())].to_owned()
        }
        mlua::Error::RuntimeError(m) if BARE_RAISES.iter().any(|p| m.starts_with(p)) => m.clone(),
        mlua::Error::RuntimeError(m) => format!("{}{m}", caller_position(traceback)),
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        other => format!("{}{other}", caller_position(traceback)),
    }
}

/// `luaL_where(L, 1)` read off mlua's traceback of the raising verb: line 0 is the header, line 1
/// the verb's own C frame, line 2 its caller, written `\tsrc:line: in …` when that frame has a
/// current line and `\t[C]: in …` (or a tail-call marker) when not. A source name may itself hold
/// `:digits`, so the last `:digits: in ` ends the position.
fn caller_position(traceback: &str) -> &str {
    let Some(frame) = traceback.lines().nth(2).and_then(|l| l.strip_prefix('\t')) else {
        return "";
    };
    frame
        .rmatch_indices(": in ")
        .map(|(at, _)| at)
        .find(|&at| {
            frame[..at].rsplit_once(':').is_some_and(|(src, line)| {
                !src.is_empty() && !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit())
            })
        })
        .map_or("", |at| &frame[..at + 2])
}
