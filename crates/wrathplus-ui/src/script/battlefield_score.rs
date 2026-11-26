//! The battleground and arena scoreboard verbs of build 12340's `WorldStateFrame.lua`, over a
//! board the app pushes with every name resolved, as the reference holds a `MSG_PVP_LOG_DATA`
//! board until the last name lands (`0x54d280`, `0x54ca30`).
//!
//! The rebuild (`0x54aa30`) recounts the faction filter and sorts the rows, then fires
//! `UPDATE_BATTLEFIELD_SCORE`. The sort (`0x54a740`) puts the filter's team first, then walks a
//! seventeen-entry most-recently-used key list (`0xbea2c0`, keys 0-16 forward at start,
//! `0x54e720`); `SortBattlefieldScoreData` moves its key to the head or flips the head's direction
//! (`0x54cee0`). Counters sort largest first, the name in byte order, the team ascending.
//!
//! Deviation: the class key compares the class ids and answers "after" on a tie
//! (`0x54a95f`), which no total order can follow; ties here are equal.

use mlua::{Lua, MultiValue, Value};

use super::Model;

/// One scoreboard row as the app resolved it, in wire order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlefieldScoreRow {
    /// The bare name, or `Name-Realm` for a cross-realm player.
    pub name: String,
    pub killing_blows: u32,
    pub honorable_kills: u32,
    pub deaths: u32,
    pub honor_gained: u32,
    /// The arena team byte in an arena, else `0` Horde, `1` Alliance, `-1` neither from the race.
    pub faction: i32,
    /// The race's display name, `None` for an unresolved row.
    pub race: Option<String>,
    pub class: Option<String>,
    /// The class's file token (`"WARRIOR"`).
    pub class_token: Option<String>,
    /// The class id, the class key's sort value.
    pub class_id: u8,
    pub damage: u32,
    pub healing: u32,
    /// The objective counters, eight slots (`+0x24`), unfilled ones zero.
    pub stats: [u32; 8],
}

/// A column header: a `WorldStateUI.dbc` `Type == 2` row for the map, its text not expanded.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlefieldStatColumn {
    pub text: String,
    pub icon: String,
    pub tooltip: String,
}

/// One arena team's block (`0xbea0b0` names, `0xbea594..` ratings): the name and the three wire
/// words, in wire order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlefieldTeamView {
    pub name: String,
    pub ratings: [u32; 3],
}

/// The whole board the app pushes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BattlefieldScores {
    pub rows: Vec<BattlefieldScoreRow>,
    /// The winner byte once the match has ended (`0xbea588`/`0xbea58c`).
    pub winner: Option<u8>,
    /// The arena team blocks; empty names until an arena board arrives.
    pub teams: [BattlefieldTeamView; 2],
    pub columns: Vec<BattlefieldStatColumn>,
}

/// The VM's half of the board: the pushed rows, the filter, the key list and the sorted order.
#[derive(Clone, Debug)]
pub(crate) struct ScoreBoard {
    pub(crate) scores: BattlefieldScores,
    /// `SetBattlefieldScoreFaction`'s store (`0xacd170`): `-1` every team, `0`, `1`.
    pub(crate) filter: i32,
    pub(crate) sort: [(u8, bool); 17],
    pub(crate) order: Vec<usize>,
    /// The filtered count (`0xbea580`).
    pub(crate) filtered: usize,
    /// A newer board waits on names (`0xbea590`): the rebuild and the sort do nothing.
    pub(crate) names_pending: bool,
}

impl Default for ScoreBoard {
    fn default() -> Self {
        Self {
            scores: BattlefieldScores::default(),
            filter: -1,
            sort: std::array::from_fn(|k| (k as u8, false)),
            order: Vec::new(),
            filtered: 0,
            names_pending: false,
        }
    }
}

