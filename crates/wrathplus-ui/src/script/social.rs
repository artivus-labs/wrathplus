//! The social API: friends, ignores and `/who`. The app pushes a display-ready [`SocialState`]
//! (12340 resolves names through its name cache, `0x67d770`, engine-side too) and drains the
//! queued [`SocialRequest`]s. Selection is engine state in 12340 (the friend guid at `+0xd488`,
//! read back by `GetSelectedFriend` `0x6b44b0`), so a set changes the snapshot at once and queues
//! the change for the app.

use mlua::{Lua, MultiValue, Value};

use super::binding_abi::{is_number, is_string, number_arg, string_arg};
use super::unit::token::sstr_eq_i;
use super::who_sort::WhoSortChain;
use super::Model;

/// One friend row in `GetFriendInfo`'s order; a class or area left empty (no DBC row) answers
/// `UNKNOWN`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FriendInfo {
    /// The contact note, `GetFriendInfo`'s seventh; `None` reads nil.
    pub note: Option<String>,
    /// A Recruit-A-Friend link (status bit 3), the eighth.
    pub refer_a_friend: bool,
    /// From the name cache; empty while the name query is in flight, and then matched by no name.
    pub name: String,
    pub level: u32,
    pub class: String,
    /// The zone name, also empty when the id has no `AreaTable` row.
    pub area: String,
    pub connected: bool,
    /// The away tag the friends-list template takes: `""`, `"<AFK>"` or `"<DND>"`.
    pub status: String,
}

/// One `/who` row in `GetWhoInfo`'s order, names localized; race comes before class, where the
/// wire has class first. An empty race, class or zone is a missing row, which `GetWhoInfo` shows as
/// `UNKNOWN` and the sort chain passes over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhoInfo {
    /// The class file name (`ChrClasses` `+0x1c`), `GetWhoInfo`'s seventh; empty reads nil.
    pub class_file: String,
    pub name: String,
    pub guild: String,
    pub level: u32,
    pub race: String,
    pub class: String,
    pub zone: String,
}

/// The social snapshot the app pushes whole; the default is a fresh login's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocialState {
    /// In display order: the app sorts, as 12340's comparator `0x6b3960` does.
    pub friends: Vec<FriendInfo>,
    /// The selected friend, 1-based, 0 for none (`0x5ad260` returns the stored slot + 1).
    pub selected_friend: u32,
    pub ignores: Vec<String>,
    /// The selected ignore, on the same scale.
    pub selected_ignore: u32,
    /// The last `/who` answer's rows, at most 49.
    pub who: Vec<WhoInfo>,
    /// The total match count, `GetNumWhoResults`'s second return; it can exceed `who.len()`.
    pub who_total: u32,
    /// The `/who` sort chain, pushed so `SortWho` can re-sort before its synchronous redraw.
    pub who_sort: WhoSortChain,
}

/// Outbound social intents, drained by the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocialRequest {
    /// `ShowFriends()`: `CMSG_FRIEND_LIST`.
    RefreshFriends,
    /// `AddFriend(name [, note])` and `AddOrRemoveFriend`'s add: CMSG `0x69` carries both.
    AddFriend { name: String, note: String },
    /// `RemoveFriend(index)`, from the friends frame's button.
    RemoveFriendIndex(u32),
    /// `RemoveFriend(name)`, from `/removefriend`; the app resolves either shape to the guid.
    RemoveFriendName(String),
    /// `AddIgnore(name)`.
    AddIgnore(String),
    /// `DelIgnore(name)`.
    DelIgnore(String),
    /// `AddOrDelIgnore(name)`, `/ignore`'s toggle; the app decides which, as it holds the list.
    ToggleIgnore(String),
    /// `SetSelectedFriend(index)`, mirrored so the next push agrees.
    SelectFriend(u32),
    /// `SetSelectedIgnore(index)`.
    SelectIgnore(u32),
    /// `SendWho(filter)`, the raw filter; parsing it needs the DBCs, so the app does it.
    Who(String),
    /// `SortWho(sortType)`, the raw argument; the binding has already sorted the snapshot.
    SortWho(String),
    /// `SetWhoToUI(flag)`: the next `/who` answer goes to the Who frame (true) or chat (false).
    SetWhoToUi(bool),
}

