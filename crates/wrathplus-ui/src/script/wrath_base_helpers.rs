//! Build 12340's base-library helpers `scrub` and `difftime`.

use mlua::{Lua, MultiValue, Value};

/// `luaL_checknumber` as the base library's `0x84fab0` does it: a number or numeric string, else
/// the standard argument error.
fn check_number(lua: &Lua, args: &MultiValue, i: usize) -> mlua::Result<f64> {
    let v = args.get(i).cloned().unwrap_or(Value::Nil);
    let got = if i < args.len() {
        v.type_name()
    } else {
        "no value"
    };
    lua.coerce_number(v)?.ok_or_else(|| {
        mlua::Error::runtime(format!(
            "bad argument #{} to 'difftime' (number expected, got {got})",
            i + 1
        ))
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // `scrub(...)` (`0x817bf0`): every argument back, in place, with each one that is not a
    // boolean, number or string (`lua_type` 1, 3, 4) replaced by nil; as many results as
    // arguments.
    lua.globals().set(
        "scrub",
        lua.create_function(|_, args: MultiValue| {
            Ok(args
                .into_iter()
                .map(|v| match v {
                    Value::Boolean(_) | Value::Integer(_) | Value::Number(_) | Value::String(_) => {
                        v
                    }
                    _ => Value::Nil,
                })
                .collect::<MultiValue>())
        })?,
    )?;

    // `difftime(t2[, t1])` (`0x8178a0`): `t2 - t1` in seconds as a number, `t1` defaulting to 0,
    // each truncated to a 32-bit time through `__ftol2_sse` (`0x8178b8`, `0x8178c7`), so one
    // outside an int is `INT_MIN`; the CRT's `__difftime32` answers 0 when either is negative.
    lua.globals().set(
        "difftime",
        lua.create_function(|lua, args: MultiValue| {
            let t1 = match args.get(1) {
                None | Some(Value::Nil) => 0.0,
                Some(_) => check_number(lua, &args, 1)?,
            };
            let t2 = check_number(lua, &args, 0)?;
            let (t1, t2) = (super::binding_abi::ftol(t1), super::binding_abi::ftol(t2));
            Ok(if t1 < 0 || t2 < 0 {
                0.0
            } else {
                f64::from(t2) - f64::from(t1)
            })
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    #[test]
    fn scrub_keeps_booleans_numbers_and_strings_only() {
        let s = UiScript::new().unwrap();
        s.run(
            "local a, b, c, d, e, f, g = scrub(1, 'x', true, {}, print, nil, false) \
             assert(a == 1 and b == 'x' and c == true and d == nil and e == nil and f == nil \
               and g == false) \
             assert(select('#', scrub(1, {}, nil)) == 3 and select('#', scrub()) == 0) \
             assert(select('#', scrub(coroutine.create(function() end))) == 1)",
        )
        .unwrap();
    }

    #[test]
    fn difftime_subtracts_truncated_32_bit_times() {
        let s = UiScript::new().unwrap();
        s.run(
            "assert(difftime(100, 40) == 60 and difftime(40, 100) == -60) \
             assert(difftime(100) == 100 and difftime(100, nil) == 100) \
             assert(difftime(10.9, 0.5) == 10 and difftime('7', '2') == 5) \
             assert(difftime(-5, 1) == 0 and difftime(5, -1) == 0 and difftime(2^32 + 5) == 0) \
             assert(type(difftime(1, 1)) == 'number')",
        )
        .unwrap();
        for (chunk, want) in [
            (
                "difftime()",
                "bad argument #1 to 'difftime' (number expected, got no value)",
            ),
            (
                "difftime({})",
                "bad argument #1 to 'difftime' (number expected, got table)",
            ),
            (
                "difftime(1, 'x')",
                "bad argument #2 to 'difftime' (number expected, got string)",
            ),
        ] {
            let e = s.run(chunk).unwrap_err().to_string();
            assert!(e.contains(want), "{chunk}: {e}");
        }
    }
}
