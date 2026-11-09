//! Item-by-name keeps its unit intent through normal item routing.
use super::*;

pub(super) fn install_by_name(lua: &Lua) -> mlua::Result<()> {
    // UseItemByName(nameOrId [, target]) (`0x51cd00`): the first bag slot holding the item is
    // used, carrying explicit unit intent into the ordered cast ladder.
    set(lua, "UseItemByName", |lua, args| {
        if !has_player(lua) {
            return none();
        }
        let unit =
            super::super::unit::token::token_text(lua, &arg(&args, 1)).filter(|t| !t.is_empty());
        let key = arg(&args, 0);
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        if unit.as_ref().is_some_and(|t| !model.resolve_token(t).ok) {
            return none();
        }
        let id = lua
            .coerce_number(key.clone())
            .ok()
            .flatten()
            .map(|n| n as u32);
        let name = match (&key, id) {
            (_, Some(_)) => None,
            (v, None) => lua.coerce_string(v.clone())?.map(|s| s.to_string_lossy()),
        };
        let mut bags: Vec<i64> = model.containers.keys().copied().collect();
        bags.sort_unstable();
        let found = bags.into_iter().find_map(|bag| {
            let c = model.containers.get(&bag)?;
            let mut slots: Vec<(&u32, &super::super::container::ContainerSlot)> =
                c.slots.iter().collect();
            slots.sort_by_key(|(s, _)| **s);
            slots
                .into_iter()
                .find(|(_, s)| {
                    s.item_id != 0
                        && match (&name, id) {
                            (_, Some(id)) => s.item_id == id,
                            (Some(n), None) => {
                                super::super::cursor::item_link_name(s.link.as_deref())
                                    .eq_ignore_ascii_case(n)
                            }
                            _ => false,
                        }
                })
                .map(|(slot, _)| (bag, *slot))
        });
        if let Some((bag, slot)) = found {
            model
                .script_calls
                .push(super::super::ScriptCall::UseContainerItem { bag, slot }.at_unit(unit));
        }
        none()
    })?;
    Ok(())
}
