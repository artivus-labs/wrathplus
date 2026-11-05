//! Build 12340's item argument. Every item verb inlines one resolver (GetItemFamily `0x510f2b`,
//! GetItemCount `0x51c308`, GetItemIcon `0x517033`, PickupItem `0x522fb8`, …): `lua_isnumber`
//! (`0x84df20`) first, then `lua_isstring` (`0x84df60`) through `0x709de0`; any other type is id 0,
//! a miss and never an error. GetItemInfo (`0x516c60`) and the inventory finders' callers parse
//! their own argument.

use mlua::{Lua, Value};

use super::Model;

/// `lua_tointeger` (`0x84e070`): `fistp` under the default control word, round half to even into
/// an i32; past the i32 range (or NaN) the FPU stores its integer indefinite, `0x80000000`.
pub(super) fn tointeger(n: f64) -> u32 {
    let r = n.round_ties_even();
    if (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&r) {
        r as i32 as u32
    } else {
        0x8000_0000
    }
}

/// `0x709de0`: a string holding `item:` (a case-sensitive `strstr`, `0x76f700` over `strncmp`) is
/// the integer after it (`0x76f0d0`); any other string is a name in the item-name cache.
pub(super) fn item_string_id(model: &Model, s: &str) -> u32 {
    match s.find("item:") {
        Some(at) => super::item_stats::reference_atoi(&s[at + 5..]) as u32,
        None => name_id(model, s),
    }
}

/// The name cache (`0xca1168`, `0x53f3b0`) → the record's id (`+0x18`), else 0. Labelled: read as
/// the cached templates, without case as the cache folds its key (`0x76f640`); of two templates
/// sharing a name the lower id answers.
pub(super) fn name_id(model: &Model, name: &str) -> u32 {
    if name.is_empty() {
        return 0;
    }
    model
        .item_templates
        .iter()
        .filter(|(_, t)| t.name.eq_ignore_ascii_case(name))
        .map(|(&id, _)| id)
        .min()
        .unwrap_or(0)
}

/// The shared resolver: a number, or a string Lua converts to one, rounds (`0x84e070`); another
/// string goes through `0x709de0`; anything else is 0. An id is raw: `-1` is `0xffffffff`.
pub(super) fn item_id_arg(lua: &Lua, model: &Model, v: &Value) -> u32 {
    match v {
        Value::Integer(_) | Value::Number(_) | Value::String(_) => {
            if let Some(n) = lua.coerce_number(v.clone()).ok().flatten() {
                return tointeger(n);
            }
            match v {
                Value::String(s) => item_string_id(model, &s.to_string_lossy()),
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// GetItemInfo's argument shape, which GetItemUniqueness shares (`0x516ee9`): a number
/// truncates (`0x84e030` + `__ftol`, the low 32 bits); a string holding `item:` is its item
/// string's id (`0x50f630`: 0 unless > 0); another string a name; any other type `usage`.
pub(super) fn info_item_id(lua: &Lua, model: &Model, v: &Value, usage: &str) -> mlua::Result<u32> {
    if let Some(n) = lua.coerce_number(v.clone())? {
        return Ok(n.trunc() as i64 as u32);
    }
    let Value::String(s) = v else {
        return Err(mlua::Error::runtime(usage.to_string()));
    };
    let s = s.to_string_lossy();
    Ok(match s.find("item:") {
        Some(at) => u32::try_from(super::item_stats::reference_atoi(&s[at + 5..])).unwrap_or(0),
        None => name_id(model, &s),
    })
}

#[cfg(test)]
mod tests {
    use crate::script::{ItemTemplateView, UiScript};

    /// What id each argument shape resolves to, read back through two verbs that share the
    /// resolver.
    #[test]
    fn every_item_verb_resolves_its_argument_as_12340_does() {
        let mut s = UiScript::new().unwrap();
        // Ids 3 and 9 neither equip nor dress; 9 shares 7's name.
        for (id, name, inventory_type) in [
            (2, "Even", 13),
            (3, "Odd", 0),
            (7, "Blade", 13),
            (9, "Blade", 0),
        ] {
            s.set_item_template(
                id,
                ItemTemplateView {
                    name: name.into(),
                    inventory_type,
                    icon: (inventory_type != 0).then(|| "x".into()),
                    ..Default::default()
                },
            );
        }
        let resolves = |arg: &str, verb: &str| {
            s.eval::<bool>(&format!("return {verb}({arg}) == 1"))
                .unwrap()
        };
        for verb in ["IsDressableItem", "IsEquippableItem"] {
            // Round half to even (`fistp`): 2.5 and 1.5 are 2, 2.6 is 3.
            assert!(resolves("2.5", verb), "{verb}");
            assert!(resolves("1.5", verb), "{verb}");
            assert!(!resolves("2.6", verb), "{verb}");
            // `lua_isnumber` first: a numeric string is an id, never a name.
            assert!(resolves("'7'", verb), "{verb}");
            assert!(resolves("' 0x7 '", verb), "{verb}");
            // `item:` is case-sensitive and read by `atoi`, so a full hyperlink parses.
            assert!(
                resolves("'|cffffffff|Hitem:7:0:0:0|h[x]|h|r'", verb),
                "{verb}"
            );
            assert!(!resolves("'ITEM:7'", verb), "{verb}");
            // A name is a cached template's, without case; a shared name answers the lower id.
            assert!(resolves("'blade'", verb), "{verb}");
            // Any other type is id 0, a miss and never an error.
            assert!(!resolves("{}", verb), "{verb}");
            assert!(!resolves("nil", verb), "{verb}");
        }
        // `-1` stays `0xffffffff`; past the i32 range is the FPU's integer indefinite.
        assert_eq!(super::tointeger(-1.0), u32::MAX);
        assert_eq!(super::tointeger(1e10), 0x8000_0000);
    }
}
