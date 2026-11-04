//! Build 12340's string globals that are C functions in its exe: `strtrim` (`0x816960`),
//! `strsplit` (`0x816a60`), `strjoin` (`0x816b60`), `strreplace` (`0x816c40`) and `strconcat`
//! (`0x816d80`). Their arguments go through `luaL_checklstring` / `luaL_optinteger`, so a number is
//! its text and anything else raises `bad argument #n to 'name' (string expected, got X)`; the
//! work is on C strings, which end at the first NUL.

use mlua::{Lua, MultiValue, Value};

/// `luaL_typename`: what a `bad argument` says it got.
fn got(args: &MultiValue, i: usize) -> &'static str {
    args.get(i).map_or("no value", typename)
}

/// `lua_typename` of a value: mlua's names, but Lua 5.1's for an integer and a light userdata.
pub(super) fn typename(v: &Value) -> &'static str {
    match v {
        Value::Integer(_) => "number",
        Value::LightUserData(_) => "userdata",
        v => v.type_name(),
    }
}

/// `luaL_argerror` (`0xa473b4`), positioned by the guards as `luaL_where(L, 1)` would.
fn bad_argument(name: &str, n: usize, expected: &str, got: &str) -> mlua::Error {
    mlua::Error::runtime(format!(
        "bad argument #{n} to '{name}' ({expected} expected, got {got})"
    ))
}

/// `luaL_checklstring` (`0x84f9f0`): a string's bytes, or a number's text.
fn check_bytes(lua: &Lua, args: &MultiValue, i: usize, name: &str) -> mlua::Result<Vec<u8>> {
    match args.get(i) {
        Some(v @ (Value::String(_) | Value::Integer(_) | Value::Number(_))) => {
            let s = lua.coerce_string(v.clone())?.expect("a string or a number");
            Ok(s.as_bytes().to_vec())
        }
        _ => Err(bad_argument(name, i + 1, "string", got(args, i))),
    }
}

/// `luaL_optinteger` (`0x84fbd0`) with default 0; `lua_tointeger` is a bare `fistp`, rounding to
/// nearest, ties to even.
fn opt_integer(lua: &Lua, args: &MultiValue, i: usize, name: &str) -> mlua::Result<i64> {
    match args.get(i) {
        None | Some(Value::Nil) => Ok(0),
        Some(v) => match lua.coerce_number(v.clone())? {
            Some(n) => Ok(n.round_ties_even() as i64),
            None => Err(bad_argument(name, i + 1, "number", got(args, i))),
        },
    }
}

/// The bytes before the first NUL, as C reads them.
fn c_str(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
}

/// `lua_concat` over `values`: none answer `""` and one answers itself untouched; from two up,
/// strings and numbers join and anything else raises `attempt to concatenate a X value` with no
/// position, as a C caller's `luaG_concaterror` does. `luaV_concat` folds from the right and
/// names the left operand of the failing pair when that one cannot join, so the last two both
/// bad name the left one, and otherwise the rightmost bad one is named. Labelled: a `__concat`
/// metamethod is not tried.
fn concat(lua: &Lua, values: &[Value]) -> mlua::Result<Value> {
    let joins = |v: &Value| matches!(v, Value::String(_) | Value::Integer(_) | Value::Number(_));
    match values {
        [] => return Ok(Value::String(lua.create_string("")?)),
        [one] => return Ok(one.clone()),
        [.., left, right] if !joins(left) && !joins(right) => {
            return Err(concat_error(left));
        }
        _ => {}
    }
    if let Some(bad) = values.iter().rev().find(|v| !joins(v)) {
        return Err(concat_error(bad));
    }
    let mut out = Vec::new();
    for v in values {
        out.extend_from_slice(&lua.coerce_string(v.clone())?.expect("checked").as_bytes());
    }
    Ok(Value::String(lua.create_string(out)?))
}

