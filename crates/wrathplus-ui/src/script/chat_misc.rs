//! The engine verbs stock `ChatFrame.lua`'s slash handlers call that are not sends, channels or
//! window state: `DoEmote` (`0x500980`), `RandomRoll` (`0x516610`), `ConsoleExec` (`0x608270`),
//! `LoggingChat` and `LoggingCombat` (`0x4fbd00`, `0x4fbd20`), and the 1.12 `AssistByName` and
//! `UninviteByName`. Each is a queue or a flag the app drains.

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{bool_or_default, flag, optional_string, string_arg};
use super::cvars::sstr_to_int;
use super::Model;

/// `DoEmote(token [, target])`: the `EmotesText.dbc` name token (`"WAVE"`, not `/wave`) and an
/// optional target word (`0x500980` reads arg 2 with `lua_tolstring`): a unit token, a player
/// name, or a `%` word for the selection. A non-string, non-number second argument reads as none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmoteRequest {
    pub token: String,
    pub target: Option<String>,
    /// When `target` is a unit token, the guid the emote carries, 0 for none; `None` leaves the
    /// word to the app's player-name search.
    pub target_guid: Option<u64>,
}

/// `0x520190`'s resolver arm (`0x5201e5`) for a `DoEmote` word: `None` when it is no unit token, or
/// one naming nobody (`0x5201f8`), which go to the name search. A known object that is no player is
/// no target (`0x52022a`), a guid with no object goes as it is, and yourself goes untargeted
/// (`0x6dda9a`).
fn emote_unit(model: &Model, word: &str) -> Option<u64> {
    let r = model.resolve_token(word);
    if !r.ok || r.guid == super::unit::token::NOBODY {
        return None;
    }
    let guid = match model.unit_by_guid(r.guid) {
        Some(u) if u.has_object && !u.is_player => 0,
        _ => r.guid,
    };
    Some(if guid == model.local_guid() { 0 } else { guid })
}

impl super::UiScript {
    /// `DoEmote` calls since the last drain.
    pub fn take_emote_requests(&mut self) -> Vec<EmoteRequest> {
        std::mem::take(&mut self.model_mut().emote_requests)
    }

