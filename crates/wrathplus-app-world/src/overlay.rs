//! The world-layer overlay helpers: what world systems that draw over the 3D scene (floating
//! combat text, chat bubbles, V-plates, the minimap composite, the pipeline warm-up) share with
//! the player-UI pass. `ui_pass` re-exports them.
//!
//! The quad pass's primitives live here too: [`UiQuad`], [`UiQuads`] and [`UiQuadMaterial`], which
//! world drawing (the minimap, bubbles, floating text) emits and `ui_pass` renders.

use std::collections::HashMap;

use bevy::camera::visibility::RenderLayers;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dKey};
use wrathplus_ui::widget::{FrameHandle, ModelFog, ModelLight};

/// The append lane's z bands, far below the scripted UI's keys (a [`wrathplus_ui::order::ZKey`]
/// region sets `1 << 20`), so the whole lane paints under the player UI.
pub(crate) mod overlay_z {
    /// Floating combat, XP and honor numbers, drawn by the client in the world scene under all UI.
    pub(crate) const WORLD_TEXT: u64 = 0;
    /// The chat bubbles: one frame level per live bubble, farthest from the camera first.
    pub(crate) const BUBBLE: u64 = 1 << 8;
    /// Keys per bubble level: its bg, edge, tail and text.
    pub(crate) const BUBBLE_STRIDE: u64 = 4;
    /// The V-plates, over every bubble; a unit never shows both, so this orders only across units.
    pub(crate) const VPLATE: u64 = 1 << 16;
    /// The highest bubble level below [`VPLATE`], a guard rail far past any real count (a bubble
    /// needs a speaker within 20 yd).
    pub(crate) const BUBBLE_MAX_LEVEL: u64 = (VPLATE - BUBBLE) / BUBBLE_STRIDE - 1;
}

/// The render layer this pass's camera and batches share, disjoint from the world camera's 0.
const UI_RENDER_LAYER: usize = 1;

pub fn ui_render_layers() -> RenderLayers {
    RenderLayers::layer(UI_RENDER_LAYER)
}

/// The mesh every `UiQuadMaterial` quad outside the batches draws with: a 1×1 rectangle placed by
/// its Transform. Its layout (POSITION + NORMAL + UV_0) keys a second pipeline beside the batch
/// mesh's (POSITION + UV_0 + COLOR), which `pipe_warm` warms by drawing this same mesh.
pub(crate) fn tile_quad_mesh() -> Mesh {
    Rectangle::new(1.0, 1.0).into()
}

/// Project a world point for a world-anchored overlay, with the reference projector's verdict:
/// `None` does not draw. `0x483ee0` rejects a point behind the near plane (`0x483f7a`) or outside
/// the WorldFrame's region (`[0xb4b2bc]`; `0x484075`, `0x484086`, `0x484096`, `0x4840a6`), which
/// `0x483970` mirrors ÷G44/÷G48 so the aspect cancels: exactly the viewport, inclusive. A viewport
/// rejection has already written a plausible out-param (`0x484065`, `0x484067`), so the verdict
/// must be honoured; plate and world-text callers destroy what they were placing. The chat bubble
/// does not call this: its seat ignores the verdict, and a bubble slides off with its speaker.
pub(crate) fn project_overlay(
    cam: &Camera,
    cam_tf: &GlobalTransform,
    world: Vec3,
    viewport: Vec2,
) -> Option<Vec2> {
    // Bevy's `Err` is the near-plane half (`0x483f7a`); `accepts` is the viewport half.
    let p = cam.world_to_viewport(cam_tf, world).ok()?;
    accepts(p, viewport).then_some(p)
}

fn accepts(p: Vec2, viewport: Vec2) -> bool {
    (0.0..=viewport.x).contains(&p.x) && (0.0..=viewport.y).contains(&p.y)
}

/// A material's `uv_clamp` with both axes disabled (`min > max`).
pub const UV_CLAMP_OFF: Vec4 = Vec4::new(1.0, 1.0, 0.0, 0.0);

/// One `(u, v)` per screen corner, in `ui_pass`'s `Run::push_quad`'s winding (top-left, top-right,
/// bottom-right, bottom-left), not a `(min, max)` rect: a `<TexCoords>` with `left > right` (the
/// PlayerFrame ring) must stay mirrored, and a backdrop top or bottom edge maps atlas u to screen
/// Y, which only per-corner UVs express.
#[derive(Clone, Copy, PartialEq)]
pub struct UvRect {
    pub corners: [[f32; 2]; 4],
}