impl ScoreBoard {
    fn compare(&self, a: &BattlefieldScoreRow, b: &BattlefieldScoreRow) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.filter != -1 && a.faction != b.faction {
            return if a.faction != self.filter {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        let columns = self.scores.columns.len();
        for &(key, reversed) in &self.sort {
            let order = match key {
                0 => b.killing_blows.cmp(&a.killing_blows),
                1 => b.deaths.cmp(&a.deaths),
                2 => b.honor_gained.cmp(&a.honor_gained),
                3 => a.name.as_bytes().cmp(b.name.as_bytes()),
                4 => b.class_id.cmp(&a.class_id),
                5 => b.honorable_kills.cmp(&a.honorable_kills),
                6 => b.damage.cmp(&a.damage),
                7 => b.healing.cmp(&a.healing),
                8 => a.faction.cmp(&b.faction),
                9..=16 if usize::from(key - 9) < columns => {
                    let i = usize::from(key - 9);
                    b.stats[i].cmp(&a.stats[i])
                }
                _ => Ordering::Equal,
            };
            if order != Ordering::Equal {
                return if reversed { order.reverse() } else { order };
            }
        }
        Ordering::Equal
    }

    /// `0x54aa30`: recount the filter and sort.
    fn rebuild(&mut self) {
        let rows = &self.scores.rows;
        self.filtered = if self.filter == -1 {
            rows.len()
        } else {
            rows.iter().filter(|r| r.faction == self.filter).count()
        };
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by(|&a, &b| self.compare(&rows[a], &rows[b]));
        self.order = order;
    }

    /// `0x54cee0`: move `key` to the head with its own direction, each entry above it shifting
    /// down with its own (`0x54cf30`-`0x54cf4c`), or flip the head's direction if it is there.
    fn choose_sort(&mut self, key: u8) {
        if let Some(i) = self.sort.iter().position(|&(k, _)| k == key) {
            choose(&mut self.sort, i);
        }
    }

    fn row(&self, index_1based: i64) -> Option<&BattlefieldScoreRow> {
        let i = usize::try_from(index_1based.checked_sub(1)?).ok()?;
        // Bounded by the wire count (`0xbea57c`), not the filtered one.
        self.order.get(i).map(|&r| &self.scores.rows[r])
    }
}

impl super::UiScript {
    /// Push the resolved board and rebuild it; the filter and the key list survive. The app fires
    /// `UPDATE_BATTLEFIELD_SCORE`, keeping its order against `UPDATE_BATTLEFIELD_STATUS`.
    pub fn set_battlefield_scores(&mut self, scores: BattlefieldScores) {
        let mut model = self.model_mut();
        model.battlefield_board.scores = scores;
        model.battlefield_board.rebuild();
    }

    /// A newer board is held for its names (`0xbea590` non-zero).
    pub fn set_battlefield_score_names_pending(&mut self, pending: bool) {
        self.model_mut().battlefield_board.names_pending = pending;
    }

    /// The status-3 reset of the filter (`0x54ae40` writes `-1`), followed by the app's rebuild.
    pub fn reset_battlefield_score_filter(&mut self) {
        let mut model = self.model_mut();
        model.battlefield_board.filter = -1;
        model.battlefield_board.rebuild();
    }

    /// `GetBattlefieldInstanceRunTime()`'s answer in ms, pushed each frame.
    pub fn set_battlefield_run_time_ms(&mut self, ms: u32) {
        self.model_mut().battlefield_run_time_ms = ms;
    }

    /// `RequestBattlefieldScoreData()` calls since the last drain.
    pub fn take_battlefield_score_requests(&mut self) -> u32 {
        std::mem::take(&mut self.model_mut().battlefield_score_requests)
    }