impl super::UiScript {
    /// Replace the social snapshot; firing `FRIENDLIST_UPDATE` and kin is the app's job.
    pub fn set_social(&mut self, state: SocialState) {
        self.model_mut().social = state;
    }

    /// Drain the social intents queued since the last call.
    pub fn take_social_requests(&mut self) -> Vec<SocialRequest> {
        std::mem::take(&mut self.model_mut().social_requests)
    }
}

/// Register the social globals against the snapshot store.
pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // ── The friend list ── (`GetNumFriends` is `unit/wrath_queries.rs`'s, two values.)
    // GetFriendInfo(index or name) (`0x6b4130`) → name, level, class, area, connected, status,
    // note, RAF. A number or numeric string is an index, truncated; a name matches through
    // `0x76ea40` past the pending rows (`0x6b3570`). A miss answers nil, 0, UNKNOWN, UNKNOWN, nil,
    // "", nil, nil (`0x6b43a7`-`0x6b43e4`).
    g.set(
        "GetFriendInfo",
        lua.create_function(|lua, arg: Value| {
            const USAGE: &str = "Usage: GetFriendInfo(index or name)";
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let friends = &model.social.friends;
            let friend = if is_number(lua, Some(&arg)) {
                friend_at(friends, i64::from(number_arg(lua, arg, USAGE)?))
            } else if is_string(Some(&arg)) {
                let name = lua.coerce_string(arg)?.map(|s| s.as_bytes().to_vec());
                let name = name.unwrap_or_default();
                friends
                    .iter()
                    .find(|f| !f.name.is_empty() && sstr_eq_i(f.name.as_bytes(), &name))
            } else {
                return Err(mlua::Error::runtime(USAGE));
            };
            let Some(friend) = friend else {
                let mut v = vec![Value::Nil, Value::Integer(0), unknown(lua)?, unknown(lua)?];
                v.extend([Value::Nil, Value::String(lua.create_string("")?)]);
                v.extend([Value::Nil, Value::Nil]);
                return Ok(MultiValue::from_vec(v));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&friend.name)?),
                Value::Integer(i64::from(friend.level)),
                or_unknown(lua, &friend.class)?,
                or_unknown(lua, &friend.area)?,
                super::binding_abi::flag(friend.connected),
                Value::String(lua.create_string(&friend.status)?),
                match &friend.note {
                    Some(n) if !n.is_empty() => Value::String(lua.create_string(n)?),
                    _ => Value::Nil,
                },
                super::binding_abi::flag(friend.refer_a_friend),
            ]))
        })?,
    )?;

    g.set(
        "GetSelectedFriend",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.social.selected_friend))
        })?,
    )?;

    // SetSelectedFriend(index): set now, since `FriendsList_Update` reads it back in the same
    // call, and queue it for the app.
    g.set(
        "SetSelectedFriend",
        lua.create_function(|lua, index: i64| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let index = clamp_index(index, model.social.friends.len());
            model.social.selected_friend = index;
            model
                .social_requests
                .push(SocialRequest::SelectFriend(index));
            Ok(())
        })?,
    )?;

    g.set(
        "ShowFriends",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.social_requests.push(SocialRequest::RefreshFriends);
            Ok(())
        })?,
    )?;

    // AddFriend(name [, note]) (`0x6b6af0`): no name does nothing; the note is a string argument
    // 2, else empty. No blank test: `0x6b5400` sends what it is given.
    g.set(
        "AddFriend",
        lua.create_function(|lua, args: MultiValue| {
            if let Some((name, note)) = friend_args(lua, &args) {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let request = SocialRequest::AddFriend { name, note };
                model.social_requests.push(request);
            }
            Ok(())
        })?,
    )?;

    // RemoveFriend(indexOrName): the friends frame passes a row, `/removefriend` a name.
    g.set(
        "RemoveFriend",
        lua.create_function(|lua, who: Value| {
            let request = match who {
                Value::Integer(i) if i >= 1 => Some(SocialRequest::RemoveFriendIndex(i as u32)),
                Value::Number(n) if n >= 1.0 => Some(SocialRequest::RemoveFriendIndex(n as u32)),
                Value::String(s) => {
                    let name = s.to_str()?.to_string();
                    (!name.trim().is_empty()).then_some(SocialRequest::RemoveFriendName(name))
                }
                _ => None,
            };
            if let Some(request) = request {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                model.social_requests.push(request);
            }
            Ok(())
        })?,
    )?;

    // ── The ignore list ──
    // GetNumIgnores(): CountIgnores `0x5ae550`, over the 25 slots at `+0x650`.
    g.set(
        "GetNumIgnores",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.social.ignores.len() as i64)
        })?,
    )?;

    // GetIgnoreName(index) (`0x6b4620`): always one string, `UNKNOWN` for an empty slot or a
    // name not yet cached.
    g.set(
        "GetIgnoreName",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: GetIgnoreName(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let name = usize::try_from(i64::from(index) - 1)
                .ok()
                .and_then(|i| model.social.ignores.get(i));
            or_unknown(lua, name.map_or("", String::as_str))
        })?,
    )?;

    // GetSelectedIgnore / SetSelectedIgnore (`0x6b4a00` / `0x6b48a0`), as the friend pair.
    g.set(
        "GetSelectedIgnore",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.social.selected_ignore))
        })?,
    )?;
    g.set(
        "SetSelectedIgnore",
        lua.create_function(|lua, index: i64| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            let index = clamp_index(index, model.social.ignores.len());
            model.social.selected_ignore = index;
            model
                .social_requests
                .push(SocialRequest::SelectIgnore(index));
            Ok(())
        })?,
    )?;

    // AddIgnore / DelIgnore / AddOrDelIgnore(name) (`0x6b6c00`, `0x6b7d70`, `0x6b6ba0`): no name
    // does nothing.
    for (global, make) in [
        ("AddIgnore", SocialRequest::AddIgnore as fn(String) -> _),
        ("DelIgnore", SocialRequest::DelIgnore as fn(String) -> _),
        (
            "AddOrDelIgnore",
            SocialRequest::ToggleIgnore as fn(String) -> _,
        ),
    ] {
        g.set(
            global,
            lua.create_function(move |lua, name: Value| {
                let name = super::binding_abi::optional_string(lua, &name).unwrap_or_default();
                if !name.trim().is_empty() {
                    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                    model.social_requests.push(make(name));
                }
                Ok(())
            })?,
        )?;
    }

    // ── /who ──
    // GetNumWhoResults() → displayed, total, the server's full match count.
    g.set(
        "GetNumWhoResults",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok((
                model.social.who.len() as i64,
                i64::from(model.social.who_total),
            ))
        })?,
    )?;

    g.set(
        "GetWhoInfo",
        lua.create_function(|lua, index: Value| {
            let index = number_arg(lua, index, "Usage: GetWhoInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let row = usize::try_from(i64::from(index) - 1)
                .ok()
                .and_then(|i| model.social.who.get(i));
            // `0x6b4a80`: seven; a race, class or zone with no row is `UNKNOWN`, the class file
            // nil, and a miss answers nil, nil, 0, then nils.
            let Some(row) = row else {
                let mut v = vec![Value::Nil, Value::Nil, Value::Integer(0)];
                v.extend([Value::Nil, Value::Nil, Value::Nil, Value::Nil]);
                return Ok(MultiValue::from_vec(v));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&row.name)?),
                Value::String(lua.create_string(&row.guild)?),
                Value::Integer(i64::from(row.level)),
                or_unknown(lua, &row.race)?,
                or_unknown(lua, &row.class)?,
                or_unknown(lua, &row.zone)?,
                if row.class_file.is_empty() {
                    Value::Nil
                } else {
                    Value::String(lua.create_string(&row.class_file)?)
                },
            ]))
        })?,
    )?;

    g.set(
        "SendWho",
        lua.create_function(|lua, filter: Option<String>| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model
                .social_requests
                .push(SocialRequest::Who(filter.unwrap_or_default()));
            Ok(())
        })?,
    )?;

    // SortWho(sortType) (`0x6b4eb0`): promotes the key in the seven-slot chain, reversing it if
    // it is already in front, sorts the list, and fires `WHO_LIST_UPDATE` synchronously
    // (`0x6b5017`, `0x81b530`), so the list redraws before the click's sound. The app gets the
    // same click for its own chain. Returns nothing; a non-string raises (`0x6b4eba`, `0xa31114`).
    g.set(
        "SortWho",
        lua.create_function(|lua, sort_type: Value| {
            let sort_type = string_arg(lua, sort_type, "Usage: SortWho(\"type\")")?;
            {
                let mut model = lua.app_data_mut::<Model>().expect("model app_data");
                let model = &mut *model;
                model.social.who_sort.promote(&sort_type);
                model.social.who_sort.sort(&mut model.social.who);
                model
                    .social_requests
                    .push(SocialRequest::SortWho(sort_type));
            }
            super::tick::fire_event_into(lua, "WHO_LIST_UPDATE", Vec::new());
            Ok(())
        })?,
    )?;

    // SetWhoToUI(flag): the Who frame passes 1 on show and 0 on hide; nil, false and 0 are off.
    g.set(
        "SetWhoToUI",
        lua.create_function(|lua, flag: Value| {
            let on = match flag {
                Value::Nil | Value::Boolean(false) => false,
                Value::Integer(0) => false,
                Value::Number(n) => n != 0.0,
                _ => true,
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.social_requests.push(SocialRequest::SetWhoToUi(on));
            Ok(())
        })?,
    )?;

    Ok(())
}