impl UvRect {
    /// The full texture, no crop.
    pub const FULL: Self = Self {
        corners: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    };

    /// From a `<TexCoords>` `[left, right, top, bottom]` tuple, unnormalized, so a mirror survives.
    pub fn from_tex_coords([left, right, top, bottom]: [f32; 4]) -> Self {
        Self {
            corners: [[left, top], [right, top], [right, bottom], [left, bottom]],
        }
    }

    /// From explicit corners in the `push_quad` winding: the backdrop's rotated edge slices.
    pub fn from_corners(corners: [[f32; 2]; 4]) -> Self {
        Self { corners }
    }

    /// From an already-normalized [`Rect`], such as a glyph atlas cell: `min` to TL, `max` to BR.
    pub fn from_rect(r: Rect) -> Self {
        Self::from_tex_coords([r.min.x, r.max.x, r.min.y, r.max.y])
    }
}

/// One resolved screen-space quad, the pass's whole input. `texture`, the flags from `additive`
/// to `uv_clamp`, and `mask` are material state: a quad differing in any of them starts a new run.
#[derive(Clone, PartialEq)]
pub struct UiQuad {
    /// Screen pixels, y-down, anchor-resolved.
    pub rect: Rect,
    /// Paint order, stable-sorted ascending: higher draws on top.
    pub z_key: u64,
    /// `None` samples the shared 1×1 white.
    pub texture: Option<Handle<Image>>,
    pub uv: UvRect,
    /// Straight-alpha tint in client-space sRGB, the raw FrameXML/Lua value, never converted.
    pub color: [f32; 4],
    /// WoW `ADD` blend (EGxBlend 3, `glBlendFunc(GL_SRC_ALPHA, GL_ONE)`) instead of straight alpha.
    pub additive: bool,
    /// Mask to the inscribed circle: the live unit portrait.
    pub circular: bool,
    /// Draw the texel's luminance (`Texture:SetDesaturated(1)`), folded in gamma bytes before the
    /// tint, so `SetItemButtonDesaturated(button, 1, 0.65, 0.65, 0.65)`
    /// (`Blizzard_TalentUI.lua:215`) lands grey and dim.
    pub desaturated: bool,
    /// The texture is already premultiplied: a booth render target ([`crate::portrait`]), where
    /// an additive particle adds colour with no coverage and a second alpha weight would erase it.
    pub premultiplied: bool,
    /// The texture holds undecoded gamma bytes (`BlpVariant::MapTile`: the reference's tile
    /// sampler sets `GL_TEXTURE_SRGB_DECODE_EXT = GL_SKIP_DECODE_EXT`), so the tint multiplies the
    /// authored byte and the ordinary arm's `linear_to_srgb` must not encode it again.
    pub gamma_texel: bool,
    /// Alpha test against this value on `texel.a × tint.a` instead of blending: a pass writes
    /// opaque, a fail discards. `Some(224.0 / 255.0)` is the WMO-interior minimap tile, drawn by
    /// the reference with EGxBlend 1 (blending off) and `glAlphaFunc(GL_GEQUAL, 0.87843144)`
    /// (`0x85ad20[1] = 224`); blended, two tiles' seam keeps `(1−a)(1−b)` of the black clear.
    pub alpha_test: Option<f32>,
    /// The half-texel-inset window `[u_min, v_min, u_max, v_max]` sampled, so an atlas cell clamps
    /// at its own edge instead of filtering in its neighbour (`ClampToEdge` stops at the image's).
    /// Per axis: UVs past `[0, 1]` are the reference's tiling idiom and never clamp.
    pub uv_clamp: Option<[f32; 4]>,
    /// CPU clip standing in for a scissor rect.
    pub clip: Option<Rect>,
    /// Radians clockwise on screen about the rect centre (the minimap player arrow), the UVs
    /// riding their corners; `clip` applies first, unrotated.
    pub rotation: f32,
    /// A screen-anchored alpha mask (the minimap's `MinimapMask.blp` circle), so a tile panning
    /// under the fixed circle masks correctly; outside [`UiQuadMask::rect`] the fragment drops.
    pub mask: Option<UiQuadMask>,
    /// Explicit screen corners (y-down px) drawn as the fan `(0,1,2)(0,2,3)` (the cooldown pie's
    /// wedge), bypassing `rotation`, `clip` and `rect`, which should still bound them.
    pub corners: Option<[Vec2; 4]>,
}

