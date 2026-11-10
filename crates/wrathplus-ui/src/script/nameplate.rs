//! The nameplate widgets: build 12340's `CGNamePlateFrame` (ctor `0x98f790`), an anonymous
//! `Button` parented to the `WorldFrame` at strata WORLD. Plate addons find it in
//! `WorldFrame:GetChildren()`, tell it by its second region's `Nameplate-Border`, and read it
//! positionally: eleven regions in creation order (threat flash, health border, cast border, cast
//! shield, spell icon, highlight, name, level, boss skull, raid icon, elite icon) and two
//! children, the health bar then the cast bar. Addons take plates over, so static properties are
//! written once, anchors when the window changes and state when it moves.
//!
//! Switching plates on and off is the app's: build 12340 has no plate verbs, and the stock V keys
//! write the `nameplateShow*` CVars it reads.

use crate::layout::{Anchor, Point};
use crate::order::DrawLayer;
use crate::widget::{FrameHandle, FrameKind, KindState, RegionHandle, RegionKind};

use super::model::Model;
use super::UiScript;
use super::{BlendMode, FontShadow, JustifyV, RegionData, TexCoords};
use widgets::{font_string, status_bar, texture};

mod widgets;

/// Region 1's art, the threat flash.
pub const FLASH_TEXTURE: &str = "Interface\\TargetingFrame\\UI-TargetingFrame-Flash";
/// Regions 2 and 3's art, the health and cast borders; region 2's is the path plate addons
/// identify a plate by.
pub const BORDER_TEXTURE: &str = "Interface\\Tooltips\\Nameplate-Border";
/// Region 4's art, shown in the cast border's place for a cast that cannot be interrupted.
pub const CAST_SHIELD_TEXTURE: &str = "Interface\\Tooltips\\Nameplate-CastBar-Shield";
/// Region 6's art, the highlight; addons read mouseover from `glow:IsShown()`.
pub const GLOW_TEXTURE: &str = "Interface\\Tooltips\\Nameplate-Glow";
/// Region 9's art, the boss and out-of-range skull.
pub const SKULL_TEXTURE: &str = "Interface\\TargetingFrame\\UI-TargetingFrame-Skull";
/// Region 10's art, a 4-column atlas.
pub const RAID_ICON_TEXTURE: &str = "Interface\\TargetingFrame\\UI-RaidTargetingIcons";
/// Region 11's art, the elite dragon.
pub const ELITE_TEXTURE: &str = "Interface\\Tooltips\\EliteNameplateIcon";
/// The cast bar's colour while a cast runs, `0x98f0f3`'s `(1.0, 0.7, 0.0)`.
pub const CAST_COLOUR: [f32; 3] = [1.0, 0.7, 0.0];
/// The health bar's fill, which `GetStatusBarTexture()` returns.
pub const BAR_FILL_TEXTURE: &str = "Interface\\TargetingFrame\\UI-TargetingFrame-BarFill";
/// `NAMEPLATE_FONT`: Friz Quadrata.
pub const PLATE_FONT: &str = "Fonts\\FRIZQT__.TTF";

/// The plate's level; the bar's is one less, set absolutely (`0x7cb33e` → `0x76a4f0`). The draw
/// walks levels outermost and each level's layers inside (`0x765920`), so the whole bar draws
/// before the border that caps it.
const PLATE_LEVEL: u16 = 1;
const BAR_LEVEL: u16 = 0;

/// The plate geometry that follows from the window rather than the unit, in FrameXML units with
/// the seam scale divided out. Written only when it changes, so an addon's bar anchor survives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateGeometry {
    pub width: f32,
    pub height: f32,
    /// The health bar, from the plate's BOTTOMLEFT.
    pub bar_off_x: f32,
    pub bar_off_y: f32,
    pub bar_width: f32,
    pub bar_height: f32,
    /// The level text and skull seat, from the plate's BOTTOMRIGHT (x is subtracted, y added).
    pub level_off_x: f32,
    pub level_off_y: f32,
    pub skull_size: f32,
    pub raid_size: f32,
    pub name_height: f32,
    pub level_height: f32,
    /// The text's drop-shadow step, seam divided out like the rest: one logical pixel at any
    /// window size, since the text path multiplies it by the seam.
    pub shadow_offset: f32,
}