/// `AddFriend`'s name and note as `0x6b6af0` reads them: `lua_tostring(1)`, then argument 2 when
/// `lua_isstring`, else `""`.
pub(super) fn friend_args(lua: &Lua, args: &MultiValue) -> Option<(String, String)> {
    let name = super::binding_abi::optional_string(lua, args.front()?)?;
    let note = args
        .get(1)
        .and_then(|v| super::binding_abi::optional_string(lua, v));
    Some((name, note.unwrap_or_default()))
}

/// 12340's `UNKNOWN` (`0x9e4110`) through `0x819d40`, which answers `""` for an unset global.
fn unknown(lua: &Lua) -> mlua::Result<Value> {
    match lua.globals().get::<Value>("UNKNOWN")? {
        v @ Value::String(_) => Ok(v),
        _ => Ok(Value::String(lua.create_string("")?)),
    }
}

/// A resolved name, or [`unknown`] for the empty miss marker.
fn or_unknown(lua: &Lua, s: &str) -> mlua::Result<Value> {
    if s.is_empty() {
        return unknown(lua);
    }
    Ok(Value::String(lua.create_string(s)?))
}

/// The 1-based friend lookup `GetFriendInfo` does, `None` past either end: the list holds what the
/// server sent, so 12340's read past its 100 slots (`0x6b41a3`, `jbe`) has no counterpart.
fn friend_at(friends: &[FriendInfo], index: i64) -> Option<&FriendInfo> {
    usize::try_from(index - 1).ok().and_then(|i| friends.get(i))
}

