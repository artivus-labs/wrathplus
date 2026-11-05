//! Wrath spell queries over the DBC catalog, independent of the learned spellbook.
//! Stock MultiCastActionBarFrame.lua reads all nine GetSpellInfo returns; SpellBookFrame.lua
//! passes a slot and book type, while PetPaperDollFrame.lua passes an unlearned spell ID.

use super::binding_abi::flag;
use super::{Model, SpellSlotView};
use mlua::{IntoLuaMulti, Lua, MultiValue, Value};

/// Resolved Spell.dbc metadata, as GetSpellInfo (`0x540a30`) answers it with no talent, level or
/// haste applied. The app supplies the full catalog, not just learned spells.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellInfoView {
    pub spell_id: u32,
    pub name: String,
    pub rank: Option<String>,
    pub texture: Option<String>,
    /// `ManaCost` over the power type's display divisor (`0x7fde00`, `0x540be9`).
    pub cost: u32,
    /// `ManaPerSecond` over the same divisor (`0x540c00`); a positive one is GetSpellInfo's fourth
    /// return in place of the cost.
    pub cost_per_second: u32,
    pub power_type: i32,
    /// 0 for a passive (`Attributes & 0x40`, `0x540c4b`).
    pub cast_time_ms: u32,
    /// GetSpellInfo's pair: (0, 0) for a melee range row (`0x7ff380`), else `0x801650`'s.
    pub min_range: f64,
    pub max_range: f64,
    pub passive: bool,
    /// The icon is a weapon's (`0x540b5a`): Attack (`Effect[0]` 78, `0x5a84d0`) or a ranged
    /// auto-repeat (`Attributes & 2` and `AttributesEx2 & 0x20`, `0x5a85e0`).
    pub weapon_icon: bool,
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
    /// Its keys into the cooldown list, for `GetSpellCooldown` off the books.
    pub cooldown: super::SpellCooldownFacts,
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

/// The spell a verb's arguments name and the book it came from: a lone number is a spell id when
/// the verb has the shortcut (`isnumber(1) && gettop == 1`, GetSpellInfo `0x540a4f`), read by
/// `round` (`__ftol` truncation, or `lua_tointeger` for IsCurrentSpell); else the shared resolver
/// ([`super::spell_arg`]), whose slot holds the spell (0 for an empty one). `None` answers no values.
fn spell_of(
    lua: &Lua,
    args: &MultiValue,
    verb: &str,
    round: Option<fn(f64) -> u32>,
) -> mlua::Result<Option<(u32, bool, Option<SpellSlotView>)>> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
    if let (Some(round), 1) = (round, args.len()) {
        if let Some(n) = lua.coerce_number(arg(0))? {
            return Ok(Some((round(n), false, None)));
        }
    }
    let model = lua.app_data_ref::<Model>().expect("model");
    let Some((id, book)) = super::spell_arg::spell_arg(lua, &model, &arg(0), &arg(1), verb, true)?
    else {
        return Ok(None);
    };
    let slot = super::spellbook::book_slot(&model, id, &book).cloned();
    let pet = super::spellbook::is_pet_book(&book);
    Ok(Some((slot.as_ref().map_or(0, |s| s.spell_id), pet, slot)))
}

/// `__ftol`'s truncation, the low 32 bits.
fn truncate(n: f64) -> u32 {
    n.trunc() as i64 as u32
}

