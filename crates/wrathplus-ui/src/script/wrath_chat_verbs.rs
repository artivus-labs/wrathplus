//! Build-12340 globals ChatFrame.lua's slash commands call that the 1.12 surface lacks (one
//! Wow.exe round: `/tmp/azb-ui-chatframe-native.log`, `/tmp/azb-ui-chatframe-rest4.log`).
//!
//! Where a 1.12 request already carries the intent, the 12340 binding feeds it (party, guild,
//! friends, container use). Verbs that read or move the selection or cast go in call order with
//! casts, as `ScriptCall`s; the pure sends go out as [`WrathChatVerbRequest`]s, labelled
//! capture-only until App sends them. Battle.net bindings answer the closed session a 3.3.5a realm always
//! leaves: every check before their usage lines asks for a connected session, so they do nothing.

use mlua::{Lua, MultiValue, Value};

use super::{Model, ScriptValue};

/// One 12340 command intent for App, in call order. Capture-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathChatVerbRequest {
    /// `CancelItemTempEnchantment(slot)` (`0x8021d0` → `0x6d7120`): 1 the main hand, 2 the off
    /// hand, whose temporary enchant is cancelled.
    CancelTempEnchantment { slot: u8 },
    /// `Sound_GameSystem_RestartSoundSystem()` (`0x985d30`): the sound system restarts.
    RestartSoundSystem,
    /// `ArenaTeamInviteByName(team, name)` (`0x515cc0`, CMSG `0x34F`); `team` is the 1-based
    /// arena-team slot App resolves to its team id (0 when the slot holds none).
    ArenaTeamInvite { team: u8, name: String },
    /// `ArenaTeamLeave(team)` (`0x515dd0`, CMSG `0x353`).
    ArenaTeamLeave { team: u8 },
    /// `ArenaTeamUninviteByName(team, name)` (`0x515eb0`, CMSG `0x354`).
    ArenaTeamUninvite { team: u8, name: String },
    /// `ArenaTeamSetLeaderByName(team, name)` (`0x515ff0`, CMSG `0x356`).
    ArenaTeamSetLeader { team: u8, name: String },
    /// `SetCurrentTitle(titleMaskID)` (`0x522e30`, CMSG `0x374`). Labelled: native's rate limit
    /// (more than ten calls in a short window raise an error) is not modelled.
    SetTitle { title: i32 },
    /// `SetTaxiBenchmarkMode([on])` (`0x517b50`, CMSG `0x389`).
    TaxiBenchmarkMode { on: bool },
}

impl super::UiScript {
    /// Drain the 12340 command intents in call order.
    pub fn take_wrath_chat_verb_requests(&mut self) -> Vec<WrathChatVerbRequest> {
        std::mem::take(&mut self.model_mut().wrath_chat_verbs)
    }
}

fn call(lua: &Lua, c: super::ScriptCall) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .script_calls
        .push(c);
}

fn push(lua: &Lua, r: WrathChatVerbRequest) {
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .wrath_chat_verbs
        .push(r);
}

fn has_player(lua: &Lua) -> bool {
    lua.app_data_ref::<Model>()
        .expect("model app_data")
        .unit_guids
        .player
        != 0
}