/// A screen-anchored alpha mask over a [`UiQuad`].
#[derive(Clone, PartialEq)]
pub struct UiQuadMask {
    /// The mask art, whose alpha is the coverage (`MinimapMask.blp`: DXT3, the circle in alpha).
    pub texture: Handle<Image>,
    /// The span it covers, in [`UiQuad::rect`]'s y-down logical px; it samples 0..1 across it.
    pub rect: Rect,
}

impl Default for UiQuad {
    fn default() -> Self {
        Self {
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            z_key: 0,
            texture: None,
            uv: UvRect::FULL,
            color: [1.0, 1.0, 1.0, 1.0],
            additive: false,
            circular: false,
            desaturated: false,
            premultiplied: false,
            gamma_texel: false,
            alpha_test: None,
            uv_clamp: None,
            clip: None,
            rotation: 0.0,
            mask: None,
            corners: None,
        }
    }
}

/// The pass's input in two lanes. [`Self::quads`], the base lane, is replaced whole by the script
/// extract, which sets [`Self::dirty`] only when it changed. [`Self::overlays`], the append lane
/// ([`crate::app_core::schedule::UiQuadAppend`]), is cleared and re-emitted every frame and
/// diffed by `ui_pass::rebuild_ui_mesh` against the last frame's; its appenders never touch
/// `dirty`.
#[derive(Resource, Default)]
pub struct UiQuads {
    pub quads: Vec<UiQuad>,
    pub overlays: Vec<UiQuad>,
    /// The append lane as of the last rebuild.
    pub last_overlays: Vec<UiQuad>,
    pub dirty: bool,
}

/// Marker on the player-UI camera, whose backdrop [`crate::world_backdrop`] points at the world
/// camera drawing this frame.
#[derive(Component)]
pub struct PlayerUiCamera;

/// The quad material: a texture and the [`UiQuad`] flags as uniforms, drawn through one
/// premultiplied-alpha pipeline (`ui_quad.wgsl`). It holds no colour, so it is its identity key
/// alone and a steady frame re-prepares none.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct UiQuadMaterial {
    #[uniform(0)]
    pub additive: u32,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    #[uniform(3)]
    pub circular: u32,
    #[uniform(7)]
    pub desaturate: u32,
    #[uniform(8)]
    pub premultiplied: u32,
    /// `<= 0` disables the alpha test.
    #[uniform(9)]
    pub alpha_ref: f32,
    #[uniform(10)]
    pub gamma_texel: u32,
    /// The mask span in physical px (the shader compares `@builtin(position)`); `z <= x` disables.
    #[uniform(4)]
    pub mask_rect: Vec4,
    #[texture(5)]
    #[sampler(6)]
    pub mask: Option<Handle<Image>>,
    /// `(u_min, v_min, u_max, v_max)`; `min > max` on an axis disables that axis.
    #[uniform(11)]
    pub uv_clamp: Vec4,
}

impl UiQuadMaterial {
    /// The WMO-interior minimap tile material: the texture and an alpha test, nothing else, for the
    /// minimap's own composite target ([`crate::minimap`]). A passing fragment emits `a = 1`, so
    /// the forced blend of [`Self::specialize`] is a plain replace: the reference's blending off.
    pub(crate) fn interior_tile(texture: Handle<Image>, alpha_ref: f32) -> Self {
        Self {
            additive: 0,
            texture: Some(texture),
            circular: 0,
            desaturate: 0,
            premultiplied: 0,
            alpha_ref,
            // The alpha-test arm decodes explicitly, so the tile's SKIP_DECODE holds without this.
            gamma_texel: 0,
            mask_rect: Vec4::new(0.0, 0.0, -1.0, -1.0),
            mask: None,
            uv_clamp: UV_CLAMP_OFF,
        }
    }
}

impl Material2d for UiQuadMaterial {
    /// Our own vertex stage, which unpacks the run's tint from the instance tag (`ui_pass::tint_tag`).
    fn vertex_shader() -> ShaderRef {
        crate::shaders::UI_QUAD.into()
    }

    fn fragment_shader() -> ShaderRef {
        crate::shaders::UI_QUAD.into()
    }

