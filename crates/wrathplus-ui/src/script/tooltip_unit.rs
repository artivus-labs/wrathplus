//! The engine unit tooltip in `0x529fe0`'s line order: name (gold; FrameXML recolours it by
//! reaction), subtitle, level, faction, "PvP", the gather line, "Civilian", "Leader", and health
//! on `<name>StatusBar`; driven by [`super::UiScript::world_tooltip_unit`] and Lua's `SetUnit`.

use mlua::{Lua, Table};

use super::object::frame_handle_of;
use super::tooltip::{append_line, clear_content, fire_cleared, show_or_hide_empty, tip_mut};
use super::unit::{is_civilian_kill, unknownobject};
use super::{KindState, Model, UnitState};
use crate::layout::{Anchor, Point};
use crate::widget::FrameHandle;

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const RED: [f32; 4] = [1.0, 32.0 / 255.0, 32.0 / 255.0, 1.0];
/// `0xc0d420` = `0xff40c040`, the satisfiable-lock green.
const LOCK_OPEN: [f32; 4] = [64.0 / 255.0, 192.0 / 255.0, 64.0 / 255.0, 1.0];
/// `0xffffd200`, the unit name's colour until FrameXML recolours it by reaction.
const GOLD: [f32; 4] = [1.0, 210.0 / 255.0, 0.0, 1.0];

/// A GameObject tooltip line's colour, as the builder `0x52aa20` picks it; the app picks the tint
/// for each line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooltipTint {
    /// `0xc0cf60`: a requirement line's normal colour (the key item's "Requires %s").
    White,
    /// `0xc0d3a8` = `0xffff2020`, the unmet "Locked" line. Deviation: the unknown-skill
    /// "Requires %s" shares it, though its reference red is `0xffff0000`, because a 32/255
    /// difference in G and B does not show.
    Red,
    /// The satisfiable "Locked" line: `0xc0d420` = `0xff40c040`, the green rung of the
    /// skill-difficulty ramp `0x529fa0`, not the ordinary green `0xc0d3ac` = `0xff00ff00`.
    LockOpen,
}

mod level;
use level::level_line;

/// Render the unit tooltip for `token`'s current snapshot; returns whether the unit existed.
fn render_unit(lua: &Lua, this: &Table, token: &str) -> mlua::Result<bool> {
    let unit = {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        model.unit(token).cloned().filter(|u| u.exists)
    };
    render_unit_view(lua, this, unit)
}

/// A `unit:<guid hex>` link (`0x62dae0` → `0x621070` by guid): a unit the VM holds with that guid,
/// by token or by guid, else nothing.
pub(super) fn render_unit_guid(lua: &Lua, this: &Table, guid: u64) -> mlua::Result<bool> {
    let unit = {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        model
            .units_by_lower
            .values()
            .find(|u| u.guid == guid)
            .or_else(|| model.units_by_guid.get(&guid))
            .cloned()
            .filter(|u| u.exists && guid != 0)
    };
    render_unit_view(lua, this, unit)
}

