//! The dressing room: the `DressUpModel` verbs (table `0x84f190`) and the intent queue behind them
//! and behind `SetUnit`/`RefreshUnit` on a dressing-room pane. The reference edits a clone of the
//! unit's model (`0x5059a0`, `0x504350`); here the app composes the look from the intents in
//! order, which matters: on a closed window `DressUpItem` resets, then tries on
//! (`DressUpFrame.lua:3-7`).

use mlua::{Lua, Table, Value};

use super::object::frame_handle_of;
use super::Model;
use crate::widget::FrameKind;

/// Registry key of the `DressUpModel` method table, its own three; the rest come by the chain.
pub(super) const REG_DRESSUPMODEL_METHODS: &str = "__wrathplus_dressupmodel_methods";

/// One queued dressing-room intent, applied in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DressUpIntent {
    /// `SetUnit`, `RefreshUnit` and `Dress` (the Reset button): the reference's rebuild from the
    /// unit (`0x505b50`), dropping every substitution.
    Dress,
    /// `Undress()`: bodyslots `0..0xb` cleared, worn and tried-on alike; the hands keep what they
    /// hold (`0x504490`).
    Undress,
    /// `TryOn(item)`: the item in its `InventoryType`'s slot, with no class, level or proficiency
    /// check (`DressUpFrame.lua:2-16`).
    TryOn(u32),
    /// The window was hidden, so the booth empties. The reference keeps its state while hidden,
    /// but the next `DressUpItem` re-issues `SetUnit("player")`, so that state is never seen.
    Close,
}

impl super::UiScript {
    /// Drain the dressing room's intents, oldest first; the app applies them in order.
    pub fn take_dressup_intents(&mut self) -> Vec<DressUpIntent> {
        std::mem::take(&mut self.model_mut().dressup_intents)
    }
}

fn queue(lua: &Lua, intent: DressUpIntent) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .dressup_intents
        .push(intent);
}

/// Queue the rebuild when `PlayerModel`'s `SetUnit`/`RefreshUnit` lands on a `DressUpModel`.
pub(super) fn redress_if_dressup(lua: &Lua, this: &Table) -> mlua::Result<()> {
    let h = frame_handle_of(lua, this)?;
    let is_dressup = lua
        .app_data_ref::<Model>()
        .expect("model app_data")
        .arena
        .frame(h)
        .is_some_and(|f| f.kind == FrameKind::DressUpModel);
    if is_dressup {
        queue(lua, DressUpIntent::Dress);
    }
    Ok(())
}

/// The item a `TryOn` names: a number truncated, a digit string, or an item string or link, whose
/// `item:<id>` field is read, as stock 3.3.5 `DressUpItemLink` passes the link as is
/// (DressUpFrame.lua:10). Labelled: an item name is not resolved; anything else is 0.
fn item_arg(v: &Value) -> i64 {
    match v {
        Value::Integer(i) => *i,
        Value::Number(n) => n.trunc() as i64,
        Value::String(s) => {
            let Ok(t) = s.to_str() else { return 0 };
            if let Some(rest) = t.split("item:").nth(1) {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                return digits.parse().unwrap_or(0);
            }
            t.trim().parse::<f64>().map_or(0, |n| n.trunc() as i64)
        }
        _ => 0,
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let m = lua.create_table()?;

    // `0x504c00`.
    m.set(
        "Undress",
        lua.create_function(|lua, this: Table| {
            frame_handle_of(lua, &this)?;
            queue(lua, DressUpIntent::Undress);
            Ok(())
        })?,
    )?;

    // `0x504cd0`, the Reset button (`DressUpFrame.xml:182`).
    m.set(
        "Dress",
        lua.create_function(|lua, this: Table| {
            frame_handle_of(lua, &this)?;
            queue(lua, DressUpIntent::Dress);
            Ok(())
        })?,
    )?;

    // `0x504d90` calls `0x504540(itemId)`; item 0 (a non-number or a link) previews nothing, and
    // so does a negative id here.
    m.set(
        "TryOn",
        lua.create_function(|lua, (this, item): (Table, Value)| {
            frame_handle_of(lua, &this)?;
            if let Ok(id) = u32::try_from(item_arg(&item)) {
                if id != 0 {
                    queue(lua, DressUpIntent::TryOn(id));
                }
            }
            Ok(())
        })?,
    )?;

    lua.set_named_registry_value(REG_DRESSUPMODEL_METHODS, m)
}

#[cfg(test)]
mod tests {
    use super::DressUpIntent;
    use crate::script::UiScript;

    fn room() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_screen_size(800.0, 600.0);
        s.run(r#"dm = CreateFrame("DressUpModel", "DM", UIParent)"#)
            .unwrap();
        s
    }

    #[test]
    fn a_dress_up_model_is_a_player_model_plus_three_and_the_chain_runs_one_way() {
        let s = room();
        s.run(r#"pm = CreateFrame("PlayerModel", "PMOnly", UIParent)"#)
            .unwrap();
        for verb in [
            "TryOn",
            "Dress",
            "Undress",
            "SetUnit",
            "RefreshUnit",
            "SetRotation",
            "SetCamera",
        ] {
            assert_eq!(
                s.eval::<String>(&format!("return type(DM.{verb})"))
                    .unwrap(),
                "function",
                "DressUpModel answers {verb}"
            );
        }
        for verb in ["TryOn", "Dress", "Undress"] {
            assert_eq!(
                s.eval::<String>(&format!("return type(PMOnly.{verb})"))
                    .unwrap(),
                "nil",
                "a PlayerModel must NOT answer {verb}"
            );
        }
        assert_eq!(
            s.eval::<String>("return DM:GetObjectType()").unwrap(),
            "DressUpModel"
        );
        for t in ["DressUpModel", "PlayerModel", "Model", "Frame", "Region"] {
            assert!(
                s.eval::<bool>(&format!("return DM:IsObjectType(\"{t}\") == 1"))
                    .unwrap(),
                "IsObjectType({t})"
            );
        }
    }

    /// A digit string is an id, and so is a link's `item:` field, which stock 3.3.5
    /// `DressUpItemLink` passes whole.
    #[test]
    fn the_verbs_queue_intents_in_order_and_try_on_reads_its_argument_like_the_client() {
        let mut s = room();
        s.run(
            r#"
            DM:SetUnit("player")
            DM:TryOn("117")
            DM:TryOn(1234.7)
            DM:Undress()
            DM:RefreshUnit()
            DM:Dress()
            DM:TryOn("|cffffffff|Hitem:117|h[Tough Jerky]|h|r")
            DM:TryOn(0)
            DM:TryOn(nil)
            DM:TryOn(-5)
            "#,
        )
        .unwrap();
        assert_eq!(
            s.take_dressup_intents(),
            vec![
                DressUpIntent::Dress,
                DressUpIntent::TryOn(117),
                DressUpIntent::TryOn(1234),
                DressUpIntent::Undress,
                DressUpIntent::Dress,
                DressUpIntent::Dress,
                DressUpIntent::TryOn(117),
            ]
        );
        // The same two PlayerModel verbs on a paper-doll pane queue nothing.
        s.run(r#"pm = CreateFrame("PlayerModel", "PMOnly", UIParent) pm:SetUnit("player") pm:RefreshUnit()"#)
            .unwrap();
        assert!(s.take_dressup_intents().is_empty());
        // Its yaw is its own facing, read by name like every other pane's.
        s.run("DM:SetRotation(0.61)").unwrap();
        assert!((s.model_pane_facing("DM") - 0.61).abs() < 1e-6);
    }
}