/// One plate's per-frame state: everything that can move while the plate is alive.
#[derive(Clone, Debug, PartialEq)]
pub struct PlateState {
    /// The unit's GUID and the plate's identity: a plate is bound to its unit for its life
    /// (`[unit+0xe60]`), and addons cache per-plate state on the frame.
    pub key: u64,
    /// Where the plate's top centre lands, in FrameXML units from the screen's bottom-left; the
    /// plate hangs below it (`0x509ec0`).
    pub top_centre: (f32, f32),
    /// `UNIT_FIELD_HEALTH` and `UNIT_FIELD_MAXHEALTH`, raw: `GetValue()` is the health, not a
    /// fraction (`0x78f5d0`), and `GetMinMaxValues()` is `(0, max)`; plate addons divide.
    pub health: f32,
    pub max_health: f32,
    /// The bar's reaction colour, exact: addons read the unit's reaction from `GetStatusBarColor`.
    pub bar_colour: [f32; 3],
    pub name: String,
    /// The level to show, or `None` before `UNIT_FIELD_LEVEL` arrives: an empty seat, not a skull.
    pub level: Option<u32>,
    /// Show the skull in the level's seat instead of a number.
    pub skull: bool,
    pub level_colour: [f32; 3],
    /// The raid-target mark, 0-based, or `None`.
    pub raid_icon: Option<u8>,
    /// The plate's alpha: exactly 1.0 for the target, dimmed otherwise. Addons find the target
    /// plate by `GetAlpha() == 1`.
    pub alpha: f32,
    /// Mouseover or target: the bar brightens.
    pub lit: bool,
    /// The pointer is on the plate: the name goes yellow (`0x7cb850`) and the glow region shows.
    pub hovered: bool,
    /// The unit's template rank is elite or rare elite: the elite icon shows (`0x98e6e0`).
    pub elite: bool,
    /// The cast bar, for the target only (`0x720e50`); `None` hides it (`0x715b20`).
    pub cast: Option<PlateCast>,
    /// The threat flash's colour, `None` hidden (`0x98eaef`): the app's threat warning decides.
    pub threat: Option<[f32; 3]>,
}

/// The cast bar's state, `0x98f040`'s arguments: the bar runs from 0 to `max` seconds at `value`,
/// filling for a cast and draining for a channel (the app supplies the value either way).
#[derive(Clone, Debug, PartialEq)]
pub struct PlateCast {
    pub max: f32,
    pub value: f32,
    /// The spell's icon path, the spell-icon region's art.
    pub icon: Option<String>,
    pub channel: bool,
    /// The shield shows in the border's place (`0x98f040`'s last argument, `0x71ab20`).
    pub not_interruptible: bool,
}

/// One pooled plate's widgets, in the reference's creation order.
#[derive(Debug)]
struct Plate {
    frame: FrameHandle,
    flash: RegionHandle,
    border: RegionHandle,
    cast_border: RegionHandle,
    cast_shield: RegionHandle,
    spell_icon: RegionHandle,
    glow: RegionHandle,
    name: RegionHandle,
    level: RegionHandle,
    skull: RegionHandle,
    raid: RegionHandle,
    elite: RegionHandle,
    bar: FrameHandle,
    fill: RegionHandle,
    cast_bar: FrameHandle,
    cast_fill: RegionHandle,
    /// Last frame's state, so [`Plate::drive`] writes only what moved; `None` while retired.
    last: Option<PlateState>,
    /// The geometry its anchors were written under, `None` before the first lay-out. Kept per
    /// plate, so a plate the pool grows later is laid out too.
    laid_out: Option<PlateGeometry>,
}

impl Plate {
    fn key(&self) -> Option<u64> {
        self.last.as_ref().map(|s| s.key)
    }
}

/// The plate pool, grown on demand and never shrunk or reordered: addons scan only the children
/// past the count they last saw, so a compacted list would hide every later plate from them.
#[derive(Debug, Default)]
pub(crate) struct NamePlates {
    plates: Vec<Plate>,
    /// Each live unit's pool slot, by [`PlateState::key`]: the reference's `[unit+0xe60]`.
    assigned: std::collections::HashMap<u64, usize>,
    /// Frame to pool slot, never removed: plates are never destroyed.
    by_frame: std::collections::HashMap<FrameHandle, usize>,
    /// Completed clicks, drained by [`UiScript::take_nameplate_clicks`].
    clicks: Vec<NamePlateClick>,
    /// The hit-test veto (`0x7cba30`). Not the mouse-enabled bit: it leaves `IsMouseEnabled()`
    /// true, where freelook (`0x60f830`) clears the bit.
    hit_test_vetoed: bool,
    geometry: Option<PlateGeometry>,
}

