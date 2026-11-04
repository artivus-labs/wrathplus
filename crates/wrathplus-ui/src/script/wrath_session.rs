//! Build 12340's session and client queries (`/tmp/azb-app-mig/ghidra/{last17,cvarc,rsc}.log`):
//! the expansion levels, the installed locales, the four CVar range verbs, the static-constants
//! table and the combat-text unit. The host pushes the account's expansion and the installed
//! locales ([`UiScript::set_wrath_session`]).

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript};

/// `0x4ddc20`'s byte (the realm/account record `+0x2f2d`) and the installed locales
/// (`0x511c80`'s flag table `0xc5de90`, in the order of its names `0xad3028`).
#[derive(Default)]
struct Session {
    expansion: Option<u8>,
    locales: Vec<String>,
}

/// `STATIC_CONSTANTS` (`0xacfac0`): the seven names `RegisterStaticConstants` writes, valued in
/// table order.
const STATIC_CONSTANTS: [&str; 7] = [
    "STATIC_CONSTANTS",
    "Loot",
    "AuctionHouse",
    "Mail",
    "Chat",
    "Movement",
    "Spell",
];

/// `farclip`'s range in the in-game table (`0x9f5798`, `0x9f57d8`).
const FARCLIP_MIN: f64 = 176.0;
const FARCLIP_MAX: f64 = 1250.0;

impl UiScript {
    /// Publish the account's expansion (`SMSG_AUTH_RESPONSE`, `None` before it) and the
    /// installed locale codes.
    pub fn set_wrath_session(&mut self, expansion: Option<u8>, locales: Vec<String>) {
        self.lua.set_app_data(Session { expansion, locales });
    }

    /// The guid `CombatTextSetActiveUnit` named (`0xca1380`), 0 for none.
    pub fn combat_text_unit(&self) -> u64 {
        self.model_mut().combat_text_unit
    }
}

