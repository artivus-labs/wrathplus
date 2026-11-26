//! Build 12340's glyph sockets for the talent frame's glyph tab: `GetGlyphSocketInfo`
//! (`0x5b7260`), `GetGlyphLink` (`0x5b7740`), `GlyphMatchesSocket` (`0x5b7410`) and
//! `GameTooltip:SetGlyph` (`0x625d00` → `0x622ba0`). The app pushes the six sockets from the
//! player's `PLAYER_FIELD_GLYPH_SLOTS_1`, `PLAYER_FIELD_GLYPHS_1` and `PLAYER_GLYPHS_ENABLED`, read
//! through `GlyphSlot.dbc`, `GlyphProperties.dbc` and `SpellIcon.dbc`. Evidence: one headless
//! Ghidra round (`/tmp/azb-wire-ghidra/census2`).
//!
//! - The getters take a 1-based socket, truncated, and an optional 1-based talent group, the
//!   active one without it (`0x5c57d0`). A socket outside 1..=6, or a group at or past the talents
//!   packet's count (`0x5c6080` → `0x5c5fc0`), answers nothing; the glyph is the group's own
//!   (`0x5c5770`, the record's `+0x3c`).
//! - `GlyphMatchesSocket` (`0x5b7410`) reads the pending glyph cast. The first `APPLY_GLYPH` effect
//!   whose glyph row and socket row both exist answers 1 when their type flags agree; otherwise it
//!   answers nothing. With no player it answers nil.
//!

use mlua::{Lua, MultiValue, Table, Value};

use super::object::frame_handle_of;
use super::tooltip::{append_line, clear_content, fire_cleared, show_or_hide_empty};
use super::Model;

/// One glyph socket as the app resolves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphSocketView {
    /// `PLAYER_GLYPHS_ENABLED`'s bit for the socket.
    pub enabled: bool,
    /// `PLAYER_FIELD_GLYPH_SLOTS_1 + i`, a `GlyphSlot.dbc` id.
    pub slot_id: u32,
    /// The slot row's type flags (0 major, 1 minor) and `GLYPH_SLOT_TOOLTIP` order; `None` with
    /// no row.
    pub slot: Option<(u32, u32)>,
    /// `PLAYER_FIELD_GLYPHS_1 + i`, a `GlyphProperties.dbc` id, 0 for none.
    pub glyph_id: u32,
    /// The glyph row's spell, its name, and its `SpellIcon.dbc` path; `None` with no row.
    pub glyph: Option<GlyphView>,
    /// The pending glyph cast fits this socket (`GlyphMatchesSocket`).
    pub matches_pending: bool,
}

/// A socketed glyph's resolved row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphView {
    pub spell_id: u32,
    /// The spell's name; empty when `Spell.dbc` has no row.
    pub name: String,
    pub icon: Option<String>,
}

impl super::UiScript {
    /// Replace the six sockets; `None` without a player.
    pub fn set_glyph_sockets(&mut self, sockets: Option<[GlyphSocketView; 6]>) {
        self.model_mut().glyph_sockets = sockets;
    }

    /// Replace the other talent group's six sockets, its glyphs from the talents packet.
    pub fn set_inactive_glyph_sockets(&mut self, sockets: Option<[GlyphSocketView; 6]>) {
        self.model_mut().inactive_glyph_sockets = sockets;
    }
}

/// The socket behind a 1-based index in a 0-based group the talents packet holds.
fn socket(model: &Model, index: i64, group: Option<i64>) -> Option<&GlyphSocketView> {
    let groups = &model.talent_groups;
    let group = group.map_or(Some(groups.active), |g| u32::try_from(g - 1).ok())?;
    if group >= groups.count || !(1..=6).contains(&index) {
        return None;
    }
    let sockets = if group == groups.active {
        &model.glyph_sockets
    } else {
        &model.inactive_glyph_sockets
    };
    sockets.as_ref().map(|s| &s[(index - 1) as usize])
}

/// The socket index, truncated (`fistp` under RC=11, `0x5b72ad`).
fn arg_int(lua: &Lua, args: &MultiValue, i: usize) -> Option<i64> {
    args.get(i)
        .cloned()
        .and_then(|v| lua.coerce_number(v).ok().flatten())
        .map(|n| n.trunc() as i64)
}