/// The `Spell.dbc` row (`0x4cfd20`), from the catalog; a book slot the catalog lacks reads itself.
fn row(model: &Model, spell_id: u32, slot: Option<SpellSlotView>) -> Option<SpellInfoView> {
    if let Some(row) = model.spell_catalog.get(&spell_id) {
        return Some(row.clone());
    }
    let slot = slot.filter(|s| s.spell_id != 0)?;
    Some(SpellInfoView {
        spell_id,
        name: slot.name,
        rank: slot.rank,
        texture: slot.texture,
        passive: slot.passive,
        ..Default::default()
    })
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetSpellInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some((spell_id, pet, slot)) = spell_of(lua, &args, "GetSpellInfo", Some(truncate))?
            else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model");
            // No row (an empty slot reads spell 0) answers nothing (`0x540d52`).
            let Some(row) = row(&model, spell_id, slot) else {
                return Ok(MultiValue::new());
            };
            // The weapon icon is the book's, which the app resolves per frame; no icon is "".
            let texture = match row.weapon_icon {
                true => (model.spellbook.slots.iter())
                    .find(|s| s.spell_id == spell_id)
                    .and_then(|s| s.texture.clone())
                    .or(row.texture),
                false => row.texture,
            };
            // A positive per-second cost replaces the cost, and the fifth return says so; the
            // reference pushes it with `lua_pushboolean` (`0x540c2e`), not 1/nil.
            let (cost, per_second) = match row.cost_per_second {
                0 => (row.cost, false),
                n => (n, true),
            };
            let mut out = vec![
                Value::String(lua.create_string(&row.name)?),
                Value::String(lua.create_string(row.rank.as_deref().unwrap_or(""))?),
                Value::String(lua.create_string(texture.as_deref().unwrap_or(""))?),
                Value::Integer(i64::from(cost)),
                Value::Boolean(per_second),
                Value::Integer(i64::from(row.power_type)),
                Value::Integer(i64::from(row.cast_time_ms)),
                Value::Number(row.min_range),
                Value::Number(row.max_range),
            ];
            // No player, or no pet for a pet-book spell, stops before the range (`0x540c84`).
            let unit = match pet {
                true => model.unit_guids.pet,
                false => model.unit_guids.player,
            };
            if unit == 0 {
                out.truncate(7);
            }
            // Labelled: the cost, per-second cost, cast time and range are the row's. The
            // reference resolves them for the caster (`0x8012f0`: level scaling, ManaCostPercentage
            // of base mana or health, the cost spell mods; `0x7ff100`, `0x7ff180` with haste, range
            // mods and the ranged min of `0x7ff400`), so a percentage-cost spell, most 3.3.5 caster
            // spells, answers cost 0 here.
            Ok(MultiValue::from_vec(out))
        })?,
    )?;
    // GetSpellCooldown(spell [, book]) (`0x540e80`) -> start, duration, enable in `GetTime`
    // seconds, as `GetActionCooldown`: enable 0 for an on-hold cooldown; an expired or empty one
    // answers the reference's no-cooldown `(0, 0, 1)`, so a re-feed cannot replay the finish
    // flash. A lone id picks its book by scanning the player's, then the pet's (`0x540ec7`); a
    // book slot answers from its pushed cooldown, and an id in neither book asks the player's
    // list (`0x540f98` → `0x809000`) through its pushed legs ([`super::spell_cooldown`]); an id
    // with no `Spell.dbc` row reads cold there (`0x4cfd20`, `0x8079bf`). Labelled: an id on the
    // pet bar but in neither book (`0x540f0f`, `0x5d3560`) reads the player's list, where the
    // reference reads the pet's; a book slot's held cooldown answers its record's start and full
    // duration, where `0x807980` answers now and, for the spell leg, 1 ms (`0x807af2`).
    lua.globals().set(
        "GetSpellCooldown",
        lua.create_function(|lua, args: MultiValue| {
            let Some((spell_id, pet, _)) =
                spell_of(lua, &args, "GetSpellCooldown", Some(truncate))?
            else {
                return Ok(MultiValue::new());
            };
            let now = crate::script::clock::now(lua);
            let model = lua.app_data_ref::<Model>().expect("model");
            let slots = |pet: bool| match pet {
                true => &model.pet_book.slots,
                false => &model.spellbook.slots,
            };
            let slot = [pet, !pet]
                .into_iter()
                .filter(|_| spell_id != 0)
                .find_map(|pet| slots(pet).iter().rev().find(|s| s.spell_id == spell_id));
            let cooldown = match slot {
                Some(s) => s.cooldown,
                None if spell_id != 0 => {
                    let now_ms = (now * 1000.0).round() as i64;
                    let row = model.spell_catalog.get(&spell_id);
                    row.and_then(|r| {
                        super::spell_cooldown::player_cooldown(lua, spell_id, &r.cooldown, now_ms)
                    })
                }
                None => None,
            };
            let triple = match cooldown {
                Some((start_ms, duration_ms, enabled)) => {
                    let (start, duration) =
                        (start_ms as f64 / 1000.0, f64::from(duration_ms) / 1000.0);
                    if start + duration > now || !enabled {
                        (start, duration, i32::from(enabled))
                    } else {
                        (0.0, 0.0, 1)
                    }
                }
                None => (0.0, 0.0, 1),
            };
            triple.into_lua_multi(lua)
        })?,
    )?;
    // `IsCurrentSpell(spell[, book])` (`0x541500` → `0x806030`): the spell is being cast, queued,
    // channelled, targeted or repeated, or is Attack while engaged; 1 or nil (`0x54159e`). Its id
    // shortcut rounds (`lua_tointeger`, `0x541529`). The trade-skill arm (a TRADE_SKILL spell
    // whose window is open) is not built.
    lua.globals().set(
        "IsCurrentSpell",
        lua.create_function(|lua, args: MultiValue| {
            let round: fn(f64) -> u32 = super::item_arg::tointeger;
            let Some((spell_id, _, _)) = spell_of(lua, &args, "IsCurrentSpell", Some(round))?
            else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model");
            let current = spell_id != 0 && model.current_spells.contains(&spell_id);
            flag(current).into_lua_multi(lua)
        })?,
    )?;
    // IsPassiveSpell(spell [, book]) (`0x541340`): `Attributes & 0x40` (`0x5413cc`), 1 or nil; no
    // id shortcut, so a lone number is a name.
    lua.globals().set(
        "IsPassiveSpell",
        lua.create_function(|lua, args: MultiValue| {
            let Some((spell_id, _, slot)) = spell_of(lua, &args, "IsPassiveSpell", None)? else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model");
            flag(row(&model, spell_id, slot).is_some_and(|s| s.passive)).into_lua_multi(lua)
        })?,
    )?;
    // GetSpellLink(spell [, book]) (`0x5408e0`) -> link, tradeLink; two nils without a row.
    // Labelled: the reference's nil for a hidden spell (`Attributes & 0x80` with `& 0x100`,
    // `0x5409a4`) is not read.
    lua.globals().set(
        "GetSpellLink",
        lua.create_function(|lua, args: MultiValue| {
            let Some((spell_id, _, slot)) = spell_of(lua, &args, "GetSpellLink", Some(truncate))?
            else {
                return Ok(MultiValue::new());
            };
            let row = row(&lua.app_data_ref::<Model>().expect("model"), spell_id, slot);
            row.map(|s| {
                (
                    Some(format!(
                        "|cff71d5ff|Hspell:{}|h[{}]|h|r",
                        s.spell_id, s.name
                    )),
                    s.trade_skill_link,
                )
            })
            .unwrap_or((None, None))
            .into_lua_multi(lua)
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
            // Attack: the weapon's icon from the book; a per-second cost replaces the cost.
            SpellInfoView {
                spell_id: 6603,
                name: "Attack".into(),
                texture: Some("own".into()),
                cost: 9,
                cost_per_second: 3,
                weapon_icon: true,
                ..Default::default()
            },
        ]);
        s.set_spellbook(SpellBookState {
            slots: vec![
                SpellSlotView {
                    spell_id: 133,
                    name: "Fireball".into(),
                    rank: Some("Rank 1".into()),
                    ..Default::default()
                },
                SpellSlotView {
                    spell_id: 6603,
                    texture: Some("sword".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        // No player: GetSpellInfo stops after the cast time (`0x540c84`).
        assert_eq!(s.arity("GetSpellInfo(133)").unwrap(), 7);
        s.model_mut().unit_guids.player = 1;
        s.run(r#"
            assert(select('#', GetSpellInfo(133)) == 9)
            local _, _, i, c, f = GetSpellInfo(6603)
            assert(i == 'sword' and c == 3 and f == true and select(3, GetSpellInfo(999)) == '')
            local n,r,i,c,f,p,t,m,x = GetSpellInfo(133)
            assert(n == 'Fireball' and r == 'Rank 1' and i == 'Interface\\Icons\\Spell_Fire_FlameBolt')
            assert(c == 30 and f == false and p == 0 and t == 1500 and m == 0 and x == 30)
            assert(GetSpellInfo(1, 'spell') == 'Fireball' and GetSpellInfo(133.9) == 'Fireball')
            assert(GetSpellInfo('Fireball(Rank 1)') == 'Fireball')
            assert(GetSpellInfo('Fireball', 'Rank 1') == 'Fireball')
            -- The id shortcut needs a lone argument; IsPassiveSpell has none; a miss is no values.
            for _, n in ipairs({select('#', GetSpellInfo(133, nil)), select('#', IsPassiveSpell(999)),
                select('#', GetSpellInfo('Fireball (Rank 1)')), select('#', GetSpellInfo(500)),
                select('#', GetSpellCooldown('Nope')), select('#', IsCurrentSpell('Nope'))}) do
                assert(n == 0)
            end
            assert(GetSpellInfo(999) == 'Passive' and IsPassiveSpell(1, 'spell') == nil)
            assert(GetSpellInfo(3, 'spell') == nil and GetSpellCooldown(133) == 0)
            assert(GetSpellLink(1, 'spell') == '|cff71d5ff|Hspell:133|h[Fireball]|h|r')
            local link,trade = GetSpellLink(999)
            assert(link == '|cff71d5ff|Hspell:999|h[Passive]|h|r' and trade == 'trade:999')
            assert(GetSpellLink(500) == nil)
        "#).unwrap();
        s.set_current_spells(vec![134]);
        s.run("assert(IsCurrentSpell(133.5) == 1 and IsCurrentSpell(133.9, nil) == nil)")
            .unwrap();
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
        s.run(
            "assert(GetSpellInfo(1, 'pet') == 'Pet skill'); assert(IsPassiveSpell(1, 'pet') == 1)",
        )
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
