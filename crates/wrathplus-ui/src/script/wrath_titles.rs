//! Build-12340 known titles (PaperDollFrame's title menu): GetNumTitles, GetTitleName,
//! IsTitleKnown and GetCurrentTitle over CharTitles.dbc and the player's title fields (one
//! Wow.exe round: `/tmp/azb-ui-barber-native.log`). `SetCurrentTitle` is in `wrath_chat_verbs`.

use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// A CharTitles.dbc row (`0xad3390`, `0x14` bytes each).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathCharTitle {
    pub id: u32,
    pub male: String,
    pub female: String,
    /// The title's bit in `PLAYER__FIELD_KNOWN_TITLES`, the id the Lua API takes (`+0x10`).
    pub mask_id: u32,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    catalog: Arc<Vec<WrathCharTitle>>,
    /// `PLAYER__FIELD_KNOWN_TITLES` (descriptor `+0x778`) as 64-bit words.
    known: Vec<u64>,
    /// `PLAYER_CHOSEN_TITLE` (`+0x2b4`).
    chosen: u32,
}

impl super::UiScript {
    /// CharTitles.dbc in source order.
    pub fn set_wrath_char_titles(&mut self, catalog: Arc<Vec<WrathCharTitle>>) {
        self.model_mut().wrath_titles.catalog = catalog;
    }

    /// The player's known-title words and chosen title, from the descriptor.
    pub fn set_wrath_player_titles(&mut self, known: Vec<u64>, chosen: u32) {
        let mut model = self.model_mut();
        model.wrath_titles.known = known;
        model.wrath_titles.chosen = chosen;
    }
}

/// The CharTitles row whose `MaskID` is `mask` (`0x719900`), 0 matching none.
pub(crate) fn by_mask(model: &Model, mask: u32) -> Option<&WrathCharTitle> {
    (mask != 0)
        .then(|| {
            model
                .wrath_titles
                .catalog
                .iter()
                .find(|t| t.mask_id == mask)
        })
        .flatten()
}

/// `0x51b4e0`'s cleanup: a `%` and the characters after it up to a space are dropped, the space
/// kept ("%s the Patient" → " the Patient").
fn strip_placeholder(name: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for c in name.chars() {
        if c == '%' {
            skipping = true;
        } else if skipping && c != ' ' {
            continue;
        } else {
            skipping = false;
            out.push(c);
        }
    }
    out
}

fn has_player(m: &Model) -> bool {
    m.unit("player").is_some_and(|p| p.exists)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // GetNumTitles() (`0x5165e0`) → CharTitles' row count plus one.
    g.set(
        "GetNumTitles",
        lua.create_function(|lua, ()| {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(m.wrath_titles.catalog.len() as i64 + 1)
        })?,
    )?;
    // GetTitleName(titleMaskID) (`0x51b4e0`) → the row's name for the player's gender (the other
    // gender's when that one is empty), placeholder stripped; nothing for an unknown mask id.
    g.set(
        "GetTitleName",
        lua.create_function(|lua, id: Value| {
            let id = super::binding_abi::number_arg(lua, id, "Usage: GetTitleName(titleMaskID)")?;
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            let female = m.unit("player").is_some_and(|p| p.exists && p.sex == 3);
            let Some(row) = m
                .wrath_titles
                .catalog
                .iter()
                .find(|r| i64::from(r.mask_id) == i64::from(id))
            else {
                return Ok(MultiValue::new());
            };
            let (first, other) = if female {
                (&row.female, &row.male)
            } else {
                (&row.male, &row.female)
            };
            let name = if first.is_empty() { other } else { first };
            Ok(MultiValue::from_vec(vec![Value::String(
                lua.create_string(strip_placeholder(name))?,
            )]))
        })?,
    )?;
    // IsTitleKnown(titleMaskID) (`0x51b410`) → 1 or 0, never nil.
    g.set(
        "IsTitleKnown",
        lua.create_function(|lua, id: Value| {
            let id = super::binding_abi::number_arg(lua, id, "Usage: IsTitleKnown(titleMaskID)")?;
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            let bit = u32::try_from(id).ok().filter(|&b| b != 0);
            let known = has_player(&m)
                && bit.is_some_and(|b| {
                    m.wrath_titles
                        .known
                        .get((b >> 6) as usize)
                        .is_some_and(|w| w & (1u64 << (b & 63)) != 0)
                });
            Ok(i64::from(known))
        })?,
    )?;
    // GetCurrentTitle() (`0x51b3b0`) → PLAYER_CHOSEN_TITLE, 0 with no player.
    g.set(
        "GetCurrentTitle",
        lua.create_function(|lua, ()| {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(if has_player(&m) {
                i64::from(m.wrath_titles.chosen)
            } else {
                0
            })
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{UiScript, UnitState};

    #[test]
    fn titles_read_the_catalog_and_the_player_fields() {
        let mut s = UiScript::new().unwrap();
        assert!(s.eval::<bool>("return GetNumTitles() == 1 and GetCurrentTitle() == 0 and IsTitleKnown(1) == 0 and select('#', GetTitleName(1)) == 0").unwrap());
        s.set_wrath_char_titles(Arc::new(vec![
            WrathCharTitle {
                id: 1,
                male: "Private %s".into(),
                female: "".into(),
                mask_id: 1,
            },
            WrathCharTitle {
                id: 2,
                male: "%s the Patient".into(),
                female: "%s the Patient".into(),
                mask_id: 65,
            },
            WrathCharTitle {
                id: 3,
                male: "Lord %s".into(),
                female: "Lady %s".into(),
                mask_id: 7,
            },
        ]));
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                exists: true,
                sex: 3,
                ..Default::default()
            }),
        );
        s.set_wrath_player_titles(vec![1 << 1, 1 << 1], 65);
        for check in [
            "GetNumTitles() == 4",
            "GetTitleName(1) == 'Private '",
            "GetTitleName(65) == ' the Patient'",
            "GetTitleName(7) == 'Lady '",
            "IsTitleKnown(1) == 1 and IsTitleKnown(65) == 1 and IsTitleKnown(7) == 0 and IsTitleKnown(0) == 0",
            "GetCurrentTitle() == 65",
        ] {
            assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
        }
        assert!(s.run("GetTitleName()").is_err() && s.run("IsTitleKnown('x')").is_err());
    }
}
