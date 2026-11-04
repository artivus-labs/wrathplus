//! Wrath arena teams for `PVPFrame.lua`, from build 12340's bindings over the client's arena-team
//! table (`0xc0f840`, three `0x38`-byte slots): `GetArenaTeam` (`0x5a3860`), `IsArenaTeamCaptain`
//! (`0x60dc70`), `ArenaTeamRoster` (`0x5a3cf0`), `CloseArenaTeamRoster` (`0x8e5250`),
//! `GetNumArenaTeamMembers` (`0x5a2930`), `GetArenaTeamRosterInfo` (`0x5a2fc0`), the roster
//! selection pair (`0x5a3410`/`0x5a3370`), `SortArenaTeamRoster` (`0x5a3490`) and
//! `SetArenaTeamRosterShowOffline` (`0x5a35e0`).
//!
//! The app's reducer (`ui_arena_team`) publishes the teams and each received roster; the sort,
//! the offline filter and the selection are the client's own and live here. A roster is sorted
//! as it lands and again when the order or the filter changes (`0x5a2bd0`): offline members last
//! while offline members are hidden, then the sort list's seven keys in turn, each descending
//! unless reversed. The sort list is most recently used first (`0x5a2e80`); choosing the head
//! again flips its direction.
//!
//! Unresolved, labelled: the name key's compare (`0x76e760`) is taken as a byte compare and the
//! class key's (`0x76ec80`) and the sort-type match's (`0x76e780`) as ASCII case-insensitive.

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript};

/// One roster row as `0x5a3e10` stores it (`0x68` bytes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArenaMemberView {
    pub guid: u64,
    /// The name as read into the row's 48-byte buffer.
    pub name: String,
    /// The roster's captain word: 0 for the captain.
    pub rank: u32,
    pub level: u8,
    /// The class's display name, `None` for a class the table does not know.
    pub class: Option<String>,
    pub online: bool,
    pub week_games: u32,
    pub week_wins: u32,
    pub season_games: u32,
    pub season_wins: u32,
    pub rating: u32,
}

/// `SMSG_ARENA_TEAM_QUERY_RESPONSE`'s cached record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArenaTeamInfoView {
    pub name: String,
    /// The team type: 2, 3 or 5.
    pub size: u32,
    /// Background colour, emblem style, emblem colour, border style, border colour; colours ARGB.
    pub emblem: [u32; 5],
}

/// `SMSG_ARENA_TEAM_STATS`, as `0x5a2d50` stores it in the slot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ArenaTeamStatsView {
    pub rating: u32,
    pub week_games: u32,
    pub week_wins: u32,
    pub season_games: u32,
    pub season_wins: u32,
    pub rank: u32,
}

/// One of the three team slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArenaTeamView {
    /// The player's `PLAYER_FIELD_ARENA_TEAM_INFO` slot: id, type, member (0 captain), week
    /// games, season games, season wins, personal rating.
    pub descriptor: [u32; 7],
    pub stats: ArenaTeamStatsView,
    /// The team's query record, `None` until it arrives.
    pub info: Option<ArenaTeamInfoView>,
}

/// One inspected player's arena team (`MSG_INSPECT_ARENA_TEAMS` `0x5e7b00` and the team's
/// query record), as `GetInspectArenaTeamData` (`0x5e8030`) reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectArenaTeamView {
    pub info: ArenaTeamInfoView,
    pub rating: u32,
    pub season_games: u32,
    pub season_wins: u32,
    /// The inspected player's games and personal rating on the team.
    pub player_games: u32,
    pub player_rating: u32,
}

/// The arena-team state the bindings read.
#[derive(Clone, Debug)]
pub(crate) struct ArenaState {
    teams: [ArenaTeamView; 3],
    rosters: [Vec<ArenaMemberView>; 3],
    /// The selected member's guid, 0 for none (`0xc0f838`); one selection across teams.
    selection: u64,
    /// `0xacf834`, initially 1.
    show_offline: bool,
    /// `0xc0f7fc`: seven (key, reversed) pairs, keys 0-6 forward at start (`0x5a40e0`).
    sort: [(u8, bool); 7],
    /// `ArenaTeamRoster` slots, 0-based, for the app's throttled send.
    roster_requests: Vec<u8>,
    /// The inspected player's three teams, `None` until a team's stats and record are both held.
    inspect: [Option<InspectArenaTeamView>; 3],
}

