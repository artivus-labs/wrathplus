//! `FollowUnit` and `FollowByName`; each call queues a [`FollowRequest`] the app drains.
//! `FollowUnit("target")` is a bare `/follow` and the `FOLLOWTARGET` binding. 12340's `FollowUnit`
//! (`0x5224c0`) also takes a player's name with an exact-only flag, which the stock menu passes
//! (`UnitPopup.lua:1324-1325`, `FollowUnit(fullname, 1)`), so it never prefix-matches someone
//! else, while `/follow rag` may. Labelled: `FollowByName` keeps 1.12's reading (`0x489ec0`); its
//! 12340 binding is untraced. WrathPlus parses `/follow` in Rust, so these globals serve the stock
//! UI and addons.

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
    // popup's `FollowUnit(fullname, 1)` (UnitPopup.lua:1324-1325) follows by name.
    g.set(
        "FollowUnit",
        lua.create_function(|lua, (who, exact): (String, Option<mlua::Value>)| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let request = if model.unit_guids.guid_of(&who).is_ok() {
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
    /// name exactly.
    #[test]
    fn follow_unit_takes_a_token_or_a_name() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"FollowUnit("target"); FollowUnit("Ally", 1); FollowUnit("Bob")"#)
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
            ]
        );
    }
}
