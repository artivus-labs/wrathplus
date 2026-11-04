//! `SendChatMessage(text [, chatType [, language [, target]]])` (`0x50d170`): the engine verb an
//! addon speaks through, with no FrameXML between it and the wire. `chatType` defaults to
//! `"SAY"`; the fourth argument is the whisper target or the channel's number.
//!
//! It queues a [`ChatSend`] rather than going through the edit box, whose drain runs the slash
//! grammar: an addon's `SendChatMessage("/dance")` says the characters. The reference splits the
//! same way, `ChatEdit_SendText` parsing and `SendChatMessage` not.
//!
//! Its raises, in order, all `luaL_error`: the Usage; `SendChatMessage(): Unknown chat type`;
//! then, past the empty-line gate, `Chat message must be UTF-8 text`, `Unknown language`, a whisper
//! with no target and a channel send with no number. A string or number third argument is matched
//! by name, case-insensitively, against every `Languages.dbc` row (`0x4fe2d0`), known to the
//! character or not; refusing a tongue the character never learned is the server's. Labelled: the
//! chat gate (`0x5191c0`), the dead-player and group gates and the escape-code check (`0x50d7b1`)
//! are not modelled.
//!
//! Deviation: no truncation at 255 characters, because the server refuses a longer line itself:
//! it is dropped where the reference sends its first 255. The server throttles in both.

use mlua::{Lua, MultiValue, Value};

use super::Model;

impl super::UiScript {
    /// Push the name `GetDefaultLanguage()` answers. `None` is the reference's no-player state,
    /// which returns zero values, not `nil`; the app resolves it per world entry from
    /// `ChrRaces.BaseLanguage` and `Languages.dbc`.
    pub fn set_default_language(&mut self, name: Option<String>) {
        self.model_mut().default_language = name;
    }
}

/// One queued `SendChatMessage` line, plain data: this crate holds no wire types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSend {
    /// The line, verbatim: a leading `/` is text, not a command.
    pub text: String,
    /// The chat type token (`"SAY"`, `"WHISPER"`, `"CHANNEL"`, …), uppercased, since the
    /// reference's compare ignores case.
    pub chat_type: String,
    /// The fourth argument as `lua_tostring` gives it (`0x49f306`): the whisper target, or the
    /// channel number the app resolves to a joined channel's name.
    pub target: Option<String>,
    /// The `Languages.dbc` id the third argument named; `None` is the speaker's default.
    pub language: Option<u32>,
}

impl ChatSend {
    /// The empty-line gate (`0x49f28d`-`0x49f2a1`): a line whose first byte is NUL ends the call,
    /// before the language is read or anything is sent, unless its type is AFK (`0x14`) or DND
    /// (`0x15`). The chat-type lookup and its "Unknown chat type" (`0x49f27f`) come first.
    pub fn ends_at_empty_line(&self) -> bool {
        ends_at_empty_line(&self.text, &self.chat_type)
    }
}

/// [`ChatSend::ends_at_empty_line`] on the binding's arguments, before the line is built.
fn ends_at_empty_line(text: &str, chat_type: &str) -> bool {
    text.bytes().next().unwrap_or(0) == 0 && !matches!(chat_type, "AFK" | "DND")
}

impl super::UiScript {
    /// Push `Languages.dbc` as `(ID, Name_lang)` rows in file order, the table `SendChatMessage`
    /// matches its language name against (`[0xc0db40]`, count `[0xc0db44]`).
    pub fn set_language_table(&mut self, rows: Vec<(u32, String)>) {
        self.model_mut().language_table = rows;
    }
}

/// `0x49f8a0`: the first row, in file order, whose name equals `name` ignoring case (`SStrCmpI`
/// `0x64a4c0`, unbounded length), checked against every row, not only the known languages.
fn language_id(table: &[(u32, String)], name: &str) -> Option<u32> {
    table
        .iter()
        .find(|(_, row)| row.eq_ignore_ascii_case(name))
        .map(|&(id, _)| id)
}