    fn alpha_mode(&self) -> bevy::sprite_render::AlphaMode2d {
        // The transparent phase, sorted by mesh z (the run order); `specialize` sets the blend.
        bevy::sprite_render::AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: Material2dKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            if let Some(target) = fragment.targets.first_mut().and_then(|t| t.as_mut()) {
                // `(One, OneMinusSrcAlpha)` over gamma values: the shader emits `(rgb·a, a)` for
                // BLEND and `(rgb·a, 0)` for ADD, so this one state is EGxBlend 2 and EGxBlend 3,
                // clamped at each write like the reference's 8-bit backbuffer.
                let premultiplied = BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                };
                target.blend = Some(BlendState {
                    color: premultiplied,
                    alpha: premultiplied,
                });
            }
        }
        Ok(())
    }
}

/// One pane's tile request, published by the extract's `ModelPane` arm: its device size, the unit
/// ladder `0x76d1a0` derives from it, and the Lua-set scene.
#[derive(Clone, Debug, PartialEq)]
pub struct TileRequest {
    /// The `SetModel` path, as written.
    pub path: String,
    /// The pane's rect in whole device pixels: the tile's cell size.
    pub size_px: UVec2,
    /// Device px per model unit: `1280 · modelScale · layoutScale` FrameXML units × seam × DPI.
    pub px_per_unit: f32,
    /// Device px per `SetPosition` unit: `768 · √(a²+1) · layoutScale` FrameXML units × seam × DPI.
    pub pos_px_per_unit: f32,
    /// Device px per model unit of a particle's half-extent, which carries no scale.
    pub star_px_per_unit: f32,
    /// `SetFacing`, radians about the screen normal (CCW positive, the reference's `+Z`).
    pub facing: f32,
    /// `SetPosition`, layout units.
    pub position: Vec3,
    /// The perspective leg's root scale, `G48 · (5/3) · modelScale · layoutScale`.
    pub root_scale: f32,
    /// The perspective leg's root translation: `SetPosition · layoutScale`, model units.
    pub root_pos: Vec3,
    /// The installed camera index (ctor 0, `SetCamera(n)`); `None` is the NULL camera an index
    /// past the file's cameras installs (`0x76cec0`), the orthographic leg of every shipped pane.
    pub camera: Option<u32>,
    /// The pane's embedded `CGLight`, disabled on a fresh `<Model>`, so a lit batch draws black.
    pub light: ModelLight,
    pub fog: Option<ModelFog>,
    /// `ReplaceIconTexture`'s path, the texture of the file's type-14 batches.
    pub icon: Option<String>,
    /// Where the cell composites: the pane's rect in the quad pass's space, y-down logical px.
    pub rect: Rect,
    /// `ZKey::callback(Artwork)`: after every texture and font string of the pane's layer.
    pub z_key: u64,
    /// The frame's effective alpha, which the composite draws at (12340 `0x95fb40`).
    pub alpha: f32,
    /// The enclosing ScrollFrame's clip, in the quad pass's space.
    pub clip: Option<Rect>,
}

/// Where a tile sits in the atlas, in texels, `y` down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub origin: UVec2,
    pub size: UVec2,
}

/// The extract-to-renderer bridge, reached through `crate::portrait::BoothBridge`.
#[derive(Resource, Default)]
pub struct UiModelTiles {
    /// The last request per pane. An entry outlives its pane, since the memoized conversion never
    /// says it vanished, so which panes draw is the engine's paint list, never this map.
    pub requests: HashMap<FrameHandle, TileRequest>,
    /// This frame's cell per tile that has something to draw.
    pub cells: HashMap<FrameHandle, Cell>,
    /// The atlas and its size; `None` until the first tile is packed.
    pub atlas: Option<Handle<Image>>,
    pub atlas_size: UVec2,
    /// Device px per logical px, written by the extract and read by its arm to size cells.
    pub dpi: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WorldFrame's region mirrored ÷G44/÷G48 (`0x483970`) is the viewport, inclusive. The
    /// last row is a measured unit two thirds of a screen below the bottom edge.
    #[test]
    fn the_accept_region_is_the_viewport_inclusive() {
        let vp = Vec2::new(1440.0, 810.0);
        assert!(accepts(Vec2::new(720.0, 405.0), vp));
        assert!(accepts(Vec2::ZERO, vp), "the corner is in view");
        assert!(accepts(vp, vp), "so is the far corner");
        for out in [
            Vec2::new(-0.5, 405.0),
            Vec2::new(1440.5, 405.0),
            Vec2::new(720.0, -0.5),
            Vec2::new(720.0, 810.5),
            Vec2::new(1409.0, 1332.0),
        ] {
            assert!(!accepts(out, vp), "{out:?} is not on screen");
        }
    }
}