impl NamePlates {
    /// Whether the `0x7cba30` veto refuses `frame`: only a plate, only while the veto stands.
    pub(crate) fn vetoes(&self, frame: FrameHandle) -> bool {
        self.hit_test_vetoed && self.by_frame.contains_key(&frame)
    }
}

/// A completed click on a plate. The reference's click slot (`0x7cb910`) selects the unit on the
/// left button (`0x4925d0`) and selects and interacts on the right (`0x492820`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePlateClick {
    /// The unit whose plate was clicked ([`PlateState::key`]).
    pub key: u64,
    /// `"LeftButton"` or `"RightButton"`.
    pub button: String,
}

/// What a sync fires outside the model borrow: `OnShow`/`OnHide` for frames whose visibility
/// flipped and `OnValueChanged` for moved bars (pfUI hooks it, `nameplates.lua:393`).
#[derive(Default)]
struct SyncEffects {
    visibility: Vec<FrameHandle>,
    values: Vec<(FrameHandle, f32)>,
}

impl UiScript {
    /// Drive the pool from the app's per-frame states, one per shown plate; a unit with no state
    /// has its plate retired (hidden, never destroyed). An unchanged `geometry` costs nothing.
    pub fn sync_nameplates(&mut self, geometry: PlateGeometry, states: &[PlateState]) {
        let lua = self.lua();
        let effects = {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // No WorldFrame (no interface loaded), no plates: no addon walks an invented root.
            let Some(world) = model.arena.lookup("WorldFrame") else {
                return;
            };
            sync(&mut model, world, geometry, states)
        };
        super::event::fire_visibility_changes(lua, effects.visibility);
        for (bar, value) in effects.values {
            super::statusbar::fire_engine_value_changed(lua, bar, value);
        }
    }

    /// The unit whose plate is the hovered frame, the mouseover its OnEnter publishes (`0x7cb850`
    /// → `0x492890`, `[0xb4e2c8]`); an addon frame on a plate takes the hover from it.
    pub fn hovered_nameplate(&self) -> Option<u64> {
        let lua = self.lua();
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let frame = model.mouseover?;
        let slot = *model.nameplates.by_frame.get(&frame)?;
        model.nameplates.plates.get(slot)?.key()
    }

    /// Drain the completed plate clicks, physical or `plate:Click()`, for the app to select from.
    pub fn take_nameplate_clicks(&mut self) -> Vec<NamePlateClick> {
        let lua = self.lua();
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        std::mem::take(&mut model.nameplates.clicks)
    }

    /// The freelook toggle (`0x60f830`): entering camera freelook disables mouse input on every
    /// plate and leaving re-enables it (called from `0x483e80` and `0x483e70`).
    pub fn set_nameplate_mouse(&mut self, enabled: bool) {
        let lua = self.lua();
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let frames: Vec<FrameHandle> = model.nameplates.plates.iter().map(|p| p.frame).collect();
        for frame in frames {
            model.arena.set_mouse_enabled(frame, enabled);
        }
    }

    /// The plate hit-test veto (`0x7cba30`): while a ground-targeted spell is armed, clicks fall
    /// through plates to the `WorldFrame`. The caller supplies the first two terms of the
    /// reference's `IsTargeting() && (flag & 0x60) && !0x6e6180()`; the third, a test of the same
    /// word against `0x878e` whose meaning is unsettled, is not applied.
    pub fn set_nameplate_hit_test_veto(&mut self, vetoed: bool) {
        let lua = self.lua();
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        model.nameplates.hit_test_vetoed = vetoed;
    }

    /// Retire every live plate (`0x608a10` over the active list, as when the master toggle
    /// clears); the app's every early return owes this, since a widget stays up until hidden.
    pub fn retire_nameplates(&mut self) {
        let lua = self.lua();
        let effects = {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let mut effects = SyncEffects::default();
            model.nameplates.assigned.clear();
            for i in 0..model.nameplates.plates.len() {
                Plate::retire(&mut model, i, &mut effects);
            }
            effects
        };
        super::event::fire_visibility_changes(lua, effects.visibility);
    }

