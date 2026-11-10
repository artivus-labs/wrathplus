//! Build-12340 methods of the Region base and the Frame and Texture tables that the 1.12 tables
//! lack: the region table's queries on textures, font strings and the title region, a frame's
//! bounds, and the stored flags.

use mlua::{Lua, MultiValue, Table, Value};

use super::binding_abi::bool_or_default;
use super::object::{decode_id, eff_scale, frame_handle_of, settle};
use super::region::region_handle_of;
use super::{
    Model, REG_FONTSTRING_METHODS, REG_FRAME_METHODS, REG_REGION_METHODS, REG_TEXTURE_METHODS,
    REG_TITLE_METHODS,
};
use crate::layout::Rect;
use crate::widget::FrameHandle;

fn one(on: bool) -> Option<i64> {
    on.then_some(1)
}

/// A flag stored per object and answered as 1 or nil, with no consumer here: the setter coerces
/// through `0x815500` with an absent argument true, and returns nothing.
fn stored_flag(lua: &Lua, tables: &[&Table], set: &str, get: &str) -> mlua::Result<()> {
    let weak = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.set("__mode", "k")?;
    weak.set_metatable(Some(meta))?;
    let store = weak.clone();
    let setter = lua.create_function(move |_, (this, rest): (Table, MultiValue)| {
        store.raw_set(this, bool_or_default(rest.front(), true).then_some(true))
    })?;
    let getter = lua.create_function(move |_, this: Table| {
        Ok(one(weak.raw_get::<Option<bool>>(this)?.is_some()))
    })?;
    for t in tables {
        t.set(set, setter.clone())?;
        t.set(get, getter.clone())?;
    }
    Ok(())
}

/// `IsMouseOver([top, bottom, left, right])` (12340 `0x49e100`, Region table `0xac1480`, so one
/// function on frames and regions): each offset a number or 0, ADDED to its edge in the object's
/// own units; the cursor must be strictly inside (`0x488570`). False before the first resolve.
fn is_mouse_over(lua: &Lua, (this, args): (Table, MultiValue)) -> mlua::Result<bool> {
    let off = |i: usize| {
        args.get(i)
            .cloned()
            .and_then(|v| lua.coerce_number(v).ok().flatten())
            .map_or(0.0, |n| n as f32)
    };
    let (top, bottom, left, right) = (off(0), off(1), off(2), off(3));
    let id = decode_id(&this)?;
    settle(lua);
    let model = lua.app_data_ref::<Model>().expect("model");
    let target = if let Some(&h) = model.id_to_frame.get(&id) {
        Some((model.resolved.get(&h).copied(), h))
    } else if let Some(&rh) = model.id_to_region.get(&id) {
        let rect = model.region_resolved.get(&rh).copied();
        model.arena.region(rh).map(|r| (rect, r.owner))
    } else {
        None
    };
    let Some((rect, owner)) = target else {
        drop(model);
        // Not a live widget: the frame lookup names why.
        frame_handle_of(lua, &this)?;
        return Ok(false);
    };
    Ok(inside(&model, rect, owner, (top, bottom, left, right)))
}

fn inside(model: &Model, rect: Option<Rect>, owner: FrameHandle, o: (f32, f32, f32, f32)) -> bool {
    let (top, bottom, left, right) = o;
    let inv = 1.0 / eff_scale(model, owner);
    let (x, y) = (model.cursor_pos.0 * inv, model.cursor_pos.1 * inv);
    rect.is_some_and(|r| {
        r.left * inv + left < x
            && x < r.right * inv + right
            && r.bottom * inv + bottom < y
            && y < r.top * inv + top
    })
}