impl super::UiScript {
    /// The languages this character knows, in `Languages.dbc` row order: a language counts when a
    /// known spell with `Effect_1 == 39` teaches it (`0x4b25b0`) and its skill line is in the skill
    /// block (`0x5ec720`). A change fires `LANGUAGE_LIST_CHANGED` (`0x49b970`, event 0x102).
    pub fn set_known_languages(&mut self, names: Vec<String>) {
        let changed = {
            let mut model = self.model_mut();
            if model.known_languages == names {
                false
            } else {
                model.known_languages = names;
                true
            }
        };
        if changed {
            self.fire_event("LANGUAGE_LIST_CHANGED", vec![]);
        }
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // GetNumLaguages(), `0x49fb30`: one number, the known languages. The misspelling is the
    // reference's registered name (`0x843628`); the correct spelling is nowhere in the binary.
    lua.globals().set(
        "GetNumLaguages",
        lua.create_function(|lua, _ignored: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.known_languages.len() as i64)
        })?,
    )?;
    // GetLanguageByIndex(i), `0x49fbe0`: the `Name_lang` of the i-th known language, positional,
    // never a row number or a language id; past the end it pushes nothing.
    lua.globals().set(
        "GetLanguageByIndex",
        lua.create_function(|lua, index: Option<i64>| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let name = index
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| model.known_languages.get(i));
            Ok(match name {
                Some(n) => MultiValue::from_vec(vec![Value::String(lua.create_string(n)?)]),
                None => MultiValue::new(),
            })
        })?,
    )?;
    // GetDefaultLanguage(), `0x49fcd0`: one string (`0x6f3890`), or zero values, never `nil`, on
    // all four failure edges (`0x49fd2a`). It reads no arguments, so an addon's
    // `GetDefaultLanguage("player")` is ignored.
    lua.globals().set(
        "GetDefaultLanguage",
        lua.create_function(|lua, _ignored: MultiValue| {
            let name = {
                let model = lua.app_data_ref::<Model>().expect("model app_data");
                model.default_language.clone()
            };
            Ok(match name {
                Some(n) => MultiValue::from_vec(vec![Value::String(lua.create_string(&n)?)]),
                None => MultiValue::new(),
            })
        })?,
    )?;

    lua.globals().set(
        "SendChatMessage",
        lua.create_function(|lua, args: MultiValue| {
            let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
            let raise = |m: &str| mlua::Error::runtime(format!("SendChatMessage(): {m}"));
            // `lua_isstring(1)` (`0x50d1ba`), else the Usage (`0x9fcce0`).
            let Some(text) = lua.coerce_string(arg(0))? else {
                return Err(mlua::Error::runtime(
                    "Usage: SendChatMessage(text [,type] [,language] [,targetPlayer])",
                ));
            };
            // A string type is looked up (`0x4fbb20`) before anything else, else SAY.
            let chat_type = match super::binding_abi::optional_string(lua, &arg(1)) {
                Some(t) => {
                    let t = t.to_ascii_uppercase();
                    if !SEND_TYPES.contains(&t.as_str()) {
                        return Err(raise("Unknown chat type"));
                    }
                    t
                }
                None => "SAY".into(),
            };
            // The language, UTF-8 and target checks come only past the empty-line gate. The line
            // is queued anyway: the drain applies the gate after the type check, as here.
            let checked =
                !ends_at_empty_line(&String::from_utf8_lossy(&text.as_bytes()), &chat_type);
            // `0x7e0ec0`; labelled: Rust's UTF-8 rule stands in for its validator.
            let text = match String::from_utf8(text.as_bytes().to_vec()) {
                Ok(t) => t,
                Err(_) if checked => return Err(raise("Chat message must be UTF-8 text")),
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            };
            let language = match super::binding_abi::optional_string(lua, &arg(2)) {
                Some(name) if checked => {
                    let model = lua.app_data_ref::<Model>().expect("model app_data");
                    let id = language_id(&model.language_table, &name);
                    Some(id.ok_or_else(|| raise("Unknown language"))?)
                }
                _ => None,
            };
            // `lua_isstring` then `lua_tostring` (`0x50d2d2`, `0x50d2e3`): a number is its text,
            // so `2.7` reaches the channel's `SStrToInt` as "2.7", which reads 2.
            let target = super::binding_abi::optional_string(lua, &arg(3));
            let no_target = target.as_deref().is_none_or(str::is_empty);
            if checked && no_target && chat_type == "WHISPER" {
                return Err(raise("Whisper message missing target player!"));
            }
            if checked && no_target && chat_type == "CHANNEL" {
                return Err(raise("Channel send missing channel number"));
            }
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .chat_sends
                .push(ChatSend {
                    text,
                    chat_type,
                    target,
                    language,
                });
            Ok(())
        })?,
    )?;
    Ok(())
}

