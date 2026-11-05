//! Wrath spell queries over the DBC catalog, independent of the learned spellbook.
//! Stock MultiCastActionBarFrame.lua reads all nine GetSpellInfo returns; SpellBookFrame.lua
//! passes a slot and book type, while PetPaperDollFrame.lua passes an unlearned spell ID.

use super::Model;
use mlua::{Lua, MultiValue, Value};

/// Resolved Spell.dbc metadata. The app supplies the full catalog, not just learned spells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellInfoView {
    pub spell_id: u32,
    pub name: String,
    pub rank: Option<String>,
    pub texture: Option<String>,
    pub cost: u32,
    pub funnel: bool,
    pub power_type: i32,
    pub cast_time_ms: u32,
    pub min_range: f64,
    pub max_range: f64,
    pub passive: bool,
    /// Optional trade link, the second GetSpellLink return used by SpellBookFrame.lua:372.
    pub trade_skill_link: Option<String>,
    /// `Effect[0]` is 78, ATTACK: `IsAttackSpell` (`0x541420`).
    pub attack: bool,
    /// The classifier `0x7fe1b0`: 1 aimed at friends, 2 at enemies, 0 neither.
    pub polarity: u8,
    /// The set `Reagent[8]` with their `ReagentCount[8]`: `IsConsumableSpell`, `GetSpellCount`.
    pub reagents: Vec<(u32, u32)>,
    /// What `IsSpellInRange`'s target gate and range read ([`super::wrath_spell_range`]).
    pub range: super::SpellRangeFacts,
}

impl super::UiScript {
    /// Replace the VM's spell catalog. It includes spells absent from either book.
    pub fn set_spell_catalog(&mut self, rows: impl IntoIterator<Item = SpellInfoView>) {
        self.model_mut().spell_catalog = rows.into_iter().map(|row| (row.spell_id, row)).collect();
    }

    /// The spells `IsCurrentSpell` answers true for this frame.
    pub fn set_current_spells(&mut self, spells: Vec<u32>) {
        self.model_mut().current_spells = spells;
    }
}

fn resolve(model: &Model, query: Value, book: Value) -> Option<SpellInfoView> {
    let id = match &query {
        Value::Integer(n) if *n > 0 => u32::try_from(*n).ok(),
        Value::Number(n) if n.is_finite() && *n > 0.0 && *n <= u32::MAX as f64 => Some(*n as u32),
        Value::String(s) => s.to_str().ok()?.parse::<u32>().ok(),
        _ => None,
    };
    if let (Some(id), Value::String(book)) = (id, &book) {
        let slot = super::spellbook::book_slot(model, id, &book.to_str().ok()?)?;
        return Some(
            model
                .spell_catalog
                .get(&slot.spell_id)
                .cloned()
                .unwrap_or_else(|| SpellInfoView {
                    spell_id: slot.spell_id,
                    name: slot.name.clone(),
                    rank: slot.rank.clone(),
                    texture: slot.texture.clone(),
                    passive: slot.passive,
                    ..Default::default()
                }),
        );
    }
    if let Some(id) = id {
        return model.spell_catalog.get(&id).cloned();
    }
    let Value::String(name) = query else {
        return None;
    };
    let name = name.to_str().ok()?;
    // A name is resolved through known ranks, not an arbitrary unlearned catalog row.
    let rank_arg = match &book {
        Value::String(s) => Some(s.to_str().ok()?),
        _ => None,
    };
    let (name, embedded_rank) = split_name(&name);
    let rank = rank_arg.as_deref().or(embedded_rank);
    model
        .spellbook
        .slots
        .iter()
        .rev()
        .find(|s| {
            s.name.eq_ignore_ascii_case(name)
                && rank.is_none_or(|rank| {
                    s.rank
                        .as_deref()
                        .is_some_and(|r| r.eq_ignore_ascii_case(rank))
                })
        })
        .map(|s| {
            model
                .spell_catalog
                .get(&s.spell_id)
                .cloned()
                .unwrap_or_else(|| SpellInfoView {
                    spell_id: s.spell_id,
                    name: s.name.clone(),
                    rank: s.rank.clone(),
                    texture: s.texture.clone(),
                    passive: s.passive,
                    ..Default::default()
                })
        })
}

