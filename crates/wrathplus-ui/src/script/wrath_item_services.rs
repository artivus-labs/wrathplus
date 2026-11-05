//! Build-12340 refund APIs used by ContainerFrame and the socketing window's open intents.
//! Evidence: 5d8d80/5d8f70 and 5c49c0..5c5470. Refund absence is zero
//! returns, not a fabricated free purchase. The socketing session itself is `wrath_sockets`.
//! Mutations below are host intents, not success.
use super::UiScript;
use mlua::{Lua, MultiValue, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRefundView {
    pub money: i32,
    pub honor: i32,
    pub arena: i32,
    pub seconds_remaining: i32,
    pub items: Vec<ItemRefundCost>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemRefundCost {
    pub texture: Option<String>,
    pub count: i32,
    pub link: Option<String>,
}
/// Capture-only until App connects the corresponding native item/guild-bank service.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemServiceRequest {
    SocketContainer { bag: i32, slot: i32 },
    SocketInventory { slot: i32 },
    WithdrawGuildBankMoney { copper: u32 },
}
#[derive(Default)]
struct State {
    refunds: BTreeMap<(bool, i32, i32), ItemRefundView>,
    requests: Vec<ItemServiceRequest>,
}
impl UiScript {
    /// Full replacement; absent/expired rows remain unavailable. No receipt events.
    pub fn set_item_refunds(&mut self, rows: Vec<(bool, i32, i32, ItemRefundView)>) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            state.refunds = rows
                .into_iter()
                .map(|(equipped, bag, slot, view)| ((equipped, bag, slot), view))
                .collect();
        }
    }
    pub fn take_item_service_requests(&mut self) -> Vec<ItemServiceRequest> {
        self.lua
            .app_data_mut::<State>()
            .map(|mut s| std::mem::take(&mut s.requests))
            .unwrap_or_default()
    }
}
fn number(lua: &Lua, v: Value, usage: &str) -> mlua::Result<i32> {
    lua.coerce_number(v)?
        .map(|n| n as i64 as i32)
        .ok_or_else(|| mlua::Error::RuntimeError(usage.into()))
}
fn optional_integer(lua: &Lua, v: Value) -> mlua::Result<bool> {
    // Native lua_tonumber, not Lua truthiness: true is not the integer 1.
    Ok(lua.coerce_number(v)?.unwrap_or(0.0) as i32 != 0)
}
fn string(lua: &Lua, v: Option<&str>) -> mlua::Result<Value> {
    v.map(|v| lua.create_string(v).map(Value::String))
        .unwrap_or(Ok(Value::Nil))
}
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let g = lua.globals();
    g.set(
        "GetContainerItemPurchaseInfo",
        lua.create_function(|lua, (bag, slot, equipped): (Value, Value, Value)| {
            let bag = number(
                lua,
                bag,
                "Usage: GetContainerItemPurchaseInfo(index, slot[, isEquipped])",
            )?;
            let slot = number(
                lua,
                slot,
                "Usage: GetContainerItemPurchaseInfo(index, slot[, isEquipped])",
            )?;
            let equipped = optional_integer(lua, equipped)?;
            let state = lua.app_data_ref::<State>().expect("item services");
            let row = state
                .refunds
                .get(&(equipped, if equipped { 0 } else { bag }, slot))
                .filter(|r| r.seconds_remaining > 0);
            Ok(row
                .map(|r| {
                    MultiValue::from_vec(vec![
                        Value::Integer(r.money.into()),
                        Value::Integer(r.honor.into()),
                        Value::Integer(r.arena.into()),
                        Value::Integer(r.items.len() as i64),
                        Value::Integer(r.seconds_remaining.into()),
                    ])
                })
                .unwrap_or_default())
        })?,
    )?;
    g.set(
        "GetContainerItemPurchaseItem",
        lua.create_function(
            |lua, (bag, slot, index, equipped): (Value, Value, Value, Value)| {
                let usage = "Usage: ContainerItemPurchaseItem(bag, slot, itemIndex[, isEquipped])";
                let bag = number(lua, bag, usage)?;
                let slot = number(lua, slot, usage)?;
                let index = number(lua, index, usage)?;
                let equipped = optional_integer(lua, equipped)?;
                let row = lua
                    .app_data_ref::<State>()
                    .expect("item services")
                    .refunds
                    .get(&(equipped, if equipped { 0 } else { bag }, slot))
                    .filter(|r| r.seconds_remaining > 0)
                    .and_then(|r| usize::try_from(index - 1).ok().and_then(|i| r.items.get(i)))
                    .cloned();
                match row {
                    Some(r) => Ok(MultiValue::from_vec(vec![
                        string(lua, r.texture.as_deref())?,
                        Value::Integer(r.count.into()),
                        string(lua, r.link.as_deref())?,
                    ])),
                    None => Ok(MultiValue::new()),
                }
            },
        )?,
    )?;
    g.set(
        "SocketContainerItem",
        lua.create_function(|lua, (bag, slot): (Value, Value)| {
            let bag = number(lua, bag, "Usage: SocketContainerItem(index, slot)")?;
            let slot = number(lua, slot, "Usage: SocketContainerItem(index, slot)")?;
            // Capture only: host must validate unlocked item and socket metadata before opening.
            lua.app_data_mut::<State>()
                .unwrap()
                .requests
                .push(ItemServiceRequest::SocketContainer { bag, slot });
            Ok(())
        })?,
    )?;
    g.set(
        "SocketInventoryItem",
        lua.create_function(|lua, slot: Value| {
            let slot = number(lua, slot, "Usage: SocketInventoryItem(slot)")?;
            if (1..=19).contains(&slot) {
                lua.app_data_mut::<State>()
                    .unwrap()
                    .requests
                    .push(ItemServiceRequest::SocketInventory { slot });
            }
            Ok(())
        })?,
    )?;
    g.set(
        "WithdrawGuildBankMoney",
        lua.create_function(|lua, amount: Value| {
            let amount = lua.coerce_number(amount)?.ok_or_else(|| {
                mlua::Error::RuntimeError("Usage: WithdrawGuildBankMoney(money)".into())
            })?;
            if amount.is_finite() && amount > 0.0 && amount <= u32::MAX as f64 {
                lua.app_data_mut::<State>().unwrap().requests.push(
                    ItemServiceRequest::WithdrawGuildBankMoney {
                        copper: amount as u32,
                    },
                );
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refunds_keep_arity_signed_costs_and_unavailable_item_fields() {
        let mut ui = UiScript::new().unwrap();
        ui.set_item_refunds(vec![
            (
                false,
                0,
                2,
                ItemRefundView {
                    money: -1,
                    honor: 4,
                    arena: 7,
                    seconds_remaining: 12,
                    items: vec![ItemRefundCost {
                        count: 2,
                        ..Default::default()
                    }],
                },
            ),
            (
                true,
                0,
                5,
                ItemRefundView {
                    seconds_remaining: 0,
                    ..Default::default()
                },
            ),
        ]);
        ui.run(
            r#"
            assert(select('#', GetContainerItemPurchaseInfo(0, 1)) == 0)
            assert(select('#', GetContainerItemPurchaseInfo(0, 5, 1)) == 0)
            local m,h,a,n,t = GetContainerItemPurchaseInfo('0', '2')
            assert(m == -1 and h == 4 and a == 7 and n == 1 and t == 12)
            assert(select('#', GetContainerItemPurchaseInfo(0,2)) == 5)
            local icon,count,link = GetContainerItemPurchaseItem(0,2,1)
            assert(icon == nil and count == 2 and link == nil)
            assert(select('#', GetContainerItemPurchaseItem(0,2,1)) == 3)
            assert(select('#', GetContainerItemPurchaseItem(0,2,0)) == 0)
            assert(not pcall(GetContainerItemPurchaseInfo, 0))
            assert(not pcall(GetContainerItemPurchaseItem, 0, 2))
        "#,
        )
        .unwrap();
        ui.set_item_refunds(vec![]);
        ui.run("assert(select('#', GetContainerItemPurchaseInfo(0,2))==0)")
            .unwrap();
    }
    #[test]
    fn socket_closed_shapes_and_capture_only_intents_do_not_claim_success() {
        let mut ui = UiScript::new().unwrap();
        ui.run(
            r#"
            local n,i,q = GetSocketItemInfo(); assert(n == nil and i == nil and q == 0)
            assert(select('#', GetSocketItemInfo()) == 3 and GetNumSockets() == 0)
            assert(select('#', GetExistingSocketInfo(1)) == 3)
            assert(select('#', GetNewSocketInfo(1)) == 3)
            assert(select('#', GetExistingSocketLink(1)) == 0)
            assert(select('#', GetNewSocketLink(1)) == 0)
            assert(select('#', GetSocketTypes(1)) == 0)
            assert(GetSocketItemRefundable() == nil)
            assert(not pcall(GetSocketTypes))
            CloseSocketInfo(); AcceptSockets(); ClickSocketButton(1)
            SocketContainerItem(0,2); SocketInventoryItem(5); WithdrawGuildBankMoney(123)
            SocketInventoryItem(99); WithdrawGuildBankMoney(0)
        "#,
        )
        .unwrap();
        assert_eq!(
            ui.take_item_service_requests(),
            vec![
                ItemServiceRequest::SocketContainer { bag: 0, slot: 2 },
                ItemServiceRequest::SocketInventory { slot: 5 },
                ItemServiceRequest::WithdrawGuildBankMoney { copper: 123 }
            ]
        );
        assert!(ui.take_item_service_requests().is_empty());
    }
}
