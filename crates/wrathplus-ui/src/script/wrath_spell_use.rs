//! Build-12340 spell use queries: `IsUsableSpell`, `IsSpellInRange`, `SpellHasRange`,
//! `IsAttackSpell`, `IsAutoRepeatSpell`, `IsHarmfulSpell`, `IsHelpfulSpell`, `IsConsumableSpell`,
//! `GetSpellCount` and `FindSpellBookSlotByID`.
//!
//! All but the last read their spell through the shared reader `0x540670`: a `(slot, bookType)`
//! pair, or a name with an optional rank or unit. The static answers come from the Spell.dbc
//! catalog ([`super::SpellInfoView`]); usability from [`SpellUseView`], which the app computes
//! with the action bar's walk for each book spell (the pet as caster for the pet's book); range
//! from [`super::wrath_spell_range`] over the units the app pushes.

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::flag;
use super::wrath_spell_range::{self as range, RangeUnitFacts};
use super::{Model, UiScript};

/// One book spell's `IsUsableSpell` pair, computed by the app each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpellUseView {
    /// The usable walk (`0x80b5d0`) passes and the power check (`0x8017e0`) too.
    pub usable: bool,
    /// The walk passes but the power check fails.
    pub not_enough_mana: bool,
}

/// The live state the use queries read, apart from [`Model`].
#[derive(Debug, Default)]
pub(crate) struct SpellUse {
    /// The player book's pairs by spell id.
    pub(crate) spells: HashMap<u32, SpellUseView>,
    /// The pet book's pairs by spell id, the pet as the caster (`0x5d3390`).
    pub(crate) pet_spells: HashMap<u32, SpellUseView>,
    /// The carried, worn and cached items' live answers by item id.
    pub(crate) items: HashMap<u32, super::wrath_item_use::ItemUseView>,
    /// The two auto-repeat words: the running spell (`0xd397d0`) and the queued one (`0xd397cc`).
    pub(crate) auto_repeat: [Option<u32>; 2],
    pub(crate) item_facts: HashMap<u32, super::wrath_item_use::ItemUseFacts>,
    /// `SpellItemEnchantment` id → its `GemID` (`+0x44`), for `GetItemGem`'s link form.
    pub(crate) gem_enchants: HashMap<u32, u32>,
    /// The units the range test reads, by guid: the player, the pet and every held unit.
    pub(crate) units: HashMap<u64, RangeUnitFacts>,
    /// The talent range modifiers (op 5) by spell: `(flat, percent)` (`0x7fd970`).
    pub(crate) range_mods: HashMap<u32, (i32, i32)>,
    /// The ranged-slot item's `RangedModRange`, the `Attributes & 2` scale (`0x7ff480`).
    pub(crate) ranged_mod: Option<f32>,
    /// `ItemSubClassMask.dbc` rows `(class, subclass mask, name)`, for `IsEquippedItemType`.
    pub(crate) subclass_masks: Vec<(u32, u32, String)>,
    /// The item entries the player's casts in flight were cast from (`0x805fc0`).
    pub(crate) cast_items: Vec<u32>,
}

impl UiScript {
    /// Replace the book spells' pairs: the player's and the pet's.
    pub fn set_wrath_spell_use(
        &mut self,
        player: HashMap<u32, SpellUseView>,
        pet: HashMap<u32, SpellUseView>,
    ) {
        let mut s = self.use_state();
        s.spells = player;
        s.pet_spells = pet;
    }

    /// The auto-repeat words, `[running, queued]`, `None` for an empty one.
    pub fn set_wrath_auto_repeat_spell(&mut self, words: [Option<u32>; 2]) {
        self.use_state().auto_repeat = words;
    }

    /// Replace the range inputs: the units, the spells' range modifiers and the ranged scale.
    pub fn set_wrath_range_inputs(
        &mut self,
        units: HashMap<u64, RangeUnitFacts>,
        mods: HashMap<u32, (i32, i32)>,
        ranged_mod: Option<f32>,
    ) {
        let mut s = self.use_state();
        s.units = units;
        s.range_mods = mods;
        s.ranged_mod = ranged_mod;
    }

