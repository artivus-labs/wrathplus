//! Wrath animation objects. Groups schedule parallel animations by order; XML and Lua use
//! the same constructors. State is private engine-owned Lua tables, not compatibility globals.
use super::Model;
use mlua::{Function, Lua, MultiValue, ObjectLike, Table, Value};
#[path = "animation/lifecycle.rs"]
mod lifecycle;
pub(crate) mod transforms;
const GROUPS: &str = "__wrathplus_animation_groups";

fn fire(object: &Table, script: &str) -> mlua::Result<()> {
    let scripts: Table = object.raw_get("_scripts")?;
    if let Some(f) = scripts.raw_get::<Option<Function>>(script)? {
        f.call::<()>(object.clone())?;
    }
    Ok(())
}
fn list(object: &Table, key: &str) -> mlua::Result<MultiValue> {
    Ok(object
        .raw_get::<Table>(key)?
        .sequence_values::<Table>()
        .map(|v| v.map(Value::Table))
        .collect::<mlua::Result<Vec<_>>>()?
        .into())
}
/// An animation's script names (`0x497500`), and a group's, which adds `OnLoop` (`0x497800`),
/// both after `OnEvent` (`0x816830`), matched case-insensitively.
fn script_kind(t: &Table, name: &str) -> mlua::Result<Option<&'static str>> {
    const KINDS: [&str; 8] = [
        "OnEvent",
        "OnLoad",
        "OnPlay",
        "OnPause",
        "OnStop",
        "OnFinished",
        "OnUpdate",
        "OnLoop",
    ];
    let group = t.raw_get::<String>("_kind")? == "AnimationGroup";
    Ok(KINDS
        .iter()
        .take(if group { 8 } else { 7 })
        .copied()
        .find(|k| k.eq_ignore_ascii_case(name)))
}

/// [`script_kind`], else the shared raise: `<name> doesn't have a "<script>" script`.
fn required_script_kind(t: &Table, name: &str) -> mlua::Result<&'static str> {
    script_kind(t, name)?.ok_or_else(|| {
        let owner = t
            .raw_get::<Option<String>>("_name")
            .ok()
            .flatten()
            .unwrap_or_else(|| "<unnamed>".to_string());
        mlua::Error::RuntimeError(format!("{owner} doesn't have a \"{name}\" script"))
    })
}

