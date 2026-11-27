//! Build-12340 petition vendor, the arena registrar (ArenaRegistrarFrame.lua): its charter rows,
//! the purchase, the banner turn-in and the close. One Wow.exe pass (Experience's
//! `/tmp/azb-exp-misc-clean.log`): `BuyPetition` `0x5a2530`, `ClosePetitionVendor` `0x5a24f0`,
//! `GetPetitionItemInfo` `0x5a1fb0`, `HasFilledPetition` `0x5a2820`, `TurnInArenaPetition`
//! `0x5a26d0`. The host owns the vendor (`PETITION_VENDOR_SHOW`/`_UPDATE`/`_CLOSED`) and the bag
//! scan behind `HasFilledPetition` and the turn-in (`0x6dd680`).

use mlua::{Lua, MultiValue, Value};

/// One charter row as the host publishes it: the item's name and icon from the item cache (`None`
/// while its query is out, `GetPetitionItemInfo`'s nil), and the row's price.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathPetitionItem {
    pub name: Option<String>,
    pub texture: Option<String>,
    pub cost: u32,
}

/// One petition-vendor verb for the host, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathPetitionVendorRequest {
    /// `BuyPetition(index, name)` after the shared name check: the row, 1-based, and the name.
    Buy { index: u32, name: String },
    /// A name the check refused, with the GlobalStrings key to show; no packet.
    NameRefused(&'static str),
    /// `ClosePetitionVendor()` with a vendor open, which fired `PETITION_VENDOR_CLOSED` (`0x1e2`)
    /// itself: the host ends the NPC session.
    Close,
    /// `TurnInArenaPetition(teamSize, ...)`: the bracket and the banner, background colour,
    /// emblem, emblem colour, border, border colour; every word `0xffffffff` when any of the eleven
    /// banner arguments is missing.
    TurnIn { team_size: i64, banner: [u32; 5] },
}

#[derive(Default)]
pub(crate) struct State {
    /// The open vendor's rows (`0xc0f6d8`, `0xc0f7d0` of them); `None` while none is open.
    rows: Option<Vec<WrathPetitionItem>>,
    /// The host's bag scan (`0x6dd680`): an arena charter is carried.
    has_filled: bool,
    requests: Vec<WrathPetitionVendorRequest>,
}

impl super::UiScript {
    /// Open the vendor with its rows, or close it with `None`.
    pub fn set_wrath_petition_vendor(&mut self, rows: Option<Vec<WrathPetitionItem>>) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            state.rows = rows;
        }
    }

    /// `HasFilledPetition`'s answer, from the host's bag scan.
    pub fn set_wrath_has_filled_petition(&mut self, filled: bool) {
        if let Some(mut state) = self.lua.app_data_mut::<State>() {
            state.has_filled = filled;
        }
    }

    /// Drain the vendor verbs in call order.
    pub fn take_wrath_petition_vendor_requests(&mut self) -> Vec<WrathPetitionVendorRequest> {
        self.lua
            .app_data_mut::<State>()
            .map(|mut s| std::mem::take(&mut s.requests))
            .unwrap_or_default()
    }
}

