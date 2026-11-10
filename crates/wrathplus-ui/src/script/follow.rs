//! `FollowUnit` and `FollowByName`; each call queues a [`FollowRequest`] the app drains.
//! The stock `/follow` is `FollowUnit(msg)` (`ChatFrame.lua:1506-1507`), so a bare one is
//! `FollowUnit("")`, the target, as the `FOLLOWTARGET` binding's `FollowUnit("target")` is.
//! 12340's `FollowUnit` (`0x5224c0`) also takes a player's name with an exact-only flag, which the
//! stock menu passes (`UnitPopup.lua:1324-1325`, `FollowUnit(fullname, 1)`), so it never
//! prefix-matches someone else, while `/follow rag` may. Labelled: `FollowByName` keeps 1.12's
//! reading (`0x489ec0`); its 12340 binding is untraced.

use mlua::Lua;

use super::Model;

/// A follow intent queued by the globals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowRequest {
    /// `FollowUnit(token)`: whoever the token names.
    ByUnit(String),
    /// `FollowByName(name, exactMatch)`: with `exact`, the resolver skips its longest-prefix tier
    /// and takes only a whole-name, case-insensitive match.
    ByName { name: String, exact: bool },
}

impl super::UiScript {
    /// Drain the follow intents queued since the last call.
    pub fn take_follow_requests(&mut self) -> Vec<FollowRequest> {
        std::mem::take(&mut self.model_mut().follow_requests)
    }
}

/// Register the follow globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // FollowUnit(unitOrName [, exactMatch]) (12340 `0x5224c0`): the resolver `0x520190` takes a
    // unit token or a player's name (typemask `0x10`, then `0x8`) with the exact flag, so the stock
    // popup's `FollowUnit(fullname, 1)` (UnitPopup.lua:1324-1325) follows by name. It resolves
    // with flag 1 (`0x5201d8`), so a missing or empty token is the target (`0x60ac15`-`0x60ac20`,
    // `0x60b029`-`0x60b043`).
    g.set(
        "FollowUnit",
        lua.create_function(|lua, (who, exact): (mlua::Value, Option<mlua::Value>)| {
            let who = super::unit::token::token_text(lua, &who)
                .filter(|w| !w.is_empty())
                .unwrap_or_else(|| "target".into());
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let request = if super::unit::token_recognised(&who, true) {
                FollowRequest::ByUnit(who)
            } else {
                FollowRequest::ByName {
                    name: who,
                    exact: truthy(exact),
                }
            };
            model.follow_requests.push(request);
            Ok(())
        })?,
    )?;

    g.set(
        "FollowByName",
        lua.create_function(|lua, (name, exact): (String, Option<mlua::Value>)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.follow_requests.push(FollowRequest::ByName {
                name,
                exact: truthy(exact),
            });
            Ok(())
        })?,
    )?;

    Ok(())
}

/// The exact flag as `GetBoolOrDefault` reads it, default false: nil, false and 0 are false and any
/// other value is true.
fn truthy(v: Option<mlua::Value>) -> bool {
    match v {
        None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false)) => false,
        Some(mlua::Value::Integer(n)) => n != 0,
        Some(mlua::Value::Number(n)) => n != 0.0,
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::FollowRequest;
    use crate::script::UiScript;

    /// A unit token follows the unit; stock `UnitPopup.lua`'s `FollowUnit(name, 1)` follows the
    /// name exactly; a bare `/follow`'s `FollowUnit("")`, like no token at all, follows the target.
    #[test]
    fn follow_unit_takes_a_token_or_a_name() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"FollowUnit("target"); FollowUnit("Ally", 1); FollowUnit("Bob")"#)
            .unwrap();
        s.run(r#"FollowUnit(""); FollowUnit(); FollowUnit(nil)"#)
            .unwrap();
        assert_eq!(
            s.take_follow_requests(),
            [
                FollowRequest::ByUnit("target".into()),
                FollowRequest::ByName {
                    name: "Ally".into(),
                    exact: true,
                },
                FollowRequest::ByName {
                    name: "Bob".into(),
                    exact: false,
                },
                FollowRequest::ByUnit("target".into()),
                FollowRequest::ByUnit("target".into()),
                FollowRequest::ByUnit("target".into()),
            ]
        );
    }
}
