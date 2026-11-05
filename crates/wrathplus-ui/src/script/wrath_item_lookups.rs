//! Build 12340's GetItemUniqueness and GetItemGem: the template's `ItemLimitCategory` row, which
//! the host pushes, and a socket's gem through `SpellItemEnchantment`'s `GemID`
//! ([`super::wrath_spell_use::SpellUse::gem_enchants`]).

use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};

use super::cursor::EQUIPMENT_BAG;
use super::wrath_spell_use::SpellUse;
use super::{Model, UiScript};

/// Item entry → `(ItemLimitCategory id, its row's Quantity)`, for templates whose category
/// (`+0x1ec`) names a row.
#[derive(Default)]
struct ItemLimits(HashMap<u32, (u32, u32)>);

impl UiScript {
    /// Publish the limit categories of the cached templates that carry one with a row.
    pub fn set_item_limit_categories(&mut self, limits: HashMap<u32, (u32, u32)>) {
        self.lua.set_app_data(ItemLimits(limits));
    }
}

/// `0x50f630`'s gem field `index` (1..=3) of an item string: the fields split at `:` and read by
/// `atoi` (`0x76f0d0`), the gems after id and enchant; all 0 when the id is not > 0 (`0x50f66e`).
fn link_enchant(text: &str, index: usize) -> u32 {
    let Some(at) = text.find("item:") else {
        return 0;
    };
    let fields: Vec<i64> = text[at + 5..]
        .split(':')
        .map(super::item_stats::reference_atoi)
        .collect();
    match fields.first() {
        Some(&id) if id > 0 => fields.get(index + 1).map_or(0, |&f| f as u32),
        _ => 0,
    }
}

