//! Build 12340's `SecureCmdOptionParse(options)` (`0x564ae0` → `0x5f0df0`): the macro option
//! grammar `[cond,cond][cond] value; [cond] value; value`. Evidence:
//! `/tmp/azb-app-mig/ghidra/{shim2,opt1,opt2}.log` and the condition table (`0xad14d8` names,
//! `0xad1560` handlers).
//!
//! The tokenizer (`0x5f0df0`) scans a clause for `[` or `;`. Each `[...]` group, read to its `]`
//! (so a `;` inside one does not end the clause), is evaluated until one holds; later groups of
//! the clause are read past unevaluated. A clause with no group holds; `;` after groups of which
//! none held moves to the next clause. The answer is the text after the clause's last `]`,
//! trimmed of spaces (not other whitespace), and the holding group's unit (`@unit` or
//! `target=unit`), else nil. With no clause holding, nothing is returned.
//!
//! A group is compiled once per distinct text (`0x5f0bb0`, cached by `0x617c50` in `0xc2438c`):
//! conditions `[no]name[:arg/arg]`, the `no` prefix and the 32 names byte-exact (`0x5f0040`). An
//! unknown name raises `ERR_UNKNOWN_MACRO_OPTION_S` as its group compiles and keeps handler 0
//! (`0x8c8de0`, true), so it holds and its `no` form fails. The cache lives as long as the VM; the
//! reference empties it in the world-leave teardown (`0x566530` → `0x5f03a0`).
//!
//! Each handler asks the Lua reader of the same state, a missing reader reading false. A
//! unit-less group tests `target`.

use std::collections::HashMap;
use std::rc::Rc;

use mlua::{Function, IntoLuaMulti, Lua, MultiValue, Value};

use super::Model;

/// `0xad14d8`: entry 0, the empty name, is handler `0x8c8de0` (true), which an unknown name keeps.
const NAMES: [&str; 33] = [
    "",
    "combat",
    "exists",
    "help",
    "harm",
    "party",
    "raid",
    "dead",
    "flyable",
    "indoors",
    "outdoors",
    "swimming",
    "flying",
    "mounted",
    "stealth",
    "group",
    "stance",
    "form",
    "pet",
    "modifier",
    "mod",
    "button",
    "btn",
    "actionbar",
    "bar",
    "bonusbar",
    "equipped",
    "worn",
    "channeling",
    "spec",
    "vehicleui",
    "unithasvehicleui",
    "cursor",
];

/// One compiled `[no]name[:a/b]` condition.
struct Condition {
    negate: bool,
    name: &'static str,
    args: Vec<String>,
}

/// One compiled group: its conditions in order and its unit, empty for none.
#[derive(Default)]
struct Group {
    conditions: Vec<Condition>,
    unit: String,
}

/// The compiled groups by their exact text (`0xc2438c`).
#[derive(Default)]
struct Compiled(HashMap<String, Rc<Group>>);

/// `0x5f0040`: a `no` prefix on a token longer than two, the name up to `:` (spaces trimmed),
/// then, past a `:` with text after it, the `/`-split arguments, each trimmed and an empty one
/// kept. An unknown name raises its message (`0x5216f0(0x23e)`) and keeps handler 0 with no
/// arguments.
fn condition(lua: &Lua, token: &str) -> Condition {
    let (negate, rest) = match token.strip_prefix("no") {
        Some(rest) if token.len() > 2 => (true, rest),
        _ => (false, token),
    };
    let (name, args) = match rest.split_once(':') {
        Some((name, args)) if !args.is_empty() => (name, Some(args)),
        Some((name, _)) => (name, None),
        None => (rest, None),
    };
    let name = name.trim_matches(' ');
    let Some(&known) = NAMES.iter().find(|n| **n == name) else {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model
            .ui_errors_s
            .push(("ERR_UNKNOWN_MACRO_OPTION_S", name.to_string()));
        return Condition {
            negate,
            name: NAMES[0],
            args: Vec::new(),
        };
    };
    let args = args.map_or_else(Vec::new, |a| {
        a.split('/')
            .map(|s| s.trim_matches(' ').to_string())
            .collect()
    });
    Condition {
        negate,
        name: known,
        args,
    }
}