    /// `RandomRoll(min, max)` calls since the last drain, each a `CMSG_RANDOM_ROLL`.
    pub fn take_roll_requests(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.model_mut().roll_requests)
    }

    /// `UninviteByName` calls since the last drain.
    pub fn take_uninvite_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.model_mut().uninvite_requests)
    }

    /// `ConsoleExec` lines that write no CVar: the app's console commands (`reloadui`, …).
    pub fn take_console_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.model_mut().console_lines)
    }

    /// `(chat, combat)`, as `LoggingChat` and `LoggingCombat` last set them.
    pub fn logging_flags(&self) -> (bool, bool) {
        let model = self.model_mut();
        (model.logging_chat, model.logging_combat)
    }

    /// Whether a logging flag moved since the last drain: the app's cue to open or close its logs.
    pub fn take_logging_changes(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().logging_changed)
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // DoEmote("emote"[, "target"]) (`0x500980`): a non-string token raises; a target word that
    // is not empty and does not open with `%` goes to `0x520190`, whose first arm is the unit
    // resolver (`0x5201e5`).
    g.set(
        "DoEmote",
        lua.create_function(|lua, (token, target): (Value, Value)| {
            let token = string_arg(lua, token, "Usage: DoEmote(\"emote\"[, \"target\"])")?;
            if token.is_empty() {
                return Ok(());
            }
            let target = optional_string(lua, &target).filter(|t| !t.is_empty());
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let target_guid = target
                .as_deref()
                .filter(|t| !t.starts_with('%'))
                .and_then(|t| emote_unit(&model, t));
            model.emote_requests.push(EmoteRequest {
                token: token.to_ascii_uppercase(),
                target,
                target_guid,
            });
            Ok(())
        })?,
    )?;

    // RandomRoll(min, max) (`0x516610`): both must pass `lua_isstring`, so the one-argument form
    // its usage line offers raises too; each is read by `SStrToInt`, and no packet goes unless
    // 0 <= min <= max <= 1000000 and not both 0 (`0x516669`-`0x516687`).
    g.set(
        "RandomRoll",
        lua.create_function(|lua, (min, max): (Value, Value)| {
            const USAGE: &str = "Usage: RandomRoll(\"max\") or RandomRoll(\"min\", \"max\")";
            let min = sstr_to_int(&string_arg(lua, min, USAGE)?);
            let max = sstr_to_int(&string_arg(lua, max, USAGE)?);
            if (min == 0 && max == 0) || min < 0 || max < min || max > 1_000_000 {
                return Ok(());
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.roll_requests.push((min as u32, max as u32));
            Ok(())
        })?,
    )?;

    g.set(
        "AssistByName",
        lua.create_function(|lua, name: Option<String>| {
            let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
                return Ok(());
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.script_calls.push(super::ScriptCall::Select(
                super::SelectionRequest::AssistByName(name),
            ));
            Ok(())
        })?,
    )?;

    g.set(
        "UninviteByName",
        lua.create_function(|lua, name: Option<String>| {
            let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
                return Ok(());
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.uninvite_requests.push(name);
            Ok(())
        })?,
    )?;

    // `ConsoleExec("name value")` (`0x608270`), a non-string raising: a registered CVar with a
    // value is written at once, as `SetCVar` writes it; anything else, a bare CVar name included
    // (the reference answers it with `CVar "%s" is "%s"`), goes to the app's command registry.
    g.set(
        "ConsoleExec",
        lua.create_function(|lua, line: Value| {
            let line = string_arg(lua, line, "Usage: ConsoleExec(\"console_command\")")?;
            let line = line.trim().to_string();
            if line.is_empty() {
                return Ok(());
            }
            let (name, value) = line
                .split_once(char::is_whitespace)
                .map_or((line.as_str(), ""), |(n, v)| (n, v.trim()));
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let is_cvar = model.cvars.contains_key(&name.to_ascii_lowercase());
            if is_cvar && !value.is_empty() {
                super::cvars::write_cvar(&mut model, name, value.to_string());
            } else {
                model.console_lines.push(line.clone());
            }
            Ok(())
        })?,
    )?;

    // LoggingChat([on]) and LoggingCombat([on]) (`0x4fbc60`): any argument, an explicit nil
    // included (`lua_gettop`), sets the flag through `0x815500` with default 1; either way the
    // binding answers 1 or nil. The app opens the file a frame later, so a failed open does not
    // clear the flag here (`0x4fbcba`): the log module's deviation.
    for (name, chat) in [("LoggingChat", true), ("LoggingCombat", false)] {
        g.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let slot = if chat {
                    &mut model.logging_chat
                } else {
                    &mut model.logging_combat
                };
                let was = *slot;
                if !args.is_empty() {
                    *slot = bool_or_default(args.front(), true);
                }
                let on = *slot;
                model.logging_changed |= on != was;
                Ok(flag(on))
            })?,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{SelectionRequest, UiScript};

    #[test]
    fn do_emote_queues_the_uppercased_token_and_a_non_empty_target() {
        let mut s = UiScript::new().unwrap();
        s.run("DoEmote('wave', 'Bob') DoEmote('DANCE', '') DoEmote('')")
            .unwrap();
        assert_eq!(
            s.take_emote_requests(),
            vec![
                EmoteRequest {
                    token: "WAVE".into(),
                    target: Some("Bob".into()),
                    target_guid: None,
                },
                EmoteRequest {
                    token: "DANCE".into(),
                    target: None,
                    target_guid: None,
                },
            ]
        );
        for call in ["DoEmote()", "DoEmote({}, 'Bob')"] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(
                err.contains("Usage: DoEmote(\"emote\"[, \"target\"])"),
                "{err}"
            );
        }
        assert!(s.take_emote_requests().is_empty(), "drained");
    }

    /// `0x520190` tries the unit resolver before the name search: a token naming a player is that
    /// guid, a held non-player or yourself none (`0x52022a`, `0x6dda9a`); a word that is no token,
    /// or names nobody, and a `%` word stay words for the app.
    #[test]
    fn do_emote_resolves_a_unit_token_before_the_name_search() {
        let mut s = UiScript::new().unwrap();
        let (me, mob, ally) = (0x10, 0xF130_0000_0000_0001, 0x21);
        s.set_unit_guids(&crate::script::UnitGuids {
            player: me,
            target: mob,
            focus: ally,
            ..Default::default()
        });
        for (guid, is_player) in [(mob, false), (ally, true)] {
            let unit = crate::script::UnitState {
                exists: true,
                has_object: true,
                is_player,
                ..Default::default()
            };
            s.set_unit_by_guid(guid, Some(unit));
        }
        s.run(
            "for _, t in ipairs({'focus', 'target', 'player', 'party1', 'Bob', '%t'}) do \
             DoEmote('WAVE', t) end DoEmote('WAVE', {})",
        )
        .unwrap();
        let got: Vec<_> = s
            .take_emote_requests()
            .into_iter()
            .map(|e| (e.target, e.target_guid))
            .collect();
        let word = |w: &str| (Some(w.to_string()), None);
        assert_eq!(
            got,
            vec![
                (Some("focus".into()), Some(ally)),
                (Some("target".into()), Some(0)),
                (Some("player".into()), Some(0)),
                word("party1"),
                word("Bob"),
                word("%t"),
                (None, None),
            ]
        );
    }

    /// `0x516610`: both arguments `lua_isstring`, each read by `SStrToInt` (no whitespace, no `+`,
    /// no exponent), and only `0 <= min <= max <= 1000000`, not both 0, is sent.
    #[test]
    fn random_roll_reads_both_strings_and_sends_only_a_valid_range() {
        let mut s = UiScript::new().unwrap();
        s.run(
            "RandomRoll('1', '100') RandomRoll(5, 10) RandomRoll('0', '0') RandomRoll('5', '3') \
             RandomRoll('-1', '5') RandomRoll('1', '1000001') RandomRoll('0', '1e2') \
             RandomRoll(' 5', '10')",
        )
        .unwrap();
        assert_eq!(
            s.take_roll_requests(),
            vec![(1, 100), (5, 10), (0, 1), (0, 10)]
        );
        for call in ["RandomRoll('100')", "RandomRoll('x', nil)", "RandomRoll()"] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(err.contains("Usage: RandomRoll(\"max\")"), "{call}: {err}");
        }
    }

    #[test]
    fn assist_and_uninvite_by_name_queue_their_names() {
        let mut s = UiScript::new().unwrap();
        s.run("AssistByName('Alice') AssistByName('') UninviteByName('Carol') UninviteByName()")
            .unwrap();
        assert_eq!(
            s.take_selection_requests(),
            vec![SelectionRequest::AssistByName("Alice".into())]
        );
        assert_eq!(s.take_uninvite_requests(), vec!["Carol".to_string()]);
    }

    #[test]
    fn console_exec_writes_a_registered_cvar_and_queues_anything_else() {
        let mut s = UiScript::new().unwrap();
        s.register_cvars([("benchUiScale", "1")]);
        s.run("ConsoleExec('benchuiscale 0.8') ConsoleExec('reloadui') ConsoleExec('  ') ConsoleExec('benchUiScale')")
            .unwrap();
        let err = s.run("ConsoleExec()").unwrap_err().to_string();
        assert!(
            err.contains("Usage: ConsoleExec(\"console_command\")"),
            "{err}"
        );
        assert_eq!(
            s.eval::<String>("return GetCVar('benchUiScale')").unwrap(),
            "0.8",
            "the CVar store is the same one SetCVar writes, matched case-insensitively"
        );
        // A bare CVar name, whose value the reference prints, goes to the command lane.
        assert_eq!(
            s.take_console_lines(),
            vec!["reloadui".to_string(), "benchUiScale".to_string()]
        );
        assert_eq!(
            s.take_cvar_changes(),
            vec![("benchUiScale".to_string(), "0.8".to_string())],
            "a bare name writes nothing"
        );
    }

    /// `0x4fbc60`: one value either way, 1 or nil; any argument sets through `0x815500`, so an
    /// explicit nil and "no" turn it off.
    #[test]
    fn the_logging_flags_answer_one_or_nil_and_any_argument_sets_them() {
        let mut s = UiScript::new().unwrap();
        assert!(s.eval::<bool>("return LoggingChat() == nil").unwrap());
        assert!(s.eval::<bool>("return LoggingCombat() == nil").unwrap());
        assert_eq!(s.eval::<i64>("return LoggingChat(true)").unwrap(), 1);
        assert_eq!(s.eval::<i64>("return LoggingChat()").unwrap(), 1);
        assert_eq!(s.logging_flags(), (true, false));
        assert!(s.take_logging_changes());
        assert_eq!(s.arity("LoggingChat(nil)").unwrap(), 1);
        assert_eq!(s.logging_flags(), (false, false), "nil is off");
        s.run("LoggingCombat(1) LoggingChat('yes') LoggingChat('no')")
            .unwrap();
        assert_eq!(s.logging_flags(), (false, true));
        assert!(s.take_logging_changes());
        s.run("LoggingCombat(true)").unwrap();
        assert!(
            !s.take_logging_changes(),
            "a write that moves nothing cues nothing"
        );
    }
}