fn render_unit_view(lua: &Lua, this: &Table, unit: Option<UnitState>) -> mlua::Result<bool> {
    let h = frame_handle_of(lua, this)?;
    let player_level = lua
        .app_data_ref::<Model>()
        .expect("model app_data")
        .player_req
        .level;
    {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        clear_content(&mut model, h);
    }
    fire_cleared(lua, h);
    let Some(u) = unit else {
        set_subject(lua, h, None);
        show_or_hide_empty(lua, h);
        return Ok(false);
    };
    // The name: `CGUnit_C::GetUnitName` `0x609210` (`0x52a187`), whose every miss falls to
    // `UNKNOWNOBJECT`, as `UnitName`'s does. A token with no object never reaches the builder
    // (`0x468460`): no plate, and `SetUnit` answers nil.
    let title = match &u.name {
        Some(n) => n.clone(),
        None => unknownobject(lua)?.to_str()?.to_string(),
    };
    append_line(lua, this, (title, GOLD), None, false)?;
    if let Some(sub) = &u.subtitle {
        append_line(lua, this, (sub.clone(), WHITE), None, false)?;
    }
    if let Some(level) = level_line(lua, &u, player_level) {
        append_line(lua, this, (level, WHITE), None, false)?;
    }
    // The faction name, between the level line and "PvP" (`0x52a7a0`); the app applied its gates.
    if let Some(faction) = &u.faction_name {
        append_line(lua, this, (faction.clone(), WHITE), None, false)?;
    }
    if u.pvp {
        append_line(lua, this, ("PvP".into(), WHITE), None, false)?;
    }
    if let Some((key, difficulty)) = u.skinnable {
        let line = super::tooltip_gather::line(lua, key, difficulty);
        append_line(lua, this, line, None, false)?;
    }
    // "Civilian" (`0x612550`): PvP-flagged, a civilian, hostile, and grey to kill (`0x5f0700`).
    if is_civilian_kill(&u, player_level) {
        append_line(lua, this, ("Civilian".into(), GREEN), None, false)?;
    }
    // "Leader" (`0x6125c0`): PvP-flagged and a racial leader, no other gate.
    if u.racial_leader && u.pvp {
        append_line(lua, this, ("Leader".into(), WHITE), None, false)?;
    }
    super::tooltip_unit_quest::append(lua, this, u.guid)?;
    // Quest lines belong before the looter tail (12340 `0x622209`).
    super::tooltip_unit_looters::append(lua, this, u.guid)?;
    // An unresolved guid (0) is no subject: nothing can push for it.
    set_subject(lua, h, Some(u.guid).filter(|&g| g != 0));
    update_bar(lua, h, Some(&u));
    super::tooltip::finish_content(lua, h, "OnTooltipSetUnit");
    Ok(true)
}

/// Mark the tooltip as world-hover-owned (the fade-on-loss gate).
fn set_world_owned(lua: &Lua, h: FrameHandle) {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    if let Some(f) = model.arena.frame_mut(h) {
        if let KindState::Tooltip(t) = &mut f.kind_state {
            t.world_owned = true;
        }
    }
}

/// Remember (or drop) the guid of the unit the tooltip shows, the health watcher's key.
fn set_subject(lua: &Lua, h: FrameHandle, guid: Option<u64>) {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    if let Some(f) = model.arena.frame_mut(h) {
        if let KindState::Tooltip(t) = &mut f.kind_state {
            t.unit_guid = guid;
        }
    }
}

/// Drive the `<name>StatusBar` child from a unit snapshot (`None` hides it); FrameXML's
/// `HealthBar_OnValueChanged` colours the fill.
fn update_bar(lua: &Lua, h: FrameHandle, unit: Option<&UnitState>) {
    let bar = {
        let model = lua.app_data_mut::<Model>().expect("model app_data");
        let name = model.arena.frame(h).and_then(|f| f.name.clone());
        name.and_then(|n| model.arena.lookup(&format!("{n}StatusBar")))
    };
    let Some(bar) = bar else { return };
    let (changed, shown) = {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let Some(f) = model.arena.frame_mut(bar) else {
            return;
        };
        match (&mut f.kind_state, unit) {
            (KindState::StatusBar(sb), Some(u)) => {
                sb.min = 0.0;
                sb.max = u.max_health.max(1) as f32;
                let v = u.health.min(u.max_health.max(1)) as f32;
                let changed = (sb.value - v).abs() > f32::EPSILON;
                sb.value = v;
                (changed, true)
            }
            _ => (false, false),
        }
    };
    {
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.arena.set_shown(bar, shown);
    }
    if changed {
        let (id, value) = {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let v = match model.arena.frame(bar).map(|f| &f.kind_state) {
                Some(KindState::StatusBar(sb)) => sb.value,
                _ => 0.0,
            };
            (model.frame_id(bar), v)
        };
        if let Err(e) = super::event::fire_widget_handler(
            lua,
            id,
            "OnValueChanged",
            vec![mlua::Value::Number(f64::from(value))],
        ) {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .record_script_error(e.to_string());
        }
    }
}

