//! Build-12340 raid browser (LFRFrame): the SearchLFG* getters over App's
//! [`WrathRaidBrowserView`], the native sort and the join/leave verbs. One Wow.exe round:
//! `/tmp/azb-ui-lfr-*.log`.
//!
//! The view is stored as published and sorted locally, as native sorts its lists on every update
//! (`0x55b700`); App fires UPDATE_LFG_LIST (event `0x1d5`) after publishing. The verbs fire it
//! themselves, as native does from `SearchLFGJoin`/`Leave`/`Sort`.
use std::cmp::Ordering;
use std::sync::Arc;

use mlua::{Lua, MultiValue, Value};

use super::{Model, UiScript, WrathLfgEncounter};
use crate::script::binding_abi::{number_arg, tointeger_arg};

/// The character block of a raid browser entry, in the native record's order (`0x5529e0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathRaidBrowserStats {
    /// `+0x64`, pushed right after the role flags. Labelled: unnamed in 3.3.5's FrameXML.
    pub unknown_64: u32,
    /// Talent points per tree (`+0x2f..+0x31`).
    pub talents: [u8; 3],
    /// `+0x8c`, a boolean. Labelled: unnamed in 3.3.5's FrameXML.
    pub flag_8c: bool,
    pub armor: u32,
    pub spell_damage: u32,
    pub spell_heal: u32,
    pub crit_melee: u32,
    pub crit_ranged: u32,
    pub crit_spell: u32,
    pub mp5: f32,
    pub mp5_combat: f32,
    pub attack_power: u32,
    pub agility: u32,
    pub health: u32,
    pub mana: u32,
    pub avg_item_level: f32,
    pub defense: u32,
    pub dodge: u32,
    pub block: u32,
    pub parry: u32,
    pub haste: u32,
    pub expertise: u32,
}

/// One player, listed alone or as a group member.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathRaidBrowserMember {
    pub guid: u64,
    /// From the name cache; `None` while unresolved.
    pub name: Option<String>,
    /// Localized class name.
    pub class_name: Option<String>,
    /// Class file ("WARRIOR").
    pub class_file: Option<String>,
    pub level: u8,
    /// The AreaTable name of the member's zone.
    pub area: Option<String>,
    pub comment: String,
    /// `+0x88`.
    pub status: bool,
    /// Role bits: 1 leader, 2 tank, 4 healer, 8 damager (`+0x90`).
    pub roles: u8,
    /// Completed encounters of the browsed dungeon (`+0x1a0`).
    pub encounter_mask: u32,
    /// On the player's ignore list (`0x6b52e0`) or friends list (`0x6b3920`).
    pub ignored: bool,
    pub friend: bool,
    pub stats: WrathRaidBrowserStats,
}

/// A listed group: its comment and kills, the leader first among its members.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathRaidBrowserGroup {
    pub guid: u64,
    pub comment: String,
    /// `+0x130`.
    pub encounter_mask: u32,
    pub members: Vec<WrathRaidBrowserMember>,
}

/// `SMSG_UPDATE_LFG_LIST` as App has applied it, whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathRaidBrowserView {
    /// The browsed dungeon, a full packed entry (`0xbeab20`), 0 for none.
    pub dungeon: u32,
    pub groups: Vec<WrathRaidBrowserGroup>,
    pub players: Vec<WrathRaidBrowserMember>,
    /// The server's totals (`0xbeab28`, `0xbeab2c`).
    pub total_groups: u32,
    pub total_players: u32,
}

/// One verb for App to send, in call order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WrathRaidBrowserRequest {
    /// CMSG `0x35E` with the full packed entry.
    Join { dungeon: u32 },
    /// CMSG `0x35F` with the joined entry.
    Leave { dungeon: u32 },
}

/// The sort keys `SearchLFGSort` names: 0 zone, 1 level, 2 class, 3 name, 4 tank, 5 healer,
/// 6 damage.
const KEYS: [(&str, u8); 7] = [
    ("zone", 0),
    ("level", 1),
    ("class", 2),
    ("name", 3),
    ("tank", 4),
    ("healer", 5),
    ("damage", 6),
];

