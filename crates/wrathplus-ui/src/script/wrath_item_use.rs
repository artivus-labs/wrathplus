//! Build-12340 item use queries: `IsCurrentItem`, `IsUsableItem`, `IsItemInRange`,
//! `ItemHasRange`, `GetItemCooldown`, `GetInventoryItemCooldown`, `GetItemCount`,
//! `GetItemFamily`, `IsEquippedItemType`, `IsConsumableItem`, `IsHarmfulItem` and
//! `IsHelpfulItem`; `GetItemGem` is [`super::wrath_item_lookups`]'s.
//!
//! An item argument is an id, an `item:` string or link, or a cached name (`0x709de0`). The
//! item's spell is its template's first on-use spell (`0x706b90`: trigger 0, nonzero id), else
//! the use-spell enchant of a worn copy ([`ItemUseView::enchant_spell`]).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::flag;
use super::wrath_item_count::get_item_count;
pub(crate) use super::wrath_item_count::item_count;
use super::wrath_spell_range as range;
use super::wrath_spell_use::SpellUse;
use super::{Model, UiScript};

/// What a template carries that [`super::ItemTemplateView`] does not, pushed with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemUseFacts {
    /// `BagFamily` (`+0x1b8`), `GetItemFamily`'s answer.
    pub bag_family: u32,
    /// The on-use (trigger 0) spell blocks with a spell, in template order.
    pub use_spells: Vec<ItemUseSpellFacts>,
}

/// One on-use template spell block, its raw template words (`0x706c50`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemUseSpellFacts {
    pub spell: u32,
    /// `SpellCharges` (`+0x128`): negative, the item is used up.
    pub charges: i32,
    /// `SpellCooldown` (`+0x13c`) and `SpellCategoryCooldown` (`+0x164`), ms; -1 the spell's.
    pub cooldown_ms: i32,
    pub category_cooldown_ms: i32,
}

/// One item's live answers by item id, computed by the app each frame: for a carried or worn
/// item every field, for any other cached template with a use spell its cooldown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemUseView {
    /// `IsUsableItem`'s pair: usable, and unusable only for lack of power.
    pub usable: bool,
    pub not_enough_mana: bool,
    /// The use spell's cooldown (`0x807980`), `(start_ms on the GetTime clock, duration_ms,
    /// enabled)`; `None` when cold.
    pub cooldown: Option<(i64, u32, bool)>,
    /// For a template without a use spell, a worn copy's use-spell enchant (`0x5a86b0`, type 7
    /// through `0x707c60`): `(spell, the enchant row's charges)` (`0x706f40`).
    pub enchant_spell: Option<(u32, i32)>,
    /// The copies' summed `|ITEM_FIELD_SPELL_CHARGES|` (`0x754d50` → `0x753f60`), without and
    /// with the bank, for `GetItemCount`'s `includeCharges`.
    pub charges: [u32; 2],
}

impl UiScript {
    /// Push one template's [`ItemUseFacts`] beside [`UiScript::set_item_template`].
    pub fn set_wrath_item_use_facts(&mut self, item: u32, facts: ItemUseFacts) {
        self.use_state().item_facts.insert(item, facts);
    }

    /// Replace the gem enchant map (`SpellItemEnchantment` id → gem item), once per data load.
    pub fn set_wrath_gem_enchants(&mut self, map: std::collections::HashMap<u32, u32>) {
        self.use_state().gem_enchants = map;
    }

    /// Replace the items' live answers, keyed by item id.
    pub fn set_wrath_item_use(&mut self, items: std::collections::HashMap<u32, ItemUseView>) {
        self.use_state().items = items;
    }

    /// Replace `IsEquippedItemType`'s `ItemSubClassMask.dbc` rows: `(class, subclass mask, name)`.
    pub fn set_wrath_subclass_masks(&mut self, rows: Vec<(u32, u32, String)>) {
        self.use_state().subclass_masks = rows;
    }
}

/// The shared item argument ([`super::item_arg::item_id_arg`]), 0 for none.
fn item_of(lua: &Lua, model: &Model, v: Option<&Value>) -> u32 {
    v.map_or(0, |v| super::item_arg::item_id_arg(lua, model, v))
}

/// The item's spell: the template's first on-use spell (`0x706b90`), else, when it has none, the
/// use-spell enchant (type 7) of a worn copy (`0x5a86b0` → `0x707c60`), which the app resolves.
fn use_spell(state: &SpellUse, item: u32) -> Option<u32> {
    let template = state
        .item_facts
        .get(&item)
        .and_then(|f| f.use_spells.first())
        .map(|s| s.spell);
    template.or_else(|| state.items.get(&item)?.enchant_spell.map(|(s, _)| s))
}