/// A selection index in `1..=len`, else 0, nothing selected.
fn clamp_index(index: i64, len: usize) -> u32 {
    if index >= 1 && index <= len as i64 {
        index as u32
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::{SocialRequest, SocialState, WhoInfo};
    use crate::script::UiScript;

    fn who(name: &str, level: u32, zone: &str) -> WhoInfo {
        WhoInfo {
            name: name.to_string(),
            guild: String::new(),
            level,
            race: "Human".to_string(),
            class: "Warrior".to_string(),
            zone: zone.to_string(),
            class_file: String::new(),
        }
    }

    /// `GetFriendInfo` (`0x6b4130`): eight values by index (a numeric string too) or name, the
    /// note and the RAF bit; a miss, a pending name's row by `""` and a class or zone with no row
    /// answer 12340's `UNKNOWN` (`0x6b43a7`-`0x6b43e4`).
    #[test]
    fn friend_info_answers_eight_by_index_or_name() {
        let mut s = UiScript::new().unwrap();
        s.run("UNKNOWN = 'Unknown'").unwrap();
        s.set_social(SocialState {
            friends: vec![
                super::FriendInfo {
                    name: "Benchtwo".into(),
                    level: 80,
                    class: "Mage".into(),
                    area: "Dalaran".into(),
                    connected: true,
                    status: String::new(),
                    note: Some("raid healer".into()),
                    refer_a_friend: true,
                },
                super::FriendInfo::default(),
            ],
            ..SocialState::default()
        });
        for call in [
            "GetFriendInfo(1)",
            "GetFriendInfo('benchtwo')",
            "GetFriendInfo('1.9')",
        ] {
            assert_eq!(
                s.eval::<(i64, String, i64)>(&format!(
                    "local n, l, c, a, on, st, note, raf = {call} \
                     return select('#', {call}), note, raf"
                ))
                .unwrap(),
                (8, "raid healer".into(), 1),
                "{call}"
            );
        }
        for call in ["GetFriendInfo(9)", "GetFriendInfo('')", "GetFriendInfo(2)"] {
            assert!(
                s.eval::<bool>(&format!(
                    "local n, l, c, a, on, st, note, raf = {call} \
                     return select('#', {call}) == 8 and l == 0 and c == 'Unknown' \
                        and a == 'Unknown' and on == nil and st == '' and note == nil \
                        and raf == nil and (n == nil or n == '')"
                ))
                .unwrap(),
                "{call}"
            );
        }
        assert!(
            s.eval::<bool>("return GetFriendInfo('') == nil").unwrap(),
            "the empty name matches no pending row (`0x6b3590`)"
        );
        let err = s.run("GetFriendInfo()").unwrap_err().to_string();
        assert!(err.contains("Usage: GetFriendInfo(index or name)"), "{err}");
        assert_eq!(s.arity("GetNumFriends()").unwrap(), 2, "total, online");
    }

    /// `GetIgnoreName` (`0x6b4620`) always answers one string, `UNKNOWN` for an empty slot or a
    /// name not cached; `GetWhoInfo` (`0x6b4a80`) shows `UNKNOWN` for a race, class or zone with no
    /// row (`0x6b4b56`, `0x6b4bae`, `0x6b4bf3`). Both raise their usage line on a non-number.
    #[test]
    fn the_ignore_and_who_getters_answer_unknown() {
        let mut s = UiScript::new().unwrap();
        s.run("UNKNOWN = 'Unknown'").unwrap();
        s.set_social(SocialState {
            ignores: vec!["Spammer".into(), String::new()],
            who: vec![WhoInfo {
                race: String::new(),
                zone: String::new(),
                ..who("Galas", 60, "")
            }],
            who_total: 1,
            ..SocialState::default()
        });
        assert_eq!(
            s.eval::<(String, String, String, String)>(
                "return GetIgnoreName('1'), GetIgnoreName(2), GetIgnoreName(3), GetIgnoreName(0)"
            )
            .unwrap(),
            (
                "Spammer".into(),
                "Unknown".into(),
                "Unknown".into(),
                "Unknown".into()
            )
        );
        assert_eq!(
            s.eval::<(String, String, String)>(
                "local _, _, _, race, class, zone = GetWhoInfo(1) return race, class, zone"
            )
            .unwrap(),
            ("Unknown".into(), "Warrior".into(), "Unknown".into())
        );
        for (call, usage) in [
            ("GetIgnoreName()", "Usage: GetIgnoreName(index)"),
            ("GetWhoInfo()", "Usage: GetWhoInfo(index)"),
        ] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(err.contains(usage), "{err}");
        }
    }

    /// Two `/who` hits and a `WHO_LIST_UPDATE` handler that records the order it sees.
    fn seated() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.run(
            r#"
            fired = 0
            order = ""
            local f = CreateFrame("Frame", "WhoWatcher")
            f:RegisterEvent("WHO_LIST_UPDATE")
            f:SetScript("OnEvent", function()
                fired = fired + 1
                order = ""
                for i = 1, GetNumWhoResults() do
                    order = order .. GetWhoInfo(i) .. ","
                end
            end)
            "#,
        )
        .unwrap();
        s.set_social(SocialState {
            who: vec![
                who("Galas", 60, "Elwynn Forest"),
                who("Erdrin", 12, "Elwynn Forest"),
            ],
            who_total: 2,
            ..Default::default()
        });
        s
    }

    /// A queued event would leave `order` one click stale.
    #[test]
    fn sort_who_sorts_in_place_and_fires_the_event_synchronously() {
        let mut s = seated();
        assert_eq!(s.eval::<i64>("return fired").unwrap(), 0);

        s.run(r#"SortWho("name")"#).unwrap();
        assert_eq!(s.eval::<i64>("return fired").unwrap(), 1);
        assert_eq!(
            s.eval::<String>("return order").unwrap(),
            "Erdrin,Galas,",
            "the handler must already see the sorted list"
        );
        assert_eq!(
            s.take_social_requests(),
            vec![SocialRequest::SortWho("name".into())],
            "and the app hears the same click, so its copy of the chain follows"
        );
        // Returns nothing (`0x6b5020`, `xor eax,eax; ret`).
        assert_eq!(s.arity(r#"SortWho("name")"#).unwrap(), 0);
        assert!(s.errors().is_empty(), "{:?}", s.errors());
    }

    #[test]
    fn a_repeated_header_click_reverses_and_the_direction_is_remembered() {
        let s = seated();
        s.run(r#"SortWho("name")"#).unwrap();
        assert_eq!(s.eval::<String>("return order").unwrap(), "Erdrin,Galas,");

        s.run(r#"SortWho("name")"#).unwrap();
        assert_eq!(
            s.eval::<String>("return order").unwrap(),
            "Galas,Erdrin,",
            "the second click on the same key reverses it"
        );

        // Level ascending puts Erdrin (12) first; name is still descending behind it.
        s.run(r#"SortWho("level")"#).unwrap();
        assert_eq!(s.eval::<String>("return order").unwrap(), "Erdrin,Galas,");

        // Back to name, promoted from slot 1: it keeps its descending direction.
        s.run(r#"SortWho("name")"#).unwrap();
        assert_eq!(
            s.eval::<String>("return order").unwrap(),
            "Galas,Erdrin,",
            "a key promoted from behind keeps the direction it was left in"
        );
        assert_eq!(
            s.eval::<i64>("return fired").unwrap(),
            4,
            "one fire per click"
        );
    }

    /// A number is stringified and falls to the name key; anything else raises 12340's usage line
    /// (`0xa31114`, spelt right: the `Usgae` typo is other `Sort*` verbs').
    #[test]
    fn sort_who_takes_the_reference_argument_abi() {
        let mut s = seated();
        s.run("SortWho(5)").unwrap();
        assert_eq!(
            s.eval::<String>("return order").unwrap(),
            "Erdrin,Galas,",
            "an unrecognised key is the name key"
        );
        let _ = s.take_social_requests();

        for call in ["SortWho({})", "SortWho()"] {
            let err = s.run(call).unwrap_err().to_string();
            assert!(err.contains(r#"Usage: SortWho("type")"#), "{err}");
        }
        assert_eq!(
            s.eval::<i64>("return fired").unwrap(),
            1,
            "a raised call sorts nothing and fires nothing"
        );
    }
}
