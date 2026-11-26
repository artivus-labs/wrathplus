//! Build 12340's auction verbs layered over the base bindings (`super::install`).

use mlua::{Lua, MultiValue, Value};

use super::super::binding_abi::flag;
use super::super::Model;
use super::{cursor, list_index, AuctionSortOp, AuctionStartRequest, OWNER, SORT_KEYS};

/// Build 12340's sort, cancel and sell verbs.
///
/// Labelled: `SetAuctionsTabShowing`'s flag has no consumer here; `CancelSell` drops a pending
/// `StartAuction`, as the multi-stack sell it stops natively is not built; a sort set before its
/// apply already orders the next push.
pub(in crate::script) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let string_at = |lua: &Lua, args: &MultiValue, i: usize| -> Option<String> {
        match args.get(i) {
            Some(Value::String(s)) => Some(s.to_string_lossy()),
            Some(v @ (Value::Integer(_) | Value::Number(_))) => lua
                .coerce_string(v.clone())
                .ok()
                .flatten()
                .map(|s| s.to_string_lossy()),
            _ => None,
        }
    };
    let push = |lua: &Lua, kind: &str, op: AuctionSortOp| {
        if let Some(i) = list_index(kind) {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .auction_sorts
                .push((i, op));
        }
    };
    g.set(
        "SortAuctionClearSort",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(kind) = string_at(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: SortAuctionClearSort(\"type\")",
                ));
            };
            push(lua, &kind, AuctionSortOp::Clear);
            Ok(())
        })?,
    )?;
    g.set(
        "SortAuctionSetSort",
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(kind), Some(key)) = (string_at(lua, &args, 0), string_at(lua, &args, 1))
            else {
                return Err(mlua::Error::runtime(
                    "Usage: SortAuctionAddSort(\"type\", \"sort\", \"reverse\")",
                ));
            };
            // The direction only from a boolean third argument (`0x84deb0 == 1`).
            let reversed = matches!(args.get(2), Some(Value::Boolean(true)));
            if SORT_KEYS.contains(&key.as_str()) {
                push(lua, &kind, AuctionSortOp::Set { key, reversed });
            }
            Ok(())
        })?,
    )?;
    g.set(
        "SortAuctionApplySort",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(kind) = string_at(lua, &args, 0) else {
                return Err(mlua::Error::runtime(
                    "Usage: SortAuctionApplySort(\"type\")",
                ));
            };
            push(lua, &kind, AuctionSortOp::Apply);
            Ok(())
        })?,
    )?;
    // GetAuctionSort(type, index) (`0x59ad40`): the list's index-th criterion, its column name
    // and 1/nil reversed; nil, nil past the stack.
    g.set(
        "GetAuctionSort",
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(kind), Some(index)) = (
                string_at(lua, &args, 0),
                args.get(1)
                    .and_then(|v| lua.coerce_integer(v.clone()).ok().flatten()),
            ) else {
                return Err(mlua::Error::runtime(
                    "Usage: GetAuctionSort(\"type\", \"index\")",
                ));
            };
            let entry = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                list_index(&kind)
                    .zip(model.auction.as_ref())
                    .and_then(|(i, a)| {
                        let k = usize::try_from(index - 1).ok().filter(|&k| k < 12)?;
                        a.lists[i].sort.get(k).cloned()
                    })
            };
            Ok(match entry {
                Some((key, reversed)) => MultiValue::from_vec(vec![
                    Value::String(lua.create_string(&key)?),
                    flag(reversed),
                ]),
                None => MultiValue::from_vec(vec![Value::Nil, Value::Nil]),
            })
        })?,
    )?;
    g.set(
        "SetAuctionsTabShowing",
        lua.create_function(|lua, args: MultiValue| {
            let Some(Value::Boolean(showing)) = args.front() else {
                return Err(mlua::Error::runtime("Usage: SetAuctionsTabShowing(bool)"));
            };
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .auctions_tab_showing = *showing;
            Ok(())
        })?,
    )?;
    // CanCancelAuction(index) (`0x59c9f0`): with an auctioneer, an owned row whose sale is not
    // pending. Labelled: AzerothCore never marks a pending sale, so every owned row qualifies.
    g.set(
        "CanCancelAuction",
        lua.create_function(|lua, args: MultiValue| {
            let Some(index) = args
                .front()
                .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
            else {
                return Err(mlua::Error::runtime("Usage: CanCancelAuction(index)"));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(a) = model.auction.as_ref() else {
                return Ok(MultiValue::new());
            };
            let k = (index.round() as i64 - 1).max(-1);
            let ok = usize::try_from(k)
                .ok()
                .is_some_and(|k| k < a.lists[OWNER].rows.len());
            Ok(MultiValue::from_vec(vec![flag(ok)]))
        })?,
    )?;
    // CancelSell (`0x59b040`): stop a multi-stack sell after the auction in flight; no failure.
    g.set(
        "CancelSell",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.auction_start = None;
            model.auction_cancel_sell = true;
            Ok(())
        })?,
    )?;
    // StartAuction(minBid, buyoutPrice, runTime, count, numStacks) (`0x59f990`): runTime is the
    // 1-based duration index (720, 1440, 2880 minutes, else 1440); a buyout below the bid is
    // raised to it.
    g.set(
        "StartAuction",
        lua.create_function(|lua, args: MultiValue| {
            let num = |i: usize| {
                args.get(i)
                    .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
                    .map(|n| n.round() as i64)
            };
            let (Some(min_bid), Some(buyout), Some(run)) = (num(0), num(1), num(2)) else {
                return Err(mlua::Error::runtime(
                    "Usage: StartAuction(minBid, buyoutPrice, runTime, count)",
                ));
            };
            let min_bid = min_bid.max(0) as u32;
            let mut buyout = buyout.max(0) as u32;
            if buyout != 0 && buyout < min_bid {
                buyout = min_bid;
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if model.auction.is_none() || model.auction_sell_item.is_none() {
                return Ok(());
            }
            model.auction_start = Some(AuctionStartRequest {
                min_bid,
                buyout,
                duration: run_minutes(run),
                stack_size: num(3).unwrap_or(0).max(0) as u32,
                num_stacks: num(4).unwrap_or(1).max(1) as u32,
            });
            Ok(())
        })?,
    )?;
    // CalculateAuctionDeposit(runTime[, count]) (`0x59d270` → `0x76dde0`): the house rate × the
    // vendor value of `count` items (else the held stack), × minutes / 240, at least 100.
    g.set(
        "CalculateAuctionDeposit",
        lua.create_function(|lua, args: MultiValue| {
            let num = |i: usize| {
                args.get(i)
                    .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
                    .map(|n| n.round() as i64)
            };
            let Some(run) = num(0) else {
                return Err(mlua::Error::runtime(
                    "Usage: CalculateAuctionDeposit(runTime)",
                ));
            };
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let Some(rate) = model.auction.as_ref().map(|a| a.deposit_percent) else {
                return Ok(0);
            };
            let Some(it) = model.auction_sell_item.as_ref() else {
                return Ok(0);
            };
            let count = num(1).map_or_else(|| cursor::held_count(&model, it), |c| c.max(0) as u32);
            let unit = model
                .item_templates
                .get(&it.item_id)
                .map_or(0, |t| t.sell_price);
            Ok(i64::from(wrath_deposit(
                rate,
                unit.saturating_mul(count),
                run_minutes(run),
            )))
        })?,
    )?;
    Ok(())
}