#[derive(Clone, Debug)]
pub(crate) struct RaidBrowser {
    view: Arc<WrathRaidBrowserView>,
    groups: Vec<WrathRaidBrowserGroup>,
    players: Vec<WrathRaidBrowserMember>,
    /// `0xbeaa14`: the joined entry, 0 for none.
    joined: u32,
    /// `0xacd660`: players listed before groups.
    players_first: bool,
    /// `0xbea820`: the keys, most recent first, each with its descending flag.
    order: [(u8, bool); 7],
    requests: Vec<WrathRaidBrowserRequest>,
}

impl Default for RaidBrowser {
    fn default() -> Self {
        Self {
            view: Arc::default(),
            groups: Vec::new(),
            players: Vec::new(),
            joined: 0,
            // World entry (`0x55d56c`-`0x55d591`): players first, keys 0..6 in order, ascending.
            players_first: true,
            order: [0, 1, 2, 3, 4, 5, 6].map(|k| (k, false)),
            requests: Vec::new(),
        }
    }
}

/// One key's comparison (`0x555a70` for group leaders, keys 0..3; `0x555cd0` for players).
fn compare(key: u8, a: &WrathRaidBrowserMember, b: &WrathRaidBrowserMember) -> Ordering {
    let fold = |s: &Option<String>| s.as_deref().map(str::to_lowercase);
    match key {
        0 => match (fold(&a.area), fold(&b.area)) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => Ordering::Equal,
        },
        1 => a.level.cmp(&b.level),
        2 => match (fold(&a.class_name), fold(&b.class_name)) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => Ordering::Equal,
        },
        3 => match (&a.name, &b.name) {
            (Some(x), Some(y)) => x.cmp(y),
            _ => Ordering::Equal,
        },
        // A role holder sorts first.
        4 => (b.roles & 2).cmp(&(a.roles & 2)),
        5 => (b.roles & 4).cmp(&(a.roles & 4)),
        6 => (b.roles & 8).cmp(&(a.roles & 8)),
        _ => Ordering::Equal,
    }
}

impl RaidBrowser {
    /// `0x55b700`: both lists re-sorted by the key order.
    fn sort(&mut self) {
        let order = self.order;
        let by = |a: &WrathRaidBrowserMember, b: &WrathRaidBrowserMember, groups: bool| {
            for (key, descending) in order {
                if groups && key > 3 {
                    continue;
                }
                let o = compare(key, a, b);
                if o != Ordering::Equal {
                    return if descending { o.reverse() } else { o };
                }
            }
            Ordering::Equal
        };
        self.players.sort_by(|a, b| by(a, b, false));
        self.groups
            .sort_by(|a, b| match (a.members.first(), b.members.first()) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
                (Some(x), Some(y)) => by(x, y, true),
            });
    }

    /// `0x553530`: the 0-based result index as (group, member record).
    fn entry(
        &self,
        index: usize,
    ) -> Option<(
        Option<&WrathRaidBrowserGroup>,
        Option<&WrathRaidBrowserMember>,
    )> {
        let group = |i: usize| self.groups.get(i).map(|g| (Some(g), g.members.first()));
        let player = |i: usize| self.players.get(i).map(|p| (None, Some(p)));
        if self.players_first {
            player(index).or_else(|| group(index.checked_sub(self.players.len())?))
        } else {
            group(index).or_else(|| player(index.checked_sub(self.groups.len())?))
        }
    }

    fn clear(&mut self) {
        self.view = Arc::default();
        self.groups.clear();
        self.players.clear();
    }
}

impl UiScript {
    /// Publish the raid browser list, whole; it is sorted locally. App fires UPDATE_LFG_LIST.
    pub fn set_wrath_raid_browser(&mut self, view: Arc<WrathRaidBrowserView>) {
        let mut model = self.model_mut();
        let rb = &mut model.wrath_lfg.raid_browser;
        rb.groups = view.groups.clone();
        rb.players = view.players.clone();
        rb.view = view;
        rb.sort();
    }

    /// Drain the raid browser verbs in call order.
    pub fn take_wrath_raid_browser_requests(&mut self) -> Vec<WrathRaidBrowserRequest> {
        std::mem::take(&mut self.model_mut().wrath_lfg.raid_browser.requests)
    }
}

fn int(n: impl Into<i64>) -> Value {
    Value::Integer(n.into())
}