fn split_name(name: &str) -> (&str, Option<&str>) {
    if let Some((name, rank)) = name
        .trim()
        .strip_suffix(')')
        .and_then(|s| s.rsplit_once('('))
    {
        (name.trim(), Some(rank.trim()))
    } else {
        (name.trim(), None)
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetSpellInfo",
        lua.create_function(|lua, (query, book): (Value, Value)| {
            let row = resolve(&lua.app_data_ref::<Model>().expect("model"), query, book);
            let Some(row) = row else {
                return Ok(MultiValue::from_vec(vec![Value::Nil]));
            };
            let string = |s: Option<&str>| -> mlua::Result<Value> {
                s.map(|s| lua.create_string(s).map(Value::String))
                    .unwrap_or(Ok(Value::Nil))
            };
            Ok(MultiValue::from_vec(vec![
                string(Some(&row.name))?,
                string(Some(row.rank.as_deref().unwrap_or("")))?,
                string(row.texture.as_deref())?,
                Value::Integer(row.cost as i64),
                Value::Boolean(row.funnel),
                Value::Integer(row.power_type as i64),
                Value::Integer(row.cast_time_ms as i64),
                Value::Number(row.min_range),
                Value::Number(row.max_range),
            ]))
        })?,
    )?;
    // `IsCurrentSpell(spell[, book])` (`0x541500` → `0x806030`): the spell is being cast, queued,
    // channelled, targeted or repeated, or is Attack while engaged. The trade-skill arm (a
    // TRADE_SKILL spell whose window is open) is not built.
    lua.globals().set(
        "IsCurrentSpell",
        lua.create_function(|lua, (query, book): (Value, Value)| {
            let model = lua.app_data_ref::<Model>().expect("model");
            Ok(resolve(&model, query, book)
                .is_some_and(|s| model.current_spells.contains(&s.spell_id)))
        })?,
    )?;
    lua.globals().set(
        "IsPassiveSpell",
        lua.create_function(|lua, (query, book): (Value, Value)| {
            Ok(
                resolve(&lua.app_data_ref::<Model>().expect("model"), query, book)
                    .is_some_and(|s| s.passive),
            )
        })?,
    )?;
    lua.globals().set(
        "GetSpellLink",
        lua.create_function(|lua, (query, book): (Value, Value)| {
            let row = resolve(&lua.app_data_ref::<Model>().expect("model"), query, book);
            Ok(row
                .map(|s| {
                    (
                        Some(format!(
                            "|cff71d5ff|Hspell:{}|h[{}]|h|r",
                            s.spell_id, s.name
                        )),
                        s.trade_skill_link,
                    )
                })
                .unwrap_or((None, None)))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{SpellBookState, SpellSlotView, UiScript};

    #[test]
    fn wrath_catalog_id_book_name_and_nine_returns() {
        let mut s = UiScript::new().unwrap();
        s.set_spell_catalog([
            SpellInfoView {
                spell_id: 133,
                name: "Fireball".into(),
                rank: Some("Rank 1".into()),
                texture: Some("Interface\\Icons\\Spell_Fire_FlameBolt".into()),
                cost: 30,
                power_type: 0,
                cast_time_ms: 1500,
                max_range: 30.0,
                ..Default::default()
            },
            SpellInfoView {
                spell_id: 999,
                name: "Passive".into(),
                passive: true,
                trade_skill_link: Some("trade:999".into()),
                ..Default::default()
            },
        ]);
        s.set_spellbook(SpellBookState {
            slots: vec![SpellSlotView {
                spell_id: 133,
                name: "Fireball".into(),
                rank: Some("Rank 1".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        s.run(r#"
            assert(select('#', GetSpellInfo(133)) == 9)
            local n,r,i,c,f,p,t,m,x = GetSpellInfo(133)
            assert(n == 'Fireball' and r == 'Rank 1' and i == 'Interface\\Icons\\Spell_Fire_FlameBolt')
            assert(c == 30 and f == false and p == 0 and t == 1500 and m == 0 and x == 30)
            assert(GetSpellInfo(1, 'spell') == 'Fireball')
            assert(GetSpellInfo('Fireball (Rank 1)') == 'Fireball')
            assert(GetSpellInfo('Fireball', 'Rank 1') == 'Fireball')
            assert(GetSpellInfo(999) == 'Passive' and IsPassiveSpell(999))
            assert(not IsPassiveSpell(1, 'spell'))
            assert(GetSpellInfo(500) == nil and GetSpellInfo(2, 'spell') == nil)
            assert(GetSpellLink(1, 'spell') == '|cff71d5ff|Hspell:133|h[Fireball]|h|r')
            local link,trade = GetSpellLink(999)
            assert(link == '|cff71d5ff|Hspell:999|h[Passive]|h|r' and trade == 'trade:999')
            assert(GetSpellLink(500) == nil)
        "#).unwrap();
    }

    #[test]
    fn pet_book_and_replacement_catalog() {
        let mut s = UiScript::new().unwrap();
        s.set_pet_book(super::super::PetBookState {
            slots: vec![SpellSlotView {
                spell_id: 42,
                name: "Pet skill".into(),
                passive: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        s.run("assert(GetSpellInfo(1, 'pet') == 'Pet skill'); assert(IsPassiveSpell(1, 'pet'))")
            .unwrap();
        s.set_spell_catalog([SpellInfoView {
            spell_id: 42,
            name: "Catalog skill".into(),
            ..Default::default()
        }]);
        s.run("assert(GetSpellInfo(42) == 'Catalog skill')")
            .unwrap();
        s.set_spell_catalog([]);
        s.run("assert(GetSpellInfo(42) == nil)").unwrap();
    }
}