/// `0x754b90`'s by-name find on the default walk: `0x7546f0` starts at slot 23 (`0x754727`) and
/// steps through the region table at `0xa37b10`, so the backpack, each bag slot with its bag's
/// contents right after it (`0x75482a`), the keyring, the token bag, then the equipment. The
/// match is `0x753b20`: the query's length in code points (`0x76eea0`) of the item's name
/// compared by `0x76ea40`, so the query is a prefix. A copy the default flags skip (broken, or
/// flagged `0x10`: `0x754792`-`0x7547c7`) is passed over before the compare. Answers `(bag, slot)`
/// in the `wrath_item_gems` numbering. Labelled: the compare ignores case, the token bag holds no
/// sockets and is skipped, and the name is the template's, without a random suffix (`0x707c20`).
fn named_item(model: &Model, query: &str) -> Option<(i64, u32)> {
    // An empty name finds nothing (`0x754ba7`).
    let n = query.chars().count();
    if n == 0 {
        return None;
    }
    let named = |flags: u32, durability, id: u32| {
        !super::wrath_item_count::skipped(flags, durability)
            && model.item_templates.get(&id).is_some_and(|t| {
                let head: String = t.name.chars().take(n).collect();
                t.name.chars().count() >= n && head.to_lowercase() == query.to_lowercase()
            })
    };
    let contents = |bag: i64| {
        let mut slots: Vec<(i64, u32)> = (model.containers.get(&bag))
            .map(|c| (c.slots.iter()).filter(|(_, s)| named(s.flags, s.durability, s.item_id)))
            .into_iter()
            .flatten()
            .map(|(&slot, _)| (bag, slot))
            .collect();
        slots.sort_unstable();
        slots.first().copied()
    };
    let worn = |slot: u32| {
        let s = model.inventory_slots.get(slot as usize)?.as_ref()?;
        named(s.flags, s.durability, s.item_id).then_some((EQUIPMENT_BAG, slot))
    };
    contents(0)
        .or_else(|| (1..=4i64).find_map(|bag| worn(19 + bag as u32).or_else(|| contents(bag))))
        .or_else(|| contents(-2))
        .or_else(|| (1..=19).find_map(worn))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(ItemLimits::default());
    let g = lua.globals();
    // GetItemUniqueness(item) (`0x516ed0`) → for a cached template flagged `0x80000`, -1 and 1;
    // else its limit category and that row's quantity; else nothing. The argument is
    // GetItemInfo's shape: truncated, an item string, a name, or the usage error (`0x516ffd`).
    g.set(
        "GetItemUniqueness",
        lua.create_function(|lua, item: Value| {
            const USAGE: &str = "Usage: GetItemUniqueness(itemID|\"name\"|\"itemlink\")";
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let id = super::item_arg::info_item_id(lua, &model, &item, USAGE)?;
            let Some(t) = model.item_templates.get(&id) else {
                return Ok(MultiValue::new());
            };
            let pair = if t.flags & 0x8_0000 != 0 {
                Some((-1.0, 1.0))
            } else {
                let limits = lua.app_data_ref::<ItemLimits>();
                let row = limits.and_then(|l| l.0.get(&id).copied());
                row.map(|(category, quantity)| (f64::from(category), f64::from(quantity)))
            };
            Ok(pair.map_or_else(MultiValue::new, |(a, b)| {
                MultiValue::from_vec(vec![Value::Number(a), Value::Number(b)])
            }))
        })?,
    )?;
    // GetItemGem("name"|"itemlink", index) (`0x51c150`) → the gem in socket 1-3 as name and link,
    // or nothing. A string holding `item:` (`0x51c1bc`) reads its gem field ([`link_enchant`]);
    // another string, with a player (`0x51c1e8`), the socket enchant `index + 2` (`0x518b30`) of
    // [`named_item`]. The enchant's gem (`0x65c290`, `+0x44`) answers only from the template cache
    // (no query, `0x51c25c`), its link built in the call (`0x51c293` → `0x61e360`), so an addon's
    // replaced `GetItemInfo` does not reach it. A non-string item or non-number index raises.
    g.set(
        "GetItemGem",
        lua.create_function(|lua, (item, index): (Value, Value)| {
            const USAGE: &str = "Usage: GetItemGem(\"name\"|\"itemlink\", index)";
            let text = match &item {
                Value::String(_) | Value::Integer(_) | Value::Number(_) => lua
                    .coerce_string(item)?
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default(),
                _ => return Err(mlua::Error::runtime(USAGE)),
            };
            let index = super::binding_abi::number_arg(lua, index, USAGE)?;
            let Ok(index @ 1..=3) = usize::try_from(index) else {
                return Ok(MultiValue::new());
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let gem = if text.contains("item:") {
                let state = lua.app_data_ref::<SpellUse>().expect("spell use app_data");
                state.gem_enchants.get(&link_enchant(&text, index)).copied()
            } else if model.unit_guids.player != 0 {
                named_item(&model, &text).and_then(|(bag, slot)| {
                    let row = (model.wrath_item_gems.iter()).find(|r| (r.0, r.1) == (bag, slot));
                    row.and_then(|r| r.2[index - 1])
                })
            } else {
                None
            };
            let Some((gem, t)) = gem.and_then(|g| Some((g, model.item_templates.get(&g)?))) else {
                return Ok(MultiValue::new());
            };
            let level = model.unit("player").map_or(0, |u| u.level);
            let link = super::item_stats::info_link(gem, [0; 3], level, t);
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&t.name)?),
                Value::String(lua.create_string(link)?),
            ]))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{
        ContainerSlot, ContainerState, InvSlotView, ItemTemplateView, UiScript, EQUIPMENT_BAG,
    };

    fn gem(s: &UiScript, args: &str) -> (Option<String>, Option<String>) {
        s.eval(&format!("return GetItemGem({args})")).unwrap()
    }

    #[test]
    fn uniqueness_and_gems_read_the_host_tables() {
        let mut s = UiScript::new().unwrap();
        s.model_mut().unit_guids.player = 1;
        let named = |name: &str, flags: u32| ItemTemplateView {
            name: name.into(),
            flags,
            quality: 3,
            ..Default::default()
        };
        s.set_item_template(1, named("Unique Ring", 0x8_0000));
        s.set_item_template(2, named("Limited Gem Box", 0));
        s.set_item_template(3, named("Plain", 0));
        s.set_item_template(40112, named("Bold Scarlet Ruby", 0));
        s.set_item_template(50, named("Helm", 0));
        s.set_item_limit_categories([(2, (7, 3))].into());
        let pair = |arg: &str| {
            s.eval::<(f64, f64)>(&format!("return GetItemUniqueness({arg})"))
                .unwrap()
        };
        assert_eq!(pair("1"), (-1.0, 1.0));
        assert_eq!(pair("'item:2:0'"), (7.0, 3.0));
        assert_eq!(pair("2.9"), (7.0, 3.0), "truncated");
        assert_eq!(s.arity("GetItemUniqueness(3)").unwrap(), 0);
        assert_eq!(s.arity("GetItemUniqueness(99)").unwrap(), 0);
        assert!(s.run("GetItemUniqueness(nil)").is_err());

        s.set_wrath_gem_enchants([(3312, 40112)].into());
        let ruby = Some("Bold Scarlet Ruby".to_string());
        let helm = "'|cff0070dd|Hitem:50:0:3312:0:0:0:0:0:80|h[Helm]|h|r', 1";
        assert_eq!(gem(&s, helm).0, ruby);
        assert!(gem(&s, "'item:50:0:3312:0:0', 1")
            .1
            .unwrap()
            .contains("|Hitem:40112:"));
        let socket = |args| gem(&s, args).0;
        assert_eq!(socket("'item:50:0:0:3312:0', 1"), None, "socket 1 is empty");
        assert_eq!(socket("'item:50:0:0:3312:0', 2"), ruby);
        assert_eq!(socket("'item:0:0:3312', 1"), None, "`0x50f630` needs an id");
        // By name: the first match on the walk, its socketed gem.
        s.set_container(
            0,
            Some(ContainerState {
                num_slots: 16,
                slots: [(
                    4,
                    ContainerSlot {
                        item_id: 50,
                        count: 1,
                        ..Default::default()
                    },
                )]
                .into(),
                ..Default::default()
            }),
        );
        s.set_wrath_item_gems(vec![
            (0, 4, [None, None, Some(40112)]),
            (EQUIPMENT_BAG, 1, [Some(40112), None, None]),
        ]);
        s.model_mut().inventory_slots[1] = Some(InvSlotView {
            item_id: 50,
            ..Default::default()
        });
        assert_eq!(gem(&s, "'helm', 3").0, ruby);
        assert_eq!(gem(&s, "'He', 3").0, ruby, "a prefix of the name");
        assert_eq!(
            gem(&s, "'Helm', 1").0,
            None,
            "the backpack copy comes first"
        );
        assert_eq!(s.arity("GetItemGem('Helm', 4)").unwrap(), 0);
        assert!(s.run("GetItemGem({}, 1)").is_err() && s.run("GetItemGem('Helm')").is_err());
        // The link is built in the call: a replaced GetItemInfo does not reach it.
        s.run("GetItemInfo = function() return 'hooked', 'hooked' end")
            .unwrap();
        assert_eq!(gem(&s, "'helm', 3").0, ruby);
        // A broken copy is passed over (`0x7547bb`): the walk moves on to the worn one.
        let mut model = s.model_mut();
        let pack = model.containers.get_mut(&0).unwrap();
        pack.slots.get_mut(&4).unwrap().durability = Some((0, 50));
        drop(model);
        assert_eq!(gem(&s, "'Helm', 1").0, ruby);
        // No player: the name leg answers nothing (`0x51c1f1`).
        s.model_mut().unit_guids.player = 0;
        assert_eq!(s.arity("GetItemGem('helm', 3)").unwrap(), 0);
    }
}