/// The chat types `SendChatMessage` resolves (`0x4fbb20`, seventeen rows, compared ignoring case).
const SEND_TYPES: [&str; 17] = [
    "SAY",
    "PARTY",
    "RAID",
    "GUILD",
    "OFFICER",
    "YELL",
    "WHISPER",
    "EMOTE",
    "CHANNEL",
    "AFK",
    "DND",
    "RAID_WARNING",
    "BATTLEGROUND",
    "BN",
    "BN_WHISPER",
    "BN_WHISPER_INFORM",
    "BN_CONVERSATION",
];

impl super::UiScript {
    /// Drain the lines `SendChatMessage` queued since the last call.
    pub fn take_chat_sends(&mut self) -> Vec<ChatSend> {
        std::mem::take(&mut self.model_mut().chat_sends)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    #[test]
    fn send_chat_message_queues_a_line_without_parsing_it() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"SendChatMessage("hello")"#).unwrap();
        s.run(r#"SendChatMessage("/dance", "say")"#).unwrap();
        assert_eq!(
            s.take_chat_sends(),
            vec![
                ChatSend {
                    text: "hello".into(),
                    chat_type: "SAY".into(),
                    target: None,
                    language: None,
                },
                ChatSend {
                    text: "/dance".into(),
                    chat_type: "SAY".into(),
                    target: None,
                    language: None,
                },
            ]
        );
        assert!(s.take_chat_sends().is_empty());
    }

    #[test]
    fn the_target_argument_is_its_lua_tostring_text() {
        let mut s = UiScript::new().unwrap();
        s.run(r#"SendChatMessage("hi", "WHISPER", nil, "Bob")"#)
            .unwrap();
        s.run(r#"SendChatMessage("lf1m", "CHANNEL", nil, 1)"#)
            .unwrap();
        s.run(r#"SendChatMessage("lf1m", "CHANNEL", nil, "General")"#)
            .unwrap();
        s.run(r#"SendChatMessage("lf1m", "CHANNEL", nil, 2.7)"#)
            .unwrap();
        // A table is no string, so no channel number (`0x50d587`).
        assert!(s
            .run(r#"SendChatMessage("lf1m", "CHANNEL", nil, {})"#)
            .is_err());
        let sent = s.take_chat_sends();
        assert_eq!(sent[0].target.as_deref(), Some("Bob"));
        assert_eq!(sent[1].target.as_deref(), Some("1"));
        assert_eq!(sent[2].target.as_deref(), Some("General"));
        assert_eq!(
            sent[3].target.as_deref(),
            Some("2.7"),
            "not rounded: the app's `SStrToInt` reads 2"
        );
        assert_eq!(sent[1].chat_type, "CHANNEL");
    }

    /// Three `Languages.dbc` rows, in the shipped file's order.
    fn with_languages(s: &mut UiScript) {
        s.set_language_table(vec![
            (1, "Orcish".into()),
            (2, "Darnassian".into()),
            (7, "Common".into()),
        ]);
    }

    #[test]
    fn a_language_name_sends_its_languages_dbc_id() {
        let mut s = UiScript::new().unwrap();
        with_languages(&mut s);
        s.run(r#"SendChatMessage("hi", "SAY", "Darnassian")"#)
            .unwrap();
        s.run(r#"SendChatMessage("hi", "YELL", "dARNASSIAN")"#)
            .unwrap();
        s.run(r#"SendChatMessage("hi", "WHISPER", "orcish", "Bob")"#)
            .unwrap();
        let sent = s.take_chat_sends();
        assert_eq!(sent[0].language, Some(2), "Darnassian's row ID");
        assert_eq!(sent[1].language, Some(2), "SStrCmpI ignores case");
        assert_eq!(
            sent[2].language,
            Some(1),
            "a language the character may not know is sent: the server refuses it"
        );
        assert_eq!(sent[2].target.as_deref(), Some("Bob"));
    }

    #[test]
    fn no_language_argument_speaks_the_default() {
        let mut s = UiScript::new().unwrap();
        with_languages(&mut s);
        s.run(r#"SendChatMessage("hi")"#).unwrap();
        s.run(r#"SendChatMessage("hi", "SAY", nil)"#).unwrap();
        s.run(r#"SendChatMessage("hi", "SAY", {})"#).unwrap();
        s.run(r#"SendChatMessage("hi", "SAY", true)"#).unwrap();
        let sent = s.take_chat_sends();
        assert_eq!(sent.len(), 4);
        assert!(
            sent.iter().all(|c| c.language.is_none()),
            "not a string, so `lua_isstring` keeps the default: {sent:?}"
        );
    }

    #[test]
    fn an_unknown_language_raises_and_sends_nothing() {
        let mut s = UiScript::new().unwrap();
        with_languages(&mut s);
        for arg in [r#""Klingon""#, r#""""#, "7", r#""Darnassian ""#] {
            let err = s
                .run(&format!(r#"SendChatMessage("hi", "SAY", {arg})"#))
                .expect_err(&format!("{arg} names no row"));
            assert!(err.to_string().contains("Unknown language"), "{arg}: {err}");
        }
        assert!(s.take_chat_sends().is_empty());
        // AFK and DND pass the empty-line gate, so their language is read.
        assert!(s.run(r#"SendChatMessage("", "AFK", "Klingon")"#).is_err());
        // An empty line of any other type ends before the language is read.
        s.run(r#"SendChatMessage("", "SAY", "Klingon")"#).unwrap();
        // The other raises, in `0x50d170`'s order, through a C `pcall`: no position.
        for (args, want) in [
            (
                "",
                "Usage: SendChatMessage(text [,type] [,language] [,targetPlayer])",
            ),
            (
                r#"{}, "SAY""#,
                "Usage: SendChatMessage(text [,type] [,language] [,targetPlayer])",
            ),
            (
                r#""x", "BOGUS", "Klingon""#,
                "SendChatMessage(): Unknown chat type",
            ),
            (
                r#""x", "SAY", "Klingon""#,
                "SendChatMessage(): Unknown language",
            ),
            (
                r#""\255", "SAY""#,
                "SendChatMessage(): Chat message must be UTF-8 text",
            ),
            (
                r#""x", "whisper""#,
                "SendChatMessage(): Whisper message missing target player!",
            ),
            (
                r#""x", "CHANNEL", nil, """#,
                "SendChatMessage(): Channel send missing channel number",
            ),
        ] {
            let sep = [", ", ""][usize::from(args.is_empty())];
            let got = s
                .eval::<String>(&format!(
                    "return select(2, pcall(SendChatMessage{sep}{args}))"
                ))
                .unwrap();
            assert_eq!(got, want, "SendChatMessage({args})");
        }
        assert!(
            s.take_chat_sends().len() == 1,
            "only the empty SAY line queued"
        );
    }

    /// The count matters: an addon passes the result straight to `SendChatMessage` as `language`.
    #[test]
    fn get_default_language_is_one_string_or_zero_values() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(
            s.arity("GetDefaultLanguage()").unwrap(),
            0,
            "no player object → ZERO values, not nil"
        );

        s.set_default_language(Some("Common".into()));
        assert_eq!(
            s.arity("GetDefaultLanguage()").unwrap(),
            1,
            "one value — not (name, id)"
        );
        assert_eq!(
            s.eval::<String>("return GetDefaultLanguage()").unwrap(),
            "Common"
        );
        // The binding reads no arguments; addons pass `"player"` anyway.
        assert_eq!(
            s.eval::<String>(r#"return GetDefaultLanguage("player")"#)
                .unwrap(),
            "Common"
        );
    }
}

#[cfg(test)]
mod language_tests {
    use crate::script::UiScript;

    #[test]
    fn the_language_walk_is_positional_over_the_known_rows() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(s.eval::<i64>("return GetNumLaguages()").unwrap(), 0);
        assert_eq!(
            s.arity("GetLanguageByIndex(1)").unwrap(),
            0,
            "past the end pushes nothing, not nil"
        );
        s.run(
            "local f = CreateFrame('Frame') f:RegisterEvent('LANGUAGE_LIST_CHANGED') \
             f:SetScript('OnEvent', function() FIRED = (FIRED or 0) + 1 end)",
        )
        .unwrap();
        s.set_known_languages(vec!["Orcish".into(), "Common".into()]);
        s.set_known_languages(vec!["Orcish".into(), "Common".into()]);
        assert_eq!(s.eval::<i64>("return GetNumLaguages()").unwrap(), 2);
        assert_eq!(
            s.eval::<String>("return GetLanguageByIndex(2)").unwrap(),
            "Common"
        );
        assert_eq!(
            s.eval::<i64>("return FIRED").unwrap(),
            1,
            "the list changing fires once; the same list again is silent"
        );
    }
}