fn common(
    lua: &Lua,
    object: &Table,
    parent: &Table,
    name: Option<String>,
    kind: &str,
) -> mlua::Result<()> {
    object.raw_set("_parent", parent.clone())?;
    object.raw_set("_name", name.clone())?;
    object.raw_set("_kind", kind)?;
    object.raw_set("_scripts", lua.create_table()?)?;
    object.set(
        "GetParent",
        lua.create_function(|_, t: Table| t.raw_get::<Table>("_parent"))?,
    )?;
    object.set(
        "GetName",
        lua.create_function(|_, t: Table| t.raw_get::<Option<String>>("_name"))?,
    )?;
    object.set(
        "GetObjectType",
        lua.create_function(|_, t: Table| t.raw_get::<String>("_kind"))?,
    )?;
    object.set(
        "IsObjectType",
        lua.create_function(|_, (t, k): (Table, String)| {
            Ok(k == t.raw_get::<String>("_kind")? || k == "Animation")
        })?,
    )?;
    object.set(
        "SetScript",
        lua.create_function(|_, (t, k, f): (Table, String, Option<Function>)| {
            let kind = required_script_kind(&t, &k)?;
            t.raw_get::<Table>("_scripts")?.raw_set(kind, f)
        })?,
    )?;
    object.set(
        "GetScript",
        lua.create_function(|_, (t, k): (Table, String)| {
            let kind = required_script_kind(&t, &k)?;
            t.raw_get::<Table>("_scripts")?
                .raw_get::<Option<Function>>(kind)
        })?,
    )?;
    object.set(
        "HasScript",
        lua.create_function(|_, (t, k): (Table, String)| {
            Ok(super::binding_abi::flag(script_kind(&t, &k)?.is_some()))
        })?,
    )?;
    if let Some(name) = name {
        lua.globals().set(name, object.clone())?;
    }
    Ok(())
}
fn scalar(lua: &Lua, t: &Table, key: &'static str, suffix: &str, initial: f64) -> mlua::Result<()> {
    t.raw_set(key, initial)?;
    t.set(
        format!("Set{suffix}"),
        lua.create_function(move |_, (t, v): (Table, f64)| {
            if !v.is_finite() {
                return Err(mlua::Error::runtime("animation value must be finite"));
            }
            t.raw_set(key, v)
        })?,
    )?;
    t.set(
        format!("Get{suffix}"),
        lua.create_function(move |_, t: Table| t.raw_get::<f64>(key))?,
    )?;
    Ok(())
}
fn group(lua: &Lua, parent: Table, name: Option<String>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    common(lua, &t, &parent, name, "AnimationGroup")?;
    t.raw_set("_animations", lua.create_table()?)?;
    t.raw_set("_elapsed", 0.0)?;
    t.raw_set("_playing", false)?;
    t.raw_set("_paused", false)?;
    t.raw_set("_looping", "NONE")?;
    t.set(
        "CreateAnimation",
        lua.create_function(
            |lua, (t, kind, name): (Table, Option<String>, Option<String>)| {
                animation(lua, t, kind.unwrap_or_else(|| "Animation".into()), name)
            },
        )?,
    )?;
    t.set(
        "GetAnimations",
        lua.create_function(|_, t: Table| list(&t, "_animations"))?,
    )?;
    t.set(
        "SetLooping",
        lua.create_function(|_, (t, v): (Table, String)| {
            if !matches!(v.as_str(), "NONE" | "REPEAT" | "BOUNCE") {
                return Err(mlua::Error::runtime("invalid looping type"));
            }
            t.raw_set("_looping", v)
        })?,
    )?;
    t.set(
        "GetLooping",
        lua.create_function(|_, t: Table| t.raw_get::<String>("_looping"))?,
    )?;
    t.set(
        "GetDuration",
        lua.create_function(|_, g: Table| {
            let mut slots = std::collections::BTreeMap::<u32, f64>::new();
            for a in g
                .raw_get::<Table>("_animations")?
                .sequence_values::<Table>()
            {
                let a = a?;
                let duration = a.raw_get::<f64>("_duration")?
                    + a.raw_get::<f64>("_start_delay")?
                    + a.raw_get::<f64>("_end_delay")?;
                slots
                    .entry(a.raw_get::<f64>("_order")? as u32)
                    .and_modify(|v| *v = v.max(duration))
                    .or_insert(duration);
            }
            Ok(slots.values().sum::<f64>())
        })?,
    )?;
    t.set(
        "IsDone",
        lua.create_function(|_, g: Table| {
            Ok(!g.raw_get::<bool>("_playing")? && !g.raw_get::<bool>("_paused")?)
        })?,
    )?;
    install_controls(lua, &t)?;
    let groups: Table = match lua.named_registry_value(GROUPS) {
        Ok(t) => t,
        Err(_) => {
            let t = lua.create_table()?;
            lua.set_named_registry_value(GROUPS, t.clone())?;
            t
        }
    };
    groups.raw_set(groups.raw_len() + 1, t.clone())?;
    let owned: Table = match parent.raw_get::<Option<Table>>("_animation_groups")? {
        Some(t) => t,
        None => {
            let l = lua.create_table()?;
            parent.raw_set("_animation_groups", l.clone())?;
            l
        }
    };
    owned.raw_set(owned.raw_len() + 1, t.clone())?;
    Ok(t)
}
fn install_controls(lua: &Lua, t: &Table) -> mlua::Result<()> {
    for verb in ["Play", "Pause", "Stop", "Finish"] {
        t.set(
            verb,
            lua.create_function(move |lua, t: Table| {
                let g = if t.raw_get::<String>("_kind")? == "AnimationGroup" {
                    t.clone()
                } else {
                    t.raw_get("_parent")?
                };
                match verb {
                    "Play" => {
                        if !g.raw_get::<bool>("_paused")? {
                            g.raw_set("_elapsed", 0.0)?;
                            g.raw_set("_fresh", true)?;
                        }
                        g.raw_set("_playing", true)?;
                        g.raw_set("_paused", false)?;
                    }
                    "Pause" => {
                        g.raw_set("_playing", false)?;
                        g.raw_set("_paused", true)?;
                    }
                    _ => {
                        g.raw_set("_playing", false)?;
                        g.raw_set("_paused", false)?;
                        restore(lua, &g)?;
                    }
                }
                // Child OnPlay is dispatched when its scheduled interval begins, not twice
                // (once for the control call and again for the first tick).
                if verb == "Play" && t.raw_get::<String>("_kind")? != "AnimationGroup" {
                    return Ok(());
                }
                fire(
                    &t,
                    match verb {
                        "Play" => "OnPlay",
                        "Pause" => "OnPause",
                        "Finish" => "OnFinished",
                        _ => "OnStop",
                    },
                )
            })?,
        )?;
    }
    for (verb, key) in [("IsPlaying", "_playing"), ("IsPaused", "_paused")] {
        t.set(
            verb,
            lua.create_function(move |_, t: Table| {
                let g = if t.raw_get::<String>("_kind")? == "AnimationGroup" {
                    t
                } else {
                    t.raw_get("_parent")?
                };
                g.raw_get::<bool>(key)
            })?,
        )?;
    }
    Ok(())
}
fn animation(lua: &Lua, parent: Table, kind: String, name: Option<String>) -> mlua::Result<Table> {
    if !matches!(
        kind.as_str(),
        "Animation" | "Alpha" | "Translation" | "Scale" | "Rotation"
    ) {
        return Err(mlua::Error::runtime("unknown animation type"));
    }
    let t = lua.create_table()?;
    common(lua, &t, &parent, name, &kind)?;
    for (key, suffix, initial) in [
        ("_duration", "Duration", 0.0),
        ("_start_delay", "StartDelay", 0.0),
        ("_end_delay", "EndDelay", 0.0),
        ("_order", "Order", 1.0),
        ("_change", "Change", 0.0),
        ("_progress", "Progress", 0.0),
    ] {
        scalar(lua, &t, key, suffix, initial)?;
    }
    t.raw_set("_smoothing", "NONE")?;
    t.raw_set("_smooth_progress", 0.0)?;
    t.set(
        "GetSmoothProgress",
        lua.create_function(|_, t: Table| t.raw_get::<f64>("_smooth_progress"))?,
    )?;
    t.set(
        "SetSmoothing",
        lua.create_function(|_, (t, v): (Table, String)| t.raw_set("_smoothing", v))?,
    )?;
    t.set(
        "GetSmoothing",
        lua.create_function(|_, t: Table| t.raw_get::<String>("_smoothing"))?,
    )?;
    if kind == "Translation" {
        t.raw_set("_x", 0.0)?;
        t.raw_set("_y", 0.0)?;
        t.set(
            "SetOffset",
            lua.create_function(|_, (t, x, y): (Table, f64, f64)| {
                t.raw_set("_x", x)?;
                t.raw_set("_y", y)
            })?,
        )?;
        t.set(
            "GetOffset",
            lua.create_function(|_, t: Table| {
                Ok((t.raw_get::<f64>("_x")?, t.raw_get::<f64>("_y")?))
            })?,
        )?;
    }
    // GetRegionParent() (`0x4a5a70`): the region that owns the animation's group.
    t.set(
        "GetRegionParent",
        lua.create_function(|_, t: Table| {
            t.raw_get::<Table>("_parent")?
                .raw_get::<Option<Table>>("_parent")
        })?,
    )?;
    transforms::install(lua, &t, &kind)?;
    install_controls(lua, &t)?;
    let children: Table = parent.raw_get("_animations")?;
    children.raw_set(children.raw_len() + 1, t.clone())?;
    Ok(t)
}
fn restore(lua: &Lua, g: &Table) -> mlua::Result<()> {
    transforms::clear(lua, g);
    if let Some(alpha) = g.raw_get::<Option<f64>>("_base_alpha")? {
        g.raw_get::<Table>("_parent")?
            .call_method::<()>("SetAlpha", alpha)?;
        g.raw_set("_base_alpha", Value::Nil)?;
    }
    if let Some(points) = g.raw_get::<Option<Table>>("_base_points")? {
        let owner: Table = g.raw_get("_parent")?;
        owner.call_method::<()>("ClearAllPoints", ())?;
        for point in points.sequence_values::<Table>() {
            let point = point?;
            owner.call_method::<()>(
                "SetPoint",
                point
                    .sequence_values::<Value>()
                    .collect::<mlua::Result<Vec<_>>>()?
                    .into_iter()
                    .collect::<MultiValue>(),
            )?;
        }
        g.raw_set("_base_points", Value::Nil)?;
    }
    Ok(())
}
pub(super) fn install(lua: &Lua, methods: &Table) -> mlua::Result<()> {
    methods.set(
        "CreateAnimationGroup",
        lua.create_function(|lua, (t, name): (Table, Option<String>)| group(lua, t, name))?,
    )?;
    // StopAnimating(): every animation group the region owns stops (labelled: native's binding is
    // unread; idle groups are left alone, so no stray OnStop fires).
    methods.set(
        "StopAnimating",
        lua.create_function(|_, t: Table| {
            if let Some(groups) = t.raw_get::<Option<Table>>("_animation_groups")? {
                for g in groups.sequence_values::<Table>() {
                    let g = g?;
                    if g.raw_get::<bool>("_playing")? || g.raw_get::<bool>("_paused")? {
                        g.call_method::<()>("Stop", ())?;
                    }
                }
            }
            Ok(())
        })?,
    )?;
    methods.set(
        "GetAnimationGroups",
        lua.create_function(|_, t: Table| {
            match t.raw_get::<Option<Table>>("_animation_groups")? {
                Some(_) => list(&t, "_animation_groups"),
                None => Ok(MultiValue::new()),
            }
        })?,
    )?;
    Ok(())
}
pub(super) fn tick(lua: &Lua, elapsed: f32) -> mlua::Result<()> {
    let Ok(groups) = lua.named_registry_value::<Table>(GROUPS) else {
        return Ok(());
    };
    for g in groups.sequence_values::<Table>() {
        let g = g?;
        if !g.raw_get::<bool>("_playing")? {
            continue;
        }
        let children: Table = g.raw_get("_animations")?;
        let mut animations = children
            .sequence_values::<Table>()
            .collect::<mlua::Result<Vec<_>>>()?;
        animations.sort_by(|a, b| {
            a.raw_get::<f64>("_order")
                .unwrap_or(1.0)
                .total_cmp(&b.raw_get::<f64>("_order").unwrap_or(1.0))
        });
        let mut slots = std::collections::BTreeMap::<u32, f64>::new();
        for a in &animations {
            let length = a.raw_get::<f64>("_duration")?
                + a.raw_get::<f64>("_start_delay")?
                + a.raw_get::<f64>("_end_delay")?;
            slots
                .entry(a.raw_get::<f64>("_order")? as u32)
                .and_modify(|v| *v = v.max(length))
                .or_insert(length);
        }
        let duration: f64 = slots.values().sum();
        let previous = g.raw_get::<f64>("_elapsed")?;
        let time = previous + f64::from(elapsed.max(0.0));
        g.raw_set("_elapsed", time)?;
        let looping = g.raw_get::<String>("_looping")?;
        let mut phase = time.min(duration);
        if duration > 0.0 && looping != "NONE" {
            phase = time % duration;
            if looping == "BOUNCE" && (time / duration) as u64 % 2 == 1 {
                phase = duration - phase;
            }
        }
        lifecycle::advance(&g, &animations, &slots, duration, previous, time, &looping)?;
        if !g.raw_get::<bool>("_playing")? {
            continue;
        }
        if let Some(f) = g
            .raw_get::<Table>("_scripts")?
            .raw_get::<Option<Function>>("OnUpdate")?
        {
            f.call::<()>((g.clone(), elapsed))?;
        }
        let owner: Table = g.raw_get("_parent")?;
        let base = match g.raw_get::<Option<f64>>("_base_alpha")? {
            Some(v) => v,
            None => {
                let v = owner.call_method::<f64>("GetAlpha", ())?;
                g.raw_set("_base_alpha", v)?;
                v
            }
        };
        let mut alpha = base;
        let mut offset = (0.0, 0.0);
        for a in &animations {
            let order = a.raw_get::<f64>("_order")? as u32;
            let start: f64 = slots.range(..order).map(|(_, v)| v).sum::<f64>()
                + a.raw_get::<f64>("_start_delay")?;
            let length = a.raw_get::<f64>("_duration")?;
            let p = if phase < start {
                0.0
            } else if length <= 0.0 {
                1.0
            } else {
                ((phase - start) / length).clamp(0.0, 1.0)
            };
            // GetProgress answers the raw fraction, GetSmoothProgress the eased one, which the
            // transforms apply.
            a.raw_set("_progress", p)?;
            let p = match a.raw_get::<String>("_smoothing")?.as_str() {
                "IN" => p * p,
                "OUT" => 1.0 - (1.0 - p) * (1.0 - p),
                "IN_OUT" => p * p * (3.0 - 2.0 * p),
                _ => p,
            };
            a.raw_set("_smooth_progress", p)?;
            if a.raw_get::<String>("_kind")? == "Translation" {
                offset.0 += a.raw_get::<f64>("_x")? * p;
                offset.1 += a.raw_get::<f64>("_y")? * p;
            }
            if a.raw_get::<String>("_kind")? == "Alpha" {
                alpha += a.raw_get::<f64>("_change")? * p;
            }
        }
        transforms::apply(lua, &g, &animations)?;
        owner.call_method::<()>("SetAlpha", alpha.clamp(0.0, 1.0))?;
        if animations.iter().any(|a| {
            a.raw_get::<String>("_kind")
                .is_ok_and(|k| k == "Translation")
        }) {
            let points = match g.raw_get::<Option<Table>>("_base_points")? {
                Some(p) => p,
                None => {
                    let points = lua.create_table()?;
                    let count = owner.call_method::<usize>("GetNumPoints", ())?;
                    for i in 1..=count {
                        let values = owner.call_method::<MultiValue>("GetPoint", i)?;
                        let point = lua.create_table()?;
                        for (j, v) in values.into_iter().enumerate() {
                            point.raw_set(j + 1, v)?;
                        }
                        points.raw_set(i, point)?;
                    }
                    g.raw_set("_base_points", points.clone())?;
                    points
                }
            };
            for point in points.sequence_values::<Table>() {
                let point = point?;
                owner.call_method::<()>(
                    "SetPoint",
                    (
                        point.raw_get::<Value>(1)?,
                        point.raw_get::<Value>(2)?,
                        point.raw_get::<Value>(3)?,
                        point.raw_get::<f64>(4)? + offset.0,
                        point.raw_get::<f64>(5)? + offset.1,
                    ),
                )?;
            }
        }

        if time >= duration && looping == "NONE" {
            g.raw_set("_playing", false)?;
            restore(lua, &g)?;
            fire(&g, "OnFinished")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::UiScript;
    #[test]
    fn alpha_bounce_pause_and_stop_restore_owner() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"f=CreateFrame('Frame','AnimationTest'); g=f:CreateAnimationGroup(); g:SetLooping('BOUNCE'); a=g:CreateAnimation('Alpha'); a:SetChange(-0.7); a:SetDuration(1); a:Play()"#).unwrap();
        s.tick(0.5);
        assert!((s.eval::<f64>("return f:GetAlpha()").unwrap() - 165.0 / 255.0).abs() < 1e-9);
        s.run("a:Pause()").unwrap();
        s.tick(0.5);
        assert!((s.eval::<f64>("return f:GetAlpha()").unwrap() - 165.0 / 255.0).abs() < 1e-9);
        s.run("a:Play()").unwrap();
        s.tick(1.0);
        assert!((s.eval::<f64>("return f:GetAlpha()").unwrap() - 165.0 / 255.0).abs() < 1e-9);
        s.run("a:Stop()").unwrap();
        assert_eq!(s.eval::<f64>("return f:GetAlpha()").unwrap(), 1.0);
    }
}