fn number(lua: &Lua, v: &Value) -> Option<f64> {
    lua.coerce_number(v.clone()).ok().flatten()
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// `lua_tostring` (`0x84e0e0`): strings and numbers, `None` otherwise.
fn text(lua: &Lua, v: &Value) -> mlua::Result<Option<String>> {
    Ok(lua.coerce_string(v.clone())?.map(|s| s.to_string_lossy()))
}

/// A name argument as the 12340 verbs check it: 48 bytes at most, else "Name too long".
fn name_ok(name: &str) -> mlua::Result<()> {
    if name.len() > 0x30 {
        return Err(mlua::Error::runtime("Name too long"));
    }
    Ok(())
}

/// An arena-team slot argument: rounded, 1-based; 0 past the three slots.
fn team(lua: &Lua, v: &Value) -> u8 {
    lua.coerce_number(v.clone())
        .ok()
        .flatten()
        .map(|n| n.round_ties_even() as i64)
        .filter(|n| (1..=3).contains(n))
        .map_or(0, |n| n as u8)
}

fn is_number(lua: &Lua, v: &Value) -> bool {
    lua.coerce_number(v.clone()).ok().flatten().is_some()
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

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    use super::guild::GuildRequest;
    use super::party::PartyRequest;
    use WrathChatVerbRequest as R;

    // ── party ──
    // InviteUnit(name [, ...]) (`0x51a6e0`): with the player object, a name of at most 48 bytes is
    // invited. Labelled: the three flag arguments (bits 2/4/8 of `0x6d4300`) are not carried.
    set(lua, "InviteUnit", |lua, args| {
        if !has_player(lua) {
            return none();
        }
        if let Some(name) = text(lua, &arg(&args, 0))? {
            name_ok(&name)?;
            lua.app_data_mut::<Model>()
                .expect("model app_data")
                .party_requests
                .push(PartyRequest::InviteName(name));
        }
        none()
    })?;
    // UninviteUnit(unitOrName [, reason]) (`0x51a7a0`): a unit token or a member's name resolves
    // to a guid (`0x60abf0`, ported as `wrath_unit_popup::guid_of`), and one that does sends
    // `CMSG_GROUP_UNINVITE_GUID` with the reason cut to 64 characters (`0x7e1830(.., 0x40, ..)`).
    set(lua, "UninviteUnit", |lua, args| {
        if !has_player(lua) {
            return none();
        }
        let Some(who) = text(lua, &arg(&args, 0))? else {
            return none();
        };
        let reason: String = text(lua, &arg(&args, 1))?
            .unwrap_or_default()
            .chars()
            .take(64)
            .collect();
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let guid = super::wrath_unit_popup::guid_of(&model, &who);
        if guid != 0 {
            model
                .party_requests
                .push(PartyRequest::UninviteGuid { guid, reason });
        }
        none()
    })?;
    // PromoteToLeader(name) (`0x573e10`).
    set(lua, "PromoteToLeader", |lua, args| {
        let Some(name) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime("Usage: PromoteToLeader(name)"));
        };
        lua.app_data_mut::<Model>()
            .expect("model app_data")
            .party_requests
            .push(PartyRequest::PromoteName(name));
        none()
    })?;

    // ── guild ── (`0x522a20`..`0x522d60`): a missing name is a game error, an over-long one raises.
    for (global, make) in [
        (
            "GuildInvite",
            GuildRequest::Invite as fn(String) -> GuildRequest,
        ),
        ("GuildUninvite", GuildRequest::Uninvite),
        ("GuildPromote", GuildRequest::Promote),
        ("GuildDemote", GuildRequest::Demote),
        ("GuildSetLeader", GuildRequest::SetLeader),
    ] {
        set(lua, global, move |lua, args| {
            if let Some(name) = text(lua, &arg(&args, 0))? {
                name_ok(&name)?;
                lua.app_data_mut::<Model>()
                    .expect("model app_data")
                    .guild_requests
                    .push(make(name));
            }
            none()
        })?;
    }

    // ── friends ──
    // AddOrRemoveFriend(name [, note]) (`0x6b7b70` → `0x6b6e10`): removes a listed friend, else
    // adds. Labelled: the note is not carried by the add request.
    set(lua, "AddOrRemoveFriend", |lua, args| {
        let Some(name) = text(lua, &arg(&args, 0))?.filter(|n| !n.trim().is_empty()) else {
            return none();
        };
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let listed = model
            .social
            .friends
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name));
        let request = if listed {
            super::social::SocialRequest::RemoveFriendName(name)
        } else {
            super::social::SocialRequest::AddFriend(name)
        };
        model.social_requests.push(request);
        none()
    })?;

    // ── arena teams ──
    for (global, usage, kind) in [
        (
            "ArenaTeamInviteByName",
            "Usage: ArenaTeamInviteByName(team, name)",
            0u8,
        ),
        (
            "ArenaTeamUninviteByName",
            "Usage: ArenaTeamUninviteByName(team, name)",
            1,
        ),
        (
            "ArenaTeamSetLeaderByName",
            "Usage: ArenaTeamSetLeaderByName(team, name)",
            2,
        ),
    ] {
        set(lua, global, move |lua, args| {
            let (t, n) = (arg(&args, 0), arg(&args, 1));
            if !is_number(lua, &t) || text(lua, &n)?.is_none() {
                return Err(mlua::Error::runtime(usage));
            }
            let name = text(lua, &n)?.unwrap_or_default();
            name_ok(&name)?;
            let team = team(lua, &t);
            match kind {
                // An empty name sends nothing (`0x515d12`).
                0 if name.is_empty() => {}
                0 => push(lua, R::ArenaTeamInvite { team, name }),
                1 => push(lua, R::ArenaTeamUninvite { team, name }),
                _ => push(lua, R::ArenaTeamSetLeader { team, name }),
            }
            none()
        })?;
    }
    set(lua, "ArenaTeamLeave", |lua, args| {
        let t = arg(&args, 0);
        if !is_number(lua, &t) {
            return Err(mlua::Error::runtime("Usage: ArenaTeamLeave(team)"));
        }
        let team = team(lua, &t);
        push(lua, R::ArenaTeamLeave { team });
        none()
    })?;
    // ── Battle.net: closed session ──
    for global in [
        "BNInviteToConversation",
        "BNIsFriend",
        "BNIsToonBlocked",
        "BNListConversation",
        "BNSendConversationMessage",
        "BNSendWhisper",
        "BNSetToonBlocked",
    ] {
        set(lua, global, |_, _| none())?;
    }

    // ── actions ──
    set(lua, "CancelShapeshiftForm", |lua, _| {
        if has_player(lua) {
            call(lua, super::ScriptCall::CancelShapeshiftForm);
        }
        none()
    })?;
    set(lua, "Dismount", |lua, _| {
        if has_player(lua) {
            super::wrath_char_state::dismount(lua);
        }
        none()
    })?;
    set(lua, "StartAttack", |lua, args| {
        if has_player(lua) {
            let unit = text(lua, &arg(&args, 0))?.filter(|u| !u.is_empty());
            call(lua, super::ScriptCall::StartAttack { unit });
        }
        none()
    })?;
    set(lua, "StopAttack", |lua, _| {
        if has_player(lua) {
            call(lua, super::ScriptCall::StopAttack);
        }
        none()
    })?;
    // StopMacro() (`0x564b90`) stops a running macro; no macro runs in this client, so it does
    // nothing.
    set(lua, "StopMacro", |_, _| none())?;
    set(lua, "TargetLastFriend", |lua, _| {
        call(lua, super::ScriptCall::TargetLastFriend);
        none()
    })?;
    set(lua, "TargetNearestEnemyPlayer", |lua, args| {
        let reverse = super::binding_abi::bool_or_default(args.front(), false);
        call(
            lua,
            super::ScriptCall::TargetNearest {
                mode: super::NearestMode::EnemyPlayer,
                reverse,
            },
        );
        none()
    })?;
    set(lua, "TargetNearestFriendPlayer", |lua, args| {
        let reverse = super::binding_abi::bool_or_default(args.front(), false);
        call(
            lua,
            super::ScriptCall::TargetNearest {
                mode: super::NearestMode::FriendPlayer,
                reverse,
            },
        );
        none()
    })?;
    // Enable/DisableSpellAutocast(spell) (`0x541140`/`0x5411a0`): the pet action slot holding the
    // named spell has its autocast set, through the existing toggle when it differs.
    for (global, on) in [
        ("EnableSpellAutocast", true),
        ("DisableSpellAutocast", false),
    ] {
        set(lua, global, move |lua, args| {
            let Some(spell) = text(lua, &arg(&args, 0))? else {
                return none();
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            if let Some(index) = super::pet::slot_index_by_name(&model, &spell)
                .filter(|i| super::pet::autocast_state(&model, *i) == Some(!on))
            {
                model.pet_autocast_toggles.push(index);
            }
            none()
        })?;
    }
    // UseItemByName(nameOrId [, target]) (`0x51cd00`): the first bag slot holding the item is
    // used. Labelled: the target argument is not carried.
    set(lua, "UseItemByName", |lua, args| {
        if !has_player(lua) {
            return none();
        }
        let key = arg(&args, 0);
        let mut model = lua.app_data_mut::<Model>().expect("model app_data");
        let id = lua
            .coerce_number(key.clone())
            .ok()
            .flatten()
            .map(|n| n as u32);
        let name = match (&key, id) {
            (_, Some(_)) => None,
            (v, None) => lua.coerce_string(v.clone())?.map(|s| s.to_string_lossy()),
        };
        let mut bags: Vec<i64> = model.containers.keys().copied().collect();
        bags.sort_unstable();
        let found = bags.into_iter().find_map(|bag| {
            let c = model.containers.get(&bag)?;
            let mut slots: Vec<(&u32, &super::container::ContainerSlot)> = c.slots.iter().collect();
            slots.sort_by_key(|(s, _)| **s);
            slots
                .into_iter()
                .find(|(_, s)| {
                    s.item_id != 0
                        && match (&name, id) {
                            (_, Some(id)) => s.item_id == id,
                            (Some(n), None) => super::cursor::item_link_name(s.link.as_deref())
                                .eq_ignore_ascii_case(n),
                            _ => false,
                        }
                })
                .map(|(slot, _)| (bag, *slot))
        });
        if let Some((bag, slot)) = found {
            model
                .script_calls
                .push(super::ScriptCall::UseContainerItem { bag, slot });
        }
        none()
    })?;

    // ── talents, titles, dev ──
    set(lua, "SetActiveTalentGroup", |lua, args| {
        let g = arg(&args, 0);
        if !is_number(lua, &g) {
            return Err(mlua::Error::runtime(
                "Usage: SetActiveTalentGroup(groupIndex)",
            ));
        }
        let group = lua.coerce_number(g)?.unwrap_or(0.0).round_ties_even() as i64;
        let ask = |name: &str| -> mlua::Result<i64> {
            match lua.globals().get::<Value>(name)? {
                Value::Function(f) => Ok(f.call::<Option<i64>>(())?.unwrap_or(1)),
                _ => Ok(1),
            }
        };
        let (active, count) = (ask("GetActiveTalentGroup")?, ask("GetNumTalentGroups")?);
        if group != active && group >= 1 && group <= count.min(2) {
            call(
                lua,
                super::ScriptCall::ActivateTalentGroup { group: group as u8 },
            );
        }
        none()
    })?;
    set(lua, "SetCurrentTitle", |lua, args| {
        let t = arg(&args, 0);
        if !is_number(lua, &t) {
            return Err(mlua::Error::runtime("Usage: SetCurrentTitle(titleMaskID)"));
        }
        if has_player(lua) {
            let title = lua.coerce_number(t)?.unwrap_or(0.0).trunc() as i64 as i32;
            push(lua, R::SetTitle { title });
        }
        none()
    })?;
    set(lua, "SetTaxiBenchmarkMode", |lua, args| {
        let on = super::binding_abi::bool_or_default(args.front(), true);
        push(lua, R::TaxiBenchmarkMode { on });
        none()
    })?;

    // ── chat and lookups ──
    // SendSystemMessage(message) (`0x50d8a0`): a system chat line.
    // CancelItemTempEnchantment(slot) (`0x8021d0`): 1 or 2, the main or off hand, holding an item.
    // Labelled: the temporary-enchant check `0x518b30` is the server's to refuse here.
    set(lua, "CancelItemTempEnchantment", |lua, args| {
        let has_player = lua
            .app_data_ref::<Model>()
            .expect("model app_data")
            .unit_guids
            .player
            != 0;
        if !has_player {
            return none();
        }
        let slot = number(lua, &arg(&args, 0)).map(|n| n.trunc() as i64);
        let Some(slot) = slot.filter(|s| (1..=2).contains(s)) else {
            return Err(mlua::Error::runtime(
                "Usage: CancelItemTempEnchantment(slot)",
            ));
        };
        let held = {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            model
                .inv_slot("player", 15 + slot as usize)
                .is_some_and(|s| s.item_id != 0)
        };
        if held {
            push(
                lua,
                WrathChatVerbRequest::CancelTempEnchantment { slot: slot as u8 },
            );
        }
        none()
    })?;
    // IsDressableItem(item) (`0x517530`) → 1 for an item with a display (labelled: read as a cached
    // template with an icon; native's pet-summon fallback for a displayless item is not read).
    set(lua, "IsDressableItem", |lua, args| {
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let first = arg(&args, 0);
        let id = match &first {
            Value::Integer(_) | Value::Number(_) => number(lua, &first).map(|n| n as u32),
            Value::String(s) => {
                let s = s.to_string_lossy();
                s.split("item:")
                    .nth(1)
                    .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
                    .and_then(|d| d.parse().ok())
                    .or_else(|| {
                        model
                            .item_templates
                            .iter()
                            .find(|(_, t)| t.name.eq_ignore_ascii_case(&s))
                            .map(|(id, _)| *id)
                    })
            }
            _ => None,
        };
        let dressable = id
            .and_then(|id| model.item_templates.get(&id))
            .is_some_and(|t| t.icon.is_some());
        Ok(MultiValue::from_vec(vec![super::binding_abi::flag(
            dressable,
        )]))
    })?;
    // Sound_GameSystem_RestartSoundSystem() (`0x985d30`): the host restarts its sound system.
    set(lua, "Sound_GameSystem_RestartSoundSystem", |lua, _| {
        push(lua, WrathChatVerbRequest::RestartSoundSystem);
        none()
    })?;
    set(lua, "SendSystemMessage", |lua, args| {
        let Some(message) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                r#"Usage: SendSystemMessage("message")"#,
            ));
        };
        super::tick::fire_event_into(lua, "CHAT_MSG_SYSTEM", vec![ScriptValue::Str(message)]);
        none()
    })?;
    // GetClickFrame(name) (`0x564130`) → the named frame, nil when none. Labelled: native looks in
    // its click-frame registry; this reads the frame by global name.
    set(lua, "GetClickFrame", |lua, args| {
        let Some(name) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(r#"Usage: GetClickFrame("name")"#));
        };
        let v: Value = lua.globals().get(name)?;
        Ok(MultiValue::from_vec(vec![match v {
            Value::Table(t) if super::object::frame_handle_of(lua, &t).is_ok() => Value::Table(t),
            _ => Value::Nil,
        }]))
    })?;
    // GetPlayerInfoByGUID(guid) (`0x613f90`) → class, englishClass, race, englishRace, sex,
    // name, realm for a player GUID whose unit is known; nothing otherwise. Labelled: native also
    // answers from its name cache for players never seen as units.
    set(lua, "GetPlayerInfoByGUID", |lua, args| {
        let Some(text) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(
                r#"Usage: GetPlayerInfoByGUID("playerGUID")"#,
            ));
        };
        let guid = parse_guid(&text);
        if guid == 0 || guid >> 60 != 0 {
            return none();
        }
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let Some(u) = model.units_by_guid.get(&guid) else {
            return none();
        };
        let s = |v: &Option<String>| -> mlua::Result<Value> {
            Ok(match v {
                Some(v) => Value::String(lua.create_string(v)?),
                None => Value::Nil,
            })
        };
        Ok(MultiValue::from_vec(vec![
            s(&u.class)?,
            s(&u.class_file)?,
            s(&u.race)?,
            s(&u.race_file)?,
            // UnitSex's scale; the unfilled 0 answers 2, as UnitSex does.
            Value::Integer(i64::from(if u.sex == 0 { 2 } else { u.sex })),
            s(&u.name)?,
            Value::String(lua.create_string("")?),
        ]))
    })?;
    // GetAutoCompleteResults(text, include, exclude, numReturns [, cursorPosition
    // [, allowFullMatch]]) (`0x57b3a0`) → up to numReturns names starting with the text: group
    // members (flag 0x1), guild members (0x2) and friends (0x4), online ones adding 0x20 (3.3.5
    // AUTOCOMPLETE_FLAG_*); a name passes when its flags meet `include` and miss `exclude`.
    // Labelled: the recently-interacted source (0x10) is not carried.
    set(lua, "GetAutoCompleteResults", |lua, args| {
        const USAGE: &str = "Usage: GetAutoCompleteResults(text, includeBitField, excludeBitField, numReturns[, cursorPosition[, allowFullMatch]])";
        let Some(prefix) = text(lua, &arg(&args, 0))? else {
            return Err(mlua::Error::runtime(USAGE));
        };
        if !(1..=3).all(|i| is_number(lua, &arg(&args, i))) {
            return Err(mlua::Error::runtime(USAGE));
        }
        let num = |i| {
            lua.coerce_number(arg(&args, i))
                .ok()
                .flatten()
                .unwrap_or(0.0)
                .round_ties_even() as i64
        };
        let (include, exclude, count) = (num(1) as u32, num(2) as u32, num(3).max(0) as usize);
        let full = super::binding_abi::bool_or_default(args.get(5), false);
        let model = lua.app_data_ref::<Model>().expect("model app_data");
        let mut seen: Vec<(String, u32)> = Vec::new();
        let mut add = |name: &str, flags: u32| {
            if let Some(e) = seen.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                e.1 |= flags;
            } else if !name.is_empty() {
                seen.push((name.to_string(), flags));
            }
        };
        for m in &model.party.members {
            add(&m.name, 0x1 | 0x20);
        }
        for m in &model.party.raid {
            add(&m.name, 0x1 | 0x20);
        }
        for m in &model.guild.roster {
            add(&m.name, 0x2 | if m.online { 0x20 } else { 0 });
        }
        for f in &model.social.friends {
            add(&f.name, 0x4 | if f.connected { 0x20 } else { 0 });
        }
        let lower = prefix.to_lowercase();
        let mut out = MultiValue::new();
        for (name, flags) in seen {
            if out.len() >= count {
                break;
            }
            let lname = name.to_lowercase();
            let matches = lname.starts_with(&lower) && (full || lname != lower);
            if matches && flags & include != 0 && flags & exclude == 0 {
                out.push_back(Value::String(lua.create_string(&name)?));
            }
        }
        Ok(out)
    })?;
    Ok(())
}