    /// How many plate widgets the pool holds.
    pub fn nameplate_pool_len(&self) -> usize {
        let lua = self.lua();
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        model.nameplates.plates.len()
    }
}

/// Record a click on `id` if it is a live plate; every button click comes here.
pub(super) fn note_click(lua: &mlua::Lua, id: u32, button: &str) {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let Some(frame) = model.id_to_frame.get(&id).copied() else {
        return;
    };
    let Some(&slot) = model.nameplates.by_frame.get(&frame) else {
        return;
    };
    let Some(key) = model.nameplates.plates.get(slot).and_then(Plate::key) else {
        return;
    };
    model.nameplates.clicks.push(NamePlateClick {
        key,
        button: button.to_string(),
    });
}

/// The per-frame drive: retire departed units, bind the rest, lay out, write what changed.
fn sync(
    model: &mut Model,
    world: FrameHandle,
    geometry: PlateGeometry,
    states: &[PlateState],
) -> SyncEffects {
    let mut effects = SyncEffects::default();
    let relayout = model.nameplates.geometry != Some(geometry);
    model.nameplates.geometry = Some(geometry);

    // Retire first, so a departing unit's slot is free for an arriving one, as the reference
    // destroys then creates through one free list in the same tick.
    let gone: Vec<u64> = model
        .nameplates
        .assigned
        .keys()
        .copied()
        .filter(|k| !states.iter().any(|s| s.key == *k))
        .collect();
    for key in gone {
        if let Some(i) = model.nameplates.assigned.remove(&key) {
            Plate::retire(model, i, &mut effects);
        }
    }

    for state in states {
        let slot = match model.nameplates.assigned.get(&state.key) {
            Some(&i) => i,
            None => {
                // The lowest free slot, else a new one at the tail. Deviation: the reference's
                // free list is FIFO; either keeps every index stable, all an addon can observe.
                let free = (0..model.nameplates.plates.len())
                    .find(|i| model.nameplates.plates[*i].last.is_none());
                let i = free.unwrap_or_else(|| {
                    let plate = Plate::create(model, world, &mut effects);
                    model.nameplates.plates.push(plate);
                    model.nameplates.plates.len() - 1
                });
                model.nameplates.assigned.insert(state.key, i);
                i
            }
        };
        if model.nameplates.plates[slot].laid_out != Some(geometry) {
            Plate::lay_out(model, slot, geometry);
        }
        Plate::drive(model, slot, world, state, &mut effects);
    }

    // A geometry change reaches retired plates too: addons still read them in the child list.
    if relayout {
        for i in 0..model.nameplates.plates.len() {
            if model.nameplates.plates[i].laid_out != Some(geometry) {
                Plate::lay_out(model, i, geometry);
            }
        }
    }
    effects
}