/// `0x5f0bb0`: the `,`-split tokens, each trimmed; `target=` or `@` names the unit (the last one
/// wins), any other is a condition. An empty token is a nameless condition, handler 0, so it is
/// skipped.
fn compile(lua: &Lua, body: &str) -> Group {
    let mut group = Group::default();
    for token in body.split(',').map(|t| t.trim_matches(' ')) {
        if token.is_empty() {
            continue;
        }
        match token
            .strip_prefix("target=")
            .or_else(|| token.strip_prefix('@'))
        {
            Some(unit) => group.unit = unit.trim_matches(' ').to_string(),
            None => group.conditions.push(condition(lua, token)),
        }
    }
    group
}

/// The group compiled from `body`, compiling it on a miss (`0x5f0ed2`).
fn compiled(lua: &Lua, body: &str) -> Rc<Group> {
    let hit = lua
        .app_data_ref::<Compiled>()
        .and_then(|c| c.0.get(body).cloned());
    hit.unwrap_or_else(|| {
        let group = Rc::new(compile(lua, body));
        if let Some(mut cache) = lua.app_data_mut::<Compiled>() {
            cache.0.insert(body.to_string(), group.clone());
        }
        group
    })
}

/// The first value of a native reader, `None` when the global is missing or raises.
fn ask(lua: &Lua, name: &str, args: impl IntoLuaMulti) -> Option<Value> {
    let f = lua.globals().get::<Function>(name).ok()?;
    f.call::<MultiValue>(args).ok()?.into_iter().next()
}

fn truthy(v: Option<Value>) -> bool {
    !matches!(v, None | Some(Value::Nil) | Some(Value::Boolean(false)))
}

fn number(lua: &Lua, v: Option<Value>) -> Option<f64> {
    lua.coerce_number(v?).ok().flatten()
}

fn text(lua: &Lua, v: Option<Value>) -> Option<String> {
    match v? {
        v @ (Value::String(_) | Value::Integer(_) | Value::Number(_)) => lua
            .coerce_string(v)
            .ok()
            .flatten()
            .map(|s| s.to_string_lossy()),
        _ => None,
    }
}

/// A numeric reader against the arguments: any argument equal, else `bare` of the value.
fn numeric(value: Option<f64>, args: &[String], bare: impl Fn(f64) -> bool) -> bool {
    let Some(v) = value else { return false };
    if args.is_empty() {
        return bare(v);
    }
    args.iter().any(|a| a.parse::<f64>() == Ok(v))
}

