//! Wrath spellbook rank view (`SpellBookFrame.lua` with `ShowAllSpellRanks` off): the client keeps
//! a second, highest-rank-only list beside the full book (`0xbe5d88` beside `0xbe6d88`).
//! `GetSpellTabInfo` (`0x53be70`) answers six values, each tab's offset and count in both lists;
//! `GetKnownSlotFromHighestRankSlot` (`0x53b650`) maps a highest-rank slot to its full-book slot;
//! `IsSelectedSpell` (`0x5412c0` → `0x53d350`) is the book's checked state, the form and
//! trade-skill arms the app stamps as `SpellSlotView::current`.

use mlua::{Lua, MultiValue, Value};

use super::spellbook::book_slot;
use super::{Model, UiScript};

impl UiScript {
    /// The highest-rank list: 0-based full-book indices of the spells no known higher rank
    /// supersedes, in book order. Pushed with each book.
    pub fn set_spellbook_highest_rank(&mut self, slots: Vec<u32>) {
        self.model_mut().spellbook_highest_rank = slots;
    }
}

/// How many highest-rank entries fall in `[start, end)` of the full book.
fn highest_in(model: &Model, start: u32, end: u32) -> u32 {
    model
        .spellbook_highest_rank
        .iter()
        .filter(|&&i| (start..end).contains(&i))
        .count() as u32
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // name, texture, offset, numSpells, highestRankOffset, highestRankNumSpells; out of range,
    // nil, nil and four zeros. Offsets sum the earlier tabs' counts.
    g.set(
        "GetSpellTabInfo",
        lua.create_function(|lua, i: Value| {
            let i = super::binding_abi::number_arg(lua, i, "Usage: GetSpellTabInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let n = usize::try_from(i64::from(i) - 1).ok();
            let Some((n, tab)) = n.and_then(|n| Some((n, model.spellbook.tabs.get(n)?))) else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(0),
                ]));
            };
            let highest_offset: u32 = model.spellbook.tabs[..n]
                .iter()
                .map(|t| highest_in(&model, t.offset, t.offset + t.num_spells))
                .sum();
            let highest = highest_in(&model, tab.offset, tab.offset + tab.num_spells);
            let texture = match &tab.texture {
                Some(t) => Value::String(lua.create_string(t)?),
                None => Value::Nil,
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&tab.name)?),
                texture,
                Value::Integer(i64::from(tab.offset)),
                Value::Integer(i64::from(tab.num_spells)),
                Value::Integer(i64::from(highest_offset)),
                Value::Integer(i64::from(highest)),
            ]))
        })?,
    )?;
    // The 1-based full-book slot of highest-rank slot `slot`, 0 when there is none.
    g.set(
        "GetKnownSlotFromHighestRankSlot",
        lua.create_function(|lua, slot: Value| {
            let slot = match slot {
                Value::Integer(i) => i,
                Value::Number(f) => f as i64,
                _ => 0,
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let full = usize::try_from(slot - 1)
                .ok()
                .and_then(|i| model.spellbook_highest_rank.get(i))
                .map_or(0, |&i| i64::from(i) + 1);
            Ok(full)
        })?,
    )?;
    super::spell_arg::set_slot_verb(lua, "IsSelectedSpell", |lua, id, book_type| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        Ok(super::binding_abi::flag(
            book_slot(&model, id, book_type).is_some_and(|s| s.current),
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{SpellBookState, SpellSlotView, SpellTabView, UiScript};

    #[test]
    fn the_rank_view_offsets_both_lists() {
        let mut s = UiScript::new().unwrap();
        let slot = |id, current| SpellSlotView {
            spell_id: id,
            current,
            ..Default::default()
        };
        s.set_spellbook(SpellBookState {
            tabs: vec![
                SpellTabView {
                    name: "General".into(),
                    texture: None,
                    offset: 0,
                    num_spells: 2,
                },
                SpellTabView {
                    name: "Fire".into(),
                    texture: None,
                    offset: 2,
                    num_spells: 3,
                },
            ],
            // Fireball ranks 1-3 in the fire tab; only rank 3 is the highest.
            slots: vec![
                slot(1, false),
                slot(2, true),
                slot(10, false),
                slot(11, false),
                slot(12, false),
            ],
        });
        s.set_spellbook_highest_rank(vec![0, 1, 4]);
        s.run(
            r#"
            local name, _, offset, num, hoffset, hnum = GetSpellTabInfo(2)
            assert(name == "Fire" and offset == 2 and num == 3 and hoffset == 2 and hnum == 1)
            _, _, offset, num, hoffset, hnum = GetSpellTabInfo(1)
            assert(offset == 0 and num == 2 and hoffset == 0 and hnum == 2)
            assert(select(6, GetSpellTabInfo(3)) == 0 and GetSpellTabInfo(3) == nil)
            assert(GetKnownSlotFromHighestRankSlot(3) == 5 and GetKnownSlotFromHighestRankSlot(4) == 0)
            assert(IsSelectedSpell(2, "spell") == 1 and IsSelectedSpell(1, "spell") == nil)
            "#,
        )
        .unwrap();
    }
}