impl Plate {
    /// Build one plate: the Button, its six regions in the reference's order, and the health bar.
    /// Written once, so a region an addon blanks stays blank.
    fn create(model: &mut Model, world: FrameHandle, effects: &mut SyncEffects) -> Plate {
        // Anonymous, as the reference's name is zeroed (`0x76c50b`): plate addons reject a named
        // `WorldFrame` child.
        let frame = model.arena.create(FrameKind::Button, None, Some(world));
        let strata = model
            .arena
            .frame(world)
            .map(|f| f.strata)
            .unwrap_or_default();
        model.arena.set_frame_strata(frame, strata);
        model.arena.set_frame_level(frame, PLATE_LEVEL, true);
        // Mouse-enabled from birth by `CSimpleButton`'s ctor (`0x7786a3`), and registered for
        // both buttons on the up edge (`RegisterForClicks(0x500)`, `0x7cb637`); the arena's
        // Button default is the left alone.
        if let Some(KindState::Button(bs)) = model.arena.frame_mut(frame).map(|f| &mut f.kind_state)
        {
            bs.registered_clicks = ["LeftButtonUp", "RightButtonUp"]
                .into_iter()
                .map(str::to_string)
                .collect();
        }

        // The ctor's order (`0x98f790`) is the ABI: `GetRegions()` and `GetChildren()` return
        // creation order. Everything is BLEND but the highlight, ADD (`0x481620(3)`), which
        // [`is_unpainted_glow`] depends on.
        let flash = texture(
            model,
            frame,
            DrawLayer::Background,
            FLASH_TEXTURE,
            BlendMode::Blend,
        );
        let border = texture(
            model,
            frame,
            DrawLayer::Artwork,
            BORDER_TEXTURE,
            BlendMode::Blend,
        );
        let (bar, fill) = status_bar(model, frame, strata);
        let cast_border = texture(
            model,
            frame,
            DrawLayer::Artwork,
            BORDER_TEXTURE,
            BlendMode::Blend,
        );
        // The same art turned upside down (`0x4831d0` with `(0, 1, 1, 0)`), so the cast bar's
        // border hangs from the health border.
        model.region_data.entry(cast_border).or_default().tex_coords =
            Some(TexCoords::Rect([0.0, 1.0, 1.0, 0.0]));
        let cast_shield = texture(
            model,
            frame,
            DrawLayer::Artwork,
            CAST_SHIELD_TEXTURE,
            BlendMode::Blend,
        );
        let (cast_bar, cast_fill) = status_bar(model, frame, strata);
        let spell_icon = model
            .arena
            .create_region(frame, RegionKind::Texture, DrawLayer::Overlay, 0)
            .expect("live plate");
        let glow = texture(
            model,
            frame,
            DrawLayer::Highlight,
            GLOW_TEXTURE,
            BlendMode::Add,
        );
        let name = font_string(model, frame, JustifyV::Bottom);
        let level = font_string(model, frame, JustifyV::Middle);
        let skull = texture(
            model,
            frame,
            DrawLayer::Overlay,
            SKULL_TEXTURE,
            BlendMode::Blend,
        );
        let raid = texture(
            model,
            frame,
            DrawLayer::Artwork,
            RAID_ICON_TEXTURE,
            BlendMode::Blend,
        );
        let elite = texture(
            model,
            frame,
            DrawLayer::Border,
            ELITE_TEXTURE,
            BlendMode::Blend,
        );

        for rh in [
            flash,
            cast_border,
            cast_shield,
            spell_icon,
            glow,
            skull,
            raid,
            elite,
        ] {
            model.region_data.entry(rh).or_default().hidden = true;
        }
        // The cast bar is hidden until the target casts (`0x98e7e0` hides it).
        effects
            .visibility
            .extend(model.arena.set_shown(cast_bar, false));

        // Born retired and hidden: addons tell a live plate from a pooled one by `IsShown()`.
        effects
            .visibility
            .extend(model.arena.set_shown(frame, false));

        model
            .nameplates
            .by_frame
            .insert(frame, model.nameplates.plates.len());

        Plate {
            frame,
            flash,
            border,
            cast_border,
            cast_shield,
            spell_icon,
            glow,
            name,
            level,
            skull,
            raid,
            elite,
            bar,
            fill,
            cast_bar,
            cast_fill,
            last: None,
            laid_out: None,
        }
    }

