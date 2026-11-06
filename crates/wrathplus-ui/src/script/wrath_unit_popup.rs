//! Build-12340 globals UnitPopup.lua, ChannelFrame.lua/.xml and BNet.lua call that the 1.12
//! surface lacks (one Wow.exe round: `/tmp/azb-ui-unitpopup-native.log`).
//!
//! The menu's verbs go out as [`WrathUnitPopupRequest`]s, labelled capture-only until App sends
//! them; the state the menu reads is published by the host ([`WrathUnitPopupState`]) and answers
//! the native "nothing" shapes until it is. Voice chat is off, so the voice getters read an empty
//! session table and the mute list is empty; Battle.net is never connected, so its verbs take the
//! native not-connected early return before reading any argument.

use mlua::{Lua, MultiValue, Table, Value};

use super::wrath_channel_display::{entries, selected_index};
use super::wrath_channel_roster::Identity;
use super::Model;

/// One menu verb for App, in call order. Capture-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathUnitPopupRequest {
    /// `SetPVP(flag)` (`0x5168b0`, CMSG `0x253` with a byte: the number given, else 0).
    SetPvp { value: u8 },
    /// `SetDungeonDifficulty(difficulty)` (`0x526050`, MSG `0x329` with the 0-based difficulty).
    SetDungeonDifficulty { difficulty: u32 },
    /// `SetRaidDifficulty(difficulty)` (`0x5261a0`, MSG `0x4EB` with the 0-based difficulty).
    SetRaidDifficulty { difficulty: u32 },
    /// `SetOptOutOfLoot(flag)` (`0x52cf00` → `0x52cb40`).
    SetOptOutOfLoot { on: bool },
    /// `GrantLevel(unit)` (`0x523160`, CMSG `0x40D` with the packed GUID).
    GrantLevel { guid: u64 },
    /// `ReportPlayerIsPVPAFK(unit)` (`0x612d50` → `0x6d74b0`), a group member other than the
    /// player.
    ReportPvpAfk { guid: u64 },
    /// `ChannelSilenceVoice(channel, name)` / `ChannelUnSilenceVoice` (`0x501140`/`0x5012f0`).
    /// A custom channel goes out as channel command `0x3CC`/`0x3CE`; a party, raid or
    /// battleground channel through the group voice path (`0x500d80`).
    ChannelSilenceVoice {
        channel: String,
        name: String,
        silence: bool,
        group: bool,
    },
}

/// A chat line `CanComplainChat` can find, as the host records it with the line's `lineID`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WrathChatLine {
    pub line_id: i32,
    pub sender: String,
    pub text: String,
    /// The sender's GUID, 0 for a line with no player sender.
    pub guid: u64,
}

/// The state UnitPopup's menu reads, as the host publishes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathUnitPopupState {
    /// `CanChangePlayerDifficulty`'s second return reads this lock (`DAT_00bd1984`, a `time()`
    /// stamp): the change is locked while it lies in the future.
    pub difficulty_locked_until: Option<i64>,
    /// GUIDs the Recruit-a-Friend grant check (`0x6de9e0`) passes for.
    pub grantable: Vec<u64>,
    /// The player's pet carries summon-property flag `0x20` (`0x5d3820`): it cannot be
    /// dismissed.
    pub pet_undismissable: bool,
    /// Recent chat lines, oldest first.
    pub chat_lines: Vec<WrathChatLine>,
}

/// The requests, published state and the opt-out flag the menu toggles.
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    requests: Vec<WrathUnitPopupRequest>,
    state: WrathUnitPopupState,
    opt_out_of_loot: bool,
}

impl super::UiScript {
    /// Drain UnitPopup's menu verbs in call order.
    pub fn take_wrath_unit_popup_requests(&mut self) -> Vec<WrathUnitPopupRequest> {
        std::mem::take(&mut self.model_mut().wrath_unit_popup.requests)
    }

    /// Publish the state UnitPopup's menu reads, whole.
    pub fn set_wrath_unit_popup_state(&mut self, state: WrathUnitPopupState) {
        self.model_mut().wrath_unit_popup.state = state;
    }

    /// The server's opt-out-of-loot flag, as the player's descriptor reports it.
    pub fn set_wrath_opt_out_of_loot(&mut self, on: bool) {
        self.model_mut().wrath_unit_popup.opt_out_of_loot = on;
    }
}