    /// `LeaveBattlefield()` calls since the last drain, each a `CMSG_LEAVE_BATTLEFIELD`.
    pub fn take_battlefield_leave_requests(&mut self) -> u32 {
        std::mem::take(&mut self.model_mut().battlefield_leave_requests)
    }
}

/// `lua_isnumber` then `fistp` under chop, its low dword (`0x54bee8`).
fn number(lua: &Lua, v: &Value) -> mlua::Result<Option<i64>> {
    Ok(lua
        .coerce_number(v.clone())?
        .map(|n| i64::from(n as i64 as i32)))
}

fn index_arg(lua: &Lua, v: &Value, usage: &'static str) -> mlua::Result<i64> {
    number(lua, v)?.ok_or_else(|| mlua::Error::RuntimeError(usage.into()))
}

/// The key-list move both lists share (`0x54cee0`, `0x5a2e80`): entry `i` to the head with its
/// own direction, the entries above it shifting down with theirs; the head itself flips.
pub(super) fn choose(sort: &mut [(u8, bool)], i: usize) {
    if i == 0 {
        sort[0].1 = !sort[0].1;
    } else {
        sort[..=i].rotate_right(1);
    }
}

fn rebuild_and_fire(model: &mut Model) {
    // `0x54aa39`: nothing while names are pending.
    if model.battlefield_board.names_pending {
        return;
    }
    model.battlefield_board.rebuild();
    model
        .pending_events
        .push(("UPDATE_BATTLEFIELD_SCORE".to_string(), Vec::new()));
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    g.set(
        "GetNumBattlefieldScores",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.battlefield_board.filtered as i64)
        })?,
    )?;

    // name, killingBlows, honorableKills, deaths, honorGained, faction, rank, race, class,
    // classToken, damageDone, healingDone: twelve values on every leg (`0x54be90`). The rank is
    // always 0.
    g.set(
        "GetBattlefieldScore",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetBattlefieldScore(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let n = |v: u32| Value::Integer(i64::from(v));
            let Some(r) = model.battlefield_board.row(index) else {
                let z = || Value::Integer(0);
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    z(),
                    z(),
                    z(),
                    z(),
                    z(),
                    z(),
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                    z(),
                    z(),
                ]));
            };
            let s = |v: &Option<String>| -> mlua::Result<Value> {
                Ok(match v {
                    Some(t) => Value::String(lua.create_string(t)?),
                    None => Value::Nil,
                })
            };
            // Class and token push together, or both nil.
            let (class, token) = match (&r.class, &r.class_token) {
                (Some(_), Some(_)) => (s(&r.class)?, s(&r.class_token)?),
                _ => (Value::Nil, Value::Nil),
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&r.name)?),
                n(r.killing_blows),
                n(r.honorable_kills),
                n(r.deaths),
                n(r.honor_gained),
                Value::Integer(i64::from(r.faction)),
                Value::Integer(0),
                s(&r.race)?,
                class,
                token,
                n(r.damage),
                n(r.healing),
            ]))
        })?,
    )?;

    g.set(
        "GetBattlefieldWinner",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(match model.battlefield_board.scores.winner {
                Some(w) => Value::Integer(i64::from(w)),
                None => Value::Nil,
            })
        })?,
    )?;

    // A non-number stores -1; a number other than -1, 0 or 1 changes nothing (`0x54c120`).
    g.set(
        "SetBattlefieldScoreFaction",
        lua.create_function(|lua, faction: Value| {
            let f = lua
                .coerce_number(faction)?
                .map_or(-1, super::binding_abi::ftol); // `0x54c145`
            if f != -1 && (f as u32) > 1 {
                return Ok(());
            }
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.battlefield_board.filter = f;
            rebuild_and_fire(&mut model);
            Ok(())
        })?,
    )?;

    g.set(
        "GetNumBattlefieldStats",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(model.battlefield_board.scores.columns.len() as i64)
        })?,
    )?;

    // text, icon, tooltip; a column the map lacks answers three nils (`0x54c170`).
    g.set(
        "GetBattlefieldStatInfo",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetBattlefieldStatInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let col = usize::try_from(index)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| model.battlefield_board.scores.columns.get(i));
            let Some(c) = col else {
                return Ok(MultiValue::from_vec(vec![
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ]));
            };
            Ok(MultiValue::from_vec(vec![
                Value::String(lua.create_string(&c.text)?),
                Value::String(lua.create_string(&c.icon)?),
                Value::String(lua.create_string(&c.tooltip)?),
            ]))
        })?,
    )?;

    // A bad row or a stat outside 1..=8 answers 0. Deviation: the reference (`0x549f60`) admits 9
    // and reads the dword past the eight-slot block; that read answers 0 here.
    g.set(
        "GetBattlefieldStatData",
        lua.create_function(|lua, (player, stat): (Value, Value)| {
            let usage = "Usage: GetBattlefieldStatData(playerIndex, statIndex)";
            let player = index_arg(lua, &player, usage)?;
            let stat = index_arg(lua, &stat, usage)?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let v = model
                .battlefield_board
                .row(player)
                .zip(usize::try_from(stat).ok().and_then(|s| s.checked_sub(1)))
                .and_then(|(r, s)| r.stats.get(s).copied())
                .unwrap_or(0);
            Ok(i64::from(v))
        })?,
    )?;

    // The app sends an empty `MSG_PVP_LOG_DATA` under the client's gates (`0x54ce30`).
    g.set(
        "RequestBattlefieldScoreData",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.battlefield_score_requests += 1;
            Ok(())
        })?,
    )?;

    // `CMSG_LEAVE_BATTLEFIELD` with the active slot's queue id, ungated (`0x54c250`).
    g.set(
        "LeaveBattlefield",
        lua.create_function(|lua, ()| {
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            model.battlefield_leave_requests += 1;
            Ok(())
        })?,
    )?;

    // Ms since the status-3 stamp, 0 with none (`0x549cd0`).
    g.set(
        "GetBattlefieldInstanceRunTime",
        lua.create_function(|lua, ()| {
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            Ok(i64::from(model.battlefield_run_time_ms))
        })?,
    )?;

    Ok(())
}

