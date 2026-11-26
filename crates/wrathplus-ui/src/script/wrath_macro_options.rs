//! Build 12340's `SecureCmdOptionParse(options)` (`0x564ae0` → `0x5f0df0`): the macro option
//! grammar `[cond,cond][cond] value; [cond] value; value`. Evidence:
//! `/tmp/azb-app-mig/ghidra/{shim2,opt1,opt2}.log` and the condition table (`0xad14d8` names,
//! `0xad1560` handlers).
//!
//! The tokenizer (`0x5f0df0`) scans a clause for `[` or `;`. Each `[...]` group, read to its `]`
//! (so a `;` inside one does not end the clause), is evaluated until one holds, all of its
//! comma-separated conditions true; later groups of the clause are read past unevaluated. A clause
//! with no group holds; `;` after groups of which none held moves to the next clause. The answer
//! is the text after the clause's last `]`, trimmed of spaces (not other whitespace), and the
//! holding group's unit (`@unit` or `target=unit`), else nil. With no clause holding, nothing is
//! returned.
//!
//! The 32 names (`0xad14d8`, five alias pairs sharing a handler): combat, exists, help, harm,
//! party, raid, dead, flyable, indoors, outdoors, swimming, flying, mounted, stealth, group,
//! stance/form, pet, modifier/mod, button/btn, actionbar/bar, bonusbar, equipped/worn,
//! channeling, spec, vehicleui, unithasvehicleui, cursor.
//!
//! Labelled: the condition compiler (`0x5f0040`) and the handlers are not decompiled. A condition
//! reads `[no]name[:arg/arg]`, the name matched case-insensitively. Each handler asks the native
//! Lua reader of the same state, and a missing reader reads false. A unit-less group tests
//! `target`, and an unknown name holds (handler 0 is not decompiled). `button` reads the last
//! mouse button pressed.

use mlua::{Function, IntoLuaMulti, Lua, MultiValue, Value};

use super::Model;

/// One `[no]name[:a/b]` condition.
struct Condition<'a> {
    negate: bool,
    name: String,
    args: Vec<&'a str>,
}

fn condition(text: &str) -> Condition<'_> {
    let (head, args) = match text.split_once(':') {
        Some((h, a)) => (
            h,
            a.split('/')
                .map(|s| s.trim_matches(' '))
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        None => (text, Vec::new()),
    };
    let head = head.trim_matches(' ').to_ascii_lowercase();
    let known = |n: &str| NAMES.contains(&n);
    match head.strip_prefix("no") {
        Some(rest) if !known(&head) && known(rest) => Condition {
            negate: true,
            name: rest.to_string(),
            args,
        },
        _ => Condition {
            negate: false,
            name: head,
            args,
        },
    }
}

const NAMES: [&str; 32] = [
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
fn numeric(value: Option<f64>, args: &[&str], bare: impl Fn(f64) -> bool) -> bool {
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
    match c.name.as_str() {
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
                    _ => truthy(ask(lua, "IsModifiedClick", *a)),
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
            .any(|a| truthy(ask(lua, "IsEquippedItemType", *a))),
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

/// One bracket group: whether it holds, and its unit.
fn group<'a>(lua: &Lua, body: &'a str) -> (bool, Option<&'a str>) {
    let mut unit = None;
    let mut conditions = Vec::new();
    for part in body.split(',') {
        let part = part.trim_matches(' ');
        if part.is_empty() {
            continue;
        }
        if let Some(u) = part
            .strip_prefix('@')
            .or_else(|| part.strip_prefix("target="))
        {
            unit = Some(u.trim_matches(' '));
        } else {
            conditions.push(condition(part));
        }
    }
    let tested = unit.filter(|u| !u.is_empty()).unwrap_or("target");
    let ok = conditions.iter().all(|c| holds(lua, c, tested) != c.negate);
    (ok, unit.filter(|u| !u.is_empty()))
}

/// The answer and the holding group's unit, or `None` when no clause holds (`0x5f0df0`).
pub(crate) fn parse<'a>(lua: &Lua, options: &'a str) -> Option<(&'a str, Option<&'a str>)> {
    let bytes = options.as_bytes();
    let end = bytes.len();
    let (mut at, mut grouped, mut held) = (0, false, None);
    loop {
        let text = at;
        let stop = (at..end)
            .find(|&i| matches!(bytes[i], b'[' | b';'))
            .unwrap_or(end);
        if stop < end && bytes[stop] == b'[' {
            let close = (stop..end).find(|&i| bytes[i] == b']').unwrap_or(end);
            if held.is_none() {
                let (ok, unit) = group(lua, options[stop + 1..close].trim_matches(' '));
                held = ok.then_some(unit);
            }
            (at, grouped) = ((close + 1).min(end), true);
            continue;
        }
        if !grouped || held.is_some() {
            return Some((options[text..stop].trim_matches(' '), held.flatten()));
        }
        if stop == end {
            return None;
        }
        (at, grouped) = (stop + 1, false);
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
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
                Some((value, Some(unit))) => vec![s(value)?, s(unit)?],
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
}