/// A GUID string: an optional `0x`, then up to 16 hex digits.
fn parse_guid(s: &str) -> u64 {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    s.chars()
        .take(16)
        .map_while(|c| c.to_digit(16))
        .fold(0u64, |acc, d| acc << 4 | u64::from(d))
}

#[cfg(test)]
mod tests {
    use super::WrathChatVerbRequest as R;
    use crate::script::UiScript;

    fn vm() -> UiScript {
        let s = UiScript::new().unwrap();
        s.model_mut().unit_guids.player = 0x10;
        s
    }

    #[test]
    fn party_guild_and_friend_verbs_feed_the_existing_requests() {
        let s = vm();
        s.model_mut()
            .party
            .members
            .push(crate::script::party::PartyMemberInfo {
                name: "Ann".into(),
                guid: 0x20,
            });
        s.run(&format!(
            "InviteUnit('Bob') UninviteUnit('ann', 'afk') UninviteUnit('Nobody') \
               UninviteUnit('Ann', '{}') PromoteToLeader('Cid') GuildInvite('Dee') \
               GuildSetLeader('Eve') GuildPromote()",
            "r".repeat(70)
        ))
        .unwrap();
        assert!(
            s.run(&format!("InviteUnit('{}')", "x".repeat(49))).is_err(),
            "Name too long"
        );
        let model = s.model_mut();
        assert_eq!(model.party_requests.len(), 4);
        assert_eq!(
            model.party_requests[1..3],
            [
                crate::script::party::PartyRequest::UninviteGuid {
                    guid: 0x20,
                    reason: "afk".into()
                },
                crate::script::party::PartyRequest::UninviteGuid {
                    guid: 0x20,
                    reason: "r".repeat(64)
                },
            ],
            "an unresolved name sends nothing; the reason stops at 64 characters"
        );
        assert!(model.uninvite_requests.is_empty());
        assert_eq!(model.guild_requests.len(), 2);
        drop(model);
        s.model_mut()
            .social
            .friends
            .push(crate::script::social::FriendInfo {
                name: "Fay".into(),
                ..Default::default()
            });
        s.run("AddOrRemoveFriend('fay') AddOrRemoveFriend('Gus', 'note')")
            .unwrap();
        let reqs = s.model_mut().social_requests.clone();
        assert_eq!(
            reqs,
            vec![
                crate::script::social::SocialRequest::RemoveFriendName("fay".into()),
                crate::script::social::SocialRequest::AddFriend("Gus".into()),
            ]
        );
    }