/// The group argument as `lua_tointeger` reads it (`0x5c57d0`), which rounds; `None` without one.
fn group(lua: &Lua, args: &MultiValue, i: usize) -> Option<i64> {
    args.get(i)
        .cloned()
        .and_then(|v| lua.coerce_number(v).ok().flatten())
        .map(|n| n.round_ties_even() as i64)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetGlyphSocketInfo(index [, group]) → enabled, glyphType (1 major, 2 minor), glyphSpell,
    // iconFilename.
    g.set(
        "GetGlyphSocketInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some(index) = arg_int(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetGlyphSocketInfo(index[, talentGroup])",
                ));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(s) = socket(&model, index, group(lua, &args, 1)) else {
                return Ok(MultiValue::new());
            };
            let flag = |on: bool| if on { Value::Integer(1) } else { Value::Nil };
            let kind = s
                .slot
                .map_or(Value::Nil, |(t, _)| Value::Integer(i64::from(t) + 1));
            let (spell, icon) = match &s.glyph {
                Some(gl) => (
                    Value::Integer(i64::from(gl.spell_id)),
                    match &gl.icon {
                        Some(p) => Value::String(lua.create_string(p)?),
                        None => Value::Nil,
                    },
                ),
                None => (Value::Nil, Value::Nil),
            };
            Ok(MultiValue::from_vec(vec![
                flag(s.enabled),
                kind,
                spell,
                icon,
            ]))
        })?,
    )?;

    // GetGlyphLink(index [, group]) (`0x61e650`): `|cff66bbff|Hglyph:<slot>:<glyph>|h[<spell>]|h|r`,
    // or "" when a row or the spell is missing.
    g.set(
        "GetGlyphLink",
        lua.create_function(|lua, args: MultiValue| {
            let Some(index) = arg_int(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetGlyphLink(index[, talentGroup])",
                ));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(s) = socket(&model, index, group(lua, &args, 1)) else {
                return Ok(MultiValue::new());
            };
            let link = match (&s.slot, &s.glyph) {
                (Some(_), Some(gl)) if !gl.name.is_empty() => format!(
                    "|cff66bbff|Hglyph:{}:{}|h[{}]|h|r",
                    s.slot_id, s.glyph_id, gl.name
                ),
                _ => String::new(),
            };
            Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string(&link)?,
            )]))
        })?,
    )?;

    g.set(
        "GlyphMatchesSocket",
        lua.create_function(|lua, args: MultiValue| {
            let Some(index) = arg_int(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: GlyphMatchesSocket(socketIndex)",
                ));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(sockets) = model.glyph_sockets.as_ref() else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            let hit = usize::try_from(index - 1)
                .ok()
                .and_then(|i| sockets.get(i))
                .is_some_and(|s| s.matches_pending);
            Ok(if hit {
                MultiValue::from_vec(vec![Value::Integer(1)])
            } else {
                MultiValue::new()
            })
        })?,
    )?;
    Ok(())
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GRAY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
const GOLD: [f32; 4] = [1.0, 0.82, 0.0, 1.0];
/// `0xad2da4`, the glyph type line.
pub(super) const GLYPH_BLUE: [f32; 4] = [0.4, 0.733, 1.0, 1.0];