impl Default for ArenaState {
    fn default() -> Self {
        Self {
            teams: Default::default(),
            rosters: Default::default(),
            selection: 0,
            show_offline: true,
            sort: std::array::from_fn(|k| (k as u8, false)),
            roster_requests: Vec::new(),
            inspect: Default::default(),
        }
    }
}

impl ArenaState {
    fn compare(&self, a: &ArenaMemberView, b: &ArenaMemberView) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if !self.show_offline && a.online != b.online {
            return if a.online {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        for &(key, reversed) in &self.sort {
            let order = match key {
                0 => a.name.as_bytes().cmp(b.name.as_bytes()),
                1 => match (&a.class, &b.class) {
                    (Some(x), Some(y)) => x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()),
                    _ => Ordering::Equal,
                },
                // The counters sort largest first.
                2 => b.week_games.cmp(&a.week_games),
                3 => b.week_wins.cmp(&a.week_wins),
                4 => b.season_games.cmp(&a.season_games),
                5 => b.season_wins.cmp(&a.season_wins),
                6 => b.rating.cmp(&a.rating),
                _ => Ordering::Equal,
            };
            if order != Ordering::Equal {
                return if reversed { order.reverse() } else { order };
            }
        }
        Ordering::Equal
    }

    fn sort_roster(&mut self, slot: usize) {
        let mut roster = std::mem::take(&mut self.rosters[slot]);
        roster.sort_by(|a, b| self.compare(a, b));
        self.rosters[slot] = roster;
    }

    /// Re-sort every non-empty roster; one `ARENA_TEAM_ROSTER_UPDATE` each (`0x5a2e80`).
    fn resort(&mut self, events: &mut Vec<(String, Vec<super::ScriptValue>)>) {
        for slot in 0..3 {
            if !self.rosters[slot].is_empty() {
                self.sort_roster(slot);
                events.push(("ARENA_TEAM_ROSTER_UPDATE".into(), Vec::new()));
            }
        }
    }

    /// `0x5a2e80`: move `key` to the head, or flip the head's direction if it is there.
    fn choose_sort(&mut self, key: u8) {
        let Some(i) = self.sort.iter().position(|&(k, _)| k == key) else {
            return;
        };
        if i == 0 {
            self.sort[0].1 = !self.sort[0].1;
            return;
        }
        // As the reference shifts: the head's direction is overwritten with the chosen entry's
        // before the shift copies it down, so the old head moves down carrying that direction.
        self.sort[0].1 = self.sort[i].1;
        for j in (1..=i).rev() {
            self.sort[j] = self.sort[j - 1];
        }
        self.sort[0].0 = key;
    }
}

impl UiScript {
    /// The three team slots, as the app's reducer holds them.
    pub fn set_wrath_arena_teams(&mut self, teams: [ArenaTeamView; 3]) {
        self.model_mut().wrath_arena.teams = teams;
    }

    /// A received roster for the 0-based `slot`, sorted as it lands (`0x5a3e10`). The caller
    /// fires `ARENA_TEAM_ROSTER_UPDATE`.
    pub fn set_wrath_arena_roster(&mut self, slot: usize, members: Vec<ArenaMemberView>) {
        if slot >= 3 {
            return;
        }
        let arena = &mut self.model_mut().wrath_arena;
        arena.rosters[slot] = members;
        arena.sort_roster(slot);
    }

    /// The inspected player's arena teams, by slot.
    pub fn set_inspect_arena_teams(&mut self, teams: [Option<InspectArenaTeamView>; 3]) {
        self.model_mut().wrath_arena.inspect = teams;
    }

    /// The frame's `ArenaTeamRoster` slots, 0-based, in call order.
    pub fn take_wrath_arena_roster_requests(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.model_mut().wrath_arena.roster_requests)
    }
}

/// `lua_isnumber` (`0x84df20`) then the binding's rounding, as a 0-based index; `None` for a
/// non-number.
fn index(v: &Value) -> Option<i64> {
    let n = match v {
        Value::Integer(i) => *i as f64,
        Value::Number(f) => *f,
        Value::String(s) => s.to_str().ok()?.trim().parse().ok()?,
        _ => return None,
    };
    Some(n.round() as i64 - 1)
}