    #[test]
    fn command_intents_split_between_call_order_and_the_outbox() {
        let mut s = vm();
        s.run(
            "ArenaTeamInviteByName(2, 'Hal') ArenaTeamInviteByName(1, '') ArenaTeamLeave(4) \
             CancelShapeshiftForm() Dismount() StartAttack() StartAttack('target') StopAttack() \
             TargetLastFriend() TargetNearestEnemyPlayer(1) SetCurrentTitle(42) \
             SetActiveTalentGroup(2) SetTaxiBenchmarkMode()",
        )
        .unwrap();
        assert_eq!(
            s.take_wrath_chat_verb_requests(),
            vec![
                R::ArenaTeamInvite {
                    team: 2,
                    name: "Hal".into()
                },
                R::ArenaTeamLeave { team: 0 },
                R::SetTitle { title: 42 },
                R::TaxiBenchmarkMode { on: true },
            ]
        );
        use crate::script::{NearestMode, ScriptCall as C};
        assert_eq!(
            s.model_mut().script_calls.clone(),
            vec![
                C::CancelShapeshiftForm,
                C::Dismount,
                C::StartAttack { unit: None },
                C::StartAttack {
                    unit: Some("target".into())
                },
                C::StopAttack,
                C::TargetLastFriend,
                C::TargetNearest {
                    mode: NearestMode::EnemyPlayer,
                    reverse: true
                },
            ],
            "selection, swing and cancel verbs go in call order; one talent group casts nothing"
        );
        assert!(s.run("ArenaTeamLeave()").is_err());
        assert!(s.run("SetCurrentTitle('x')").is_err());
    }