fn holds(lua: &Lua, c: &Condition, unit: &str) -> bool {
    let args = c.args.as_slice();
    let any_text =
        |v: Option<String>| v.is_some_and(|v| args.iter().any(|a| a.eq_ignore_ascii_case(&v)));
    match c.name {
        "combat" => truthy(ask(lua, "UnitAffectingCombat", "player")),
        "exists" => truthy(ask(lua, "UnitExists", unit)),
        "help" => truthy(ask(lua, "UnitCanAssist", ("player", unit))),
        "harm" => truthy(ask(lua, "UnitCanAttack", ("player", unit))),
        "party" => truthy(ask(lua, "UnitInParty", unit)),
        "raid" => truthy(ask(lua, "UnitInRaid", unit)),
        "dead" => truthy(ask(lua, "UnitIsDeadOrGhost", unit)),
        "flyable" => truthy(ask(lua, "IsFlyableArea", ())),
        "indoors" => truthy(ask(lua, "IsIndoors", ())),
        "outdoors" => truthy(ask(lua, "IsOutdoors", ())),
        "swimming" => truthy(ask(lua, "IsSwimming", ())),
        "flying" => truthy(ask(lua, "IsFlying", ())),
        "mounted" => truthy(ask(lua, "IsMounted", ())),
        "stealth" => truthy(ask(lua, "IsStealthed", ())),
        "group" => {
            let party = number(lua, ask(lua, "GetNumPartyMembers", ())).unwrap_or(0.0) > 0.0;
            let raid = number(lua, ask(lua, "GetNumRaidMembers", ())).unwrap_or(0.0) > 0.0;
            if args.is_empty() {
                party || raid
            } else {
                args.iter().any(|a| {
                    (a.eq_ignore_ascii_case("party") && party)
                        || (a.eq_ignore_ascii_case("raid") && raid)
                })
            }
        }
        "stance" | "form" => numeric(number(lua, ask(lua, "GetShapeshiftForm", ())), args, |f| {
            f != 0.0
        }),
        "pet" => {
            truthy(ask(lua, "UnitExists", "pet"))
                && (args.is_empty()
                    || any_text(text(lua, ask(lua, "UnitName", "pet")))
                    || any_text(text(lua, ask(lua, "UnitCreatureFamily", "pet"))))
        }
        "modifier" | "mod" => {
            let (shift, ctrl, alt) = lua.app_data_ref::<Model>().expect("model").modifiers;
            if args.is_empty() {
                shift || ctrl || alt
            } else {
                args.iter().any(|a| match a.to_ascii_lowercase().as_str() {
                    "shift" => shift,
                    "ctrl" => ctrl,
                    "alt" => alt,
                    _ => truthy(ask(lua, "IsModifiedClick", a.as_str())),
                })
            }
        }
        "button" | "btn" => {
            let pressed = lua
                .app_data_ref::<Model>()
                .expect("model")
                .last_mouse_button
                .clone();
            let n = match pressed.as_deref() {
                Some("RightButton") => 2,
                Some("MiddleButton") => 3,
                Some("Button4") => 4,
                Some("Button5") => 5,
                _ => 1,
            };
            args.is_empty() || args.iter().any(|a| a.parse::<i32>() == Ok(n))
        }
        "actionbar" | "bar" => numeric(number(lua, ask(lua, "GetActionBarPage", ())), args, |_| {
            true
        }),
        "bonusbar" => numeric(number(lua, ask(lua, "GetBonusBarOffset", ())), args, |b| {
            b > 0.0
        }),
        "equipped" | "worn" => args
            .iter()
            .any(|a| truthy(ask(lua, "IsEquippedItemType", a.as_str()))),
        "channeling" => {
            let spell = text(lua, ask(lua, "UnitChannelInfo", "player"));
            spell.is_some() && (args.is_empty() || any_text(spell))
        }
        "spec" => numeric(
            number(lua, ask(lua, "GetActiveTalentGroup", ())),
            args,
            |_| true,
        ),
        "vehicleui" => truthy(ask(lua, "UnitHasVehicleUI", "player")),
        "unithasvehicleui" => truthy(ask(lua, "UnitHasVehicleUI", unit)),
        "cursor" => {
            let kind = text(lua, ask(lua, "GetCursorInfo", ()));
            kind.is_some() && (args.is_empty() || any_text(kind))
        }
        _ => true,
    }
}

/// `0x5ef5c0`: every condition in order, inverted under `no`, to the first false.
fn evaluate(lua: &Lua, group: &Group) -> bool {
    let unit = match group.unit.as_str() {
        "" => "target",
        unit => unit,
    };
    group
        .conditions
        .iter()
        .all(|c| holds(lua, c, unit) != c.negate)
}

