//! Build 12340's auction verbs layered over the base bindings (`super::install`).

use mlua::{Lua, MultiValue, Value};

use super::super::binding_abi::{flag, number_arg, optional_string, string_arg};
use super::super::Model;
use super::{
    cursor, list_index, AuctionBid, AuctionHighBidder, AuctionSortOp, AuctionStartRequest, LIST,
    OWNER, SORT_KEYS,
};

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
    // PlaceAuctionBid(type, index, bid) (`0x59da40`): the bid caps at the buyout, then the row's
    // minimum, its high bid and increment, and the purse (less a high bid of our own) refuse it
    // with their errors before anything is sent. Labelled: the unique-count check (`0x754dd0`)
    // is not modelled.
    g.set(
        "PlaceAuctionBid",
        lua.create_function(|lua, (kind, index, bid): (Value, Value, Value)| {
            const USAGE: &str = "Usage: PlaceAuctionBid(\"type\", index, bid)";
            let kind = string_arg(lua, kind, USAGE)?.to_ascii_lowercase();
            let index = number_arg(lua, index, USAGE)?;
            let mut bid = number_arg(lua, bid, USAGE)? as u32;
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let Some(list) = list_index(&kind) else {
                return Ok(());
            };
            let row = model.auction.as_ref().and_then(|a| {
                let i = usize::try_from(index.wrapping_sub(1)).ok()?;
                a.lists[list].rows.get(i).cloned()
            });
            let Some(r) = row else {
                return Ok(());
            };
            if r.buyout_price > 0 && bid > r.buyout_price {
                bid = r.buyout_price;
            }
            let next = r.bid_amount.wrapping_add(r.min_increment);
            let refusal = if bid < r.min_bid {
                Some("ERR_AUCTION_MIN_BID")
            } else if bid <= r.bid_amount {
                Some("ERR_AUCTION_HIGHER_BID")
            } else if r.bid_amount != 0
                && bid < next
                && (r.buyout_price == 0 || bid < r.buyout_price)
            {
                Some("ERR_AUCTION_BID_INCREMENT")
            } else if bid > 2_000_000_000 && bid > next {
                return Ok(());
            } else {
                let own = matches!(r.high_bidder, AuctionHighBidder::Player);
                let need = if own { bid - r.bid_amount } else { bid };
                (model.money < u64::from(need)).then_some("ERR_NOT_ENOUGH_MONEY")
            };
            match refusal {
                Some(key) => model.ui_errors.push(key),
                None => model.auction_bids.push(AuctionBid {
                    list,
                    index: index as u32,
                    amount: bid,
                }),
            }
            Ok(())
        })?,
    )?;
    // CanSendAuctionQuery([type]) (`0x59c740`) → canQuery, canMassQuery, per list (any other type
    // is Browse). Only Browse is throttled here: the owner and bidder pages always send.
    // Deviation: `canMassQuery` is always nil, as the `getAll` query it gates is not built.
    g.set(
        "CanSendAuctionQuery",
        lua.create_function(|lua, kind: Option<Value>| {
            let kind = kind.and_then(|v| optional_string(lua, &v));
            let list = kind.and_then(|k| list_index(&k.to_ascii_lowercase()));
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let can = list.unwrap_or(LIST) != LIST || model.auction_can_query;
            Ok((flag(can), Value::Nil))
        })?,
    )?;
    // IsAuctionSortReversed(type, sort) (`0x59ab90`) → reversed 1/nil and 1 for a column the list's
    // stack holds, nil, nil otherwise; nothing for a column that is none. Both match in any case.
    g.set(
        "IsAuctionSortReversed",
        lua.create_function(|lua, (kind, key): (Value, Value)| {
            const USAGE: &str = "Usage: IsAuctionSortReversed(\"type\", \"sort\")";
            let kind = string_arg(lua, kind, USAGE)?.to_ascii_lowercase();
            let key = string_arg(lua, key, USAGE)?.to_ascii_lowercase();
            if !SORT_KEYS.contains(&key.as_str()) {
                return Ok(MultiValue::new());
            }
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let found = list_index(&kind)
                .zip(model.auction.as_ref())
                .and_then(|(i, a)| a.lists[i].reversed(&key));
            Ok(MultiValue::from_vec(match found {
                Some(rev) => vec![flag(rev), Value::Integer(1)],
                None => vec![Value::Nil, Value::Nil],
            }))
        })?,
    )?;
    // CalculateAuctionDeposit(runTime[, count]) (`0x59d270` → `0x76dde0`): the house rate × the
    // vendor value of `count` items (else the held stack), × minutes / 240, at least 100; both
    // arguments truncate, the run time as `arg - 1` (`0x59d340`).
    g.set(
        "CalculateAuctionDeposit",
        lua.create_function(|lua, args: MultiValue| {
            let num = |i: usize| {
                args.get(i)
                    .and_then(|v| lua.coerce_number(v.clone()).ok().flatten())
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
            // The count truncates to its low dword and the value is a 32-bit product (`0x59d3c2`).
            let count = num(1).map_or_else(|| cursor::held_count(&model, it), |c| c as i64 as u32);
            let unit = model
                .item_templates
                .get(&it.item_id)
                .map_or(0, |t| t.sell_price);
            Ok(i64::from(wrath_deposit(
                rate,
                unit.wrapping_mul(count),
                deposit_minutes(run),
            )))
        })?,
    )?;
    Ok(())
}