fn push(lua: &Lua, r: WrathUnitPopupRequest) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .wrath_unit_popup
        .requests
        .push(r);
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// `lua_isnumber`: numbers and numeric strings.
fn number(lua: &Lua, v: &Value) -> Option<f64> {
    lua.coerce_number(v.clone()).ok().flatten()
}

/// `lua_isstring` then `lua_tostring`: strings and numbers.
fn text(lua: &Lua, v: &Value) -> mlua::Result<Option<String>> {
    Ok(match v {
        Value::String(_) | Value::Integer(_) | Value::Number(_) => {
            lua.coerce_string(v.clone())?.map(|s| s.to_string_lossy())
        }
        _ => None,
    })
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(vec![v]))
}

fn flag(on: bool) -> Value {
    super::binding_abi::flag(on)
}

fn grouped(m: &Model) -> bool {
    !m.party.members.is_empty() || !m.party.raid.is_empty()
}

fn player_guid(m: &Model) -> u64 {
    m.unit_guids.player
}

/// A unit token's GUID, or a party or raid member's by name (`0x60abf0`); 0 for nobody.
pub(in crate::script) fn guid_of(m: &Model, name: &str) -> u64 {
    m.unit_guid(name).unwrap_or(0)
}

fn in_group(m: &Model, guid: u64) -> bool {
    guid != 0
        && (m.party.members.iter().any(|p| p.guid == guid)
            || m.party.raid.iter().any(|p| p.guid == guid))
}

/// The player's flags in the selected display channel; `None` when the selection is no server
/// channel (a header or a party/raid row, which 0x4ffa70 rejects through its `+0x6c` index).
fn display_channel_flags(m: &Model) -> Option<u8> {
    let row = entries(m).get(selected_index(m)?)?.identity.clone()?;
    if !matches!(row, Identity::Server(_)) {
        return None;
    }
    let me = player_guid(m);
    Some(
        m.wrath_channel_roster
            .members
            .iter()
            .find(|r| r.guid == me)
            .map_or(0, |r| r.flags),
    )
}

/// A difficulty change while grouped needs the leader: message `0x54`, ERR_NOT_LEADER (table
/// `0xac8af0`, kind 0, a system line).
fn leader_gate(lua: &Lua) -> mlua::Result<bool> {
    let refused = {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        grouped(&m) && m.party.leader_guid != player_guid(&m)
    };
    if refused {
        system_line(lua, "ERR_NOT_LEADER")?;
    }
    Ok(!refused)
}

/// A kind-0 message: the global string as a system chat line.
fn system_line(lua: &Lua, key: &str) -> mlua::Result<()> {
    let message = match lua.globals().get::<Value>(key)? {
        Value::String(s) => s.to_string_lossy(),
        _ => key.to_string(),
    };
    super::tick::fire_event_into(
        lua,
        "CHAT_MSG_SYSTEM",
        vec![super::ScriptValue::Str(message)],
    );
    Ok(())
}