/// [`use_spell`]'s Spell.dbc row (`0x4cfd20`).
fn use_record<'m>(
    model: &'m Model,
    state: &SpellUse,
    item: u32,
) -> Option<&'m super::SpellInfoView> {
    model.spell_catalog.get(&use_spell(state, item)?)
}

/// An item's use-spell cooldown as start, duration, enable in seconds (`* 0.001`, `0x9e56b0`):
/// (0, 0, 0) without a use spell, its running cooldown (`0x807980`), else (0, 0, 1).
fn cooldown_values(lua: &Lua, state: &SpellUse, item: u32) -> MultiValue {
    let spell = use_spell(state, item).filter(|&s| (s as i32) > 0);
    let now = super::clock::now(lua);
    let cd = state
        .items
        .get(&item)
        .and_then(|v| v.cooldown)
        .map(|(s, d, e)| (s as f64 / 1000.0, f64::from(d) / 1000.0, e));
    let (start, duration, enable) = match (spell, cd) {
        (None, _) => (0.0, 0.0, 0),
        (Some(_), Some((s, d, e))) if s + d > now || !e => (s, d, i64::from(e)),
        (Some(_), _) => (0.0, 0.0, 1),
    };
    MultiValue::from_vec(vec![
        Value::Number(start),
        Value::Number(duration),
        Value::Integer(enable),
    ])
}

/// A one-argument item predicate binding.
fn item_binding<F>(lua: &Lua, answer: F) -> mlua::Result<mlua::Function>
where
    F: Fn(&Lua, &Model, &SpellUse, u32, &MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
{
    lua.create_function(move |lua, args: MultiValue| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let state = lua.app_data_ref::<SpellUse>().expect("spell use app_data");
        let item = item_of(lua, &model, args.front());
        answer(lua, &model, &state, item, &args)
    })
}