/// A `set_unit` push of the unit a tooltip shows updates its health bar without rebuilding the
/// lines, as the reference's HEALTH field watcher does: it is keyed on the unit's guid (`0x52a0a8`
/// registers `0x529560` on `+0x368`/`+0x36c`), so a token that stops naming the unit, as
/// `"mouseover"` does when the hover ends, leaves a fading plate's bar standing.
pub(super) fn on_unit_push(lua: &Lua, token: &str) {
    let (unit, hits): (UnitState, Vec<FrameHandle>) = {
        let model = lua.app_data_mut::<Model>().expect("model app_data");
        let Some(unit) = model.pushed_unit(token).filter(|u| u.exists && u.guid != 0) else {
            return;
        };
        // Registered frames only, as `update_bar` writes layout.
        let hits: Vec<FrameHandle> = model
            .arena
            .tooltip_kinds()
            .iter()
            .copied()
            .filter(|&h| {
                model.frame_to_id.contains_key(&h)
                    && matches!(
                        model.arena.frame(h).map(|f| &f.kind_state),
                        Some(KindState::Tooltip(t)) if t.unit_guid == Some(unit.guid)
                    )
            })
            .collect();
        if hits.is_empty() {
            return;
        }
        (unit.clone(), hits)
    };
    for h in hits {
        update_bar(lua, h, Some(&unit));
    }
}

impl super::UiScript {
    /// Show the world-mouseover unit tooltip: fire `OnTooltipSetDefaultAnchor`, render `token`,
    /// fire `UPDATE_MOUSEOVER_UNIT`. Call on a hover-target change; a re-show mid-fade returns to
    /// full alpha. `false` when there is no tooltip frame or no such unit.
    pub fn world_tooltip_unit(&mut self, token: &str) -> bool {
        let h = {
            let mut model = self.model_mut();
            let Some(h) = model.arena.lookup("GameTooltip") else {
                return false;
            };
            let id = model.frame_id(h);
            (h, id)
        };
        let (h, id) = h;
        if let Err(e) =
            super::event::fire_widget_handler(&self.lua, id, "OnTooltipSetDefaultAnchor", vec![])
        {
            self.push_error(e);
        }
        let wrapper = match super::object::frame_wrapper(&self.lua, id) {
            Ok(w) => w,
            Err(e) => {
                self.push_error(e);
                return false;
            }
        };
        let shown = match render_unit(&self.lua, &wrapper, token) {
            Ok(s) => s,
            Err(e) => {
                self.push_error(e);
                false
            }
        };
        if shown {
            set_world_owned(&self.lua, h);
            let mut model = self.model_mut();
            if let Some(f) = model.arena.frame_mut(h) {
                if let KindState::Tooltip(t) = &mut f.kind_state {
                    if t.fade_start.take().is_some() {
                        model.arena.set_alpha(h, 1.0);
                    }
                }
            }
            drop(model);
            self.fire_event("UPDATE_MOUSEOVER_UNIT", vec![]);
        }
        shown
    }