/// The answer and the holding group's unit, or `None` when no clause holds.
pub(crate) fn parse<'a>(lua: &Lua, options: &'a str) -> Option<(&'a str, Option<String>)> {
    let bytes = options.as_bytes();
    let end = bytes.len();
    let (mut at, mut grouped, mut held) = (0, false, None::<Rc<Group>>);
    loop {
        let text = at;
        let stop = (at..end)
            .find(|&i| matches!(bytes[i], b'[' | b';'))
            .unwrap_or(end);
        if stop < end && bytes[stop] == b'[' {
            let close = (stop..end).find(|&i| bytes[i] == b']').unwrap_or(end);
            let body = options[stop + 1..close].trim_matches(' ');
            if held.is_none() {
                held = Some(compiled(lua, body)).filter(|g| evaluate(lua, g));
            }
            (at, grouped) = ((close + 1).min(end), true);
            continue;
        }
        if !grouped || held.is_some() {
            let unit = held.map(|g| g.unit.clone()).filter(|u| !u.is_empty());
            return Some((options[text..stop].trim_matches(' '), unit));
        }
        if stop == end {
            return None;
        }
        (at, grouped) = (stop + 1, false);
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(Compiled::default());
    lua.globals().set(
        "SecureCmdOptionParse",
        lua.create_function(|lua, options: Value| {
            // `lua_tolstring`: a number is parsed as its text.
            let options = super::binding_abi::string_arg(
                lua,
                options,
                "Usage: SecureCmdOptionParse(\"options\")",
            )?;
            let s = |t: &str| lua.create_string(t).map(Value::String);
            Ok(MultiValue::from_vec(match parse(lua, &options) {
                Some((value, Some(unit))) => vec![s(value)?, s(&unit)?],
                Some((value, None)) => vec![s(value)?, Value::Nil],
                None => Vec::new(),
            }))
        })?,
    )
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    fn vm() -> UiScript {
        let s = UiScript::new().unwrap();
        s.run(
            "UnitExists = function(u) return u == 'focus' or u == 'pet' end \
             UnitCanAttack = function(_, u) return u == 'focus' end \
             GetShapeshiftForm = function() return 2 end \
             UnitName = function() return 'Wolf' end",
        )
        .unwrap();
        s
    }

    fn eval(s: &UiScript, options: &str) -> String {
        s.eval::<String>(&format!(
            "local v, t = SecureCmdOptionParse({options:?}) \
             return tostring(v) .. '|' .. tostring(t)"
        ))
        .unwrap()
    }

    #[test]
    fn the_first_holding_clause_gives_its_value_and_unit() {
        let mut s = vm();
        assert_eq!(
            eval(&s, "[@focus,harm] Frostbolt; Fireball"),
            "Frostbolt|focus"
        );
        assert_eq!(eval(&s, "[harm] Frostbolt; Fireball"), "Fireball|nil");
        assert_eq!(eval(&s, "[@target,exists][@focus,exists] a ; b"), "a|focus");
        assert_eq!(eval(&s, "[form:1/2,pet:wolf] Bite"), "Bite|nil");
        assert_eq!(eval(&s, "[noform] Bite; Claw"), "Claw|nil");
        assert_eq!(eval(&s, "[mod:shift] x"), "nil|nil", "no clause holds");
        s.set_modifiers(true, false, false);
        assert_eq!(eval(&s, "[mod:shift] x"), "x|nil");
        assert_eq!(eval(&s, "  plain  "), "plain|nil");
        assert!(s.run("SecureCmdOptionParse()").is_err());
        // `0x5f0df0` scans the whole clause for `[`, answers the text after its last `]`, and
        // reads a group to its `]` before looking for the clause's `;`.
        assert_eq!(eval(&s, "Frostbolt [form]"), "|nil");
        assert_eq!(eval(&s, "[form] a [noform] b"), "b|nil");
        assert_eq!(eval(&s, "[@focus;x] a; b"), "a|focus;x");
        // `lua_tolstring`: a number is its text.
        assert_eq!(
            s.eval::<String>("return SecureCmdOptionParse(5)").unwrap(),
            "5"
        );
    }

    /// `0x5f0040`: the names and the `no` prefix are byte-exact; an unknown name holds (and its
    /// `no` form fails) and is reported as its group text compiles, once per text.
    #[test]
    fn names_are_exact_and_an_unknown_one_holds_reported_once() {
        let mut s = vm();
        assert_eq!(eval(&s, "[NoForm] a; b"), "a|nil");
        assert_eq!(
            eval(&s, "[no form] a; b"),
            "b|nil",
            "`no`, then the name trimmed"
        );
        assert_eq!(eval(&s, "[nofoo] a; b"), "b|nil");
        let reported = vec![
            ("ERR_UNKNOWN_MACRO_OPTION_S", "NoForm".to_string()),
            ("ERR_UNKNOWN_MACRO_OPTION_S", "foo".to_string()),
        ];
        assert_eq!(s.take_ui_errors_s(), reported);
        assert_eq!(eval(&s, "[NoForm] a; b"), "a|nil");
        assert!(
            s.take_ui_errors_s().is_empty(),
            "a compiled group is cached"
        );
    }
}
