//! Build 12340's spell argument, `0x540670`, which every spellbook verb shares (GetSpellName
//! `0x5407f0` through IsSpellInRange `0x541c60`, 23 call sites). It answers a slot of one of the
//! two book arrays (player `0xbe6d88`, pet `0xbe7d98`), never a bare spell id:
//!
//! - `lua_isnumber(1)` and `lua_isstring(2)` (`0x540682`, `0x54068e`; a number is a string): the
//!   slot `__ftol(arg1 - 1)`, which must fall in `[0, 0x400)`; arg 2 compared whole and without
//!   case to `"pet"` (`0x9ff164`) picks the pet book, any other string or number the player's;
//! - else, when the verb allows a name (all but CastSpell, which pushes 0 at `0x54125c`), arg 1 a
//!   string or number: with arg 2 a string that is not a unit token (`0x60abf0`), arg 1 is a name
//!   and arg 2 its rank (`0x5401d0`); otherwise arg 1 alone is a name (`0x540200`). A name that
//!   resolves to no slot answers nothing and raises nothing (`0x5407ba`);
//! - anything else raises `<Verb>(): Invalid spell slot` (`0xa0b138`).
//!
//! GetSpellInfo, GetSpellCooldown, GetSpellLink and IsCurrentSpell first take a lone number as a
//! spell id; that shortcut is theirs, not the resolver's.

use mlua::{IntoLuaMulti, Lua, MultiValue, Value};

use super::spellbook::{rank_number, SpellSlotView};
use super::unit::{parse_unit_token_in, UnitTokenParse};
use super::Model;

/// `lua_isstring`: a string, or a number Lua converts to one.
fn is_string(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Integer(_) | Value::Number(_))
}

/// The argument as `(1-based book id, "spell" | "pet")`, the shape the book verbs index with;
/// `None` is a name that resolves to no slot.
pub(super) fn spell_arg(
    lua: &Lua,
    model: &Model,
    arg1: &Value,
    arg2: &Value,
    verb: &str,
    allow_name: bool,
) -> mlua::Result<Option<(u32, String)>> {
    let at = spell_arg_typed(lua, model, arg1, arg2, verb, allow_name)?;
    Ok(at.map(|(id, book, _)| (id, book)))
}

/// [`spell_arg`] with the resolver's `argType` out (`[ebp+8]`): the Lua index of the argument
/// after the spell, 3 after a slot and book or a name and rank, else 2 (`0x5406fa`, `0x540783`,
/// `0x5407b1`). IsSpellInRange reads its unit token there (`0x541d34`).
pub(super) fn spell_arg_typed(
    lua: &Lua,
    model: &Model,
    arg1: &Value,
    arg2: &Value,
    verb: &str,
    allow_name: bool,
) -> mlua::Result<Option<(u32, String, usize)>> {
    let invalid = || mlua::Error::runtime(format!("{verb}(): Invalid spell slot"));
    let book = |pet: bool| if pet { "pet" } else { "spell" }.to_string();
    if let Some(n) = lua.coerce_number(arg1.clone())?.filter(|_| is_string(arg2)) {
        // `fsub 1.0` then `__ftol` (`0x5406a5`): truncation toward zero, so 0.5 is slot 1.
        let index = (n - 1.0).trunc();
        if !(0.0..1024.0).contains(&index) {
            return Err(invalid());
        }
        let pet = matches!(arg2, Value::String(s) if s.as_bytes().eq_ignore_ascii_case(b"pet"));
        return Ok(Some((index as u32 + 1, book(pet), 3)));
    }
    if !allow_name || !is_string(arg1) {
        return Err(invalid());
    }
    // `lua_tolstring`: a lone number is the name "5308", which no spell has.
    let name = lua
        .coerce_string(arg1.clone())?
        .map(|s| s.to_string_lossy());
    let name = name.unwrap_or_default();
    let rank = match is_string(arg2) {
        true => lua
            .coerce_string(arg2.clone())?
            .map(|s| s.to_string_lossy()),
        false => None,
    };
    let (found, arg_type) = match rank {
        Some(rank) if !is_unit_token(model, &rank) => (lookup(model, &name, &rank), 3),
        _ => (named(model, &name), 2),
    };
    // `0x53b4e0`: the last slot of the chosen book holding the spell.
    Ok(found.and_then(|(spell_id, pet)| {
        let at = book_slots(model, pet)
            .iter()
            .rposition(|s| s.spell_id == spell_id)?;
        Some((at as u32 + 1, book(pet), arg_type))
    }))
}