    #[test]
    fn battle_net_is_inert_and_lookups_answer_truthfully() {
        let s = vm();
        assert_eq!(
            s.eval::<i64>("return select('#', BNIsFriend(1)) + select('#', BNSendWhisper())")
                .unwrap(),
            0
        );
        s.model_mut().units_by_guid.insert(
            0x42,
            crate::script::UnitState {
                name: Some("Ivy".into()),
                class: Some("Mage".into()),
                class_file: Some("MAGE".into()),
                race: Some("Gnome".into()),
                race_file: Some("Gnome".into()),
                sex: 3,
                ..Default::default()
            },
        );
        assert_eq!(
            s.eval::<(String, String, i64, String)>(
                "local c, cf, _, _, sex, n = GetPlayerInfoByGUID('0x0000000000000042') return c, cf, sex, n"
            )
            .unwrap(),
            ("Mage".into(), "MAGE".into(), 3, "Ivy".into())
        );
        assert_eq!(
            s.eval::<i64>("return select('#', GetPlayerInfoByGUID('0xF130000000000042'))")
                .unwrap(),
            0,
            "not a player GUID"
        );
        s.run(
            "SEEN = nil local f = CreateFrame('Frame') f:RegisterEvent('CHAT_MSG_SYSTEM') \
               f:SetScript('OnEvent', function(_, _, m) SEEN = m end) SendSystemMessage('hi')",
        )
        .unwrap();
        assert_eq!(s.eval::<String>("return SEEN").unwrap(), "hi");
    }

