//! The chat-channel globals: the joined-channel lookups, the join, leave and management verbs the
//! app sends, and the guild-recruitment auto-join latch. `GetChannelName` (`0x4fe850`) looks up by
//! 1-based slot or by name and returns three values: the slot (`CHAT_MSG_*` arg8), the name and
//! `instanceID` (arg10).

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{ftol, is_string};
use super::Model;

/// A joined channel as the VM mirrors build 12340's record (`0xbcf094`, stride 0xa8): the name
/// `+0x04`, the `ChatChannels.dbc` id `+0x94` (0 for a custom channel), and whether the state
/// `+0x9c` is 0, the confirmed join.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinedChannel {
    pub name: String,
    pub id: u32,
    pub confirmed: bool,
}

/// One `ChatChannels.dbc` row for the channel verbs: the id, the Shortcut a typed name is matched
/// against case-folded, the name composed for the player's zone (`None` while the zone text is
/// empty, the verbs' nil leg), and whether it is listed (a `flags & 0x10` city row only in a city).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneChannelRow {
    pub id: u32,
    pub shortcut: String,
    pub resolved: Option<String>,
    pub listed: bool,
}

/// A channel verb's ask, drained by the app into its `CMSG_*`: the stock `ChatFrame.lua` slash
/// handlers' calls, one variant each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelCommand {
    /// `JoinChannelByName(name, password)`, sent on both non-nil legs.
    Join {
        name: String,
        password: String,
    },
    /// `LeaveChannelByName(name)`.
    Leave {
        name: String,
    },
    /// `ListChannelByName(name)`: `CMSG_CHANNEL_LIST`.
    List {
        name: String,
    },
    /// `ListChannels()`: the roster of every joined channel.
    ListAll,
    /// `DisplayChannelOwner(name)`: `CMSG_CHANNEL_OWNER`.
    DisplayOwner {
        name: String,
    },
    /// `SetChannelOwner(name, player)`.
    SetOwner {
        name: String,
        player: String,
    },
    /// `SetChannelPassword(name, password)`.
    SetPassword {
        name: String,
        password: String,
    },
    Ban {
        name: String,
        player: String,
    },
    Invite {
        name: String,
        player: String,
    },
    Kick {
        name: String,
        player: String,
    },
    Moderator {
        name: String,
        player: String,
    },
    Unmoderator {
        name: String,
        player: String,
    },
    Mute {
        name: String,
        player: String,
    },
    Unmute {
        name: String,
        player: String,
    },
    Unban {
        name: String,
        player: String,
    },
    /// `ChannelModerate(name)`: toggles moderation.
    Moderate {
        name: String,
    },
    /// `ChannelToggleAnnouncements(name)`.
    ToggleAnnouncements {
        name: String,
    },
}

impl super::UiScript {
    /// Feed the zone-channel catalog, whenever the zone, and so every resolved name, changes.
    pub fn set_zone_channel_catalog(&mut self, rows: Vec<ZoneChannelRow>) {
        self.model_mut().zone_channel_catalog = rows;
    }

    /// Channel verbs called since the last drain, in call order.
    pub fn take_channel_commands(&mut self) -> Vec<ChannelCommand> {
        std::mem::take(&mut self.model_mut().channel_commands)
    }

    /// The guild-recruitment auto-join latch: `0` STANDARD, `1` AUTO.
    pub fn guild_recruitment_mode(&self) -> u8 {
        self.model_ref().guild_recruitment_mode
    }

    /// Seat the latch from the chat cache at login. A host write is not a player gesture, so it
    /// does not arm [`Self::take_guild_recruitment_change`]: a login must not save what it read.
    pub fn set_guild_recruitment_mode(&mut self, mode: u8) {
        let mut model = self.model_mut();
        model.guild_recruitment_mode = mode;
        model.guild_recruitment_changed = false;
    }

    /// Whether Lua moved the latch since the last drain: the chat cache's dirty signal.
    pub fn take_guild_recruitment_change(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().guild_recruitment_changed)
    }

    /// Whether Lua called `SetGuildRecruitmentMode(1)` since the last drain: the cue for the
    /// cascade (`0x49ea70` → `0x49ea90`).
    pub fn take_guild_recruitment_cascade(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().guild_recruitment_cascade)
    }

    /// A manual join or leave of `GuildRecruitment` forces the latch to 0 (`0x49ed3d`, `0x49ef8f`)
    /// and, as a player gesture, arms the save; mode 0 has no cascade, and the cascade's own join
    /// passes a flag that skips this reset. Returns whether the latch moved.
    pub fn reset_guild_recruitment_mode(&mut self) -> bool {
        let mut model = self.model_mut();
        if model.guild_recruitment_mode == 0 {
            return false;
        }
        model.guild_recruitment_mode = 0;
        model.guild_recruitment_changed = true;
        true
    }
}

