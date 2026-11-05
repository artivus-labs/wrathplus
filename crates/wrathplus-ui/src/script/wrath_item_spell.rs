//! GetItemSpell's template on-use path (installed ChatFrame.lua:684,844).
//! Build 12340: 0x706b90 selects the first nonzero trigger-0 spell; 0x517100 returns
//! Spell.dbc name/rank, not an ID. Missing rows do not promote later template entries.
use super::Model;
use mlua::{Lua, Value};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "GetItemSpell",
        lua.create_function(|lua, query: Value| {
            let result = {
                let mut model = lua.app_data_mut::<Model>().expect("model");
                // The shared item resolver (`0x51711e`); a negative id names no record to ask for.
                let id = super::item_arg::item_id_arg(lua, &model, &query);
                if (id as i32) > 0 && !model.item_templates.contains_key(&id) {
                    model.item_stat_asks.insert(id);
                }
                let spell = model
                    .item_templates
                    .get(&id)
                    .and_then(|row| {
                        row.item_spells
                            .iter()
                            .find(|(trigger, id, _)| *trigger == 0 && *id != 0)
                    })
                    .and_then(|(_, id, _)| model.spell_catalog.get(id));
                // TODO: held-item enchant fallback (effect type 7) awaits verified inventory
                // scope and a host snapshot. Do not guess it from tooltip trigger descriptions.
                spell.map(|row| (row.name.clone(), row.rank.clone().unwrap_or_default()))
            };
            Ok(match result {
                Some((name, rank)) => (Some(name), Some(rank)),
                None => (None, None),
            })
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::super::{ItemTemplateView, SpellInfoView, UiScript};
    fn script() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_spell_catalog([
            SpellInfoView {
                spell_id: 10,
                name: "First Use".into(),
                rank: Some("Rank 2".into()),
                ..Default::default()
            },
            SpellInfoView {
                spell_id: 20,
                name: "Later Use".into(),
                ..Default::default()
            },
        ]);
        s.set_item_template(
            100,
            ItemTemplateView {
                name: "Test Trinket".into(),
                item_spells: vec![
                    (1, 20, None),
                    (5, 20, None),
                    (6, 20, None),
                    (0, 0, None),
                    (0, 10, None),
                    (0, 20, None),
                ],
                ..Default::default()
            },
        );
        s
    }
    #[test]
    fn template_order_returns_two_strings_from_catalog() {
        let s = script();
        s.run(r#"for _, q in ipairs({100, '100', 'item:100:0:0', '|cffffffff|Hitem:100:0|h[Test Trinket]|h|r', 'test trinket'}) do
            assert(select('#',GetItemSpell(q))==2)
            local name, rank=GetItemSpell(q); assert(name=='First Use' and rank=='Rank 2')
        end"#).unwrap();
    }
    #[test]
    fn missing_first_row_does_not_promote_and_empty_rank_is_string() {
        let mut s = script();
        s.set_item_template(
            101,
            ItemTemplateView {
                item_spells: vec![(0, 999, Some("Not a catalog row".into())), (0, 20, None)],
                ..Default::default()
            },
        );
        s.set_item_template(
            102,
            ItemTemplateView {
                item_spells: vec![(0, 20, None)],
                ..Default::default()
            },
        );
        s.run("local a,b=GetItemSpell(101); assert(a==nil and b==nil); assert(select('#',GetItemSpell(101))==2); local c,d=GetItemSpell(102); assert(c=='Later Use' and d==''); assert(GetItemSpell(999)==nil)").unwrap();
    }
}