    #[test]
    fn autocomplete_matches_group_guild_and_friends_by_flags() {
        let s = vm();
        {
            let mut m = s.model_mut();
            m.party.members.push(crate::script::party::PartyMemberInfo {
                name: "Alda".into(),
                ..Default::default()
            });
            m.guild.roster.push(crate::script::guild::GuildMemberInfo {
                name: "Alric".into(),
                online: false,
                ..Default::default()
            });
            m.social.friends.push(crate::script::social::FriendInfo {
                name: "Albin".into(),
                connected: true,
                ..Default::default()
            });
        }
        assert_eq!(
            s.eval::<Vec<String>>("return {GetAutoCompleteResults('al', 0xFFFFFFFF, 0, 5)}")
                .unwrap(),
            vec!["Alda".to_string(), "Alric".to_string(), "Albin".to_string()]
        );
        assert_eq!(
            s.eval::<Vec<String>>("return {GetAutoCompleteResults('al', 0x20, 0x1, 5)}")
                .unwrap(),
            vec!["Albin".to_string()],
            "online, not in the group"
        );
        assert!(s.run("GetAutoCompleteResults('al')").is_err());
    }

    #[test]
    fn use_item_by_name_finds_the_first_bag_slot() {
        let s = vm();
        let mut slots = std::collections::HashMap::new();
        slots.insert(
            3,
            crate::script::container::ContainerSlot {
                item_id: 2589,
                link: Some("|Hitem:2589|h[Linen Cloth]|h".into()),
                ..Default::default()
            },
        );
        s.model_mut().containers.insert(
            0,
            crate::script::container::ContainerState {
                slots,
                ..Default::default()
            },
        );
        s.run("UseItemByName('linen cloth') UseItemByName(2589) UseItemByName('Nope')")
            .unwrap();
        let calls = s.model_mut().script_calls.clone();
        assert_eq!(
            calls,
            vec![
                crate::script::ScriptCall::UseContainerItem { bag: 0, slot: 3 },
                crate::script::ScriptCall::UseContainerItem { bag: 0, slot: 3 },
            ]
        );
    }