/// One of the range verbs: the CVar named by a string argument (raising `usage` for anything
/// else, and `Couldn't find CVar named` for an unknown name), answered by `pick` over its
/// lower-cased name.
fn install_range(
    lua: &Lua,
    name: &str,
    usage: &'static str,
    pick: fn(&str) -> Option<f64>,
) -> mlua::Result<()> {
    lua.globals().set(
        name,
        lua.create_function(move |lua, cvar: Value| {
            let Value::String(cvar) = cvar else {
                return Err(mlua::Error::runtime(usage));
            };
            let cvar = cvar.to_str()?.to_string();
            let known = lua
                .app_data_ref::<Model>()
                .expect("model app_data")
                .cvars
                .contains_key(&cvar.to_ascii_lowercase());
            if !known {
                return Err(mlua::Error::runtime(format!(
                    "Couldn't find CVar named '{cvar}'"
                )));
            }
            Ok(pick(&cvar.to_ascii_lowercase()))
        })?,
    )
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(Session::default());
    let g = lua.globals();
    // GetAccountExpansionLevel() and GetExpansionLevel() bind the one reader (`0x4ddc20`).
    for name in ["GetAccountExpansionLevel", "GetExpansionLevel"] {
        g.set(
            name,
            lua.create_function(|lua, ()| {
                Ok(lua
                    .app_data_ref::<Session>()
                    .and_then(|s| s.expansion)
                    .map_or(0, i64::from))
            })?,
        )?;
    }
    // GetExistingLocales() (`0x511c80`): each installed locale's code, one value per locale.
    g.set(
        "GetExistingLocales",
        lua.create_function(|lua, ()| {
            let locales = lua
                .app_data_ref::<Session>()
                .map(|s| s.locales.clone())
                .unwrap_or_default();
            let values = locales
                .iter()
                .map(|l| lua.create_string(l).map(Value::String))
                .collect::<mlua::Result<Vec<_>>>()?;
            Ok(MultiValue::from_vec(values))
        })?,
    )?;
    // The range verbs answer only the two CVars build 12340 bounds, nil for the rest. Labelled:
    // `extShadowQuality`'s maxima are the hardware's supported shadow levels (`0x873f50`,
    // `0x8740d0`); WrathPlus draws none, so they read 0.
    install_range(
        lua,
        "GetCVarMin",
        "Usage: GetCVarMin(\"cvar\")",
        |name| match name {
            "extshadowquality" => Some(0.0),
            "farclip" => Some(FARCLIP_MIN),
            _ => None,
        },
    )?;
    install_range(
        lua,
        "GetCVarMax",
        "Usage: GetCVarMax(\"cvar\")",
        |name| match name {
            "extshadowquality" => Some(0.0),
            "farclip" => Some(FARCLIP_MAX),
            _ => None,
        },
    )?;
    install_range(
        lua,
        "GetCVarAbsoluteMin",
        "Usage: GetCVarAbsoluteMin(\"cvar\")",
        |name| (name == "extshadowquality").then_some(0.0),
    )?;
    install_range(
        lua,
        "GetCVarAbsoluteMax",
        "Usage: GetCVarAbsoluteMax(\"cvar\")",
        |name| (name == "extshadowquality").then_some(0.0),
    )?;
    // RegisterStaticConstants(table) (`0x5ac320`): the seven names valued 0-6; a non-table
    // raises (labelled: the native usage text is unread).
    g.set(
        "RegisterStaticConstants",
        lua.create_function(|_, table: Value| {
            let Value::Table(table) = table else {
                return Err(mlua::Error::runtime(
                    "Usage: RegisterStaticConstants(table)",
                ));
            };
            for (i, name) in STATIC_CONSTANTS.iter().enumerate() {
                table.set(*name, i as i64)?;
            }
            Ok(())
        })?,
    )?;
    // CombatTextSetActiveUnit(unit) (`0x74d580`): the guid the token names, 0 for none.
    // Labelled: the host's combat-text feed still reads the player's events.
    g.set(
        "CombatTextSetActiveUnit",
        lua.create_function(|lua, token: Option<String>| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let guid = token
                .as_deref()
                .and_then(|t| model.unit(t))
                .filter(|u| u.exists)
                .map_or(0, |u| u.guid);
            model.combat_text_unit = guid;
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{UiScript, UnitState};

    #[test]
    fn the_session_queries_answer_the_pushed_account_and_the_native_ranges() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetExpansionLevel()").unwrap(), 0);
        s.set_wrath_session(Some(2), vec!["enUS".into()]);
        for check in [
            "GetAccountExpansionLevel() == 2 and GetExpansionLevel() == 2",
            "select('#', GetExistingLocales()) == 1 and GetExistingLocales() == 'enUS'",
            "GetCVarMin('farclip') == 176 and GetCVarMax('farClip') == 1250",
            "GetCVarAbsoluteMax('extShadowQuality') == 0 and GetCVarMin('scriptErrors') == nil",
        ] {
            assert!(
                s.eval::<bool>(&format!("return {check}")).unwrap(),
                "{check}"
            );
        }
        assert!(s.run("GetCVarMin('nosuchcvar')").is_err());
        assert!(s.run("GetCVarMax()").is_err());
        s.run("T = {} RegisterStaticConstants(T)").unwrap();
        assert!(s
            .eval::<bool>("return T.STATIC_CONSTANTS == 0 and T.Mail == 3 and T.Spell == 6")
            .unwrap());
        assert!(s.run("RegisterStaticConstants(1)").is_err());
        s.set_unit(
            "target",
            Some(UnitState {
                exists: true,
                guid: 0x99,
                ..Default::default()
            }),
        );
        s.run("CombatTextSetActiveUnit('target')").unwrap();
        assert_eq!(s.combat_text_unit(), 0x99);
        s.run("CombatTextSetActiveUnit('focus')").unwrap();
        assert_eq!(s.combat_text_unit(), 0);
    }
}