/// `0x60abf0` answering true: arg 2 parses as a unit token, whether or not it names anyone
/// (`0x60b00c` stores a sentinel guid and still answers 1). Two tokens the shared parse lacks are
/// read here: `none`, the whole text (`0x60afaa`), and `commentator` followed only by digits
/// (`0x60ae91`, `0x76f190`; no commentator is named here, so more text falls to the name arm at
/// `0x60afdb`). Labelled: the name arm (`0x60a630`: the player's, the group's and their pets'
/// names, as `wrath_char_state`'s `token_for_name` reads them) is not wired here, so such a name
/// reads as a rank.
fn is_unit_token(model: &Model, s: &str) -> bool {
    let commentator = s
        .get(..11)
        .is_some_and(|h| h.eq_ignore_ascii_case("commentator"))
        && s[11..].bytes().all(|b| b.is_ascii_digit());
    s.eq_ignore_ascii_case("none")
        || commentator
        || matches!(
            parse_unit_token_in(s, model.unit_guids.wrath),
            UnitTokenParse::Unit { .. }
        )
}

fn book_slots(model: &Model, pet: bool) -> &[SpellSlotView] {
    if pet {
        &model.pet_book.slots
    } else {
        &model.spellbook.slots
    }
}

/// `0x540200`, a lone name: one leading `!` stripped (`0x54021e`), the whole text looked up first,
/// then cut at its first `(` with the rank running to the next `)` (`0x540272`, `0x5402c0`). The
/// cut is untrimmed, so `"Fireball (Rank 1)"` looks up `"Fireball "` and misses.
fn named(model: &Model, query: &str) -> Option<(u32, bool)> {
    let query = query.strip_prefix('!').unwrap_or(query);
    lookup(model, query, "").or_else(|| {
        let (name, rest) = query.split_once('(')?;
        lookup(model, name, rest.split(')').next().unwrap_or(rest))
    })
}

/// `0x53f5e0` over the name table `0xbe8e64`, built as the books fill (`0x541df0`, from the book
/// add at `0x542699` and the pet's at `0x542f26`): one chain per name, without case. The player's
/// spells go before the first lower rank (`0x541f41`, by `0x53b540`'s leading number), so the head
/// is the highest; the pet's append. An empty rank takes the head, else the first node whose rank
/// matches without case (`0x76ea40`); a rankless node never matches one. Labelled: ties keep book
/// order, where the reference keeps learn order, and a digit-less rank reads 0, where `0x53b540`
/// falls back to the spell level.
fn lookup(model: &Model, name: &str, rank: &str) -> Option<(u32, bool)> {
    if name.is_empty() {
        return None;
    }
    let same = |s: &&SpellSlotView| s.name.eq_ignore_ascii_case(name);
    let mut player: Vec<&SpellSlotView> = model.spellbook.slots.iter().filter(same).collect();
    // A stable sort: equal ranks keep book order.
    player.sort_by_key(|s| std::cmp::Reverse(rank_number(s.rank.as_deref())));
    let chain = player
        .into_iter()
        .map(|s| (s, false))
        .chain(model.pet_book.slots.iter().filter(same).map(|s| (s, true)));
    let mut chain = chain;
    let hit = if rank.is_empty() {
        chain.next()
    } else {
        chain.find(|(s, _)| {
            s.rank
                .as_deref()
                .is_some_and(|r| !r.is_empty() && r.eq_ignore_ascii_case(rank))
        })
    };
    hit.map(|(s, pet)| (s.spell_id, pet))
}

/// Install `verb` over [`spell_arg`]: `f` gets the slot as `(1-based id, book type)`, and a name
/// that resolves to nothing answers no values, as every caller returns 0 on a failed resolve
/// (`0x54081a`, `0x540e17`, `0x541b1a`, …).
pub(super) fn set_slot_verb<R, F>(lua: &Lua, verb: &'static str, f: F) -> mlua::Result<()>
where
    R: IntoLuaMulti,
    F: Fn(&Lua, u32, &str) -> mlua::Result<R> + 'static,
{
    let verb_fn = lua.create_function(move |lua, (arg1, arg2): (Value, Value)| {
        let at = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            spell_arg(lua, &model, &arg1, &arg2, verb, true)?
        };
        match at {
            Some((id, book)) => f(lua, id, &book)?.into_lua_multi(lua),
            None => Ok(MultiValue::new()),
        }
    })?;
    lua.globals().set(verb, verb_fn)
}