fn push(lua: &Lua, cmd: ChannelCommand) {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    model.channel_commands.push(cmd);
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The record named `name`, whole and case-folded, whatever its state (`0x4fdfc0`).
fn slot_of(model: &Model, name: &str) -> Option<usize> {
    model
        .joined_channels
        .iter()
        .position(|c| {
            c.as_ref()
                .is_some_and(|c| c.name.eq_ignore_ascii_case(name))
        })
        .map(|i| i + 1)
}

/// The confirmed record numbered `n`: `1 <= n <= count`, the record's own number `n` (a freed
/// one holds 0) and its state 0 (`0x4fe87d`-`0x4fe8a7`, `0x4fe17a`-`0x4fe1a3`).
fn confirmed_at(model: &Model, n: i64) -> Option<&JoinedChannel> {
    let i = usize::try_from(n).ok()?.checked_sub(1)?;
    model
        .joined_channels
        .get(i)?
        .as_ref()
        .filter(|c| c.confirmed)
}

/// The name `GetChannelList` and `GetChannelDisplayInfo` answer (`0x4fe6a0`-`0x4fe6d5`,
/// `0x4fdf8f`-`0x4fdfab`): the row's Shortcut when the record's DBC id names one, else the name.
pub(super) fn listed_name<'a>(model: &'a Model, name: &'a str, id: u32) -> &'a str {
    model
        .zone_channel_catalog
        .iter()
        .find(|r| id != 0 && r.id == id)
        .map_or(name, |r| r.shortcut.as_str())
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // ── The guild-recruitment auto-join pair ──────────────────────────────────
    // `GetGuildRecruitmentMode` (`0x4a0040`) and `SetGuildRecruitmentMode` (`0x4a0060`) store the
    // Auto-join Guild Recruitment Channel option, which has no cvar: `UIOptionsFrame.lua` binds it
    // by special arms (`:243`, `:328`, `:647`). The state is one int, `[0x843608]`, a `.data`
    // initialiser of 1 written only by `0x49ea70`, and the chat cache saves it at teardown
    // (`0x499a80`), never at set time.
    g.set(
        "GetGuildRecruitmentMode",
        // Always a number, never nil: `UIOptionsFrame_Load` tests it `== 1`.
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(f64::from(model.guild_recruitment_mode))
        })?,
    )?;
    // ── The `ecx == 1` cascade ──────────────────────────────────────────
    // For mode 1 only, `0x49ea70` tail-jumps into `0x49ea90`, which acts on the wire (a guilded
    // player leaves `GuildRecruitment - City`, an unguilded one in a capital joins it); that runs
    // app-side in `ui_chat::recruitment`, and an acting cascade fires `UPDATE_CHAT_WINDOWS` again.
    g.set(
        "SetGuildRecruitmentMode",
        // As `0x4a0060`: `lua_isnumber` (`0x6f34d0`, so `"1"` passes), truncation toward zero
        // (`0x40a2b0`), then the 0..2 range gate (`0x4a0094`); success returns zero values.
        lua.create_function(|lua, mode: Option<Value>| {
            let n = match mode.as_ref() {
                Some(Value::Integer(i)) => *i as f64,
                Some(Value::Number(n)) => *n,
                // `lua_isnumber` is true for a string `luaO_str2d` fully consumes.
                Some(Value::String(s)) => s
                    .to_str()
                    .ok()
                    .and_then(|s| s.trim().parse::<f64>().ok())
                    .ok_or_else(|| mlua::Error::runtime("Usage: SetGuildRecruitmentMode(mode)"))?,
                _ => return Err(mlua::Error::runtime("Usage: SetGuildRecruitmentMode(mode)")),
            };
            let n = n.trunc();
            if !(0.0..2.0).contains(&n) {
                return Err(mlua::Error::runtime(
                    "SetGuildRecruitmentMode: invalid mode",
                ));
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let mode = n as u8;
            if model.guild_recruitment_mode != mode {
                model.guild_recruitment_mode = mode;
                model.guild_recruitment_changed = true;
            }
            // `0x49ea79 jne` / `0x49ea7b jmp 0x49ea90`: the cascade, on the new value alone.
            if mode == 1 {
                model.guild_recruitment_cascade = true;
            }
            // `0x4a00a4`/`0x4a00a9`: `UPDATE_CHAT_WINDOWS`, on every successful call.
            model
                .pending_events
                .push(("UPDATE_CHAT_WINDOWS".to_string(), Vec::new()));
            Ok(())
        })?,
    )?;

    // GetChannelName(index | name) (`0x4fe850`): a `lua_isnumber` argument truncates to a
    // confirmed slot, so `"1"`, `" 1"` and `"1.0"` are slot 1; any other string is a record name,
    // a pending one included; anything else raises. Three values on every other path: the slot or
    // 0, the record name or nil, and `+0x98`, which AzerothCore's `YOU_JOINED` leaves 0.
    g.set(
        "GetChannelName",
        lua.create_function(|lua, key: Value| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let slot = match lua.coerce_number(key.clone())? {
                Some(n) => {
                    let n = i64::from(ftol(n));
                    confirmed_at(&model, n).map(|_| n as usize)
                }
                None if is_string(Some(&key)) => slot_of(&model, &key.to_string()?),
                None => {
                    return Err(mlua::Error::runtime(
                        "Usage: GetChannelName([channelIndex] or [channelName])",
                    ))
                }
            };
            let name = slot.and_then(|n| model.joined_channels[n - 1].as_ref());
            Ok(MultiValue::from_vec(vec![
                Value::Integer(slot.map_or(0, |n| n as i64)),
                match name {
                    Some(c) => Value::String(lua.create_string(&c.name)?),
                    None => Value::Nil,
                },
                Value::Integer(0),
            ]))
        })?,
    )?;

    // GetChannelList() (`0x4fe650`): slot, name pairs for the confirmed records in slot order,
    // a DBC channel named by its Shortcut, as `CreateChatChannelList` ticks it against a window's
    // `channelList` (ChatConfigFrame.lua:1495-1520); none joined is zero values.
    g.set(
        "GetChannelList",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::with_capacity(model.joined_channels.len() * 2);
            for (i, c) in model.joined_channels.iter().enumerate() {
                let Some(c) = c.as_ref().filter(|c| c.confirmed) else {
                    continue;
                };
                out.push(Value::Integer(i as i64 + 1));
                let name = listed_name(&model, &c.name, c.id);
                out.push(Value::String(lua.create_string(name)?));
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // JoinChannelByName(name [, password [, frameId]]) (`0x49ff00` → `0x49eb70`): a DBC row
    // answers `(ChannelID, resolvedName)` and a custom channel `(0, nil)`, both sending; a name
    // with a space, or a row whose zone text is empty, answers nil and sends nothing. A `frameId`
    // that truncates to 1..=10 registers the channel in that window's list, deduplicated by name
    // (`0x49ec24`), which is what brings a `/leave`-stripped channel's lines back to the window.
    g.set(
        "JoinChannelByName",
        lua.create_function(
            |lua, (name, password, frame): (Option<String>, Option<String>, Value)| {
                let Some(name) = non_empty(name) else {
                    return Ok(MultiValue::from_vec(vec![Value::Nil]));
                };
                if name.contains(' ') {
                    return Ok(MultiValue::from_vec(vec![Value::Nil]));
                }
                let row = {
                    let model = lua.app_data_ref::<Model>().expect("model app_data");
                    model
                        .zone_channel_catalog
                        .iter()
                        .find(|r| r.shortcut.eq_ignore_ascii_case(&name))
                        .cloned()
                };
                let (id, resolved, listed_as) = match row {
                    None => (0, None, name.clone()),
                    Some(ZoneChannelRow { resolved: None, .. }) => {
                        return Ok(MultiValue::from_vec(vec![Value::Nil]))
                    }
                    Some(ZoneChannelRow {
                        id,
                        resolved,
                        shortcut,
                        ..
                    }) => (id, resolved, shortcut),
                };
                // `0x49ff6b edi = -1` / `0x49ff8a dec edi`; `0x49ec27 cmp esi,0xa; jae` skips.
                let window = match frame {
                    Value::Integer(n) => Some(n as f64),
                    Value::Number(n) => Some(n),
                    _ => None,
                }
                .map(|n| n.trunc() as i64 - 1)
                .and_then(|i| usize::try_from(i).ok())
                .filter(|i| *i < 10);
                if let Some(i) = window {
                    lua.app_data_mut::<Model>()
                        .expect("model app_data")
                        .register_window_channel(i, listed_as, id);
                }
                push(
                    lua,
                    ChannelCommand::Join {
                        name: resolved.clone().unwrap_or_else(|| name.clone()),
                        password: password.unwrap_or_default(),
                    },
                );
                Ok(MultiValue::from_vec(vec![
                    Value::Integer(i64::from(id)),
                    match resolved {
                        Some(r) => Value::String(lua.create_string(&r)?),
                        None => Value::Nil,
                    },
                ]))
            },
        )?,
    )?;

    // EnumerateServerChannels() (`0x4ff200`): the `ChatChannels.dbc` Shortcuts in row order, never
    // the composed names, a `flags & 0x10` row only in a city (`AreaTable.Flags & 0x8`); zero
    // values while the zone is unresolved. Label: an empty Shortcut answers the row's name
    // (`0x4ff27d`), which no shipped row has, so it is not modelled.
    g.set(
        "EnumerateServerChannels",
        lua.create_function(|lua, _ignored: MultiValue| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let mut out = Vec::new();
            for row in model.zone_channel_catalog.iter().filter(|r| r.listed) {
                out.push(Value::String(lua.create_string(&row.shortcut)?));
            }
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // LeaveChannelByName(name) (`0x4a0000` → `0x49ee70`) returns nothing and raises `Usage:` on a
    // non-string, non-number. A key whose `SStrToInt` is non-zero is a joined slot, resolved
    // app-side and never a window entry; a Shortcut leaves the name composed for this zone (a
    // no-op with no zone text) and strips by Shortcut; anything else is a custom channel.
    // Deviation: the reference composes the Shortcut with `GetRealZoneText` alone (`0x49efe8`), so
    // in a capital `/leave Trade` sends `Trade - Stormwind City` and leaves nothing; ours uses the
    // join's city-aware name, because the reference's form cannot leave the channel it joined.
    // The slot is freed app-side when the server's `YOU_LEFT` arrives.
    g.set(
        "LeaveChannelByName",
        lua.create_function(|lua, name: Option<Value>| {
            let key = match name {
                Some(Value::String(s)) => s.to_str().map(|s| s.to_string())?,
                Some(Value::Integer(i)) => i.to_string(),
                Some(Value::Number(n)) => format!("{n:.14}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string(),
                _ => return Err(mlua::Error::runtime(r#"Usage: LeaveChannelByName("name")"#)),
            };
            if super::chat_window::leading_int(&key) != 0 {
                push(lua, ChannelCommand::Leave { name: key });
                return Ok(());
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let row = model
                .zone_channel_catalog
                .iter()
                .find(|r| r.shortcut.eq_ignore_ascii_case(&key))
                .cloned();
            let (wire, strip_key) = match row {
                Some(ZoneChannelRow { resolved: None, .. }) => return Ok(()),
                Some(ZoneChannelRow {
                    resolved: Some(r),
                    shortcut,
                    ..
                }) => (r, shortcut),
                None => (key.clone(), key),
            };
            model.strip_window_channel(&strip_key);
            model
                .channel_commands
                .push(ChannelCommand::Leave { name: wire });
            Ok(())
        })?,
    )?;

    // The one-name verbs: each queues its `CMSG_*` and returns nothing.
    for (verb, make) in [
        (
            "ListChannelByName",
            (|name| ChannelCommand::List { name }) as fn(String) -> ChannelCommand,
        ),
        ("DisplayChannelOwner", |name| ChannelCommand::DisplayOwner {
            name,
        }),
        ("ChannelModerate", |name| ChannelCommand::Moderate { name }),
        ("ChannelToggleAnnouncements", |name| {
            ChannelCommand::ToggleAnnouncements { name }
        }),
    ] {
        g.set(
            verb,
            lua.create_function(move |lua, name: Option<String>| {
                if let Some(name) = non_empty(name) {
                    push(lua, make(name));
                }
                Ok(())
            })?,
        )?;
    }

    g.set(
        "ListChannels",
        lua.create_function(|lua, _ignored: MultiValue| {
            push(lua, ChannelCommand::ListAll);
            Ok(())
        })?,
    )?;

    // The two-name verbs: (channel, player), or (channel, password) for the password.
    for (verb, make) in [
        (
            "SetChannelOwner",
            (|name, player| ChannelCommand::SetOwner { name, player })
                as fn(String, String) -> ChannelCommand,
        ),
        ("SetChannelPassword", |name, password| {
            ChannelCommand::SetPassword { name, password }
        }),
        ("ChannelBan", |name, player| ChannelCommand::Ban {
            name,
            player,
        }),
        ("ChannelInvite", |name, player| ChannelCommand::Invite {
            name,
            player,
        }),
        ("ChannelKick", |name, player| ChannelCommand::Kick {
            name,
            player,
        }),
        ("ChannelModerator", |name, player| {
            ChannelCommand::Moderator { name, player }
        }),
        ("ChannelUnmoderator", |name, player| {
            ChannelCommand::Unmoderator { name, player }
        }),
        ("ChannelMute", |name, player| ChannelCommand::Mute {
            name,
            player,
        }),
        ("ChannelUnmute", |name, player| ChannelCommand::Unmute {
            name,
            player,
        }),
        ("ChannelUnban", |name, player| ChannelCommand::Unban {
            name,
            player,
        }),
    ] {
        g.set(
            verb,
            lua.create_function(
                move |lua, (name, second): (Option<String>, Option<String>)| {
                    if let Some(name) = non_empty(name) {
                        // A password may be empty (`/password General` clears it); a player name
                        // may not.
                        let second = second.unwrap_or_default();
                        if verb == "SetChannelPassword" || !second.trim().is_empty() {
                            push(lua, make(name, second.trim().to_string()));
                        }
                    }
                    Ok(())
                },
            )?,
        )?;
    }

    Ok(())
}

impl super::UiScript {
    /// Mirror the app's channel records in slot order, `None` for a freed slot; `ui_chat::feed`
    /// pushes it on each `YOU_JOINED` and `YOU_LEFT`.
    pub fn set_joined_channels(&mut self, joined: Vec<Option<JoinedChannel>>) {
        self.model_mut().joined_channels = joined;
    }
}

#[cfg(test)]
mod command_tests {
    use super::{ChannelCommand, ZoneChannelRow};
    use crate::script::UiScript;

    fn catalog() -> Vec<ZoneChannelRow> {
        vec![
            ZoneChannelRow {
                id: 1,
                shortcut: "General".into(),
                resolved: Some("General - Elwynn Forest".into()),
                listed: true,
            },
            ZoneChannelRow {
                id: 2,
                shortcut: "Trade".into(),
                resolved: None,
                listed: false,
            },
        ]
    }

    /// The row, custom-channel and nil legs, and that 0 is truthy.
    #[test]
    fn join_channel_by_name_answers_the_row_custom_and_nil_legs() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(catalog());
        s.run(
            "A = {JoinChannelByName('General', 'pw', 1)} \
             B = {JoinChannelByName('MyChan', nil, 1)} \
             C = {JoinChannelByName('Trade', nil, 1)} \
             D = {JoinChannelByName('two words', nil, 1)}",
        )
        .unwrap();
        assert_eq!(
            s.eval::<(i64, String)>("return A[1], A[2]").unwrap(),
            (1, "General - Elwynn Forest".to_string())
        );
        assert!(s
            .eval::<bool>("return B[1] == 0 and B[2] == nil and table.getn(B) == 1")
            .unwrap());
        assert!(
            s.eval::<bool>("return C[1] == nil").unwrap(),
            "empty substitution"
        );
        assert!(s.eval::<bool>("return D[1] == nil").unwrap(), "a space");
        assert_eq!(
            s.take_channel_commands(),
            vec![
                ChannelCommand::Join {
                    name: "General - Elwynn Forest".into(),
                    password: "pw".into()
                },
                ChannelCommand::Join {
                    name: "MyChan".into(),
                    password: String::new()
                },
            ],
            "sent on both non-nil legs, the resolved name for a row"
        );
    }

    #[test]
    fn enumerate_server_channels_lists_the_shortcuts_the_zone_admits() {
        let mut s = UiScript::new().unwrap();
        assert_eq!(
            s.arity("EnumerateServerChannels()").unwrap(),
            0,
            "no zone yet — 0 values"
        );
        s.set_zone_channel_catalog(catalog());
        assert_eq!(
            s.eval::<Vec<String>>("return {EnumerateServerChannels()}")
                .unwrap(),
            vec!["General".to_string()],
            "the shortcut, never the composed name; a city row only in a city"
        );
    }

    #[test]
    fn the_management_verbs_queue_in_call_order_and_drop_empty_names() {
        let mut s = UiScript::new().unwrap();
        s.run(
            "LeaveChannelByName('General') ListChannelByName('') ListChannels() \
             SetChannelPassword('General', '') ChannelKick('General', 'Bob') \
             ChannelKick('General', '') ChannelModerate('General') \
             ChannelToggleAnnouncements('General') SetChannelOwner('General', 'Ann')",
        )
        .unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![
                ChannelCommand::Leave {
                    name: "General".into()
                },
                ChannelCommand::ListAll,
                ChannelCommand::SetPassword {
                    name: "General".into(),
                    password: String::new()
                },
                ChannelCommand::Kick {
                    name: "General".into(),
                    player: "Bob".into()
                },
                ChannelCommand::Moderate {
                    name: "General".into()
                },
                ChannelCommand::ToggleAnnouncements {
                    name: "General".into()
                },
                ChannelCommand::SetOwner {
                    name: "General".into(),
                    player: "Ann".into()
                },
            ]
        );
    }

    /// `frameId - 1` indexes the ten window records (`0x49ec24`); a missing, non-numeric or
    /// out-of-range frame skips the registration and nothing else.
    #[test]
    fn join_channel_by_name_registers_the_channel_in_the_named_window() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(catalog());
        s.run(
            "JoinChannelByName('General', nil, 1) \
             JoinChannelByName('general', nil, 1) \
             JoinChannelByName('MyChan', nil, 2) \
             JoinChannelByName('Trade', nil, nil) \
             JoinChannelByName('LocalDefense', nil, 11) \
             JoinChannelByName('WorldDefense', nil, 0)",
        )
        .unwrap();
        let looks = s.chat_window_looks();
        assert_eq!(
            looks[0].channels,
            vec![("General".to_string(), 1)],
            "the DBC Shortcut and its id, once — the second join deduplicated by name"
        );
        assert_eq!(
            looks[1].channels,
            vec![("MyChan".to_string(), 0)],
            "a custom channel: the name and id 0"
        );
        assert!(
            looks[2..].iter().all(|l| l.channels.is_empty()),
            "nil, 11 and 0 register nowhere"
        );
        assert_eq!(
            s.take_chat_window_changes(),
            vec![0, 1],
            "…and both windows are owed a save"
        );
        // The joins went out whatever the frame argument: five, as this catalog's Trade is the
        // nil leg, which registers nothing either.
        assert_eq!(s.take_channel_commands().len(), 5);
    }

    /// `LeaveChannelByName`'s four legs and its raise (`0x4a0000` → `0x49ee70`); a Shortcut strips
    /// its entry from every window.
    #[test]
    fn leave_channel_by_name_composes_strips_and_raises_the_way_the_reference_does() {
        let mut s = UiScript::new().unwrap();
        s.set_zone_channel_catalog(catalog());
        s.run(
            "JoinChannelByName('General', nil, 1) JoinChannelByName('MyChan', nil, 1) \
             JoinChannelByName('General', nil, 2)",
        )
        .unwrap();
        s.take_channel_commands();
        s.take_chat_window_changes();

        s.run("LeaveChannelByName('2')").unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![ChannelCommand::Leave { name: "2".into() }],
            "a number goes through as typed — the app holds the slot states"
        );
        assert!(
            s.take_chat_window_changes().is_empty(),
            "…and strips no window: a slot name never equals a window entry"
        );

        s.run("LeaveChannelByName('general')").unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![ChannelCommand::Leave {
                name: "General - Elwynn Forest".into()
            }],
            "a shortcut composes for the zone"
        );
        let looks = s.chat_window_looks();
        assert_eq!(
            looks[0].channels,
            vec![("MyChan".to_string(), 0)],
            "General stripped from window 1, MyChan kept"
        );
        assert!(
            looks[1].channels.is_empty(),
            "…and from window 2 — all ten are walked"
        );
        assert_eq!(s.take_chat_window_changes(), vec![0, 1]);

        s.run("LeaveChannelByName('Trade')").unwrap();
        assert!(
            s.take_channel_commands().is_empty(),
            "unresolvable here (no city word in this catalog): a complete no-op"
        );

        s.run("LeaveChannelByName('mychan')").unwrap();
        assert_eq!(
            s.take_channel_commands(),
            vec![ChannelCommand::Leave {
                name: "mychan".into()
            }],
            "custom: verbatim"
        );
        assert!(
            s.chat_window_looks()[0].channels.is_empty(),
            "…and stripped verbatim, case-folded"
        );

        let e = s.run("LeaveChannelByName()").unwrap_err();
        assert!(e.to_string().contains("Usage: LeaveChannelByName"), "{e}");
        assert_eq!(s.arity("LeaveChannelByName('x')").unwrap(), 0);
    }
}