fn concat_error(bad: &Value) -> mlua::Error {
    mlua::Error::runtime(format!("attempt to concatenate a {} value", typename(bad)))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // `strtrim(s [, chars])`: `chars` is a byte set, `" \t\r\n"` by default (`0xa16860`). When
    // neither end is in the set, or either string is empty, `s` comes back as it is (`0x8169e5`).
    g.set(
        "strtrim",
        lua.create_function(|lua, args: MultiValue| {
            let s = check_bytes(lua, &args, 0, "strtrim")?;
            let chars = match args.get(1) {
                None | Some(Value::Nil) => b" \t\r\n".to_vec(),
                Some(_) => check_bytes(lua, &args, 1, "strtrim")?,
            };
            let set = c_str(&chars);
            let body = c_str(&s);
            let trims = |b: Option<&u8>| b.is_some_and(|b| set.contains(b));
            if !trims(body.first()) && !trims(body.last()) {
                return lua.create_string(&s);
            }
            let start = body
                .iter()
                .position(|b| !set.contains(b))
                .unwrap_or(body.len());
            let end = body
                .iter()
                .rposition(|b| !set.contains(b))
                .map_or(start, |e| e + 1);
            lua.create_string(&body[start..end.max(start)])
        })?,
    )?;
    // `strsplit(delims, s [, limit])`: `delims` is a byte set; every hit cuts, so runs and ends
    // give empty pieces; at `limit - 1` cuts the rest is the last piece. `limit` 0 is no limit,
    // and a `limit` of 1 or less, or an empty `s`, answers `s` whole (`0x816aa4`).
    g.set(
        "strsplit",
        lua.create_function(|lua, args: MultiValue| {
            let delims = check_bytes(lua, &args, 0, "strsplit")?;
            let s = check_bytes(lua, &args, 1, "strsplit")?;
            let limit = opt_integer(lua, &args, 2, "strsplit")?;
            let (set, s) = (c_str(&delims), c_str(&s));
            let mut out = MultiValue::new();
            let mut start = 0;
            if (limit == 0 || limit > 1) && !s.is_empty() {
                for (i, b) in s.iter().enumerate() {
                    if !set.contains(b) {
                        continue;
                    }
                    // `lua_checkstack` past `LUAI_MAXCSTACK` (`0x816b46`).
                    if out.len() >= 7999 {
                        return Err(mlua::Error::runtime("strsplit(): Stack overflow"));
                    }
                    out.push_back(Value::String(lua.create_string(&s[start..i])?));
                    start = i + 1;
                    if out.len() as i64 == limit - 1 {
                        break;
                    }
                }
            }
            out.push_back(Value::String(lua.create_string(&s[start..])?));
            Ok(out)
        })?,
    )?;
    // `strjoin(delim, ...)`: an empty `delim` is `lua_concat` of every argument, itself included
    // (`0x816b8e`); else no further arguments answer "" (`0x816ba5`), and otherwise the arguments
    // with `delim` between them, so a single one comes back untouched (`0x816c1d`).
    g.set(
        "strjoin",
        lua.create_function(|lua, args: MultiValue| {
            let delim = check_bytes(lua, &args, 0, "strjoin")?;
            if delim.is_empty() {
                return concat(lua, &Vec::from(args));
            }
            let delim = Value::String(lua.create_string(&delim)?);
            let mut parts = Vec::with_capacity(args.len() * 2);
            for (k, v) in args.iter().skip(1).enumerate() {
                if k > 0 {
                    parts.push(delim.clone());
                }
                parts.push(v.clone());
            }
            concat(lua, &parts)
        })?,
    )?;
    // `strconcat(...)`: `lua_concat` of every argument.
    g.set(
        "strconcat",
        lua.create_function(|lua, args: MultiValue| concat(lua, &Vec::from(args)))?,
    )?;
    // `strreplace(s, find, repl [, count])`: plain `strstr` replacement, at most `count` times when
    // `count` is not 0, into a 4096-byte buffer; answers the string and the count (`0x816d6b`).
    // With no hit `s` comes back whole. Labelled: an empty `find` never ends in 12340
    // (`strstr` hits at once and the cursor does not move); here it replaces nothing.
    let strreplace = lua.create_function(|lua, args: MultiValue| {
        let s = check_bytes(lua, &args, 0, "strreplace")?;
        let find = check_bytes(lua, &args, 1, "strreplace")?;
        let repl = check_bytes(lua, &args, 2, "strreplace")?;
        let count = opt_integer(lua, &args, 3, "strreplace")?;
        let (subject, find, repl) = (c_str(&s), c_str(&find), c_str(&repl));
        let (mut out, mut at, mut n) = (Vec::new(), 0, 0i64);
        while !find.is_empty() && (count == 0 || n < count) {
            let Some(hit) = subject[at..].windows(find.len()).position(|w| w == find) else {
                break;
            };
            out.extend_from_slice(&subject[at..at + hit]);
            out.extend_from_slice(repl);
            at += hit + find.len();
            n += 1;
        }
        if n == 0 {
            return Ok((lua.create_string(&s)?, 0));
        }
        out.extend_from_slice(&subject[at..]);
        out.truncate(0x1000);
        Ok((lua.create_string(out)?, n))
    })?;
    g.set("strreplace", strreplace.clone())?;
    g.get::<mlua::Table>("string")?.set("replace", strreplace)
}