/// `GameTooltip:SetGlyph(socket [, group])` (`0x622ba0`):
/// - an empty socket: `GLYPH_INACTIVE` in gray, or `GLYPH_LOCKED` in red when locked;
/// - a socketed glyph: its spell name in white;
/// - then `MAJOR_GLYPH` or `MINOR_GLYPH` in glyph blue;
/// - then, in wrapped gold, `GLYPH_EMPTY_DESC` for an open socket or `GLYPH_SLOT_TOOLTIP<order>`
///   for a locked one;
/// - or, for a glyph, the spell's description, then `GLYPH_SLOT_REMOVE_TOOLTIP` in gray. Labelled:
///   the description is the spell tooltip's, which resolves on its first ask, so a first hover
///   can end after the type line.
pub(super) fn install_tooltip_method(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.set(
        "SetGlyph",
        lua.create_function(|lua, (this, args): (Table, MultiValue)| {
            let Some(index) = arg_int(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: GameTooltip:SetGlyph(glyphSlot[, groupIndex])",
                ));
            };
            let s = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                match socket(&model, index, group(lua, &args, 1)) {
                    Some(s) => s.clone(),
                    None => return Ok(()),
                }
            };
            let text = |key: &str| lua.globals().get::<String>(key).unwrap_or_default();
            let h = frame_handle_of(lua, &this)?;
            {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                clear_content(&mut model, h);
            }
            fire_cleared(lua, h);
            let glyph = s
                .glyph
                .as_ref()
                .filter(|gl| s.glyph_id != 0 && !gl.name.is_empty());
            match glyph {
                Some(gl) => append_line(lua, &this, (gl.name.clone(), WHITE), None, false)?,
                None if s.enabled => {
                    append_line(lua, &this, (text("GLYPH_INACTIVE"), GRAY), None, false)?
                }
                None => append_line(lua, &this, (text("GLYPH_LOCKED"), RED), None, false)?,
            }
            let minor = s.slot.is_some_and(|(t, _)| t != 0);
            let kind = if minor { "MINOR_GLYPH" } else { "MAJOR_GLYPH" };
            append_line(lua, &this, (text(kind), GLYPH_BLUE), None, false)?;
            match (glyph, s.enabled, s.slot) {
                (Some(gl), _, _) => {
                    // The description is the spell tooltip's, resolved on first ask; an empty one
                    // ends the tooltip, as `0x622ba0` does.
                    let description = super::tooltip_spell::spell_view_of(lua, gl.spell_id, false)
                        .map(|v| v.description)
                        .unwrap_or_default();
                    if !description.is_empty() {
                        append_line(lua, &this, (description, GOLD), None, true)?;
                        append_line(
                            lua,
                            &this,
                            (text("GLYPH_SLOT_REMOVE_TOOLTIP"), GRAY),
                            None,
                            false,
                        )?;
                    }
                }
                (None, true, _) => {
                    append_line(lua, &this, (text("GLYPH_EMPTY_DESC"), GOLD), None, true)?
                }
                (None, false, Some((_, order))) => append_line(
                    lua,
                    &this,
                    (text(&format!("GLYPH_SLOT_TOOLTIP{order}")), GOLD),
                    None,
                    true,
                )?,
                (None, false, None) => {}
            }
            // `0x622ba0` runs no content script: it ends in the show (`0x48f660`).
            show_or_hide_empty(lua, h);
            Ok(())
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    fn script() -> UiScript {
        let mut s = UiScript::new().unwrap();
        let mut sockets: [GlyphSocketView; 6] = Default::default();
        sockets[0] = GlyphSocketView {
            enabled: true,
            slot_id: 21,
            slot: Some((0, 0)),
            glyph_id: 182,
            glyph: Some(GlyphView {
                spell_id: 54815,
                name: "Glyph of Shred".into(),
                icon: Some("Interface\\Spellbook\\UI-Glyph-Rune-1".into()),
            }),
            matches_pending: false,
        };
        sockets[1] = GlyphSocketView {
            enabled: true,
            slot_id: 22,
            slot: Some((1, 1)),
            matches_pending: true,
            ..Default::default()
        };
        sockets[5] = GlyphSocketView {
            slot_id: 41,
            slot: Some((0, 5)),
            ..Default::default()
        };
        s.set_glyph_sockets(Some(sockets));
        s.set_talent_groups(crate::script::WrathTalentGroups::single());
        s
    }

    #[test]
    fn socket_info_and_links_read_the_pushed_sockets() {
        let s = script();
        let (en, kind, spell, icon): (i64, i64, i64, String) =
            s.eval("return GetGlyphSocketInfo(1)").unwrap();
        assert_eq!((en, kind, spell), (1, 1, 54815));
        assert!(icon.contains("Rune-1"));
        assert_eq!(
            s.eval::<i64>("return select('#', GetGlyphSocketInfo(2))")
                .unwrap(),
            4
        );
        assert_eq!(
            s.eval::<i64>("return (select(2, GetGlyphSocketInfo(2)))")
                .unwrap(),
            2
        );
        assert!(s
            .eval::<bool>("return GetGlyphSocketInfo(6) == nil")
            .unwrap());
        assert_eq!(
            s.eval::<i64>("return select('#', GetGlyphSocketInfo(7))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetGlyphSocketInfo(1, 2))")
                .unwrap(),
            0
        );
        assert_eq!(
            s.eval::<String>("return GetGlyphLink(1)").unwrap(),
            "|cff66bbff|Hglyph:21:182|h[Glyph of Shred]|h|r"
        );
        assert_eq!(s.eval::<String>("return GetGlyphLink(2)").unwrap(), "");
        assert_eq!(s.eval::<i64>("return GlyphMatchesSocket(2)").unwrap(), 1);
        assert_eq!(
            s.eval::<i64>("return select('#', GlyphMatchesSocket(1))")
                .unwrap(),
            0
        );
        assert!(s.run("GetGlyphLink()").is_err());
        assert_eq!(
            s.eval::<i64>("return (select(3, GetGlyphSocketInfo(1.9)))")
                .unwrap(),
            54815
        );

        // Two groups, the second active: no group reads it; group 1 is the other's sockets.
        let mut other: [GlyphSocketView; 6] = Default::default();
        other[0].glyph = Some(GlyphView {
            spell_id: 54810,
            ..Default::default()
        });
        let mut s = s;
        s.set_inactive_glyph_sockets(Some(other));
        s.set_talent_groups(crate::script::WrathTalentGroups {
            count: 2,
            active: 1,
            inactive: None,
        });
        assert_eq!(
            s.eval::<(i64, i64, i64)>(
                "return select(3, GetGlyphSocketInfo(1)), select(3, GetGlyphSocketInfo(1, 2)), \
                 select(3, GetGlyphSocketInfo(1, 1))"
            )
            .unwrap(),
            (54815, 54815, 54810)
        );
        s.set_talent_groups(Default::default());
        assert_eq!(
            s.eval::<i64>("return select('#', GetGlyphSocketInfo(1))")
                .unwrap(),
            0,
            "no talents packet yet, no group (`0x5c5fc0`)"
        );
    }

    #[test]
    fn no_player_answers_the_native_misses() {
        let s = UiScript::new().unwrap();
        assert_eq!(
            s.eval::<i64>("return select('#', GlyphMatchesSocket(1))")
                .unwrap(),
            1
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetGlyphSocketInfo(1))")
                .unwrap(),
            0
        );
    }

    #[test]
    fn set_glyph_writes_the_empty_and_locked_socket_lines() {
        let s = script();
        s.run(
            r#"
            GLYPH_INACTIVE = "Empty" GLYPH_LOCKED = "Locked" MAJOR_GLYPH = "Major Glyph"
            MINOR_GLYPH = "Minor Glyph" GLYPH_EMPTY_DESC = "Use a glyph."
            GLYPH_SLOT_TOOLTIP5 = "Unlocked at level 80."
            local a = CreateFrame("Button", "GB1")
            local tt = CreateFrame("GameTooltip", "TT")
            tt:SetOwner(a, "ANCHOR_RIGHT")
            tt:SetGlyph(2)
            assert(tt:NumLines() == 3, "got " .. tt:NumLines())
            assert(TTTextLeft1:GetText() == "Empty")
            assert(TTTextLeft2:GetText() == "Minor Glyph")
            assert(TTTextLeft3:GetText() == "Use a glyph.")
            tt:SetGlyph(6)
            assert(TTTextLeft1:GetText() == "Locked")
            assert(TTTextLeft2:GetText() == "Major Glyph")
            assert(TTTextLeft3:GetText() == "Unlocked at level 80.")
            local r, g, b = TTTextLeft1:GetTextColor()
            assert(r == 1 and g == 0 and b == 0, "locked is red")
            "#,
        )
        .unwrap();
        assert!(s.run("TT:SetGlyph()").is_err());
    }
}