    /// The item entries of the player's casts in flight that a copy of an item started.
    pub fn set_wrath_cast_items(&mut self, items: Vec<u32>) {
        self.use_state().cast_items = items;
    }

    /// The books' spell ids, `(player, pet)`, the sets the app computes [`SpellUseView`]s for.
    pub fn wrath_spell_use_book_ids(&self) -> (Vec<u32>, Vec<u32>) {
        let model = self.model_ref();
        let ids = |b: &[super::SpellSlotView]| b.iter().map(|s| s.spell_id).collect();
        (ids(&model.spellbook.slots), ids(&model.pet_book.slots))
    }

    pub(super) fn use_state(&self) -> mlua::AppDataRefMut<'_, SpellUse> {
        self.lua
            .app_data_mut::<SpellUse>()
            .expect("spell use app_data")
    }
}

/// The spell `0x540670` resolved: the spell id behind the slot (0 for an empty slot, which every
/// binding answers like a spell with no record), its book, and the 1-based index of the argument
/// after the spell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpellArg {
    pub(crate) spell_id: u32,
    pub(crate) pet: bool,
    pub(crate) next: usize,
}

fn is_number(lua: &Lua, v: Option<&Value>) -> bool {
    v.is_some_and(|v| lua.coerce_number(v.clone()).ok().flatten().is_some())
}

fn as_string(lua: &Lua, v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(_) | Value::Integer(_) | Value::Number(_) => lua
            .coerce_string(v?.clone())
            .ok()
            .flatten()
            .map(|s| s.to_string_lossy()),
        _ => None,
    }
}

/// `0x540670`. `(slot, bookType)` when the first argument is a number and the second a string or
/// number: the slot less one must fall in `[0, 0x400)` (`0x5406a5`..`0x5406c2`), else
/// `"<verb>(): Invalid spell slot"` raises, and `"pet"` (`0x9ff164`) selects the pet's book.
/// Otherwise a string names the spell: with a second string that is not a unit token, that is
/// its rank (`0x5401d0`); else the name may carry `!` and `(Rank)` (`0x540200`), and a unit, if
/// any, is the second argument. A name that misses, or names a spell outside its book
/// (`0x53b4e0`), answers `Ok(None)`: the bindings then return nothing.
///
/// Inferred: the name map (`0x53f5e0`) is read as the two books, the player's first, and a name
/// without a rank as its last (highest) rank in book order.
pub(crate) fn spell_arg(
    lua: &Lua,
    model: &Model,
    args: &MultiValue,
    verb: &str,
) -> mlua::Result<Option<SpellArg>> {
    let (a1, a2) = (args.front(), args.get(1));
    let a2_string = as_string(lua, a2);
    if is_number(lua, a1) && a2_string.is_some() {
        let n = lua
            .coerce_number(a1.cloned().unwrap_or(Value::Nil))?
            .unwrap_or(0.0);
        let index = (n - 1.0).trunc();
        if !(0.0..1024.0).contains(&index) {
            return Err(mlua::Error::runtime(format!(
                "{verb}(): Invalid spell slot"
            )));
        }
        let pet = a2_string.is_some_and(|b| b.eq_ignore_ascii_case("pet"));
        let book = if pet {
            &model.pet_book.slots
        } else {
            &model.spellbook.slots
        };
        let spell_id = book.get(index as usize).map_or(0, |s| s.spell_id);
        return Ok(Some(SpellArg {
            spell_id,
            pet,
            next: 3,
        }));
    }
    let Some(name) = as_string(lua, a1) else {
        return Err(mlua::Error::runtime(format!(
            "{verb}(): Invalid spell slot"
        )));
    };
    let rank = a2_string.filter(|s| model.unit_guids.resolve(s).is_err());
    let found = match &rank {
        Some(rank) => lookup(model, &name, Some(rank)).map(|f| (f, 3)),
        None => by_name(model, &name).map(|f| (f, 2)),
    };
    Ok(found.map(|((spell_id, pet), next)| SpellArg {
        spell_id,
        pet,
        next,
    }))
}