fn text(lua: &Lua, s: &str) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(s)?))
}

fn localized(lua: &Lua, key: &str) -> mlua::Result<Value> {
    Ok(match lua.globals().get::<Value>(key)? {
        v @ Value::String(_) => v,
        _ => text(lua, key)?,
    })
}

fn or_unknown(lua: &Lua, s: &Option<String>) -> mlua::Result<Value> {
    match s {
        Some(s) => text(lua, s),
        None => localized(lua, "UNKNOWN"),
    }
}

/// `0x5529e0`: the 28 trailing values, from a record or as nils around a false.
fn tail(out: &mut Vec<Value>, m: Option<&WrathRaidBrowserMember>) {
    let Some(m) = m else {
        out.extend(std::iter::repeat_n(Value::Nil, 8));
        out.push(Value::Boolean(false));
        out.extend(std::iter::repeat_n(Value::Nil, 19));
        return;
    };
    let s = &m.stats;
    let trunc = |f: f32| int(f as i64);
    out.extend([1u8, 2, 4, 8].map(|b| Value::Boolean(m.roles & b != 0)));
    out.push(int(s.unknown_64));
    out.extend(s.talents.map(int));
    out.push(Value::Boolean(s.flag_8c));
    out.extend(
        [
            s.armor,
            s.spell_damage,
            s.spell_heal,
            s.crit_melee,
            s.crit_ranged,
            s.crit_spell,
        ]
        .map(int),
    );
    out.extend([trunc(s.mp5), trunc(s.mp5_combat)]);
    out.extend([s.attack_power, s.agility, s.health, s.mana].map(int));
    out.push(trunc(s.avg_item_level));
    out.extend([s.defense, s.dodge, s.block, s.parry, s.haste, s.expertise].map(int));
}

/// `0x553830`: the browsed dungeon's encounter count and kills under `mask`.
fn kills(m: &Model, mask: u32) -> (i64, i64) {
    let lfg = &m.wrath_lfg;
    let Some(row) = lfg.row(lfg.raid_browser.view.dungeon & 0xff_ffff) else {
        return (0, 0);
    };
    let rows = lfg
        .queue
        .encounters()
        .iter()
        .filter(|e| e.map == row.map_id && e.difficulty == row.difficulty);
    let (mut total, mut done) = (0, 0);
    for e in rows {
        total += 1;
        if mask & (1 << (e.bit & 31)) != 0 {
            done += 1;
        }
    }
    (total, done)
}

fn fire_update(lua: &Lua) {
    let event = "UPDATE_LFG_LIST";
    lua.app_data_mut::<Model>()
        .expect("model app_data")
        .pending_events
        .push((event.to_string(), Vec::new()));
}