/// `CalculateAuctionDeposit`'s run time: entry `trunc(arg - 1)` of `0xa15248`, else 1440.
fn deposit_minutes(arg: f64) -> u32 {
    let index = (arg - 1.0) as i64 as u32;
    [720, 1440, 2880]
        .get(index as usize)
        .copied()
        .unwrap_or(1440)
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

/// `0x76dde0`: `trunc(minutes × floor(rate × value / 100) × 1/240)`, at least 100, the product
/// 32-bit and the scale the f32 at `0xa394c8`; a duration off the table pays 100.
fn wrath_deposit(rate: u32, value: u32, minutes: u32) -> u32 {
    const SCALE: f32 = 1.0 / 240.0;
    if ![720, 1440, 2880].contains(&minutes) {
        return 100;
    }
    let base = rate.wrapping_mul(value) / 100;
    let n = f64::from(base) * f64::from(minutes) * f64::from(SCALE);
    (n as i64 as u32).max(100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::auction::{
        AuctionBid, AuctionHighBidder, AuctionItemRow, AuctionState, LIST,
    };
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
        // The f32 1/240 sits 5.2e-8 high and the product truncates: past ~1,900 gold, N + k.
        assert_eq!(wrath_deposit(5, 400_000_000, 2880), 240_000_012);
        // `rate × value` is a 32-bit multiply (`0x76de0c`).
        assert_eq!(wrath_deposit(100, 1_000_000_000, 2880), 145_890_259);
        // The run time indexes by `trunc(arg - 1)` (`0x59d340`): 1.6 is 720, 0.5 is 720 too.
        assert_eq!(deposit_minutes(1.6), 720);
        assert_eq!(deposit_minutes(0.5), 720);
        assert_eq!(deposit_minutes(3.9), 2880);
        assert_eq!(deposit_minutes(4.0), 1440);
        assert_eq!(deposit_minutes(-0.5), 1440);
    }

    fn bid_row(min_bid: u32, buyout_price: u32, bid_amount: u32) -> AuctionItemRow {
        AuctionItemRow {
            auction_id: 7,
            min_bid,
            min_increment: 100,
            buyout_price,
            bid_amount,
            ..Default::default()
        }
    }

    /// `PlaceAuctionBid` (`0x59da40`) caps at the buyout and refuses locally, raising the error
    /// and sending nothing.
    #[test]
    fn a_bid_is_capped_and_refused_before_it_is_sent() {
        let mut s = UiScript::new().unwrap();
        s.set_money(3000);
        let mut state = AuctionState::default();
        let mut own = bid_row(1000, 0, 2000);
        own.high_bidder = AuctionHighBidder::Player;
        state.lists[LIST].rows = vec![bid_row(1000, 2500, 0), bid_row(1000, 0, 2000), own];
        s.run("PlaceAuctionBid('list', 1, 500)").unwrap();
        assert!(
            s.take_auction_bids().is_empty(),
            "nothing without an auctioneer"
        );
        s.set_auction(Some(state));
        let bid = |s: &mut UiScript, call: &str| {
            s.run(call).unwrap();
            (s.take_auction_bids(), s.take_ui_errors())
        };
        let sent = |index, amount| {
            vec![AuctionBid {
                list: LIST,
                index,
                amount,
            }]
        };
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('list', 1, 500)"),
            (vec![], vec!["ERR_AUCTION_MIN_BID"])
        );
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('LIST', 1, 9000)"),
            (sent(1, 2500), vec![]),
            "capped"
        );
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('list', 2, 2000)"),
            (vec![], vec!["ERR_AUCTION_HIGHER_BID"])
        );
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('list', 2, 2050)"),
            (vec![], vec!["ERR_AUCTION_BID_INCREMENT"])
        );
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('list', 2, 3100)"),
            (vec![], vec!["ERR_NOT_ENOUGH_MONEY"])
        );
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('list', 3, 4900)"),
            (sent(3, 4900), vec![]),
            "our own bid is credited"
        );
        assert_eq!(
            bid(&mut s, "PlaceAuctionBid('list', 9, 4900)"),
            (vec![], vec![]),
            "no row"
        );
        let err = s.run("PlaceAuctionBid('list', 1)").unwrap_err().to_string();
        assert!(
            err.contains("Usage: PlaceAuctionBid(\"type\", index, bid)"),
            "{err}"
        );
    }

    /// `CanSendAuctionQuery` (`0x59c740`) and `IsAuctionSortReversed` (`0x59ab90`) answer two values.
    #[test]
    fn the_query_gate_and_the_sort_direction_answer_two_values() {
        let mut s = UiScript::new().unwrap();
        let pair = |s: &UiScript, call: &str| -> String {
            s.eval(&format!(
                "local n = select('#', {call}) local a, b = {call} \
                             return n .. ' ' .. tostring(a) .. ' ' .. tostring(b)"
            ))
            .unwrap()
        };
        s.set_auction_can_query(false);
        assert_eq!(pair(&s, "CanSendAuctionQuery()"), "2 nil nil");
        assert_eq!(
            pair(&s, "CanSendAuctionQuery('owner')"),
            "2 1 nil",
            "unthrottled list"
        );
        s.set_auction_can_query(true);
        assert_eq!(pair(&s, "CanSendAuctionQuery('list')"), "2 1 nil");
        let mut state = AuctionState::default();
        state.lists[LIST].sort = vec![("quality".into(), true), ("level".into(), false)];
        s.set_auction(Some(state));
        assert_eq!(
            pair(&s, "IsAuctionSortReversed('LIST', 'Quality')"),
            "2 1 1"
        );
        assert_eq!(
            pair(&s, "IsAuctionSortReversed('list', 'level')"),
            "2 nil 1"
        );
        assert_eq!(
            pair(&s, "IsAuctionSortReversed('list', 'bid')"),
            "2 nil nil",
            "not on the stack"
        );
        assert_eq!(
            pair(&s, "IsAuctionSortReversed('nope', 'bid')"),
            "2 nil nil"
        );
        assert_eq!(
            s.arity("IsAuctionSortReversed('list', 'nope')").unwrap(),
            0,
            "no such column"
        );
        let err = s
            .run("IsAuctionSortReversed('list')")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Usage: IsAuctionSortReversed(\"type\", \"sort\")"),
            "{err}"
        );
    }
}