/// `0x540200`: the whole string (less a leading `!`) as a name, else the text before the first
/// `(` as the name, uncut, and up to the `)` as the rank.
fn by_name(model: &Model, text: &str) -> Option<(u32, bool)> {
    let text = text.strip_prefix('!').unwrap_or(text);
    lookup(model, text, None).or_else(|| {
        let (name, rest) = text.split_once('(')?;
        let rank = rest.split(')').next().unwrap_or("");
        lookup(model, name, Some(rank))
    })
}

/// `0x53f5e0` then `0x53b4e0`: a known spell by name and rank, both case-insensitive; an empty
/// rank is no rank.
fn lookup(model: &Model, name: &str, rank: Option<&str>) -> Option<(u32, bool)> {
    if name.is_empty() {
        return None;
    }
    let rank = rank.filter(|r| !r.is_empty());
    let find = |slots: &[super::SpellSlotView]| {
        slots
            .iter()
            .rfind(|s| {
                s.name.eq_ignore_ascii_case(name)
                    && rank.is_none_or(|r| {
                        s.rank
                            .as_deref()
                            .is_some_and(|have| have.eq_ignore_ascii_case(r))
                    })
            })
            .map(|s| s.spell_id)
    };
    find(&model.spellbook.slots)
        .map(|id| (id, false))
        .or_else(|| find(&model.pet_book.slots).map(|id| (id, true)))
}

