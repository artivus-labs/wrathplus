//! The `GameTooltip` kind's state.

use super::{FrameHandle, RegionHandle, TooltipAnchor};

/// A `GameTooltip`'s state. Its lines are FontString regions named `<name>TextLeftN`/`TextRightN`,
/// made on demand and published as Lua globals, as the stock template's 30 pairs are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TooltipState {
    /// Lines currently filled (`NumLines`). Regions past this index exist but are hidden.
    pub num_lines: usize,
    /// The left-column FontString pool, index i = line i+1, grown on demand with no line cap.
    pub left_lines: Vec<RegionHandle>,
    /// The right-column pool (the `AddDoubleLine`/`TextRightN` half), parallel to `left_lines`.
    pub right_lines: Vec<RegionHandle>,
    /// `SetOwner`'s frame, dropped on hide.
    pub owner: Option<FrameHandle>,
    /// The last `SetOwner`'s mode, read back by `GetAnchorType()`.
    pub anchor: TooltipAnchor,
    /// `SetMinimumWidth`: a floor on the auto-sized width, cleared with the content.
    pub min_width: f32,
    /// `FadeOut()`'s start on the `GetTime` clock; any fresh content or `Show` cancels the fade.
    pub fade_start: Option<f64>,
    /// The guid of the unit this tooltip shows (`+0x368`/`+0x36c`, stored at `0x52a070`), whose
    /// health watcher (`0x467e70`, registered at `0x52a0a8`) drives the bar; a `set_unit` push of
    /// that unit, under any token, re-drives it.
    pub unit_guid: Option<u64>,
    /// Shows world-hover content, which fades when the hover is lost; a window hover never fades.
    pub world_owned: bool,
    /// Armed for a compare render: the next `SetInventoryItem` prepends "Currently Equipped"
    /// (`[arg+0x18]`, not the compact `[arg+0x14]`), surviving the stock caller's `SetOwner` in
    /// between (`MerchantFrame.xml:67-72`); the reference's route to `0x52b650` is untraced.
    pub equipped_header_armed: bool,
    /// `SetPadding`: extra width past the content (`ItemRef.xml:40` sets 16 for the close button).
    pub padding: f32,
    /// Line 1 came from XML (`ShoppingTooltipTemplate`), so later lines clone the previous fonts.
    pub xml_declared_lines: bool,
    /// Wrath `AddTexture`: the 0-based line each used `$parentTextureN` sits on, in use order.
    pub texture_lines: Vec<usize>,
    /// The item the content shows (`+0x360`) and its displayed name, for `GetItem` and
    /// `IsEquippedItem`; cleared with the content.
    pub item: Option<(u32, String)>,
    /// The spell the content shows (`+0x364`, set at `0x623941`), for `GetSpell`; cleared with it.
    pub spell: Option<u32>,
}