#[cfg(test)]
mod tests {
    use crate::script::{PetBookState, SpellBookState, SpellSlotView, UiScript};

    fn slot(spell_id: u32, name: &str, rank: Option<&str>, passive: bool) -> SpellSlotView {
        SpellSlotView {
            spell_id,
            name: name.into(),
            rank: rank.map(Into::into),
            texture: Some(format!("icon{spell_id}")),
            passive,
            ..Default::default()
        }
    }

    /// The resolver's three legs, read back through GetSpellTexture (no id shortcut) and the
    /// slot-only CastSpell.
    #[test]
    fn every_spell_verb_resolves_its_argument_as_12340_does() {
        let mut s = UiScript::new().unwrap();
        s.set_spellbook(SpellBookState {
            tabs: Vec::new(),
            slots: vec![
                slot(133, "Fireball", Some("Rank 1"), false),
                slot(143, "Fireball", Some("Rank 2"), false),
                slot(81, "Dodge", None, true),
                slot(5308, "Execute", Some("Rank 1"), false),
            ],
        });
        s.set_pet_book(PetBookState {
            token: Some("PET".into()),
            slots: vec![slot(2649, "Growl", Some("Rank 1"), false)],
        });
        let texture = |args: &str| {
            s.eval::<Option<String>>(&format!("return GetSpellTexture({args})"))
                .unwrap()
        };
        let none = |args: &str| {
            s.eval::<i64>(&format!("return select('#', GetSpellTexture({args}))"))
                .unwrap()
                == 0
        };
        // Slot leg: `trunc(arg1 - 1)`; a number or any string is the player's book, "pet"
        // without case the pet's.
        for (args, want) in [
            ("1, 'spell'", "icon133"),
            ("0.5, 'spell'", "icon133"),
            ("'2', 'x'", "icon143"),
            ("4, 7", "icon5308"),
            ("1, 'PeT'", "icon2649"),
        ] {
            assert_eq!(texture(args).as_deref(), Some(want), "{args}");
        }
        // A name: the highest rank, passives included, either book, one `!` stripped, the
        // `(rank)` cut untrimmed; a unit token as arg 2 is ignored, another string is the rank.
        for (args, want) in [
            ("'fireball'", "icon143"),
            ("'Dodge'", "icon81"),
            ("'Growl'", "icon2649"),
            ("'!Fireball(Rank 1)'", "icon133"),
            ("'Fireball(rank 1'", "icon133"),
            ("'Fireball', 'target'", "icon143"),
            ("'Fireball', 'NONE'", "icon143"),
            ("'Fireball', 'commentator12'", "icon143"),
            ("'Fireball', 'RANK 1'", "icon133"),
        ] {
            assert_eq!(texture(args).as_deref(), Some(want), "{args}");
        }
        // Misses answer no values and raise nothing: a spaced cut, a rank string after a name, an
        // unknown name, a lone number (the name "1"), a book type that is not a unit token.
        for args in [
            "'Fireball (Rank 1)'",
            "'Fireball', 'spell'",
            "'Fireball', 'commentatortarget'",
            "'!!Fireball'",
            "'Dodge', 'Rank 1'",
            "'Nope'",
            "1",
            "5308",
            "1, nil",
        ] {
            assert!(none(args), "{args}");
        }
        // Inside `[0, 0x400)` but past the book: the verb's own empty answer, not an error.
        assert_eq!(texture("1024, 'spell'"), None);
        for bad in [
            "0, 'spell'",
            "-11, 'spell'",
            "1025, 'spell'",
            "nil",
            "{}",
            "{}, 'spell'",
        ] {
            let err = s
                .eval::<mlua::Value>(&format!("return GetSpellTexture({bad})"))
                .expect_err(bad)
                .to_string();
            assert!(
                err.contains("GetSpellTexture(): Invalid spell slot"),
                "{bad}: {err}"
            );
        }
        // CastSpell takes no name (`0x54125c` pushes 0): the book type is not optional there.
        for bad in ["'Fireball'", "1"] {
            let err = s.run(&format!("CastSpell({bad})")).expect_err(bad);
            assert!(err.to_string().contains("CastSpell(): Invalid spell slot"));
        }
    }
}