    /// Write the window-derived sizes and anchors, on the first drive and a geometry change. The
    /// reference's are real anchors too, through `SetPoint`'s own store (`0x767c70`).
    fn lay_out(model: &mut Model, i: usize, g: PlateGeometry) {
        let p = &model.nameplates.plates[i];
        let (frame, bar, cast_bar) = (p.frame, p.bar, p.cast_bar);
        let (border, glow, name, level, skull, raid) =
            (p.border, p.glow, p.name, p.level, p.skull, p.raid);
        let (flash, cast_border, cast_shield, spell_icon, elite) =
            (p.flash, p.cast_border, p.cast_shield, p.spell_icon, p.elite);
        let plate_id = model.frame_id(frame);

        // The plate's size; its position is per-frame, in `drive`.
        let input = model.layout_inputs.entry(frame).or_default();
        input.width = g.width;
        input.height = g.height;

        for rh in [flash, border, glow] {
            model.region_data.entry(rh).or_default().anchors = vec![
                Anchor::new(Point::TopLeft, plate_id, Point::TopLeft, 0.0, 0.0),
                Anchor::new(Point::BottomRight, plate_id, Point::BottomRight, 0.0, 0.0),
            ];
        }

        let bar_input = model.layout_inputs.entry(bar).or_default();
        bar_input.anchors = vec![Anchor::new(
            Point::BottomLeft,
            plate_id,
            Point::BottomLeft,
            g.bar_off_x,
            g.bar_off_y,
        )];
        bar_input.width = g.bar_width;
        bar_input.height = g.bar_height;

        // The name's (`0x7cb456`) and level's (`0x7cb7f5`) anchors, which `_Nameplates` checks.
        let name_data = model.region_data.entry(name).or_default();
        name_data.anchors = vec![Anchor::new(
            Point::Bottom,
            plate_id,
            Point::Center,
            0.0,
            0.0,
        )];
        name_data.font_height = Some(g.name_height);

        for rh in [name, level] {
            model.region_data.entry(rh).or_default().font_shadow = Some(FontShadow {
                offset: [g.shadow_offset, -g.shadow_offset],
                color: [0.0, 0.0, 0.0, 1.0],
            });
        }

        let level_data = model.region_data.entry(level).or_default();
        level_data.anchors = vec![Anchor::new(
            Point::Center,
            plate_id,
            Point::BottomRight,
            -g.level_off_x,
            g.level_off_y,
        )];
        level_data.font_height = Some(g.level_height);
        level_data.font_explicit.height = true;
        model.touch_measure(level);

        let skull_data = model.region_data.entry(skull).or_default();
        skull_data.anchors = vec![Anchor::new(
            Point::Center,
            plate_id,
            Point::BottomRight,
            -g.level_off_x,
            g.level_off_y,
        )];
        skull_data.size = Some((g.skull_size, g.skull_size));

        let raid_data = model.region_data.entry(raid).or_default();
        raid_data.anchors = vec![Anchor::new(Point::Right, plate_id, Point::Left, 0.0, 0.0)];
        raid_data.size = Some((g.raid_size, g.raid_size));

        // The elite dragon over the level's seat (`0x98ff12`, anchored to the level text).
        let level_id = model.region_id(level);
        let elite_data = model.region_data.entry(elite).or_default();
        elite_data.anchors = vec![Anchor::new(
            Point::Center,
            level_id,
            Point::Center,
            0.0,
            0.0,
        )];
        elite_data.size = Some((g.skull_size * 2.0, g.skull_size * 2.0));

        // The cast bar under the health border: the health bar's size, its border the health
        // border turned over, so the bar sits as far from the border's top as the health bar from
        // its bottom. Unresolved, labelled: the attach (`0x98f390`) scales these by runtime
        // globals the decompile leaves unread, so the seat is the mirror image rather than the
        // reference's own offsets, and the spell icon's size is the border's height.
        let cast_input = model.layout_inputs.entry(cast_bar).or_default();
        cast_input.anchors = vec![Anchor::new(
            Point::TopLeft,
            plate_id,
            Point::BottomLeft,
            g.bar_off_x,
            -g.bar_off_y,
        )];
        cast_input.width = g.bar_width;
        cast_input.height = g.bar_height;
        let cast_id = model.frame_id(cast_bar);
        for rh in [cast_border, cast_shield] {
            model.region_data.entry(rh).or_default().anchors = vec![
                Anchor::new(
                    Point::TopLeft,
                    cast_id,
                    Point::TopLeft,
                    -g.bar_off_x,
                    g.bar_off_y,
                ),
                Anchor::new(
                    Point::BottomRight,
                    cast_id,
                    Point::BottomRight,
                    g.width - g.bar_off_x - g.bar_width,
                    -(g.height - g.bar_off_y - g.bar_height),
                ),
            ];
        }
        let border_id = model.region_id(cast_border);
        let icon_data = model.region_data.entry(spell_icon).or_default();
        icon_data.anchors = vec![Anchor::new(Point::Right, border_id, Point::Left, 0.0, 0.0)];
        icon_data.size = Some((g.height, g.height));

        // The first lay-out adds anchor targets, so the full touch; once per geometry change.
        model.touch_layout();
        model.nameplates.plates[i].laid_out = Some(g);
    }