fn set(
    lua: &Lua,
    name: &str,
    f: impl Fn(&Lua, MultiValue) -> mlua::Result<MultiValue> + Send + 'static,
) -> mlua::Result<()> {
    lua.globals()
        .set(name, lua.create_function(move |lua, args| f(lua, args))?)
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn encounter(lua: &Lua, e: Option<&WrathLfgEncounter>, mask: u32) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_vec(match e {
        Some(e) => vec![
            text(lua, &e.name)?,
            match &e.icon {
                Some(icon) => text(lua, icon)?,
                None => Value::Nil,
            },
            Value::Boolean(mask & (1 << (e.bit & 31)) != 0),
        ],
        None => vec![Value::Nil, Value::Nil, Value::Nil],
    }))
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    // SearchLFGGetNumResults() (`0x553ba0`) → listed results, server total; 0, 0 with no dungeon.
    set(lua, "SearchLFGGetNumResults", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let rb = &m.wrath_lfg.raid_browser;
        Ok(MultiValue::from_vec(if rb.view.dungeon == 0 {
            vec![int(0), int(0)]
        } else {
            vec![
                int((rb.groups.len() + rb.players.len()) as i64),
                int(i64::from(rb.view.total_groups) + i64::from(rb.view.total_players)),
            ]
        }))
    })?;
    // SearchLFGGetJoinedID() (`0x5529a0`) → the joined dungeon id, nothing when none.
    set(lua, "SearchLFGGetJoinedID", |lua, _| {
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let id = m.wrath_lfg.raid_browser.joined & 0xff_ffff;
        Ok(MultiValue::from_vec(if id == 0 {
            Vec::new()
        } else {
            vec![int(id)]
        }))
    })?;
    // SearchLFGGetResults(index) (`0x555fd0`) → name, level, area, class name, comment, other
    // members, status, class file, encounters total and killed, then the 28-value tail.
    set(lua, "SearchLFGGetResults", |lua, args| {
        // `lua_tointeger` (`0x556005`) rounds the index.
        let i = tointeger_arg(lua, arg(&args, 0), "Usage: SearchLFGGetResults(index)")?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let rb = &m.wrath_lfg.raid_browser;
        if m.wrath_lfg.row(rb.view.dungeon & 0xff_ffff).is_none() {
            return Ok(MultiValue::new());
        }
        let Some((group, Some(rec))) = usize::try_from(i.wrapping_sub(1))
            .ok()
            .and_then(|i| rb.entry(i))
        else {
            return Ok(MultiValue::new());
        };
        let name = or_unknown(lua, &rec.name)?;
        // The class name is looked up by the cached name (`0x72aab0`), so an unresolved name
        // reads UNKNOWN too.
        let class_name = if rec.name.is_some() {
            or_unknown(lua, &rec.class_name)?
        } else {
            localized(lua, "UNKNOWN")?
        };
        let comment = group.map_or(&rec.comment, |g| &g.comment);
        let others = group.map_or(0, |g| g.members.len().saturating_sub(1));
        let mask = group.map_or(rec.encounter_mask, |g| g.encounter_mask);
        let (total, done) = kills(&m, mask);
        let mut out = vec![
            name,
            int(rec.level),
            or_unknown(lua, &rec.area)?,
            class_name,
            text(lua, comment)?,
            int(others as i64),
            Value::Boolean(rec.status),
            match (&rec.name, &rec.class_file) {
                (Some(_), Some(c)) => text(lua, c)?,
                _ => Value::Nil,
            },
            int(total),
            int(done),
        ];
        tail(&mut out, Some(rec));
        Ok(MultiValue::from_vec(out))
    })?;
    // SearchLFGGetPartyResults(index, partyIndex) (`0x556280`) → a group's member partyIndex, 0
    // the leader: name, level, relationship, class name, area, comment and the tail; an
    // unresolved or missing member answers nil, "", nil, nil, nil and a nil tail.
    set(lua, "SearchLFGGetPartyResults", |lua, args| {
        const USAGE: &str = "Usage: SearchLFGGetPartyResults(index, partyIndex)";
        let i = number_arg(lua, arg(&args, 0), USAGE)?;
        let p = number_arg(lua, arg(&args, 1), USAGE)?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let rb = &m.wrath_lfg.raid_browser;
        let Some((group, _)) = usize::try_from(i.wrapping_sub(1))
            .ok()
            .and_then(|i| rb.entry(i))
        else {
            return Ok(MultiValue::new());
        };
        // `0x553490(p - 1)` reads row `p` of the leader-first members: 0 is the leader.
        let member = group.and_then(|g| usize::try_from(p).ok().and_then(|p| g.members.get(p)));
        let mut out = Vec::new();
        match member.filter(|r| r.name.is_some()) {
            Some(r) => {
                out.push(or_unknown(lua, &r.name)?);
                out.push(int(r.level));
                out.push(if r.ignored {
                    text(lua, "ignored")?
                } else if r.friend {
                    text(lua, "friend")?
                } else {
                    Value::Nil
                });
                out.push(text(lua, r.class_name.as_deref().unwrap_or(""))?);
                out.push(or_unknown(lua, &r.area)?);
                out.push(text(lua, &r.comment)?);
                tail(&mut out, Some(r));
            }
            None => {
                out.extend([
                    Value::Nil,
                    text(lua, "")?,
                    Value::Nil,
                    Value::Nil,
                    Value::Nil,
                ]);
                tail(&mut out, None);
            }
        }
        Ok(MultiValue::from_vec(out))
    })?;
    // SearchLFGGetEncounterResults(index, encounterIndex) (`0x553c30` → `0x5538b0`).
    set(lua, "SearchLFGGetEncounterResults", |lua, args| {
        const USAGE: &str = "Usage: SearchLFGGetEncounterResults(index, encounterIndex)";
        let i = number_arg(lua, arg(&args, 0), USAGE)?;
        let n = number_arg(lua, arg(&args, 1), USAGE)?;
        let m = lua.app_data_ref::<Model>().expect("model app_data");
        let lfg = &m.wrath_lfg;
        let rb = &lfg.raid_browser;
        let Some(row) = lfg.row(rb.view.dungeon & 0xff_ffff) else {
            return Ok(MultiValue::new());
        };
        let Some((group, rec)) = usize::try_from(i.wrapping_sub(1))
            .ok()
            .and_then(|i| rb.entry(i))
        else {
            return Ok(MultiValue::new());
        };
        let Some(rec) = rec else {
            return encounter(lua, None, 0);
        };
        let mask = group.map_or(rec.encounter_mask, |g| g.encounter_mask);
        let found = usize::try_from(n.wrapping_sub(1)).ok().and_then(|n| {
            lfg.queue
                .encounters()
                .iter()
                .filter(|e| e.map == row.map_id && e.difficulty == row.difficulty)
                .nth(n)
        });
        encounter(lua, found, mask)
    })?;
    // SearchLFGJoin(typeID, lfgID) (`0x559400` → `0x558ed0`): a new entry clears the list, fires
    // UPDATE_LFG_LIST and sends the join. Labelled: the gate `0x5191c0(9)` passes for the trusted
    // UI.
    set(lua, "SearchLFGJoin", |lua, args| {
        const USAGE: &str = "Usage: SearchLFGJoin(typeID, lfgID)";
        let kind = number_arg(lua, arg(&args, 0), USAGE)?;
        let id = number_arg(lua, arg(&args, 1), USAGE)? as u32;
        let changed = {
            let mut m = lua.app_data_mut::<Model>().expect("model app_data");
            // The row is found by the unmasked id (`0x65c290`, `0x559497`).
            if kind >= 7 || m.wrath_lfg.row(id).is_none() {
                return Ok(MultiValue::new());
            }
            let entry = (kind as u32) << 24 | id & 0xff_ffff;
            let rb = &mut m.wrath_lfg.raid_browser;
            let changed = rb.joined != entry;
            if changed {
                rb.clear();
                rb.joined = entry;
                rb.requests
                    .push(WrathRaidBrowserRequest::Join { dungeon: entry });
            }
            changed
        };
        if changed {
            fire_update(lua);
        }
        Ok(MultiValue::new())
    })?;
    // SearchLFGLeave() (`0x5594e0` → `0x558f90`): with an entry joined, the leave, the list
    // cleared and UPDATE_LFG_LIST.
    set(lua, "SearchLFGLeave", |lua, _| {
        let left = {
            let mut m = lua.app_data_mut::<Model>().expect("model app_data");
            let rb = &mut m.wrath_lfg.raid_browser;
            let joined = rb.joined;
            if joined != 0 {
                rb.requests
                    .push(WrathRaidBrowserRequest::Leave { dungeon: joined });
                rb.clear();
                rb.joined = 0;
            }
            joined != 0
        };
        if left {
            fire_update(lua);
        }
        Ok(MultiValue::new())
    })?;
    // SearchLFGSort("type") (`0x55d2b0`): "group" flips groups and players; any other key moves
    // to the front of the order (a repeat flips its direction; an unknown name sorts by name),
    // then the lists re-sort and UPDATE_LFG_LIST fires.
    set(lua, "SearchLFGSort", |lua, args| {
        let Some(kind) = (match arg(&args, 0) {
            v @ (Value::String(_) | Value::Integer(_) | Value::Number(_)) => {
                lua.coerce_string(v)?.map(|s| s.to_string_lossy())
            }
            _ => None,
        }) else {
            return Err(mlua::Error::runtime(r#"Usage: SearchLFGSort("type")"#));
        };
        {
            let mut m = lua.app_data_mut::<Model>().expect("model app_data");
            let rb = &mut m.wrath_lfg.raid_browser;
            if kind.eq_ignore_ascii_case("group") {
                rb.players_first = !rb.players_first;
            } else {
                let key = KEYS
                    .iter()
                    .find(|(n, _)| kind.eq_ignore_ascii_case(n))
                    .map_or(3, |&(_, k)| k);
                if let Some(at) = rb.order.iter().position(|&(k, _)| k == key) {
                    let (_, descending) = rb.order[at];
                    if at == 0 {
                        rb.order[0].1 = !descending;
                    } else {
                        rb.order.copy_within(0..at, 1);
                        rb.order[0] = (key, descending);
                    }
                }
                rb.sort();
            }
        }
        fire_update(lua);
        Ok(MultiValue::new())
    })?;
    // RefreshLFGList() (`0x55d280`): copies the staged list (`0xbeaa98`) over the displayed one
    // (`0xbeab20`, `0x55b1a0`) and fires UPDATE_LFG_LIST; no request. Every list update already
    // makes the same copy (`0x55b700`), so the published view stands and only the event fires.
    set(lua, "RefreshLFGList", |lua, _| {
        fire_update(lua);
        Ok(MultiValue::new())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{WrathLfgCatalog, WrathLfgDungeon};

    fn member(name: &str, level: u8, roles: u8) -> WrathRaidBrowserMember {
        WrathRaidBrowserMember {
            guid: level.into(),
            name: Some(name.into()),
            class_name: Some("Warrior".into()),
            class_file: Some("WARRIOR".into()),
            level,
            area: Some("Dalaran".into()),
            comment: format!("{name} lfm"),
            roles,
            stats: WrathRaidBrowserStats {
                armor: 900,
                avg_item_level: 200.7,
                expertise: 26,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.set_wrath_lfg_catalog(
            Arc::new(WrathLfgCatalog {
                dungeons: vec![WrathLfgDungeon {
                    id: 50,
                    dungeon_type: 2,
                    map_id: 615,
                    difficulty: 0,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            None,
        );
        s.set_wrath_lfg_encounters(Arc::new(vec![WrathLfgEncounter {
            map: 615,
            difficulty: 0,
            bit: 1,
            name: "Sartharion".into(),
            icon: None,
        }]));
        s
    }

    #[test]
    fn results_party_and_encounters_in_native_shapes() {
        let mut s = vm();
        assert!(s.eval::<bool>("local a, b = SearchLFGGetNumResults() return a == 0 and b == 0 and select('#', SearchLFGGetJoinedID()) == 0").unwrap());
        let mut friend = member("Pal", 70, 4);
        friend.friend = true;
        s.set_wrath_raid_browser(Arc::new(WrathRaidBrowserView {
            dungeon: 2 << 24 | 50,
            groups: vec![WrathRaidBrowserGroup {
                guid: 9,
                comment: "group lfm".into(),
                encounter_mask: 2,
                members: vec![member("Lead", 80, 3), friend],
            }],
            players: vec![member("Zed", 79, 8), member("Amy", 75, 2)],
            total_groups: 4,
            total_players: 10,
        }));
        assert!(
            s.eval::<bool>(
                "return SearchLFGGetResults(0.6) == 'Amy' and SearchLFGGetResults(3) == 'Lead'"
            )
            .unwrap(),
            "players first, by level, before any sort"
        );
        s.run("SearchLFGSort('group')").unwrap();
        for check in [
            "select(1, SearchLFGGetNumResults()) == 3 and select(2, SearchLFGGetNumResults()) == 14",
            "select('#', SearchLFGGetResults(1)) == 38",
            "select(1, SearchLFGGetResults(1)) == 'Lead' and select(5, SearchLFGGetResults(1)) == 'group lfm'",
            "select(6, SearchLFGGetResults(1)) == 1 and select(9, SearchLFGGetResults(1)) == 1 and select(10, SearchLFGGetResults(1)) == 1",
            "select(11, SearchLFGGetResults(1)) == true and select(12, SearchLFGGetResults(1)) == true and select(13, SearchLFGGetResults(1)) == false",
            "select(20, SearchLFGGetResults(1)) == 900 and select(32, SearchLFGGetResults(1)) == 200 and select(38, SearchLFGGetResults(1)) == 26",
            "select(1, SearchLFGGetResults(2)) == 'Amy' and select(6, SearchLFGGetResults(2)) == 0 and select(10, SearchLFGGetResults(2)) == 0",
            "select(1, SearchLFGGetPartyResults(1, 1)) == 'Pal' and select(3, SearchLFGGetPartyResults(1, 1)) == 'friend'",
            "select('#', SearchLFGGetPartyResults(1, 1)) == 34 and select('#', SearchLFGGetPartyResults(1, 2)) == 33",
            "select(2, SearchLFGGetPartyResults(1, 2)) == ''",
            "SearchLFGGetPartyResults(1, 0) == 'Lead' and SearchLFGGetPartyResults(1, -1) == nil",
            "select(1, SearchLFGGetEncounterResults(1, 1)) == 'Sartharion' and select(3, SearchLFGGetEncounterResults(1, 1)) == true",
            "select(3, SearchLFGGetEncounterResults(2, 1)) == false and select('#', SearchLFGGetEncounterResults(2, 2)) == 3",
            "select('#', SearchLFGGetResults(4)) == 0",
        ] {
            assert!(s.eval::<bool>(&format!("return {check}")).unwrap(), "{check}");
        }
        assert!(s.run("SearchLFGGetResults()").is_err());
    }

    #[test]
    fn sort_join_and_leave_follow_the_native_state() {
        let mut s = vm();
        s.set_wrath_raid_browser(Arc::new(WrathRaidBrowserView {
            dungeon: 2 << 24 | 50,
            players: vec![
                member("Zed", 79, 8),
                member("Amy", 75, 2),
                member("Bob", 80, 4),
            ],
            ..Default::default()
        }));
        let order = |s: &UiScript| {
            s.eval::<String>(
                "return SearchLFGGetResults(1)..SearchLFGGetResults(2)..SearchLFGGetResults(3)",
            )
            .unwrap()
        };
        assert_eq!(order(&s), "AmyZedBob", "zone, then level, before any sort");
        s.run("SearchLFGSort('name')").unwrap();
        assert_eq!(order(&s), "AmyBobZed");
        s.run("SearchLFGSort('name')").unwrap();
        assert_eq!(order(&s), "ZedBobAmy", "a repeat flips the direction");
        s.run("SearchLFGSort('level')").unwrap();
        assert_eq!(order(&s), "AmyZedBob");
        s.run("SearchLFGSort('tank')").unwrap();
        assert_eq!(order(&s), "AmyZedBob", "the tank first, then level");
        s.run("SearchLFGSort('healer')").unwrap();
        assert_eq!(order(&s), "BobAmyZed");
        s.run("SearchLFGJoin(2, 50) SearchLFGJoin(2, 50) SearchLFGJoin(9, 50) SearchLFGLeave() SearchLFGLeave() SearchLFGJoin(2, 16777266)")
            .unwrap();
        assert_eq!(
            s.take_wrath_raid_browser_requests(),
            vec![
                WrathRaidBrowserRequest::Join {
                    dungeon: 2 << 24 | 50
                },
                WrathRaidBrowserRequest::Leave {
                    dungeon: 2 << 24 | 50
                },
            ]
        );
        assert!(s
            .eval::<bool>(
                "return select('#', SearchLFGGetJoinedID()) == 0 and SearchLFGGetNumResults() == 0"
            )
            .unwrap());
        assert!(s.run("SearchLFGSort()").is_err() && s.run("SearchLFGJoin(2)").is_err());
    }

    #[test]
    fn refresh_fires_update_lfg_list_over_the_current_list_without_a_request() {
        let mut s = vm();
        s.set_wrath_raid_browser(Arc::new(WrathRaidBrowserView {
            dungeon: 2 << 24 | 50,
            players: vec![member("Amy", 75, 2)],
            ..Default::default()
        }));
        s.run(
            "n = 0 local f = CreateFrame('Frame') f:RegisterEvent('UPDATE_LFG_LIST') \
               f:SetScript('OnEvent', function() n = n + 1 end)",
        )
        .unwrap();
        assert_eq!(
            s.eval::<i64>("return select('#', RefreshLFGList())")
                .unwrap(),
            0
        );
        s.tick(0.0);
        assert_eq!(s.eval::<i64>("return n").unwrap(), 1);
        assert_eq!(
            s.eval::<String>("return SearchLFGGetResults(1)").unwrap(),
            "Amy"
        );
        assert!(s.take_wrath_raid_browser_requests().is_empty());
    }
}