/// `StartAuction`'s duration table (`0xa15248`): index 1-3, else 1440.
fn run_minutes(run: i64) -> u32 {
    match run {
        1 => 720,
        2 => 1440,
        3 => 2880,
        _ => 1440,
    }
}

/// `0x76dde0`: `round(minutes × floor(rate × value / 100) / 240)`, at least 100; a duration off the
/// table pays 100.
fn wrath_deposit(rate: u32, value: u32, minutes: u32) -> u32 {
    if ![720, 1440, 2880].contains(&minutes) {
        return 100;
    }
    let base = (u64::from(rate) * u64::from(value) / 100) as f64;
    ((f64::from(minutes) * base * (1.0 / 240.0)).round() as u32).max(100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::auction::LIST;
    use crate::script::UiScript;

    /// Build 12340's sort verbs queue their ops; `GetAuctionSort` reads App's stack.
    #[test]
    fn the_wrath_sort_verbs_queue_and_read_the_stack() {
        let mut s = UiScript::new().unwrap();
        s.run(
            "SortAuctionClearSort('owner') SortAuctionSetSort('owner', 'duration', true) \
             SortAuctionSetSort('list', 'nope') SortAuctionSetSort('list', 'quantity') \
             SortAuctionApplySort('owner') SortAuctionApplySort('elsewhere')",
        )
        .unwrap();
        assert_eq!(
            s.take_auction_sorts(),
            vec![
                (OWNER, AuctionSortOp::Clear),
                (
                    OWNER,
                    AuctionSortOp::Set {
                        key: "duration".into(),
                        reversed: true
                    }
                ),
                (
                    LIST,
                    AuctionSortOp::Set {
                        key: "quantity".into(),
                        reversed: false
                    }
                ),
                (OWNER, AuctionSortOp::Apply),
            ]
        );
        assert!(s.run("SortAuctionSetSort('list')").is_err());
        assert!(s.run("SetAuctionsTabShowing(1)").is_err());
        s.run("SetAuctionsTabShowing(true)").unwrap();
        // No auctioneer: CanCancelAuction answers nothing; GetAuctionSort nil, nil.
        assert_eq!(
            s.eval::<i64>("return select('#', CanCancelAuction(1))")
                .unwrap(),
            0
        );
        assert!(s
            .eval::<bool>("local k, r = GetAuctionSort('owner', 1) return k == nil and r == nil")
            .unwrap());
    }

    /// Build 12340's deposit (`0x76dde0`) and `StartAuction`'s duration index.
    #[test]
    fn the_wrath_deposit_and_run_times() {
        assert_eq!(run_minutes(1), 720);
        assert_eq!(run_minutes(3), 2880);
        assert_eq!(run_minutes(9), 1440);
        // 5% of 10000 is 500 a 12-hour unit: × 720 / 240 = 1500.
        assert_eq!(wrath_deposit(5, 10_000, 720), 1500);
        assert_eq!(wrath_deposit(5, 10, 720), 100, "at least 100");
        assert_eq!(wrath_deposit(5, 10_000, 30), 100, "off the table");
    }
}