    #[test]
    fn edit_box_cursor_positions_in_bytes_and_characters() {
        let s = vm();
        s.run("E = CreateFrame('EditBox') E:SetText('héllo') E:SetCursorPosition(3)")
            .unwrap();
        assert_eq!(
            s.eval::<(i64, i64)>("return E:GetCursorPosition(), E:GetUTF8CursorPosition()")
                .unwrap(),
            (3, 2)
        );
    }

    #[test]
    fn cancel_unit_buff_cancels_a_cancelable_player_aura() {
        let s = vm();
        s.model_mut()
            .player_auras
            .push(crate::script::aura::AuraState {
                spell_id: 1243,
                name: Some("Power Word: Fortitude".into()),
                helpful: true,
                cancelable: true,
                ..Default::default()
            });
        s.run(
            "CancelUnitBuff('player', 1) CancelUnitBuff('player', 'Power Word: Fortitude') \
               CancelUnitBuff('target', 1)",
        )
        .unwrap();
        assert_eq!(s.model_mut().cancel_aura_requests, vec![1243, 1243]);
    }

    #[test]
    fn temp_enchant_cancel_dressable_and_sound_restart() {
        use crate::script::{InvSlotView, ItemTemplateView};
        let mut s = UiScript::new().unwrap();
        s.model_mut().unit_guids.player = 0x10;
        s.set_unit(
            "player",
            Some(crate::script::UnitState {
                guid: 0x10,
                exists: true,
                ..Default::default()
            }),
        );
        let mut slots: crate::script::InventorySlots = Default::default();
        slots[16] = Some(InvSlotView {
            item_id: 7,
            ..Default::default()
        });
        s.set_inventory_slots(slots);
        s.set_item_template(
            7,
            ItemTemplateView {
                name: "Blade".into(),
                icon: Some("I".into()),
                ..Default::default()
            },
        );
        s.run("CancelItemTempEnchantment(1) CancelItemTempEnchantment(2) Sound_GameSystem_RestartSoundSystem()").unwrap();
        assert!(s.run("CancelItemTempEnchantment(3)").is_err());
        assert!(s.eval::<bool>("return IsDressableItem(7) == 1 and IsDressableItem('Blade') == 1 and IsDressableItem(8) == nil").unwrap());
        assert_eq!(
            s.take_wrath_chat_verb_requests(),
            vec![R::CancelTempEnchantment { slot: 1 }, R::RestartSoundSystem]
        );
    }
}