/// A 0-based team slot under 3.
fn slot(i: i64) -> Option<usize> {
    usize::try_from(i).ok().filter(|&s| s < 3)
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

/// `0x815500`'s boolean read: an absent argument takes `default`, else Lua truth.
fn flag(args: &MultiValue, i: usize, default: bool) -> bool {
    match args.get(i) {
        None => default,
        Some(v) => !matches!(v, Value::Nil | Value::Boolean(false)),
    }
}

fn int(n: u32) -> Value {
    Value::Integer(i64::from(n))
}

/// An ARGB colour's red, green and blue over 255 (`0xa45564`).
fn rgb(colour: u32) -> [Value; 3] {
    [16, 8, 0].map(|shift| Value::Number(f64::from(((colour >> shift) & 0xff) as f32 / 255.0)))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let set = |name: &str, f: fn(&Lua, MultiValue) -> mlua::Result<MultiValue>| {
        g.set(name, lua.create_function(f)?)
    };
    // IsInArenaTeam() (`0x60dbf0`): a boolean, any of the three descriptor slots holds a team id.
    set("IsInArenaTeam", |lua, _| {
        let model = lua.app_data_ref::<Model>().expect("model");
        let any = model.wrath_arena.teams.iter().any(|t| t.descriptor[0] != 0);
        Ok(MultiValue::from_vec(vec![Value::Boolean(any)]))
    })?;
    // teamName, teamSize, teamRating, weekPlayed, weekWins, seasonPlayed, seasonWins,
    // playerPlayed, seasonPlayerPlayed, teamRank, playerRating, then the background colour, the
    // emblem and its colour, the border and its colour: 22 values, `nil` and zeros for an empty
    // slot or a team whose query record has not arrived.
    set("GetArenaTeam", |lua, args| {
        let Some(i) = index(&arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: GetArenaTeam(team)"));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let team = slot(i).map(|s| &model.wrath_arena.teams[s]);
        let Some((team, info)) = team
            .filter(|t| t.descriptor[0] != 0)
            .and_then(|t| Some((t, t.info.as_ref()?)))
        else {
            let mut miss = vec![Value::Nil];
            miss.resize(22, Value::Integer(0));
            return Ok(MultiValue::from_vec(miss));
        };
        let name = Value::String(lua.create_string(&info.name)?);
        let s = &team.stats;
        let d = &team.descriptor;
        let mut out = vec![
            name,
            int(info.size),
            int(s.rating),
            int(s.week_games),
            int(s.week_wins),
            int(s.season_games),
            int(s.season_wins),
            int(d[3]),
            int(d[4]),
            int(s.rank),
            int(d[6]),
        ];
        out.extend(rgb(info.emblem[0]));
        out.push(int(info.emblem[1]));
        out.extend(rgb(info.emblem[2]));
        out.push(int(info.emblem[3]));
        out.extend(rgb(info.emblem[4]));
        Ok(MultiValue::from_vec(out))
    })?;
    // 1 while the slot's descriptor member word is 0, which an empty slot's is too.
    // name, size, rating, played, wins, playerPlayed, playerRating and the emblem's 11 values:
    // 18, or nothing off slots 1-3 and for a team not yet held (`0x5e8030`).
    set("GetInspectArenaTeamData", |lua, args| {
        let Some(i) = index(&arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetInspectArenaTeamData(index)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let Some(t) = slot(i).and_then(|s| model.wrath_arena.inspect[s].as_ref()) else {
            return Ok(MultiValue::new());
        };
        let mut out = vec![
            Value::String(lua.create_string(&t.info.name)?),
            int(t.info.size),
            int(t.rating),
            int(t.season_games),
            int(t.season_wins),
            int(t.player_games),
            int(t.player_rating),
        ];
        out.extend(rgb(t.info.emblem[0]));
        out.push(int(t.info.emblem[1]));
        out.extend(rgb(t.info.emblem[2]));
        out.push(int(t.info.emblem[3]));
        out.extend(rgb(t.info.emblem[4]));
        Ok(MultiValue::from_vec(out))
    })?;
    set("IsArenaTeamCaptain", |lua, args| {
        let Some(i) = index(&arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: IsArenaTeamCaptain(index)"));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let captain = slot(i).is_some_and(|s| model.wrath_arena.teams[s].descriptor[2] == 0);
        Ok(MultiValue::from_vec(vec![if captain {
            Value::Integer(1)
        } else {
            Value::Nil
        }]))
    })?;
    // CMSG_ARENA_TEAM_ROSTER for the slot's team, at most every 10 s a slot (the app's send).
    set("ArenaTeamRoster", |lua, args| {
        let Some(i) = index(&arg(&args, 0)) else {
            return Err(mlua::Error::runtime("Usage: ArenaTeamRoster(team)"));
        };
        if let Some(s) = slot(i) {
            lua.app_data_mut::<Model>()
                .expect("model")
                .wrath_arena
                .roster_requests
                .push(s as u8);
        }
        Ok(MultiValue::new())
    })?;
    set("CloseArenaTeamRoster", |_, _| Ok(MultiValue::new()))?;
    // The online count unless offline members show or the caller asks for them, else all.
    set("GetNumArenaTeamMembers", |lua, args| {
        let Some(i) = index(&arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetNumArenaTeamMembers(team [,showOffline])",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let arena = &model.wrath_arena;
        let n = slot(i).map_or(0, |s| {
            let roster = &arena.rosters[s];
            if arena.show_offline || flag(&args, 1, false) {
                roster.len()
            } else {
                roster.iter().filter(|m| m.online).count()
            }
        });
        Ok(MultiValue::from_vec(vec![Value::Integer(n as i64)]))
    })?;
    // name, rank, level, class, online, played, win, seasonPlayed, seasonWin, rating; an
    // index off the roster is nil, 0, 0, nil, nil, 0, 0, 0, 0, 0.
    set("GetArenaTeamRosterInfo", |lua, args| {
        let (Some(team), Some(row)) = (index(&arg(&args, 0)), index(&arg(&args, 1))) else {
            return Err(mlua::Error::runtime(
                "Usage: GetArenaTeamRosterInfo(team, index)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let member = slot(team)
            .zip(usize::try_from(row).ok())
            .and_then(|(s, r)| model.wrath_arena.rosters[s].get(r));
        let Some(m) = member else {
            let z = || Value::Integer(0);
            return Ok(MultiValue::from_vec(vec![
                Value::Nil,
                z(),
                z(),
                Value::Nil,
                Value::Nil,
                z(),
                z(),
                z(),
                z(),
                z(),
            ]));
        };
        let class = match &m.class {
            Some(c) => Value::String(lua.create_string(c)?),
            None => Value::Nil,
        };
        Ok(MultiValue::from_vec(vec![
            Value::String(lua.create_string(&m.name)?),
            int(m.rank),
            Value::Integer(i64::from(m.level)),
            class,
            if m.online {
                Value::Integer(1)
            } else {
                Value::Nil
            },
            int(m.week_games),
            int(m.week_wins),
            int(m.season_games),
            int(m.season_wins),
            int(m.rating),
        ]))
    })?;
    // The selected member's 1-based row in the team's roster, 0 when it is not there.
    set("GetArenaTeamRosterSelection", |lua, args| {
        let Some(i) = index(&arg(&args, 0)) else {
            return Err(mlua::Error::runtime(
                "Usage: GetArenaTeamRosterSelection(team)",
            ));
        };
        let model = lua.app_data_ref::<Model>().expect("model");
        let arena = &model.wrath_arena;
        let row = slot(i)
            .filter(|_| arena.selection != 0)
            .and_then(|s| {
                arena.rosters[s]
                    .iter()
                    .position(|m| m.guid == arena.selection)
            })
            .map_or(0, |r| r as i64 + 1);
        Ok(MultiValue::from_vec(vec![Value::Integer(row)]))
    })?;
    // Select the row's member; an index off the roster clears the selection.
    set("SetArenaTeamRosterSelection", |lua, args| {
        let (Some(team), Some(row)) = (index(&arg(&args, 0)), index(&arg(&args, 1))) else {
            return Err(mlua::Error::runtime(
                "Usage: SetArenaTeamRosterSelection(team, index)",
            ));
        };
        let mut model = lua.app_data_mut::<Model>().expect("model");
        let arena = &mut model.wrath_arena;
        arena.selection = slot(team)
            .zip(usize::try_from(row).ok())
            .and_then(|(s, r)| arena.rosters[s].get(r))
            .map_or(0, |m| m.guid);
        Ok(MultiValue::new())
    })?;
    // "name", "class", "played", "won", "seasonplayed", "seasonwon" or "rating"; anything else
    // sorts by name.
    set("SortArenaTeamRoster", |lua, args| {
        let key = match arg(&args, 0) {
            Value::String(s) => s.to_str()?.to_ascii_lowercase(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            _ => return Err(mlua::Error::runtime("Usgae: SortArenaTeamRoster(\"type\")")),
        };
        let key = [
            "name",
            "class",
            "played",
            "won",
            "seasonplayed",
            "seasonwon",
            "rating",
        ]
        .iter()
        .position(|&k| k == key)
        .unwrap_or(0) as u8;
        let mut model = lua.app_data_mut::<Model>().expect("model");
        let model = &mut *model;
        model.wrath_arena.choose_sort(key);
        model.wrath_arena.resort(&mut model.pending_events);
        Ok(MultiValue::new())
    })?;
    // A change re-sorts every roster (`0x5a2e10`).
    set("SetArenaTeamRosterShowOffline", |lua, args| {
        let show = flag(&args, 0, true);
        let mut model = lua.app_data_mut::<Model>().expect("model");
        let model = &mut *model;
        if model.wrath_arena.show_offline != show {
            model.wrath_arena.show_offline = show;
            model.wrath_arena.resort(&mut model.pending_events);
        }
        Ok(MultiValue::new())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(guid: u64, name: &str, online: bool, week_games: u32, rating: u32) -> ArenaMemberView {
        ArenaMemberView {
            guid,
            name: name.into(),
            rank: 1,
            level: 80,
            class: Some("Mage".into()),
            online,
            week_games,
            week_wins: 1,
            season_games: 9,
            season_wins: 4,
            rating,
        }
    }

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        let mut team = ArenaTeamView {
            descriptor: [77, 2, 0, 3, 8, 5, 1500],
            stats: ArenaTeamStatsView {
                rating: 1600,
                week_games: 4,
                week_wins: 3,
                season_games: 20,
                season_wins: 11,
                rank: 42,
            },
            info: None,
        };
        team.info = Some(ArenaTeamInfoView {
            name: "Two".into(),
            size: 2,
            emblem: [0xff_ff_00_00, 7, 0xff_00_ff_00, 3, 0xff_00_00_ff],
        });
        s.set_wrath_arena_teams([team, Default::default(), Default::default()]);
        s.set_wrath_arena_roster(
            0,
            vec![
                m(3, "Cyd", false, 9, 1400),
                m(1, "Abe", true, 2, 1700),
                m(2, "Bo", true, 5, 1500),
            ],
        );
        s
    }

    #[test]
    fn in_arena_team_reads_the_descriptor_slots() {
        let mut s = UiScript::new().unwrap();
        assert!(!s.eval::<bool>("return IsInArenaTeam()").unwrap());
        let mut team = ArenaTeamView::default();
        team.descriptor[0] = 9;
        s.set_wrath_arena_teams([Default::default(), Default::default(), team]);
        assert!(s.eval::<bool>("return IsInArenaTeam()").unwrap());
    }

    #[test]
    fn team_and_roster_read_in_native_shapes() {
        let s = vm();
        s.run(
            r##"
            local n, size, rating, wp, ww, sp, sw, pp, psp, rank, pr, r, g, b, e, er, eg, eb, bo, br, bg, bb = GetArenaTeam(1)
            assert(n == "Two" and size == 2 and rating == 1600 and wp == 4 and ww == 3)
            assert(sp == 20 and sw == 11 and pp == 3 and psp == 8 and rank == 42 and pr == 1500)
            assert(r == 1 and g == 0 and b == 0 and e == 7 and eg == 1 and bo == 3 and bb == 1)
            assert(select("#", GetArenaTeam(1)) == 22 and select("#", GetArenaTeam(2)) == 22)
            assert(GetArenaTeam(2) == nil and select(22, GetArenaTeam(4)) == 0)
            assert(IsArenaTeamCaptain(1) == 1 and IsArenaTeamCaptain(2) == 1 and IsArenaTeamCaptain(4) == nil)
            assert(GetNumArenaTeamMembers(1) == 3 and GetNumArenaTeamMembers(2, 1) == 0)
            -- Sorted by name on arrival.
            local name, rank, level, class, online, p, w, spl, sw2, rt = GetArenaTeamRosterInfo(1, 1)
            assert(name == "Abe" and rank == 1 and level == 80 and class == "Mage" and online == 1)
            assert(p == 2 and w == 1 and spl == 9 and sw2 == 4 and rt == 1700)
            assert(select(5, GetArenaTeamRosterInfo(1, 3)) == nil)
            assert(select("#", GetArenaTeamRosterInfo(1, 9)) == 10 and GetArenaTeamRosterInfo(1, 9) == nil)
            assert(not pcall(GetArenaTeamRosterInfo, 1) and not pcall(GetArenaTeam))
            "##,
        )
        .unwrap();
    }

    #[test]
    fn inspected_teams_answer_eighteen_or_nothing() {
        let mut s = vm();
        assert_eq!(s.arity("GetInspectArenaTeamData(1)").unwrap(), 0);
        let mut teams: [Option<InspectArenaTeamView>; 3] = Default::default();
        teams[1] = Some(InspectArenaTeamView {
            info: ArenaTeamInfoView {
                name: "Trio".into(),
                size: 3,
                emblem: [0xff_00_00_ff, 4, 0, 2, 0],
            },
            rating: 1800,
            season_games: 40,
            season_wins: 22,
            player_games: 30,
            player_rating: 1750,
        });
        s.set_inspect_arena_teams(teams);
        assert_eq!(s.arity("GetInspectArenaTeamData(2)").unwrap(), 18);
        assert_eq!(s.arity("GetInspectArenaTeamData(4)").unwrap(), 0);
        let got = s
            .eval::<String>(
                "local n, z, r, p, w, pp, pr, br, bg, bb, e = GetInspectArenaTeamData(2) \
                 return table.concat({n, z, r, p, w, pp, pr, bb, e}, '|')",
            )
            .unwrap();
        assert_eq!(got, "Trio|3|1800|40|22|30|1750|1|4");
        assert!(s.run("GetInspectArenaTeamData()").is_err());
    }

    #[test]
    fn sort_list_filter_and_selection_follow_the_client() {
        let mut s = vm();
        let first = |s: &UiScript| {
            s.eval::<String>("return (GetArenaTeamRosterInfo(1, 1))")
                .unwrap()
        };
        s.run("SortArenaTeamRoster('rating')").unwrap();
        assert_eq!(first(&s), "Abe");
        s.run("SortArenaTeamRoster('RATING')").unwrap();
        assert_eq!(first(&s), "Cyd");
        s.run("SortArenaTeamRoster('played')").unwrap();
        assert_eq!(first(&s), "Cyd");
        // The old head (rating, reversed) moved down carrying played's forward direction.
        assert_eq!(
            s.model_mut().wrath_arena.sort[..4],
            [(2, false), (6, false), (0, false), (1, false)]
        );
        s.run("SetArenaTeamRosterShowOffline(false)").unwrap();
        assert_eq!(first(&s), "Bo");
        assert_eq!(
            s.eval::<i64>("return GetNumArenaTeamMembers(1)").unwrap(),
            2
        );
        assert_eq!(
            s.eval::<i64>("return GetNumArenaTeamMembers(1, 1)")
                .unwrap(),
            3
        );
        let events: Vec<_> = s
            .model_mut()
            .pending_events
            .drain(..)
            .map(|(e, _)| e)
            .filter(|e| e.starts_with("ARENA_"))
            .collect();
        assert_eq!(events, vec!["ARENA_TEAM_ROSTER_UPDATE"; 4]);
        s.run("SetArenaTeamRosterSelection(1, 2)").unwrap();
        assert_eq!(s.model_mut().wrath_arena.selection, 1);
        s.run("SortArenaTeamRoster('name')").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetArenaTeamRosterSelection(1)")
                .unwrap(),
            1
        );
        assert_eq!(
            s.eval::<i64>("return GetArenaTeamRosterSelection(2)")
                .unwrap(),
            0
        );
        s.run("SetArenaTeamRosterSelection(1, 9)").unwrap();
        assert_eq!(s.model_mut().wrath_arena.selection, 0);
        s.run("ArenaTeamRoster(2) ArenaTeamRoster(7) CloseArenaTeamRoster()")
            .unwrap();
        assert_eq!(s.take_wrath_arena_roster_requests(), [1]);
    }
}