/// A binding over `0x540670`: nothing when the name misses, else `answer`'s values.
fn spell_binding<F>(lua: &Lua, verb: &'static str, answer: F) -> mlua::Result<mlua::Function>
where
    F: Fn(&Lua, &Model, &SpellUse, SpellArg, &MultiValue) -> mlua::Result<MultiValue>
        + Send
        + 'static,
{
    lua.create_function(move |lua, args: MultiValue| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(arg) = spell_arg(lua, &model, &args, verb)? else {
            return Ok(MultiValue::new());
        };
        let state = lua.app_data_ref::<SpellUse>().expect("spell use app_data");
        answer(lua, &model, &state, arg, &args)
    })
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

/// The catalog row of a resolved spell, `None` for an empty slot or an unknown id (`0x4cfd20`).
fn record(model: &Model, arg: SpellArg) -> Option<&super::SpellInfoView> {
    model.spell_catalog.get(&arg.spell_id)
}

/// The pair of a book spell: the player's, or the pet's with the pet as the caster.
fn live(state: &SpellUse, arg: SpellArg) -> Option<SpellUseView> {
    let book = if arg.pet {
        &state.pet_spells
    } else {
        &state.spells
    };
    book.get(&arg.spell_id).copied()
}

/// The caster `0x541c60` and `0x541af0` hand the range code: the player, or for the pet's book
/// the pet (`0x5d3390`); `None` without one.
fn caster(model: &Model, arg: SpellArg) -> Option<u64> {
    let g = if arg.pet {
        model.unit_guids.pet
    } else {
        model.unit_guids.player
    };
    (g != 0).then_some(g)
}

/// `0x7ffa30`: how many casts the carried reagents allow, the least `count / ReagentCount` over
/// the set reagents (id not 0 or -2, a positive count); 0 for a spell without any, or with no
/// player.
pub(crate) fn reagent_casts(model: &Model, r: &super::SpellInfoView) -> u32 {
    if model.unit_guids.player == 0 {
        return 0;
    }
    r.reagents
        .iter()
        .filter(|&&(id, n)| id != 0 && id != 0xffff_fffe && n > 0)
        .map(|&(id, n)| super::wrath_item_use::item_count(model, id, false) / n)
        .min()
        .unwrap_or(0)
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(SpellUse::default());
    let g = lua.globals();
    // IsUsableSpell(spell) (`0x541680`) → usable, notEnoughMana: 1 or nil each, always two.
    g.set(
        "IsUsableSpell",
        spell_binding(lua, "IsUsableSpell", |_, model, state, arg, _| {
            let v = record(model, arg).and(live(state, arg)).unwrap_or_default();
            Ok(MultiValue::from_vec(vec![
                flag(v.usable),
                flag(v.not_enough_mana),
            ]))
        })?,
    )?;
    // IsSpellInRange(spell[, unit]) (`0x541c60`) → 1 or 0 from the range test, nil when the
    // unit names nobody or the spell cannot take it (`0x809610`).
    g.set(
        "IsSpellInRange",
        spell_binding(lua, "IsSpellInRange", |_, model, state, arg, args| {
            let answer = record(model, arg)
                .zip(caster(model, arg))
                .and_then(|(r, c)| range::in_range(model, state, r, c, args.get(arg.next - 1)));
            one(answer.map_or(Value::Nil, |b| Value::Integer(i64::from(b))))
        })?,
    )?;
    // SpellHasRange(spell) (`0x541af0`) → 1 when the target-less range (`0x801650`) is nonzero.
    g.set(
        "SpellHasRange",
        spell_binding(lua, "SpellHasRange", |_, model, state, arg, _| {
            let hit = record(model, arg)
                .zip(caster(model, arg))
                .is_some_and(|(r, c)| range::has_range(state, r, c));
            one(flag(hit))
        })?,
    )?;
    // IsAttackSpell(spell) (`0x541420`) → 1 when `Effect[0]` (`+0x11c`) is 78, ATTACK.
    g.set(
        "IsAttackSpell",
        spell_binding(lua, "IsAttackSpell", |_, model, _, arg, _| {
            one(flag(record(model, arg).is_some_and(|r| r.attack)))
        })?,
    )?;
    // IsAutoRepeatSpell(spell) (`0x5415d0`) → 1 for a player-book spell that is either
    // auto-repeat word, the running one (`0xd397d0`) or the queued one (`0xd397cc`).
    g.set(
        "IsAutoRepeatSpell",
        spell_binding(lua, "IsAutoRepeatSpell", |_, _, state, arg, _| {
            let hit =
                !arg.pet && arg.spell_id != 0 && state.auto_repeat.contains(&Some(arg.spell_id));
            one(flag(hit))
        })?,
    )?;
    // IsHarmfulSpell (`0x5418f0`) and IsHelpfulSpell (`0x541800`) → 1 when the classifier
    // `0x7fe1b0` answers 2 (enemies) or 1 (friends).
    for (verb, want) in [("IsHarmfulSpell", 2u8), ("IsHelpfulSpell", 1)] {
        g.set(
            verb,
            spell_binding(lua, verb, move |_, model, _, arg, _| {
                one(flag(record(model, arg).is_some_and(|r| r.polarity == want)))
            })?,
        )?;
    }
    // IsConsumableSpell(spell) (`0x5419f0`) → 1 when a `Reagent[i]` is set with a positive count.
    g.set(
        "IsConsumableSpell",
        spell_binding(lua, "IsConsumableSpell", |_, model, _, arg, _| {
            let hit = record(model, arg)
                .is_some_and(|r| r.reagents.iter().any(|&(id, n)| id != 0 && n > 0));
            one(flag(hit))
        })?,
    )?;
    // GetSpellCount(spell) (`0x540df0` → `0x801770`) → [`reagent_casts`].
    g.set(
        "GetSpellCount",
        spell_binding(lua, "GetSpellCount", |_, model, _, arg, _| {
            let casts = record(model, arg).map_or(0, |r| reagent_casts(model, r));
            one(Value::Integer(i64::from(casts)))
        })?,
    )?;
    // FindSpellBookSlotByID(spellID[, isPet]) (`0x53b6c0`) → the book index from `0x53b4e0`,
    // pushed unadjusted (`0x53b726`), so 0-based; nil when the book lacks it. A spell id that is
    // not a positive number raises the usage.
    g.set(
        "FindSpellBookSlotByID",
        lua.create_function(|lua, (id, pet): (Value, Value)| {
            const USAGE: &str = "Usage: FindSpellBookSlotByID(spellID[, isPet])";
            let id = super::binding_abi::number_arg(lua, id, USAGE)?;
            if id <= 0 {
                return Err(mlua::Error::runtime(USAGE));
            }
            let pet = super::wrath_surface::wrath_spell_queries::pet_flag(lua, pet)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let book = if pet {
                &model.pet_book.slots
            } else {
                &model.spellbook.slots
            };
            Ok(book
                .iter()
                .rposition(|s| s.spell_id == id as u32)
                .map_or(Value::Nil, |i| Value::Integer(i as i64)))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "wrath_spell_use_tests.rs"]
mod tests;