/// The names build 12340 adds past 1.12, installed in the Wrath block.
pub(super) fn install_wrath(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // GetBattlefieldTeamInfo(index), 0-based: name and the three rating words; nil and zeros past
    // 1 (`0x54a180`).
    g.set(
        "GetBattlefieldTeamInfo",
        lua.create_function(|lua, index: Value| {
            let index = index_arg(lua, &index, "Usage: GetBattlefieldTeamInfo(index)")?;
            let model = lua.app_data_ref::<Model>().expect("model app_data");
            let team = usize::try_from(index)
                .ok()
                .and_then(|i| model.battlefield_board.scores.teams.get(i));
            let mut out = vec![match team {
                Some(t) => Value::String(lua.create_string(&t.name)?),
                None => Value::Nil,
            }];
            out.extend((0..3).map(|k| Value::Integer(team.map_or(0, |t| i64::from(t.ratings[k])))));
            Ok(MultiValue::from_vec(out))
        })?,
    )?;

    // "kills", "deaths", "cp", "name", "class", "hk", "damage", "healing", "team" or "stat<n>"
    // (key 8 + n, refused past 16); anything else sorts by kills (`0x54de00`).
    g.set(
        "SortBattlefieldScoreData",
        lua.create_function(|lua, kind: Value| {
            let kind = match kind {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Integer(i) => i.to_string(),
                Value::Number(n) => n.to_string(),
                _ => {
                    return Err(mlua::Error::runtime(
                        "Usgae: SortBattlefieldScoreData(\"type\")",
                    ))
                }
            };
            let lower = kind.to_ascii_lowercase();
            let named = [
                "kills", "deaths", "cp", "name", "class", "hk", "damage", "healing", "team",
            ]
            .iter()
            .position(|&k| k == lower);
            let key = match named {
                Some(k) => k as u32,
                None if lower.starts_with("stat") => {
                    let n: i64 = lower[4..]
                        .trim()
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '-')
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                    match u32::try_from(n + 8).ok().filter(|&k| k <= 16) {
                        Some(k) => k,
                        None => return Ok(()),
                    }
                }
                None => 0,
            };
            let mut model = lua.app_data_mut::<Model>().expect("model app_data");
            // `0x54cee3`: no key moves while names are pending.
            if !model.battlefield_board.names_pending {
                model.battlefield_board.choose_sort(key as u8);
            }
            rebuild_and_fire(&mut model);
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    fn row(name: &str, faction: i32, kb: u32, deaths: u32, honor: u32) -> BattlefieldScoreRow {
        BattlefieldScoreRow {
            name: name.into(),
            killing_blows: kb,
            honorable_kills: kb + 1,
            deaths,
            honor_gained: honor,
            faction,
            race: Some("Orc".into()),
            class: Some("Warrior".into()),
            class_token: Some("WARRIOR".into()),
            class_id: 1,
            damage: 1000 * kb,
            healing: 10,
            stats: [kb, 2, 0, 0, 0, 0, 0, 0],
        }
    }

    fn board() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_battlefield_scores(BattlefieldScores {
            rows: vec![
                row("Bob", 0, 3, 1, 50),
                row("Alice", 1, 5, 2, 10),
                row("Carl", 1, 4, 1, 10),
                row("Dana", 0, 6, 1, 20),
            ],
            winner: None,
            teams: [
                BattlefieldTeamView {
                    name: "Gold".into(),
                    ratings: [1500, 1512, 1490],
                },
                Default::default(),
            ],
            columns: vec![BattlefieldStatColumn {
                text: "Flags Captured".into(),
                icon: "icon".into(),
                tooltip: "Flags captured".into(),
            }],
        });
        s
    }

    fn names(s: &UiScript) -> String {
        s.eval::<String>(
            "local t = {} for i = 1, 4 do t[i] = (GetBattlefieldScore(i)) end \
             return table.concat(t, ',')",
        )
        .unwrap()
    }

    #[test]
    fn rows_answer_twelve_values_in_kills_order() {
        let s = board();
        assert_eq!(names(&s), "Dana,Alice,Carl,Bob");
        assert_eq!(s.arity("GetBattlefieldScore(1)").unwrap(), 12);
        assert_eq!(s.arity("GetBattlefieldScore(9)").unwrap(), 12);
        assert!(s
            .eval::<bool>("return GetBattlefieldScore(1.9) == 'Dana'")
            .unwrap());
        let got = s
            .eval::<String>(
                "local n, kb, hk, d, h, f, rank, race, class, token, dmg, heal = \
                 GetBattlefieldScore(1) return table.concat({n, kb, hk, d, h, f, rank, race, \
                 class, token, dmg, heal}, '|')",
            )
            .unwrap();
        assert_eq!(got, "Dana|6|7|1|20|0|0|Orc|Warrior|WARRIOR|6000|10");
        assert!(s
            .eval::<bool>("return GetBattlefieldWinner() == nil")
            .unwrap());
        assert_eq!(
            s.eval::<String>("local n, a, b, c = GetBattlefieldTeamInfo(0) return n..a..b..c")
                .unwrap(),
            "Gold150015121490"
        );
        assert_eq!(
            s.eval::<String>("local n, a = GetBattlefieldTeamInfo(1) return '['..n..']'..a")
                .unwrap(),
            "[]0"
        );
        assert!(s
            .eval::<bool>("return GetBattlefieldTeamInfo(2) == nil")
            .unwrap());
        assert_eq!(
            s.eval::<i64>("return GetBattlefieldStatData(1, 1)")
                .unwrap(),
            6
        );
        assert_eq!(
            s.eval::<i64>("return GetBattlefieldStatData(1, 9)")
                .unwrap(),
            0
        );
    }

    #[test]
    fn the_key_list_and_the_filter_reorder_and_fire() {
        let mut s = board();
        s.run("SortBattlefieldScoreData('name')").unwrap();
        assert_eq!(names(&s), "Alice,Bob,Carl,Dana");
        s.run("SortBattlefieldScoreData('NAME')").unwrap();
        assert_eq!(names(&s), "Dana,Carl,Bob,Alice");
        s.run("SortBattlefieldScoreData('stat1')").unwrap();
        assert_eq!(names(&s), "Dana,Alice,Carl,Bob");
        // The old head (name, reversed) moves down with its own direction (`0x54cf30`).
        assert_eq!(
            s.model_ref().battlefield_board.sort[..2],
            [(9, false), (3, true)]
        );
        // `0xbea590`: while a board waits on names, no key moves and nothing rebuilds or fires.
        s.set_battlefield_score_names_pending(true);
        s.run("SortBattlefieldScoreData('kills') SetBattlefieldScoreFaction(0)")
            .unwrap();
        assert_eq!(s.model_ref().battlefield_board.sort[0], (9, false));
        s.set_battlefield_score_names_pending(false);
        // stat9 is key 17, past the list: refused, no event.
        s.run("SortBattlefieldScoreData('stat9')").unwrap();
        s.run("SetBattlefieldScoreFaction(1)").unwrap();
        assert_eq!(
            s.eval::<i64>("return GetNumBattlefieldScores()").unwrap(),
            2
        );
        assert_eq!(names(&s), "Alice,Carl,Dana,Bob");
        s.run("SetBattlefieldScoreFaction(5) SetBattlefieldScoreFaction()")
            .unwrap();
        assert_eq!(
            s.eval::<i64>("return GetNumBattlefieldScores()").unwrap(),
            4
        );
        let events = s
            .model_mut()
            .pending_events
            .drain(..)
            .filter(|(e, _)| e == "UPDATE_BATTLEFIELD_SCORE")
            .count();
        assert_eq!(events, 5);
        s.run("RequestBattlefieldScoreData() LeaveBattlefield()")
            .unwrap();
        assert_eq!(s.take_battlefield_score_requests(), 1);
        assert_eq!(s.take_battlefield_leave_requests(), 1);
    }
}