    /// Show the world-mouseover GameObject tooltip (builder `0x52aa20`): the name in gold, then
    /// the caller's lock lines. The anchor forks on the object's `[vtbl+0x5c]`: true (`0x492a01`)
    /// seats it at the cursor, anchor state 6 (`0x52ffe0(owner, 6, 0, 0)`); false (`0x492a42`)
    /// keeps the default corner, as units (`0x492983`) always do. `cursor` is that verdict, and
    /// [`Self::world_tooltip_move`] follows a cursor-seated plate. Hover loss arms the unit flow's
    /// fade, where the reference's loss path (`0x530ae0`) hides an anchor-state-6 plate at once.
    pub fn world_tooltip_gameobject(
        &mut self,
        name: &str,
        lines: &[(String, TooltipTint)],
        cursor: Option<(f32, f32)>,
    ) -> bool {
        let (h, id, root, root_id) = {
            let mut model = self.model_mut();
            let Some(h) = model.arena.lookup("GameTooltip") else {
                return false;
            };
            let Some(root) = model.arena.lookup("UIParent") else {
                return false;
            };
            let (id, root_id) = (model.frame_id(h), model.frame_id(root));
            (h, id, root, root_id)
        };
        match cursor {
            // The cursor arm: centred above the pointer; compare-then-touch, so a still pointer
            // never re-layouts.
            Some((ui_x, ui_y)) => {
                let mut model = self.model_mut();
                // The owner: this arm is the SetOwner core (`0x52ffe0`), which stores it at
                // `+0x314` (`0x53000c`); the other arms get theirs from FrameXML's
                // `OnTooltipSetDefaultAnchor`. Without one, any Lua `GameTooltip:Show()` hides the
                // plate: `0x530a80` shows only when `+0x314` and `+0x31c` are both set, else it
                // takes `0x530a60`.
                if let Ok(t) = tip_mut(&mut model, h) {
                    t.owner = Some(root);
                }
                let input = model.layout_inputs.entry(h).or_default();
                let new = Anchor::new(Point::Bottom, root_id, Point::BottomLeft, ui_x, ui_y);
                let same = input.anchors.len() == 1
                    && super::object::anchor_bits_eq(&input.anchors[0], &new);
                if !same {
                    input.anchors = vec![new];
                    model.touch_layout();
                }
            }
            // The corner arm: the default-anchor handler the unit flow fires too.
            None => {
                if let Err(e) = super::event::fire_widget_handler(
                    &self.lua,
                    id,
                    "OnTooltipSetDefaultAnchor",
                    vec![],
                ) {
                    self.push_error(e);
                }
            }
        }
        let wrapper = match super::object::frame_wrapper(&self.lua, id) {
            Ok(w) => w,
            Err(e) => {
                self.push_error(e);
                return false;
            }
        };
        let render = || -> mlua::Result<()> {
            {
                let mut model = self.lua.app_data_mut::<Model>().expect("model app_data");
                clear_content(&mut model, h);
            }
            fire_cleared(&self.lua, h);
            append_line(&self.lua, &wrapper, (name.to_string(), GOLD), None, false)?;
            for (text, tint) in lines {
                let colour = match tint {
                    TooltipTint::White => WHITE,
                    TooltipTint::Red => RED,
                    TooltipTint::LockOpen => LOCK_OPEN,
                };
                append_line(&self.lua, &wrapper, (text.clone(), colour), None, false)?;
            }
            Ok(())
        };
        if let Err(e) = render() {
            self.push_error(e);
            return false;
        }
        set_world_owned(&self.lua, h);
        {
            let mut model = self.model_mut();
            if let Some(f) = model.arena.frame_mut(h) {
                if let KindState::Tooltip(t) = &mut f.kind_state {
                    if t.fade_start.take().is_some() {
                        model.arena.set_alpha(h, 1.0);
                    }
                }
            }
        }
        show_or_hide_empty(&self.lua, h);
        true
    }

    /// Show the minimap blip tooltip (an `AreaPOI` or quest-dot name): one gold line, faint gold
    /// for a cross-interior dot, its bottom at the given UI point above the cursor.
    /// [`Self::world_tooltip_move`] follows the pointer, and hover loss arms
    /// [`Self::world_tooltip_fade`].
    pub fn minimap_tooltip(&mut self, text: &str, ui_x: f32, ui_y: f32, grey: bool) -> bool {
        let (h, id, root_id) = {
            let mut model = self.model_mut();
            let Some(h) = model.arena.lookup("GameTooltip") else {
                return false;
            };
            let Some(root) = model.arena.lookup("UIParent") else {
                return false;
            };
            let (id, root_id) = (model.frame_id(h), model.frame_id(root));
            (h, id, root_id)
        };
        let wrapper = match super::object::frame_wrapper(&self.lua, id) {
            Ok(w) => w,
            Err(e) => {
                self.push_error(e);
                return false;
            }
        };
        {
            let mut model = self.model_mut();
            clear_content(&mut model, h);
            let input = model.layout_inputs.entry(h).or_default();
            // Anchor only: the frame's own clamp flag (`Frame::clamped_to_screen`, G flags bit 4)
            // slides a plate near the window edge back on screen.
            let new = Anchor::new(Point::Bottom, root_id, Point::BottomLeft, ui_x, ui_y);
            // Compare-then-touch, so a still cursor does not dirty the layout.
            let same =
                input.anchors.len() == 1 && super::object::anchor_bits_eq(&input.anchors[0], &new);
            if !same {
                input.anchors = vec![new];
                model.touch_layout();
            }
        }
        fire_cleared(&self.lua, h);
        // A cross-interior entry's `|cffb0b0b0` wrap modulates the gold: faint gold, not grey.
        let dim = 0xb0 as f32 / 255.0;
        let color = if grey {
            [GOLD[0] * dim, GOLD[1] * dim, GOLD[2] * dim, 1.0]
        } else {
            GOLD
        };
        if let Err(e) = append_line(&self.lua, &wrapper, (text.to_string(), color), None, false) {
            self.push_error(e);
            return false;
        }
        set_world_owned(&self.lua, h);
        {
            let mut model = self.model_mut();
            if let Some(f) = model.arena.frame_mut(h) {
                if let KindState::Tooltip(t) = &mut f.kind_state {
                    if t.fade_start.take().is_some() {
                        model.arena.set_alpha(h, 1.0);
                    }
                }
            }
        }
        show_or_hide_empty(&self.lua, h);
        true
    }