    /// The per-frame write, each property only when it moved.
    fn drive(
        model: &mut Model,
        i: usize,
        world: FrameHandle,
        state: &PlateState,
        effects: &mut SyncEffects,
    ) {
        let p = &model.nameplates.plates[i];
        let (frame, bar, cast_bar) = (p.frame, p.bar, p.cast_bar);
        let (glow, name, level, skull, raid, fill) =
            (p.glow, p.name, p.level, p.skull, p.raid, p.fill);
        let (cast_border, cast_shield, spell_icon, elite, cast_fill) = (
            p.cast_border,
            p.cast_shield,
            p.spell_icon,
            p.elite,
            p.cast_fill,
        );
        let flash = p.flash;
        let last = p.last.clone();
        let world_id = model.frame_id(world);

        // Re-seated whenever it moves, as the reference does (`0x509ec0`). A moved offset keeps
        // the graph's edges, so the precise touch; the first seat adds an edge, so the retarget.
        if last
            .as_ref()
            .is_none_or(|l| l.top_centre != state.top_centre)
        {
            let fresh = model
                .layout_inputs
                .get(&frame)
                .is_none_or(|i| i.anchors.is_empty());
            let input = model.layout_inputs.entry(frame).or_default();
            input.anchors = vec![Anchor::new(
                Point::Top,
                world_id,
                Point::BottomLeft,
                state.top_centre.0,
                state.top_centre.1,
            )];
            if fresh {
                model.touch_layout_retarget_frame(frame, &[], &[world_id]);
            } else {
                model.touch_layout_frame(frame);
            }
        }

        if last.is_none() {
            effects
                .visibility
                .extend(model.arena.set_shown(frame, true));
        }
        if last.as_ref().is_none_or(|l| l.alpha != state.alpha) {
            model.arena.set_alpha(frame, f64::from(state.alpha));
        }

        // Raw health on the bar; the fill's reaction colour brightens while lit (`LIT_BOOST`).
        if last
            .as_ref()
            .is_none_or(|l| l.health != state.health || l.max_health != state.max_health)
        {
            if let Some(KindState::StatusBar(sb)) =
                model.arena.frame_mut(bar).map(|f| &mut f.kind_state)
            {
                sb.min = 0.0;
                sb.max = state.max_health;
                sb.value = state.health.clamp(0.0, state.max_health);
            }
            effects.values.push((bar, state.health));
        }
        if last
            .as_ref()
            .is_none_or(|l| l.bar_colour != state.bar_colour || l.lit != state.lit)
        {
            let boost = if state.lit { LIT_BOOST } else { 1.0 };
            let c = state.bar_colour;
            model.region_data.entry(fill).or_default().vertex_color =
                Some([c[0] * boost, c[1] * boost, c[2] * boost, 1.0]);
        }

        if last.as_ref().is_none_or(|l| l.name != state.name) {
            model.region_data.entry(name).or_default().text = Some(state.name.clone());
            model.touch_measure(name);
            model.touch_layout_region(name);
        }
        if last.as_ref().is_none_or(|l| l.hovered != state.hovered) {
            let c = if state.hovered {
                [1.0, 1.0, 0.0, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            model.region_data.entry(name).or_default().vertex_color = Some(c);
            // The glow really shows, as addons read mouseover from it; it only paints nothing.
            model.region_data.entry(glow).or_default().hidden = !state.hovered;
        }

        if last.as_ref().is_none_or(|l| {
            l.level != state.level || l.skull != state.skull || l.level_colour != state.level_colour
        }) {
            match (state.level, state.skull) {
                // The skull flag wins the seat, whatever the level.
                (_, true) => {
                    model.region_data.entry(level).or_default().hidden = true;
                    model.region_data.entry(skull).or_default().hidden = false;
                }
                (Some(n), false) => {
                    let c = state.level_colour;
                    let data = model.region_data.entry(level).or_default();
                    data.text = Some(n.to_string());
                    data.vertex_color = Some([c[0], c[1], c[2], 1.0]);
                    data.hidden = false;
                    model.region_data.entry(skull).or_default().hidden = true;
                    model.touch_measure(level);
                    model.touch_layout_region(level);
                }
                // No level yet and no skull: the seat stays empty.
                (None, false) => {
                    model.region_data.entry(level).or_default().hidden = true;
                    model.region_data.entry(skull).or_default().hidden = true;
                }
            }
        }

        if last.as_ref().is_none_or(|l| l.raid_icon != state.raid_icon) {
            match state.raid_icon {
                Some(idx) => {
                    let (u0, v0) = (f32::from(idx & 3) * 0.25, f32::from(idx >> 2) * 0.25);
                    let data = model.region_data.entry(raid).or_default();
                    data.tex_coords = Some(TexCoords::Rect([u0, u0 + 0.25, v0, v0 + 0.25]));
                    data.hidden = false;
                }
                None => model.region_data.entry(raid).or_default().hidden = true,
            }
        }

        if last.as_ref().is_none_or(|l| l.threat != state.threat) {
            let data = model.region_data.entry(flash).or_default();
            data.hidden = state.threat.is_none();
            if let Some(c) = state.threat {
                data.vertex_color = Some([c[0], c[1], c[2], 1.0]);
            }
        }

        if last.as_ref().is_none_or(|l| l.elite != state.elite) {
            model.region_data.entry(elite).or_default().hidden = !state.elite;
        }

        if last.as_ref().is_none_or(|l| l.cast != state.cast) {
            match &state.cast {
                Some(cast) => {
                    if let Some(KindState::StatusBar(sb)) =
                        model.arena.frame_mut(cast_bar).map(|f| &mut f.kind_state)
                    {
                        sb.min = 0.0;
                        sb.max = cast.max;
                        sb.value = cast.value.clamp(0.0, cast.max);
                    }
                    effects.values.push((cast_bar, cast.value));
                    let c = CAST_COLOUR;
                    model.region_data.entry(cast_fill).or_default().vertex_color =
                        Some([c[0], c[1], c[2], 1.0]);
                    // The border, or the shield in its place (`0x98f040`'s last argument).
                    model.region_data.entry(cast_border).or_default().hidden =
                        cast.not_interruptible;
                    model.region_data.entry(cast_shield).or_default().hidden =
                        !cast.not_interruptible;
                    let icon = model.region_data.entry(spell_icon).or_default();
                    icon.texture = cast.icon.clone();
                    icon.hidden = false;
                    if last.as_ref().is_none_or(|l| l.cast.is_none()) {
                        effects
                            .visibility
                            .extend(model.arena.set_shown(cast_bar, true));
                    }
                }
                None => {
                    for rh in [cast_border, cast_shield, spell_icon] {
                        model.region_data.entry(rh).or_default().hidden = true;
                    }
                    effects
                        .visibility
                        .extend(model.arena.set_shown(cast_bar, false));
                }
            }
        }

        model.nameplates.plates[i].last = Some(state.clone());
    }

    /// Retire a plate: hide it, never destroy it (`0x608a10`), so it keeps its child-list index and
    /// comes back as the same Lua object addons key their per-plate state by.
    fn retire(model: &mut Model, i: usize, effects: &mut SyncEffects) {
        if model.nameplates.plates[i].last.is_none() {
            return;
        }
        let frame = model.nameplates.plates[i].frame;
        effects
            .visibility
            .extend(model.arena.set_shown(frame, false));
        model.nameplates.plates[i].last = None;
    }
}

/// Whether a text's owner is a plate. A plate's rect snaps to device pixels as it slides, and its
/// text must follow that rect, not the UI's coarser grid, or it jumps. Asked of the frame so an
/// addon's strings on the plate get it too; a string on the plate's health bar does not.
pub(super) fn is_world_seated(model: &Model, frame: FrameHandle) -> bool {
    model.nameplates.by_frame.contains_key(&frame)
}

/// Whether this is the plate's own glow art on its ADD mode, which the quad walk
/// ([`super::extract`]) skips. Deviation: the reference adds this rim over the bar; we brighten
/// the bar ([`LIT_BOOST`]), because the rim reads as hard edge lines in our linear-blending
/// pipeline. The art has no alpha and is mostly black, so as BLEND it would be an opaque box.
pub(super) fn is_unpainted_glow(data: &RegionData) -> bool {
    data.blend == BlendMode::Add && data.texture.as_deref() == Some(GLOW_TEXTURE)
}

/// The lit bar's brighten, a multiply of the fill tint in encoded sRGB like the client's modulate:
/// 255/215 takes the fill texture's encoded peak exactly to white, so no gradient row clips.
const LIT_BOOST: f32 = 255.0 / 215.0;