/// The bounds union of `0x4913c0`: the frame's own rect when it has one, each shown region's
/// (`+0xcc & 0x10`, `IsShown`'s bit, `0x48c63b`), and each shown child's (`+0xdc`, `0x49febb`)
/// union in turn, min of left and bottom, max of right and top.
fn union_bounds(model: &Model, h: FrameHandle, acc: &mut Rect) {
    let mut add = |r: &Rect| {
        acc.left = acc.left.min(r.left);
        acc.bottom = acc.bottom.min(r.bottom);
        acc.right = acc.right.max(r.right);
        acc.top = acc.top.max(r.top);
    };
    let Some(f) = model.arena.frame(h) else {
        return;
    };
    if let Some(r) = model.resolved.get(&h) {
        add(r);
    }
    for &rh in &f.regions {
        let detached = model.arena.region(rh).is_none_or(|r| r.detached);
        let hidden = model.region_data.get(&rh).is_some_and(|d| d.hidden);
        if let (false, false, Some(r)) = (detached, hidden, model.region_resolved.get(&rh)) {
            add(r);
        }
    }
    for &c in &f.children {
        if model.arena.frame(c).is_some_and(|cf| cf.shown) {
            union_bounds(model, c, acc);
        }
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let table = |key: &str| lua.named_registry_value::<Table>(key);
    let (frames, textures, strings, title) = (
        table(REG_FRAME_METHODS)?,
        table(REG_TEXTURE_METHODS)?,
        table(REG_FONTSTRING_METHODS)?,
        table(REG_TITLE_METHODS)?,
    );

    // The Region table (`0xac1480`) every class registers first (`0x49ca90`): the frames'
    // protection queries (`0x49cad0`, `0x49cb60`) and `IsMouseOver` on textures, font strings and
    // the title region too, and the title region's size and animation methods (`0x49ce50`,
    // `0x49d6f0`, `0x49d7a0`, `0x49e350`, `0x49e260`, `0x49e070`).
    let mouse_over = lua.create_function(is_mouse_over)?;
    frames.set("IsMouseOver", mouse_over.clone())?;
    for leaf in [&textures, &strings, &title] {
        for name in ["IsProtected", "CanChangeProtectedState"] {
            leaf.set(name, frames.get::<Value>(name)?)?;
        }
        leaf.set("IsMouseOver", mouse_over.clone())?;
    }
    let regions = table(REG_REGION_METHODS)?;
    for name in ["SetSize", "GetSize", "GetRect"] {
        title.set(name, regions.get::<Value>(name)?)?;
    }
    super::animation::install(lua, &title)?;

    // IsDesaturated() (`0x48ca30`): 1 while `SetDesaturated` holds; the shader always exists here.
    textures.set(
        "IsDesaturated",
        lua.create_function(|lua, this: Table| {
            let rh = region_handle_of(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(one(model
                .region_data
                .get(&rh)
                .is_some_and(|d| d.desaturated)))
        })?,
    )?;
    // Stored; the client has no consumer: SetNonBlocking (`0x48caa0`, bit 0 of `+0x160`) and
    // EnableJoystick (`0x4a11d0`, input class 4 of `+0xd8`, read back by `0x4a1260`).
    stored_flag(lua, &[&textures], "SetNonBlocking", "GetNonBlocking")?;
    stored_flag(lua, &[&frames], "EnableJoystick", "IsJoystickEnabled")?;

    // SetDontSavePosition([flag]) (`0x4a08c0`, absent true) and GetDontSavePosition (`0x4a0910`):
    // `+0xb4 & 0x80000`, which keeps the frame out of the layout cache.
    frames.set(
        "SetDontSavePosition",
        lua.create_function(|lua, (this, rest): (Table, MultiValue)| {
            let h = frame_handle_of(lua, &this)?;
            let on = bool_or_default(rest.front(), true);
            let mut model = lua.app_data_mut::<Model>().expect("model");
            if let Some(f) = model.arena.frame_mut(h) {
                f.dont_save_position = on;
            }
            Ok(())
        })?,
    )?;
    frames.set(
        "GetDontSavePosition",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(one(model
                .arena
                .frame(h)
                .is_some_and(|f| f.dont_save_position)))
        })?,
    )?;

    // GetBoundsRect() (`0x49e700`): [`union_bounds`] from a rect seeded `FLT_MAX` low and 0 high,
    // as `left, bottom, width, height` in the frame's units, or nothing unless the union has
    // area (`0x491562`-`0x49157d`).
    frames.set(
        "GetBoundsRect",
        lua.create_function(|lua, this: Table| {
            let h = frame_handle_of(lua, &this)?;
            settle(lua);
            let model = lua.app_data_ref::<Model>().expect("model");
            let mut r = Rect {
                left: f32::MAX,
                bottom: f32::MAX,
                right: 0.0,
                top: 0.0,
            };
            union_bounds(&model, h, &mut r);
            if !(r.right > r.left && r.top > r.bottom) {
                return Ok(MultiValue::new());
            }
            let inv = 1.0 / eff_scale(&model, h);
            let n = |v: f32| Value::Number(f64::from(v * inv));
            Ok(MultiValue::from_vec(vec![
                n(r.left),
                n(r.bottom),
                n(r.right - r.left),
                n(r.top - r.bottom),
            ]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::UiScript;

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(1024.0, 768.0);
        s.run(
            r#"
            F = CreateFrame("Frame", "WidgetF") F:SetWidth(200) F:SetHeight(100) F:SetPoint("BOTTOMLEFT", 100, 100)
            T = F:CreateTexture() T:SetWidth(50) T:SetHeight(50) T:SetPoint("BOTTOMLEFT", F, "BOTTOMLEFT")
            S = F:CreateFontString() R = F:CreateTitleRegion()
            "#,
        )
        .unwrap();
        s
    }

    fn check(s: &UiScript, checks: &[&str]) {
        for c in checks {
            assert!(s.eval::<bool>(&format!("return {c}")).unwrap(), "{c}");
        }
    }

    /// The Region table (`0xac1480`): one `IsMouseOver` on frames and every region, over the
    /// region's own rect, and the protection queries; the title region's size and animations.
    #[test]
    fn the_region_table_reaches_textures_strings_and_the_title_region() {
        let mut s = vm();
        s.run("R:SetSize(80, 20) R:SetPoint('TOPLEFT')").unwrap();
        check(
            &s,
            &[
                "T.IsMouseOver == F.IsMouseOver and S.IsMouseOver == F.IsMouseOver",
                "R.IsMouseOver == F.IsMouseOver and T.IsProtected == F.IsProtected",
                "select('#', S:IsProtected()) == 2 and R:CanChangeProtectedState() == 1",
                "select(3, R:GetRect()) == 80 and select(2, R:GetSize()) == 20",
                "R:CreateAnimationGroup() == R:GetAnimationGroups()",
            ],
        );
        // Inside the texture's 50 x 50 at (100, 100), then right of it but inside the frame.
        s.mouse_move(120.0, 120.0);
        check(&s, &["T:IsMouseOver() and F:IsMouseOver()"]);
        s.mouse_move(160.0, 120.0);
        check(&s, &["not T:IsMouseOver() and F:IsMouseOver()"]);
        check(&s, &["T:IsMouseOver(0, 0, 0, 20)", "not R:IsMouseOver()"]);
    }

    /// `IsDesaturated` reads `SetDesaturated`'s state (`0x48ca30`); the flag setters coerce through
    /// `0x815500`, an absent argument true; `SetDontSavePosition` keeps a user-placed frame out of
    /// the layout cache (`0x517e37`), and the other two only store.
    #[test]
    fn desaturation_and_the_stored_flags_read_back() {
        let s = vm();
        s.run("F:SetMovable(true) F:SetUserPlaced(true)").unwrap();
        let cached = |s: &UiScript| s.user_placed_layouts().len();
        s.run("T:SetDesaturated(1) T:SetNonBlocking() F:EnableJoystick(1) F:SetDontSavePosition()")
            .unwrap();
        check(
            &s,
            &[
                "T:IsDesaturated() == 1 and T:GetNonBlocking() == 1",
                "F:IsJoystickEnabled() == 1 and F:GetDontSavePosition() == 1",
            ],
        );
        assert_eq!(cached(&s), 0, "dontSavePosition drops the row");
        s.run("T:SetDesaturated(nil) T:SetNonBlocking('0') F:EnableJoystick(nil)")
            .unwrap();
        s.run("F:SetDontSavePosition(false)").unwrap();
        check(
            &s,
            &[
                "T:IsDesaturated() == nil and T:GetNonBlocking() == nil",
                "F:IsJoystickEnabled() == nil and F:GetDontSavePosition() == nil",
            ],
        );
        assert_eq!(cached(&s), 1, "written again");
    }

    /// `GetBoundsRect` (`0x49e700`): the frame's rect, its shown regions' and its shown children's
    /// bounds, whatever the frame's own visibility; nothing without area.
    #[test]
    fn the_bounds_rect_unions_shown_regions_and_children() {
        let s = vm();
        s.run(
            r#"T:SetPoint("BOTTOMLEFT", F, "BOTTOMLEFT", -50, 0)
            C = CreateFrame("Frame", nil, F) C:SetWidth(10) C:SetHeight(10)
            C:SetPoint("BOTTOMLEFT", F, "TOPRIGHT")
            function B() return table.concat({ F:GetBoundsRect() }, " ") end"#,
        )
        .unwrap();
        check(
            &s,
            &[
                "B() == '50 100 260 110'",
                "select('#', CreateFrame('Frame'):GetBoundsRect()) == 0",
            ],
        );
        s.run("T:Hide() C:Hide() F:Hide()").unwrap();
        check(&s, &["B() == '100 100 200 100'"]);
    }
}