    /// Re-seat the shown world-owned tooltip at a new pointer point, anchor only; else a no-op.
    pub fn world_tooltip_move(&mut self, ui_x: f32, ui_y: f32) {
        let mut model = self.model_mut();
        let Some(h) = model.arena.lookup("GameTooltip") else {
            return;
        };
        let Some(root) = model.arena.lookup("UIParent") else {
            return;
        };
        let owned_shown = model
            .arena
            .frame(h)
            .map(|f| matches!(&f.kind_state, KindState::Tooltip(t) if t.world_owned) && f.shown)
            .unwrap_or(false);
        if !owned_shown {
            return;
        }
        let root_id = model.frame_id(root);
        let input = model.layout_inputs.entry(h).or_default();
        let new = Anchor::new(Point::Bottom, root_id, Point::BottomLeft, ui_x, ui_y);
        // Compare-then-touch: this runs on every pointer event.
        let same =
            input.anchors.len() == 1 && super::object::anchor_bits_eq(&input.anchors[0], &new);
        if !same {
            input.anchors = vec![new];
            model.touch_layout();
        }
    }

    /// Whether `GameTooltip` is still the shown plate a world hover put up, with no Lua
    /// `SetOwner`, content `Set*` or `Hide` since.
    pub fn world_tooltip_up(&mut self) -> bool {
        let model = self.model_mut();
        let Some(h) = model.arena.lookup("GameTooltip") else {
            return false;
        };
        model
            .arena
            .frame(h)
            .map(|f| matches!(&f.kind_state, KindState::Tooltip(t) if t.world_owned) && f.shown)
            .unwrap_or(false)
    }

    /// On hover loss, arm the world tooltip's timed fade (the reference's unit plate fades rather
    /// than hiding); a no-op unless a world-owned plate is shown.
    pub fn world_tooltip_fade(&mut self) {
        let now = self.now();
        let mut model = self.model_mut();
        let Some(h) = model.arena.lookup("GameTooltip") else {
            return;
        };
        if let Some(f) = model.arena.frame_mut(h) {
            if let KindState::Tooltip(t) = &mut f.kind_state {
                if t.world_owned && f.shown && t.fade_start.is_none() {
                    t.fade_start = Some(now);
                }
            }
        }
    }
}

/// Register the unit content channel into the GameTooltip kind method table.
pub(super) fn install_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    // GameTooltip:SetUnit(token) → 1 or nil, which `UnitFrame_OnEnter` reads to arm its re-poll.
    m.set(
        "SetUnit",
        lua.create_function(|lua, (this, token): (Table, String)| {
            let ok = render_unit(lua, &this, &token)?;
            Ok(if ok {
                mlua::Value::Integer(1)
            } else {
                mlua::Value::Nil
            })
        })?,
    )?;
    Ok(())
}
