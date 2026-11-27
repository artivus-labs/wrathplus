//! Bounded build-12340 loot ABI for manual solo corpse loot.
//!
//! The mature Vanilla bindings remain the state store. This adapter carries the Wrath ABI:
//! `GetLootSlotInfo` returns `locked` as its fifth result, `LootSlot` is the actual take verb, and
//! the predicates, the count and `CloseLoot` follow `0x588190`-`0x5888b0`.

use mlua::{Function, Lua, MultiValue, Value};

use super::Model;
use crate::script::binding_abi::{bool_or_default, flag, number_arg};

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let vanilla_info: Function = globals.get("GetLootSlotInfo")?;

    // Build 12340 returns texture, name, quantity, quality, locked (`0x588570`): 1 for a `LOCKED`
    // row, else nil. The existing fifth Vanilla extension is an item id and must not leak into the
    // Wrath stock script as a truthy lock flag.
    globals.set(
        "GetLootSlotInfo",
        lua.create_function(move |lua, slot: u32| {
            let mut values: MultiValue = vanilla_info.call(slot)?;
            while values.len() < 4 {
                values.push_back(Value::Nil);
            }
            values.truncate(4);
            let locked = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .loot
                .as_ref()
                .and_then(|l| slot.checked_sub(1).and_then(|n| l.rows.get(n as usize)))
                .is_some_and(|row| row.as_ref().is_some_and(|r| r.locked));
            values.push_back(if locked {
                Value::Integer(1)
            } else {
                Value::Nil
            });
            Ok(values)
        })?,
    )?;

    // In build 12340 LootButton_OnClick calls LootSlot directly to take the displayed row. This is
    // intentionally distinct from the Vanilla LOOT_BIND-confirmation-only ABI.
    globals.set(
        "LootSlot",
        lua.create_function(|lua, slot: u32| {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .loot_picks
                .push(slot);
            Ok(())
        })?,
    )?;

    // GetNumLootItems() (`0x588190`): the highest occupied slot, cleared slots below it included;
    // the coin row, taken or not, is the layout's first.
    globals.set(
        "GetNumLootItems",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(loot) = model.loot.as_ref() else {
                return Ok(0);
            };
            Ok(match loot.rows.iter().rposition(Option::is_some) {
                Some(i) => i as i64 + 1,
                None => i64::from(loot.coin_slot),
            })
        })?,
    )?;
    // LootSlotIsItem(slot) / LootSlotIsCoin(slot) (`0x588750`, `0x588810`): 1 or nil.
    for (verb, usage, coin) in [
        ("LootSlotIsItem", "Usage: LootSlotIsItem(slot)", false),
        ("LootSlotIsCoin", "Usage: LootSlotIsCoin(slot)", true),
    ] {
        globals.set(
            verb,
            lua.create_function(move |lua, slot: Value| {
                let slot = number_arg(lua, slot, usage)?;
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                Ok(flag(
                    usize::try_from(slot.wrapping_sub(1))
                        .ok()
                        .and_then(|n| model.loot.as_ref()?.rows.get(n)?.as_ref())
                        .is_some_and(|r| r.is_coin == coin),
                ))
            })?,
        )?;
    }
    // CloseLoot([failed]) (`0x5888b0`): `failed` (`0x815500`) raises `ERR_LOOT_NO_UI` (`0x5236b3`)
    // when loot is open; stock passes it when the frame cannot show (`LootFrame.lua:21`).
    globals.set(
        "CloseLoot",
        lua.create_function(|lua, args: MultiValue| {
            let failed = bool_or_default(args.front(), false);
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if failed && model.loot.is_some() {
                model.ui_errors.push("ERR_LOOT_NO_UI");
            }
            model.loot_close = true;
            Ok(())
        })?,
    )?;

    // The host already receives real modifier state each frame. For the supported manual mode the
    // auto-loot key is Shift; no CVar or permanently false key state is invented.
    globals.set(
        "IsAutoLootKeyDown",
        lua.create_function(|lua, ()| {
            let shift = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .modifiers
                .0;
            Ok(shift)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{LootRow, LootState, UiScript};

    fn row() -> LootRow {
        LootRow {
            name: Some("Linen Cloth".into()),
            texture: Some("Interface\\Icons\\INV_Fabric_Linen_01".into()),
            quantity: 2,
            quality: Some(1),
            is_coin: false,
            item_id: 2589,
            link: Some("|cffffffff|Hitem:2589:0:0:0:0:0:0:0|h[Linen Cloth]|h|r".into()),
            random_property_id: 0,
            locked: false,
        }
    }

    #[test]
    fn wrath_loot_slot_is_take_and_info_fifth_result_is_locked() {
        let mut script = UiScript::new().unwrap();
        script.set_loot(Some(LootState {
            rows: vec![Some(row())],
            ..LootState::default()
        }));

        assert!(script
            .eval::<bool>(
                "local texture,name,count,quality,locked=GetLootSlotInfo(1); \
                 return texture~=nil and name=='Linen Cloth' and count==2 and quality==1 \
                    and locked==nil"
            )
            .unwrap());
        script.set_loot(Some(LootState {
            rows: vec![Some(LootRow {
                locked: true,
                ..row()
            })],
            ..LootState::default()
        }));
        assert_eq!(
            script
                .eval::<i64>("local _,_,_,_,locked=GetLootSlotInfo(1); return locked")
                .unwrap(),
            1
        );
        script.run("LootSlot(1)").unwrap();
        assert_eq!(script.take_loot_picks(), [1]);
        assert!(script.take_loot_confirms().is_empty());
    }

    /// `LootSlotIsItem`/`LootSlotIsCoin` push the number 1 or nil (`0x5887d6`, `0x588882`) and raise
    /// their own Usage; `GetNumLootItems` stops at the highest occupied slot (`0x588190`); a
    /// `CloseLoot(failed)` with loot open raises `ERR_LOOT_NO_UI` (`0x5236b3`).
    #[test]
    fn wrath_loot_predicates_count_and_close() {
        let mut s = UiScript::new().unwrap();
        let coin = LootRow {
            is_coin: true,
            item_id: 0,
            ..row()
        };
        s.set_loot(Some(LootState {
            rows: vec![Some(coin), Some(row()), Some(row()), None],
            ..LootState::default()
        }));
        let probe = "return LootSlotIsCoin(1), LootSlotIsItem(1), LootSlotIsItem(2), \
                     LootSlotIsItem(4), GetNumLootItems()";
        assert_eq!(
            s.eval::<(Option<i64>, Option<i64>, Option<i64>, Option<i64>, i64)>(probe)
                .unwrap(),
            (Some(1), None, Some(1), None, 3),
            "1/nil, never booleans; the trailing cleared slot drops out of the count"
        );
        assert!(s
            .eval::<bool>("return type(LootSlotIsCoin(1)) == 'number'")
            .unwrap());
        let mut middle = LootState {
            rows: vec![Some(row()), None, Some(row())],
            ..LootState::default()
        };
        s.set_loot(Some(middle.clone()));
        assert_eq!(s.eval::<i64>("return GetNumLootItems()").unwrap(), 3);
        middle.rows[2] = None;
        s.set_loot(Some(middle));
        assert_eq!(s.eval::<i64>("return GetNumLootItems()").unwrap(), 1);
        s.set_loot(Some(LootState {
            rows: vec![None, None],
            coin_slot: true,
            ..LootState::default()
        }));
        assert_eq!(
            s.eval::<i64>("return GetNumLootItems()").unwrap(),
            1,
            "a taken coin still counts its slot"
        );
        for verb in ["LootSlotIsItem", "LootSlotIsCoin"] {
            let err = s.run(&format!("{verb}()")).unwrap_err().to_string();
            assert!(err.contains(&format!("Usage: {verb}(slot)")), "{err}");
        }

        s.run("CloseLoot()").unwrap();
        assert!(s.take_ui_errors().is_empty(), "no failure argument");
        s.run("CloseLoot(1)").unwrap();
        assert_eq!(s.take_ui_errors(), vec!["ERR_LOOT_NO_UI"]);
        assert!(s.take_loot_close());
        s.set_loot(None);
        s.run("CloseLoot(true)").unwrap();
        assert!(s.take_ui_errors().is_empty(), "no loot source: no error");
    }

    #[test]
    fn wrath_auto_loot_key_reports_the_real_shift_modifier() {
        let mut script = UiScript::new().unwrap();
        assert!(!script.eval::<bool>("return IsAutoLootKeyDown()").unwrap());
        script.set_modifiers(true, false, false);
        assert!(script.eval::<bool>("return IsAutoLootKeyDown()").unwrap());
    }
}