/// The `difficulty` argument as native's 0-based index, `None` past `max`.
fn difficulty(lua: &Lua, args: &MultiValue, usage: &str, max: u32) -> mlua::Result<Option<u32>> {
    let Some(n) = number(lua, &arg(args, 0)) else {
        return Err(mlua::Error::runtime(usage.to_string()));
    };
    let index = (n - 1.0).round_ties_even() as i64 as u32;
    Ok((index <= max).then_some(index))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    use WrathUnitPopupRequest as R;

    // ── difficulty, PvP, loot ──
    // CanChangePlayerDifficulty() (`0x52d000`) → two booleans: may change (labelled: `0x52cbc0`
    // unread, taken as ungrouped or leader, the gate SetDungeonDifficulty applies) and not locked.
    set(lua, "CanChangePlayerDifficulty", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let may = !grouped(&m) || m.party.leader_guid == player_guid(&m);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let locked = m
            .wrath_unit_popup
            .state
            .difficulty_locked_until
            .is_some_and(|t| now < t);
        Ok(MultiValue::from_vec(vec![
            Value::Boolean(may),
            Value::Boolean(!locked),
        ]))
    })?;
    set(lua, "SetDungeonDifficulty", |lua, args| {
        let Some(d) = difficulty(lua, &args, "Usage: SetDungeonDifficulty(difficulty)", 1)? else {
            return none();
        };
        if leader_gate(lua)? {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .wrath_group
                .dungeon_difficulty = Some(d as u8 + 1);
            push(lua, R::SetDungeonDifficulty { difficulty: d });
        }
        none()
    })?;
    set(lua, "SetRaidDifficulty", |lua, args| {
        let Some(d) = difficulty(lua, &args, "Usage: SetRaidDifficulty(difficulty)", 3)? else {
            return none();
        };
        if leader_gate(lua)? {
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .wrath_group
                .raid_difficulty = Some(d as u8 + 1);
            push(lua, R::SetRaidDifficulty { difficulty: d });
        }
        none()
    })?;
    // GetPVPDesired() (`0x51bca0`) → PLAYER_FLAGS bit 0x200 as a number, 0 with no player.
    set(lua, "GetPVPDesired", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let on = m
            .unit("player")
            .is_some_and(|p| p.player_flags & 0x200 != 0);
        one(Value::Integer(i64::from(on)))
    })?;
    set(lua, "SetPVP", |lua, args| {
        let value = number(lua, &arg(&args, 0)).map_or(0, |n| n.round_ties_even() as i64 as u8);
        push(lua, R::SetPvp { value });
        none()
    })?;
    // SetOptOutOfLoot(flag) (`0x52cf00`): the flag, then OPT_OUT_LOOT_TOGGLE_ON/OFF as a system
    // line. GetOptOutOfLoot reads it back.
    set(lua, "SetOptOutOfLoot", |lua, args| {
        let on = super::binding_abi::bool_or_default(args.front(), false);
        lua.app_data_mut::<Model>()
            .expect("model app_data")
            .wrath_unit_popup
            .opt_out_of_loot = on;
        push(lua, R::SetOptOutOfLoot { on });
        let key = if on {
            "OPT_OUT_LOOT_TOGGLE_ON"
        } else {
            "OPT_OUT_LOOT_TOGGLE_OFF"
        };
        system_line(lua, key)?;
        none()
    })?;
    set(lua, "GetOptOutOfLoot", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        one(flag(m.wrath_unit_popup.opt_out_of_loot))
    })?;

    // ── level grant, PvP AFK report, pet, guild ──
    // CanGrantLevel(unit) (`0x51d230`) → 1 when the player exists and the grant check passes.
    set(lua, "CanGrantLevel", |lua, args| {
        let name = text(lua, &arg(&args, 0))?.unwrap_or_default();
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let guid = guid_of(&m, &name);
        one(flag(
            player_guid(&m) != 0 && guid != 0 && m.wrath_unit_popup.state.grantable.contains(&guid),
        ))
    })?;
    // GrantLevel(unit) (`0x523160`). Labelled: on a failed check native prints the check's
    // refusal (`0x5219e0`), which needs the host's reason code; nothing is printed here.
    set(lua, "GrantLevel", |lua, args| {
        let name = text(lua, &arg(&args, 0))?.unwrap_or_default();
        let guid = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            let guid = guid_of(&m, &name);
            (player_guid(&m) != 0
                && guid != 0
                && m.wrath_unit_popup.state.grantable.contains(&guid))
            .then_some(guid)
        };
        if let Some(guid) = guid {
            push(lua, R::GrantLevel { guid });
        }
        none()
    })?;
    set(lua, "ReportPlayerIsPVPAFK", |lua, args| {
        let Some(name) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                r#"Usage: ReportPlayerIsPVPAFK("unit")"#,
            ));
        };
        let guid = {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            let me = player_guid(&m);
            let guid = guid_of(&m, &name);
            (me != 0 && guid != me && in_group(&m, guid)).then_some(guid)
        };
        if let Some(guid) = guid {
            push(lua, R::ReportPvpAfk { guid });
        }
        none()
    })?;
    // PetCanBeDismissed() (`0x5d3820`) → 1 unless the player's own pet carries the flag.
    set(lua, "PetCanBeDismissed", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        one(flag(!m.wrath_unit_popup.state.pet_undismissable))
    })?;
    // UnitIsInMyGuild(name) (`0x60c550`): a unit both sides of which are streamed compares guild
    // ids (here the guild names); otherwise the name against the guild roster, case-insensitive.
    set(lua, "UnitIsInMyGuild", |lua, args| {
        let Some(name) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(r#"Usage: UnitIsInMyGuild("name")"#));
        };
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let mine = m.unit("player").filter(|p| p.exists);
        if let (Some(unit), Some(me)) = (m.unit(&name).filter(|u| u.exists), mine) {
            let same = matches!((&unit.guild, &me.guild), (Some(a), Some(b)) if a.name == b.name);
            return one(flag(same));
        }
        let name = m.unit(&name).and_then(|u| u.name.clone()).unwrap_or(name);
        one(flag(
            m.guild
                .roster
                .iter()
                .any(|r| r.name.eq_ignore_ascii_case(&name)),
        ))
    })?;

    // ── chat complaints ──
    // CanComplainChat(lineID) or (name [, text]) (`0x500c80`): the line (`0x4fb300`), then its
    // sender (`0x4ffdc0`, labelled: unread, taken as a player other than self).
    set(lua, "CanComplainChat", |lua, args| {
        let first = arg(&args, 0);
        let (line, name, said) = if let Some(n) = number(lua, &first) {
            (Some(n.round_ties_even() as i64 as i32), None, None)
        } else if let Some(name) = text(lua, &first)? {
            (None, Some(name), text(lua, &arg(&args, 1))?)
        } else {
            return Err(mlua::Error::runtime(
                "Usage: CanComplainChat(lineID) or CanComplainChat(name) or CanComplainChat(name, text)",
            ));
        };
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let found = m
            .wrath_unit_popup
            .state
            .chat_lines
            .iter()
            .rev()
            .find(|l| match line {
                Some(id) => l.line_id == id,
                None => {
                    name.as_deref()
                        .is_some_and(|n| l.sender.eq_ignore_ascii_case(n))
                        && said.as_deref().is_none_or(|t| l.text == t)
                }
            });
        let me = player_guid(&m);
        one(match found {
            Some(l) if l.guid != 0 && l.guid != me => Value::Integer(1),
            _ => Value::Nil,
        })
    })?;

    // ── channels and voice ──
    // IsDisplayChannelOwner() (`0x4ffa70`, bit 1) / IsDisplayChannelModerator() (`0x4ffae0`,
    // bits 1|2): the player's flags in the selected display channel.
    for (global, bits) in [
        ("IsDisplayChannelOwner", 1u8),
        ("IsDisplayChannelModerator", 3),
    ] {
        set(lua, global, move |lua, _| {
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            one(flag(
                display_channel_flags(&m).is_some_and(|f| f & bits != 0),
            ))
        })?;
    }
    // IsSilenced(name) (`0x4ffb50`): 1 when the name is in the selected channel's roster without
    // roster flag 0x10; no return for a non-string.
    set(lua, "IsSilenced", |lua, args| {
        let Some(name) = text(lua, &arg(&args, 0))? else {
            return none();
        };
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let row = m.wrath_channel_roster.members.iter().find(|r| {
            r.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(&name))
        });
        one(flag(row.is_some_and(|r| r.flags & 0x10 == 0)))
    })?;
    // UnitIsSilenced(name [, channel]) (`0x4fccf0`): the voice session's silence list, empty with
    // voice off.
    set(lua, "UnitIsSilenced", |lua, args| {
        if text(lua, &arg(&args, 0))?.is_none() {
            return none();
        }
        one(Value::Nil)
    })?;
    // IsMuted(name) (`0x6b6cf0`): the voice mute list, empty with voice off.
    set(lua, "IsMuted", |_, _| one(Value::Nil))?;
    // ChannelSilenceVoice / ChannelUnSilenceVoice(channel, name).
    for (global, silence) in [
        ("ChannelSilenceVoice", true),
        ("ChannelUnSilenceVoice", false),
    ] {
        set(lua, global, move |lua, args| {
            let (Some(channel), Some(name)) =
                (text(lua, &arg(&args, 0))?, text(lua, &arg(&args, 1))?)
            else {
                return none();
            };
            let mut group = false;
            for key in ["PARTY", "RAID", "BATTLEFIELDS"] {
                let localized = match lua.globals().get::<Value>(key)? {
                    Value::String(s) => s.to_string_lossy(),
                    _ => key.to_string(),
                };
                group |= channel.eq_ignore_ascii_case(&localized);
            }
            group |= ["Party", "Raid", "Battleground"]
                .iter()
                .any(|n| channel.eq_ignore_ascii_case(n));
            push(
                lua,
                R::ChannelSilenceVoice {
                    channel,
                    name,
                    silence,
                    group,
                },
            );
            none()
        })?;
    }
    // GetVoiceSessionMemberInfoBySessionID(session, index) (`0x7deef0`): name, voiceActive,
    // sessionActive, muted, squelched for a live session; no session exists with voice off.
    set(lua, "GetVoiceSessionMemberInfoBySessionID", |_, _| none())?;
    // SetActiveVoiceChannel(id) (`0x500ae0`): only a voice-enabled channel activates, and none is
    // with voice off. Labelled: id 0 also writes console line `0x225`, not modelled.
    set(lua, "SetActiveVoiceChannel", |_, _| none())?;
    // SetActiveVoiceChannelBySessionID(id) (VoiceChat.lua:259): the same with voice off; no
    // session exists to activate (labelled: its binding is unread).
    set(lua, "SetActiveVoiceChannelBySessionID", |_, _| none())?;

    // ── Battle.net: never connected ──
    // BNCreateConversation (`0x535ce0`), BNGetConversationMemberInfo (`0x53a150`),
    // BNGetMatureLanguageFilter (`0x5377c0`) and BNIsSelf (`0x5378a0`) take the same
    // not-connected exit before their usage checks.
    for global in [
        "BNReportPlayer",
        "BNSetAFK",
        "BNSetDND",
        "BNCreateConversation",
        "BNGetConversationMemberInfo",
        "BNGetMatureLanguageFilter",
        "BNIsSelf",
    ] {
        set(lua, global, |_, _| none())?;
    }
    // GetAutoCompletePresenceID(name) (`0x57b500`): the Battle.net auto-complete entry, none.
    set(lua, "GetAutoCompletePresenceID", |lua, args| {
        if text(lua, &arg(&args, 0))?.is_none() {
            return Err(mlua::Error::runtime(
                r#"Usage: GetAutoCompletePresenceID("name")"#,
            ));
        }
        none()
    })?;

    // ── Frame:IsEventRegistered(event): 1 when the frame registered the event by name ──
    let methods: Table = lua.named_registry_value(super::REG_FRAME_METHODS)?;
    methods.set(
        "IsEventRegistered",
        lua.create_function(|lua, (this, event): (Table, String)| {
            let h = super::object::frame_handle_of(lua, &this)?;
            let m = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(flag(
                m.frame_events.get(&h).is_some_and(|s| s.contains(&event)),
            ))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::script::{
        PartyMemberInfo, PartyState, UiScript, UnitState, WrathChatLine,
        WrathUnitPopupRequest as R, WrathUnitPopupState,
    };

    fn script() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_unit(
            "player",
            Some(UnitState {
                guid: 1,
                exists: true,
                name: Some("Me".into()),
                player_flags: 0x200,
                ..Default::default()
            }),
        );
        s.model_mut().unit_guids.player = 1;
        s
    }

    #[test]
    fn difficulty_pvp_and_loot_verbs_follow_the_native_gates() {
        let mut s = script();
        s.run("assert(GetPVPDesired()==1); SetPVP(1); SetPVP(nil); SetDungeonDifficulty(2); SetDungeonDifficulty(3); SetRaidDifficulty(4)").unwrap();
        assert!(s
            .eval::<bool>("local a,b=CanChangePlayerDifficulty(); return a==true and b==true and GetDungeonDifficulty()==2 and GetRaidDifficulty()==4")
            .unwrap());
        assert!(s.run("SetDungeonDifficulty('x')").is_err());
        s.run("SEEN=nil local f=CreateFrame('Frame') f:RegisterEvent('CHAT_MSG_SYSTEM') f:SetScript('OnEvent', function(_, _, msg) SEEN=msg end) assert(f:IsEventRegistered('CHAT_MSG_SYSTEM')==1 and f:IsEventRegistered('BAG_UPDATE')==nil) SetOptOutOfLoot(1)").unwrap();
        assert!(s
            .eval::<bool>("return GetOptOutOfLoot()==1 and SEEN~=nil")
            .unwrap());
        assert_eq!(
            s.take_wrath_unit_popup_requests(),
            vec![
                R::SetPvp { value: 1 },
                R::SetPvp { value: 0 },
                R::SetDungeonDifficulty { difficulty: 1 },
                R::SetRaidDifficulty { difficulty: 3 },
                R::SetOptOutOfLoot { on: true },
            ]
        );
        // Grouped and not the leader: refused with the leader error, nothing sent.
        s.set_party(PartyState {
            members: vec![PartyMemberInfo {
                name: "Lead".into(),
                guid: 2,
            }],
            leader_guid: 2,
            own_guid: 1,
            ..Default::default()
        });
        s.set_wrath_unit_popup_state(WrathUnitPopupState {
            difficulty_locked_until: Some(i64::MAX),
            ..Default::default()
        });
        s.run("SEEN=nil SetRaidDifficulty(1)").unwrap();
        assert!(s.take_wrath_unit_popup_requests().is_empty());
        assert!(s.take_ui_errors().is_empty());
        assert!(s
            .eval::<bool>("return SEEN=='ERR_NOT_LEADER' or SEEN==ERR_NOT_LEADER")
            .unwrap());
        assert!(s
            .eval::<bool>("local a,b=CanChangePlayerDifficulty(); return a==false and b==false")
            .unwrap());
    }

    #[test]
    fn grant_report_complaint_guild_and_inert_voice_and_battle_net() {
        let mut s = script();
        s.set_party(PartyState {
            members: vec![PartyMemberInfo {
                name: "Pal".into(),
                guid: 2,
            }],
            leader_guid: 1,
            own_guid: 1,
            ..Default::default()
        });
        s.set_wrath_unit_popup_state(WrathUnitPopupState {
            grantable: vec![2],
            chat_lines: vec![
                WrathChatLine {
                    line_id: 7,
                    sender: "Pal".into(),
                    text: "hi".into(),
                    guid: 2,
                },
                WrathChatLine {
                    line_id: 8,
                    sender: "Me".into(),
                    text: "yo".into(),
                    guid: 1,
                },
            ],
            ..Default::default()
        });
        for check in [
            "CanGrantLevel('Pal')==1",
            "CanGrantLevel('Nobody')==nil",
            "CanComplainChat(7)==1",
            "CanComplainChat(8)==nil",
            "CanComplainChat('pal','hi')==1",
            "CanComplainChat('Pal','no')==nil",
            "PetCanBeDismissed()==1",
            "IsMuted('Pal')==nil",
            "UnitIsSilenced('Pal','party')==nil",
            "select('#', GetVoiceSessionMemberInfoBySessionID(1,1))==0",
            "IsDisplayChannelOwner()==nil",
            "IsDisplayChannelModerator()==nil",
            "IsSilenced('Pal')==nil",
            "select('#', IsSilenced(nil))==0",
            "UnitIsInMyGuild('Pal')==nil",
            "select('#', GetAutoCompletePresenceID('x'))==0",
        ] {
            assert!(
                s.eval::<bool>(&format!("return {check}")).unwrap(),
                "{check}"
            );
        }
        assert!(s.run("CanComplainChat()").is_err());
        assert!(s.run("GetAutoCompletePresenceID()").is_err());
        s.run("GrantLevel('Pal'); GrantLevel('Nobody'); ReportPlayerIsPVPAFK('Pal'); ReportPlayerIsPVPAFK('player'); ChannelSilenceVoice('Trade','Pal'); ChannelUnSilenceVoice('Party','Pal'); BNSetAFK(true); BNSetDND(); BNReportPlayer(); SetActiveVoiceChannel(0)").unwrap();
        assert_eq!(
            s.take_wrath_unit_popup_requests(),
            vec![
                R::GrantLevel { guid: 2 },
                R::ReportPvpAfk { guid: 2 },
                R::ChannelSilenceVoice {
                    channel: "Trade".into(),
                    name: "Pal".into(),
                    silence: true,
                    group: false,
                },
                R::ChannelSilenceVoice {
                    channel: "Party".into(),
                    name: "Pal".into(),
                    silence: false,
                    group: true,
                },
            ]
        );
    }
}