/// [`item_binding`] that first asks for the item's template when it is uncached and `asks`
/// allows: `0x67ca30` with the request callback `0x80abe0`.
fn asking_binding<A, F>(lua: &Lua, asks: A, answer: F) -> mlua::Result<mlua::Function>
where
    A: Fn(&Model, u32) -> bool + Send + 'static,
    F: Fn(&Lua, &Model, &SpellUse, u32, &MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
{
    lua.create_function(move |lua, args: MultiValue| {
        let (item, ask) = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let item = item_of(lua, &model, args.front());
            let uncached = item != 0 && !model.item_templates.contains_key(&item);
            (item, uncached && asks(&model, item))
        };
        if ask {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.item_stat_asks.insert(item);
        }
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let state = lua.app_data_ref::<SpellUse>().expect("spell use app_data");
        answer(lua, &model, &state, item, &args)
    })
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

/// Whether a worn item matches `IsEquippedItemType`'s non-slot form (`0x517740`): its
/// `ItemClass` name, its `ItemSubClass` display or verbose name, or the name of an
/// `ItemSubClassMask.dbc` row of its class whose mask has its subclass.
fn class_matches(state: &SpellUse, t: &super::ItemTemplateView, want: &str) -> bool {
    let named = [&t.item_type, &t.item_sub_type, &t.sub_class_display]
        .into_iter()
        .flatten()
        .any(|n| n.eq_ignore_ascii_case(want));
    named
        || state.subclass_masks.iter().any(|(class, mask, name)| {
            *class == t.class
                && t.subclass < 32
                && mask & (1 << t.subclass) != 0
                && name.eq_ignore_ascii_case(want)
        })
}

/// `IsEquippedItemType`'s slot keys (`0xac7fd8`), `InventoryType` 1..=28.
const INVTYPE_KEYS: [&str; 28] = [
    "INVTYPE_HEAD",
    "INVTYPE_NECK",
    "INVTYPE_SHOULDER",
    "INVTYPE_BODY",
    "INVTYPE_CHEST",
    "INVTYPE_WAIST",
    "INVTYPE_LEGS",
    "INVTYPE_FEET",
    "INVTYPE_WRIST",
    "INVTYPE_HAND",
    "INVTYPE_FINGER",
    "INVTYPE_TRINKET",
    "INVTYPE_WEAPON",
    "INVTYPE_SHIELD",
    "INVTYPE_RANGED",
    "INVTYPE_CLOAK",
    "INVTYPE_2HWEAPON",
    "INVTYPE_BAG",
    "INVTYPE_TABARD",
    "INVTYPE_ROBE",
    "INVTYPE_WEAPONMAINHAND",
    "INVTYPE_WEAPONOFFHAND",
    "INVTYPE_HOLDABLE",
    "INVTYPE_AMMO",
    "INVTYPE_THROWN",
    "INVTYPE_RANGEDRIGHT",
    "INVTYPE_QUIVER",
    "INVTYPE_RELIC",
];

/// `0x51c740`'s match: a key or its `GlobalStrings` text names an `InventoryType`.
fn inventory_type_of(lua: &Lua, want: &str) -> Option<u32> {
    let g = lua.globals();
    INVTYPE_KEYS
        .iter()
        .position(|key| {
            key.eq_ignore_ascii_case(want)
                || g.get::<Option<String>>(*key)
                    .ok()
                    .flatten()
                    .is_some_and(|text| text.eq_ignore_ascii_case(want))
        })
        .map(|i| i as u32 + 1)
}

/// `IsUsableItem`'s arena gate (`0x51c450`): on an arena map (`Map.dbc` `InstanceType` 4) a
/// template without `ITEM_FLAG_USEABLE_IN_ARENA` (`0x200000`) is refused when its use spell's
/// block spends the item or its cooldown or category cooldown passes 15 minutes. For an enchant
/// spell (`0x706f40`, `0x707e90`) the enchant's charges are read; its cooldown pair is the
/// spell's own, which these facts do not carry (inferred: never past 15 minutes).
fn arena_refuses(model: &Model, state: &SpellUse, item: u32) -> bool {
    let arena = model
        .wrath_state
        .instance
        .as_ref()
        .is_some_and(|i| i.instance_type == 4);
    let Some(t) = model.item_templates.get(&item).filter(|_| arena) else {
        return false;
    };
    if t.flags & 0x20_0000 != 0 {
        return false;
    }
    let block = state
        .item_facts
        .get(&item)
        .and_then(|f| f.use_spells.first());
    match (block, state.items.get(&item).and_then(|v| v.enchant_spell)) {
        (Some(b), _) => {
            b.charges < 0 || b.cooldown_ms > 900_000 || b.category_cooldown_ms > 900_000
        }
        (None, Some((_, charges))) => charges < 0,
        (None, None) => false,
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // IsCurrentItem(item) (`0x5110d0`) → 1 while one of the player's casts in flight
    // (`0x805fc0`'s walk of `0xaf5254`) was cast from a copy of the item.
    g.set(
        "IsCurrentItem",
        item_binding(lua, |_, _, state, item, _| {
            one(flag(item != 0 && state.cast_items.contains(&item)))
        })?,
    )?;
    // IsUsableItem(item) (`0x51c450`) → usable, notEnoughMana, 1 or nil each: a carried or worn
    // item ([`ItemUseView`], the walk with the item), less [`arena_refuses`]. A held item's
    // uncached template is asked for: with a player and a count (`0x51c4e3`), `0x80ac10` passes
    // the request callback (`0x80ac26`).
    let held =
        |model: &Model, item| model.unit_guids.player != 0 && item_count(model, item, false) != 0;
    g.set(
        "IsUsableItem",
        asking_binding(lua, held, |_, model, state, item, _| {
            let v = state
                .items
                .get(&item)
                .copied()
                .filter(|_| {
                    item_count(model, item, false) != 0 && !arena_refuses(model, state, item)
                })
                .unwrap_or_default();
            Ok(MultiValue::from_vec(vec![
                flag(v.usable),
                flag(v.not_enough_mana),
            ]))
        })?,
    )?;
    // IsItemInRange(item[, unit]) (`0x51c9c0`) → the use spell's `IsSpellInRange` answer, the
    // player casting: 1, 0, or nil.
    g.set(
        "IsItemInRange",
        item_binding(lua, |_, model, state, item, args| {
            let answer = use_record(model, state, item).and_then(|r| {
                range::in_range(model, state, r, model.unit_guids.player, args.get(1))
            });
            one(answer.map_or(Value::Nil, |b| Value::Integer(i64::from(b))))
        })?,
    )?;
    // ItemHasRange(item) (`0x51c870`) → 1 when the use spell's target-less range is nonzero.
    g.set(
        "ItemHasRange",
        item_binding(lua, |_, model, state, item, _| {
            let hit = use_record(model, state, item)
                .is_some_and(|r| range::has_range(state, r, model.unit_guids.player));
            one(flag(hit))
        })?,
    )?;
    // GetItemCooldown(item) (`0x510fc0`) → [`cooldown_values`]. The app prices every cached
    // template with a use spell. An uncached template is asked for: `0x809030` passes the request
    // callback (`0x809046`) for any nonzero id.
    g.set(
        "GetItemCooldown",
        asking_binding(
            lua,
            |_, _| true,
            |lua, _, state, item, _| Ok(cooldown_values(lua, state, item)),
        )?,
    )?;
    // GetInventoryItemCooldown(unit, slot) (`0x5e7e60`) → zero values for a slot the reader
    // (`0x5e7c00`) refuses; (0, 0, 0) for the ammo slot, a unit not the player or the inspected
    // one (`0x5e7a00`) or an empty slot; else the worn item's [`cooldown_values`] (`0x8090c0` →
    // `0x807980` with `0x707c60`'s spell). Labelled: an item whose charges are spent (`0x706f40`,
    // `0x707dc0`), (0, 0, 0) natively, reads its cooldown.
    g.set(
        "GetInventoryItemCooldown",
        lua.create_function(|lua, (unit, slot): (Value, Value)| {
            let usage = "Usage: GetInventoryItemCooldown(unit, slot)";
            let token = super::binding_abi::string_arg(lua, unit, usage)?;
            let slot0 = super::binding_abi::inventory_slot0(lua, slot)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            if !model.inventory_slot_reader_accepts(slot0) {
                return Ok(MultiValue::new());
            }
            let worn = usize::try_from(slot0 + 1).ok().filter(|&i| i > 0);
            let item = worn.and_then(|i| model.inv_slot(&token, i));
            let state = lua.app_data_ref::<SpellUse>().expect("spell use app_data");
            Ok(cooldown_values(lua, &state, item.map_or(0, |v| v.item_id)))
        })?,
    )?;
    // GetItemCount(item[, includeBank[, includeCharges]]) (`0x51c2e0`) → the carried count
    // ([`get_item_count`]); both flags read through `GetBoolOrDefault` (`0x51c2f1`, `0x51c2fd`).
    g.set(
        "GetItemCount",
        item_binding(lua, |_, model, state, item, args| {
            let flag_at = |i: usize| super::binding_abi::bool_or_default(args.get(i), false);
            let n = get_item_count(model, state, item, flag_at(1), flag_at(2));
            one(Value::Integer(i64::from(n)))
        })?,
    )?;
    // GetItemFamily(item) (`0x510f20`) → the template's `BagFamily`; nothing while it is not
    // cached (`0x67ca30` asked without a callback).
    g.set(
        "GetItemFamily",
        item_binding(lua, |_, _, state, item, _| {
            match state.item_facts.get(&item) {
                Some(f) => one(Value::Integer(i64::from(f.bag_family))),
                None => Ok(MultiValue::new()),
            }
        })?,
    )?;
    // IsEquippedItemType("type") (`0x51c740`) → 1 when a worn item (equipment slots only, walk
    // flag 1) has the slot an `INVTYPE_*` key or its text names, else whose class or subclass
    // name is `type`. Inferred: both compares ignore case. A missing or empty type raises.
    g.set(
        "IsEquippedItemType",
        lua.create_function(|lua, kind: Value| {
            const USAGE: &str = "Usage: IsEquippedItemType(\"type\")";
            let want = super::binding_abi::optional_string(lua, &kind).unwrap_or_default();
            if want.is_empty() {
                return Err(mlua::Error::runtime(USAGE));
            }
            let slot_type = inventory_type_of(lua, &want);
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let state = lua.app_data_ref::<SpellUse>().expect("spell use app_data");
            let hit = model.inventory_slots[1..=19]
                .iter()
                .flatten()
                .filter_map(|s| model.item_templates.get(&s.item_id))
                .any(|t| match slot_type {
                    Some(it) => t.inventory_type == it,
                    None => class_matches(&state, t, &want),
                });
            Ok(flag(hit))
        })?,
    )?;
    // IsConsumableItem(item) (`0x5173e0`) → 1 for ammo or thrown (`InventoryType` 24, 25), or an
    // on-use spell with a reagent or negative charges; nil for an uncached template.
    g.set(
        "IsConsumableItem",
        item_binding(lua, |_, model, state, item, _| {
            let thrown = model
                .item_templates
                .get(&item)
                .is_some_and(|t| matches!(t.inventory_type, 24 | 25));
            let spends = state.item_facts.get(&item).is_some_and(|f| {
                f.use_spells.iter().any(|b| {
                    model.spell_catalog.get(&b.spell).is_some_and(|r| {
                        b.charges < 0 || r.reagents.iter().any(|&(id, n)| id != 0 && n > 0)
                    })
                })
            });
            one(flag(thrown || spends))
        })?,
    )?;
    // IsHarmfulItem (`0x5172e0`) and IsHelpfulItem (`0x5171f0`) → 1 when the use spell's
    // classifier (`0x7fe1b0`) answers 2 or 1.
    for (verb, want) in [("IsHarmfulItem", 2u8), ("IsHelpfulItem", 1)] {
        g.set(
            verb,
            item_binding(lua, move |_, model, state, item, _| {
                let hit = use_spell(state, item)
                    .and_then(|s| model.spell_catalog.get(&s))
                    .is_some_and(|r| r.polarity == want);
                one(flag(hit))
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "wrath_item_use_tests.rs"]
mod tests;