fn push(lua: &Lua, request: WrathPetitionVendorRequest) {
    if let Some(mut state) = lua.app_data_mut::<State>() {
        state.requests.push(request);
    }
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// A Lua number, as the reference's `lua_isnumber` admits (a numeric string included).
fn number(lua: &Lua, v: Value) -> Option<f64> {
    match v {
        Value::Integer(n) => Some(n as f64),
        Value::Number(n) => Some(n),
        Value::String(_) => lua.coerce_number(v).ok().flatten(),
        _ => None,
    }
}

/// One `0x61be50` colour: opaque ARGB from the frame's 0..1 vertex colour. Labelled: the packer is
/// untraced; this is the inverse of the arena tabard's `ARGB / 255` reader (`0xa45564`).
fn argb(r: f64, g: f64, b: f64) -> u32 {
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    0xff00_0000 | c(r) << 16 | c(g) << 8 | c(b)
}

/// The turn-in's banner from the eleven arguments after the team size (`0x5a2733`-`0x5a27c3`);
/// the two styles cut by `__ftol2_sse` (`0x5a277e`, `0x5a27a1`).
fn banner(lua: &Lua, args: &MultiValue) -> [u32; 5] {
    let n: Option<Vec<f64>> = (1..12).map(|i| number(lua, arg(args, i))).collect();
    let Some(n) = n else {
        return [u32::MAX; 5];
    };
    [
        argb(n[0], n[1], n[2]),
        super::binding_abi::ftol(n[3]) as u32,
        argb(n[4], n[5], n[6]),
        super::binding_abi::ftol(n[7]) as u32,
        argb(n[8], n[9], n[10]),
    ]
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let g = lua.globals();

    // BuyPetition(index, name) (`0x5a2530`): the shared name check, whose refusal shows its line
    // and returns nil; otherwise the purchase is queued and 1 returned.
    g.set(
        "BuyPetition",
        lua.create_function(|lua, args: MultiValue| {
            let index = number(lua, arg(&args, 0));
            let name = match arg(&args, 1) {
                Value::String(s) => Some(s.to_string_lossy()),
                v @ (Value::Integer(_) | Value::Number(_)) => {
                    lua.coerce_string(v)?.map(|s| s.to_string_lossy())
                }
                _ => None,
            };
            let (Some(index), Some(name)) = (index, name) else {
                return Err(mlua::Error::runtime("Usage: BuyPetition(index, name)"));
            };
            match super::petition::validate_guild_name(&name) {
                Ok(()) => {
                    let index = index as i64 as u32; // `fistp` under chop
                    push(lua, WrathPetitionVendorRequest::Buy { index, name });
                    Ok(Value::Integer(1))
                }
                Err(key) => {
                    push(lua, WrathPetitionVendorRequest::NameRefused(key));
                    Ok(Value::Nil)
                }
            }
        })?,
    )?;

    // ClosePetitionVendor() (`0x5a24f0`): only with a vendor open.
    g.set(
        "ClosePetitionVendor",
        lua.create_function(|lua, ()| {
            let open = lua
                .app_data_mut::<State>()
                .is_some_and(|mut s| s.rows.take().is_some());
            if open {
                push(lua, WrathPetitionVendorRequest::Close);
                super::tick::fire_event_into(lua, "PETITION_VENDOR_CLOSED", Vec::new());
            }
            Ok(())
        })?,
    )?;

    // GetPetitionItemInfo(index) (`0x5a1fb0`) → name, texture, price; nil, nil, 0 for a row out of
    // range or whose item is not cached yet.
    g.set(
        "GetPetitionItemInfo",
        lua.create_function(|lua, args: MultiValue| {
            let Some(index) = number(lua, arg(&args, 0)) else {
                return Err(mlua::Error::runtime("Usage: GetPetitionItemInfo(index)"));
            };
            let row = lua.app_data_ref::<State>().and_then(|s| {
                let i = usize::try_from(i64::from(super::binding_abi::ftol(index)).checked_sub(1)?)
                    .ok()?;
                s.rows.as_ref()?.get(i).cloned()
            });
            let text = |s: Option<String>| -> mlua::Result<Value> {
                s.map_or(Ok(Value::Nil), |s| {
                    Ok(Value::String(lua.create_string(&s)?))
                })
            };
            Ok(match row.filter(|r| r.name.is_some()) {
                Some(r) => MultiValue::from_vec(vec![
                    text(r.name)?,
                    text(Some(r.texture.unwrap_or_default()))?,
                    Value::Integer(i64::from(r.cost)),
                ]),
                None => MultiValue::from_vec(vec![Value::Nil, Value::Nil, Value::Integer(0)]),
            })
        })?,
    )?;

    // HasFilledPetition() (`0x5a2820`) → 1 or nil.
    g.set(
        "HasFilledPetition",
        lua.create_function(|lua, ()| {
            let filled = lua.app_data_ref::<State>().is_some_and(|s| s.has_filled);
            Ok(if filled {
                Value::Integer(1)
            } else {
                Value::Nil
            })
        })?,
    )?;

    // TurnInArenaPetition(teamSize, bgR, bgG, bgB, emblem, eR, eG, eB, border, bR, bG, bB)
    // (`0x5a26d0`): only with a vendor open; the host finds the bracket's charter.
    g.set(
        "TurnInArenaPetition",
        lua.create_function(|lua, args: MultiValue| {
            let Some(size) = number(lua, arg(&args, 0)) else {
                return Err(mlua::Error::runtime("Usage: TurnInArenaPetition(teamSize)"));
            };
            let open = lua
                .app_data_ref::<State>()
                .is_some_and(|s| s.rows.is_some());
            if open {
                let banner = banner(lua, &args);
                push(
                    lua,
                    WrathPetitionVendorRequest::TurnIn {
                        team_size: i64::from(size as i64 as i32), // `fistp` (`0x5a273c`)
                        banner,
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
    use crate::script::UiScript;

    fn vm() -> UiScript {
        UiScript::new().unwrap()
    }

    #[test]
    fn the_rows_answer_their_item_and_price_once_named() {
        let mut s = vm();
        assert!(s
            .eval::<bool>(
                "local n, t, c = GetPetitionItemInfo(1) return n == nil and t == nil and c == 0"
            )
            .unwrap());
        s.set_wrath_petition_vendor(Some(vec![
            WrathPetitionItem {
                name: Some("Arena Team Charter".into()),
                texture: Some("Interface\\Icons\\INV_Misc_StoneTablet_04".into()),
                cost: 800000,
            },
            WrathPetitionItem::default(),
        ]));
        assert!(s
            .eval::<bool>(
                "local n, t, c = GetPetitionItemInfo(1) \
                 local n2, _, c2 = GetPetitionItemInfo(2) \
                 return n == 'Arena Team Charter' and t:find('StoneTablet') ~= nil and c == 800000 \
                 and n2 == nil and c2 == 0"
            )
            .unwrap());
        assert!(s.run("GetPetitionItemInfo()").is_err());
        assert!(s.eval::<bool>("return HasFilledPetition() == nil").unwrap());
        s.set_wrath_has_filled_petition(true);
        assert_eq!(s.eval::<i64>("return HasFilledPetition()").unwrap(), 1);
    }

    #[test]
    fn buy_close_and_turn_in_queue_in_order() {
        let mut s = vm();
        s.run("TurnInArenaPetition(2) ClosePetitionVendor()")
            .unwrap();
        assert!(
            s.take_wrath_petition_vendor_requests().is_empty(),
            "no vendor open"
        );
        s.set_wrath_petition_vendor(Some(vec![WrathPetitionItem::default()]));
        assert_eq!(
            s.eval::<i64>("return BuyPetition(2, 'Gladiators')")
                .unwrap(),
            1
        );
        assert!(s.eval::<bool>("return BuyPetition(2, '') == nil").unwrap());
        s.run("TurnInArenaPetition(3.9, 1, 0, 0, 12.9, 0, 0, 1, 3.9, 0.5, 0.5, 0.5)")
            .unwrap();
        s.run("TurnInArenaPetition(5)").unwrap();
        s.run("ClosePetitionVendor() ClosePetitionVendor()")
            .unwrap();
        assert_eq!(
            s.take_wrath_petition_vendor_requests(),
            [
                WrathPetitionVendorRequest::Buy {
                    index: 2,
                    name: "Gladiators".into()
                },
                WrathPetitionVendorRequest::NameRefused("ERR_NAME_NO_NAME"),
                WrathPetitionVendorRequest::TurnIn {
                    team_size: 3,
                    banner: [0xffff_0000, 12, 0xff00_00ff, 3, 0xff80_8080]
                },
                WrathPetitionVendorRequest::TurnIn {
                    team_size: 5,
                    banner: [u32::MAX; 5]
                },
                WrathPetitionVendorRequest::Close,
            ]
        );
        assert!(s.run("BuyPetition(1)").is_err() && s.run("TurnInArenaPetition()").is_err());
    }
}
