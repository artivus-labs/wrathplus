//! `FollowUnit` and `FollowByName`; each call queues a [`FollowRequest`] the app drains.
//! The stock `/follow` is `FollowUnit(msg)` (`ChatFrame.lua:1506-1507`) and the unit menu's is
//! `FollowUnit(fullname, 1)` (`UnitPopup.lua:1324-1325`). `FollowByName` is 1.12's (`0x489ec0`);
//! 12340 has no such global (no `FollowByName` string in `Wow.exe`).

use mlua::Lua;

use super::unit::token::NOBODY;
use super::Model;

/// A follow intent queued by the globals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowRequest {
    /// A unit token and the guid `0x60abf0` gave it, 0 for none; `text` picks the miss's key.
    ByUnit { text: String, guid: u64 },
    /// A name for `0x520190`'s name pass; `exact` is `FollowUnit`'s second argument.
    ByName { name: String, exact: bool },
}

impl super::UiScript {
    /// Drain the follow intents queued since the last call.
    pub fn take_follow_requests(&mut self) -> Vec<FollowRequest> {
        std::mem::take(&mut self.model_mut().follow_requests)
    }

    /// `FollowUnit(text, exact)` for a `/follow` line that skipped the edit box.
    pub fn queue_follow(&mut self, text: &str, exact: bool) {
        let mut model = self.model_mut();
        let request = follow_request(&model, text.to_owned(), exact);
        model.follow_requests.push(request);
    }
}

/// `FollowUnit`'s resolve (`0x5224c0` → `0x520190`): the token pass `0x60abf0` with flag 1, so an
/// empty token is the target (`0x60b029`); a token naming nobody, or none at all, goes to the
/// name pass (`0x5201f8`-`0x520203`).
fn follow_request(model: &Model, text: String, exact: bool) -> FollowRequest {
    let resolved = if text.is_empty() {
        Some(model.unit_guids.target)
    } else {
        let r = model.resolve_token(&text);
        (r.ok && r.guid != NOBODY).then_some(r.guid)
    };
    match resolved {
        Some(guid) => FollowRequest::ByUnit { text, guid },
        None => FollowRequest::ByName { name: text, exact },
    }
}

/// Register the follow globals.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // FollowUnit(unitOrName [, exactMatch]) (12340 `0x5224c0`); the flag reads `0x815500(L, 2, 0)`.
    g.set(
        "FollowUnit",
        lua.create_function(|lua, (who, exact): (mlua::Value, Option<mlua::Value>)| {
            let who = super::unit::token::token_text(lua, &who).unwrap_or_default();
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let request = follow_request(&model, who, truthy(exact));
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

    /// A unit token follows the guid `0x60abf0` gives it; an empty or missing one is the target; a
    /// name, or a token naming nobody, goes to the name pass with the exact flag.
    #[test]
    fn follow_unit_takes_a_token_or_a_name() {
        let mut s = UiScript::new().unwrap();
        s.model_mut().unit_guids.target = 0x77;
        s.model_mut().unit_guids.focus = 0x99;
        s.run(r#"FollowUnit("target"); FollowUnit("Ally", 1); FollowUnit("Targon")"#)
            .unwrap();
        s.run(r#"FollowUnit(""); FollowUnit(); FollowUnit("FOCUS"); FollowUnit("party1")"#)
            .unwrap();
        s.model_mut().unit_guids.target = 0;
        s.run(r#"FollowUnit()"#).unwrap();
        let unit = |text: &str, guid| FollowRequest::ByUnit {
            text: text.into(),
            guid,
        };
        let name = |name: &str, exact| FollowRequest::ByName {
            name: name.into(),
            exact,
        };
        assert_eq!(
            s.take_follow_requests(),
            [
                unit("target", 0x77),
                name("Ally", true),
                // A token base with an unparsable tail is a name.
                name("Targon", false),
                unit("", 0x77),
                unit("", 0x77),
                // Every base `0x60abf0` knows, case-insensitive.
                unit("FOCUS", 0x99),
                // Recognised but naming nobody: the name pass gets the text.
                name("party1", false),
                // No target: the token pass's guid 0, a miss.
                unit("", 0),
            ]
        );
    }
}
