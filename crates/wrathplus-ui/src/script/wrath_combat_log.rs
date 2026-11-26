//! Build-12340 combat log: the entry store behind `CombatLog*`, its filters and the two events,
//! over typed entries App builds (seam agreed with App; App's evidence
//! `/tmp/azb-app-mig/ghidra/combatlog-*.log`, ours `/tmp/azb-ui-clog-native.log` and
//! `/tmp/azb-ui-clog-helpers.log`).
//!
//! Native: entries live in a list (cursor `0xca1390`); a new entry fires `COMBAT_LOG_EVENT` when
//! the active filters admit it, then `COMBAT_LOG_EVENT_UNFILTERED` (`0x74f910`), both with the
//! pusher's arguments (`0x74e290`): timestamp, event name (table `0xadb758`), source GUID
//! (`0x%016X`), source name or nil, source flags, the same three for the destination, then
//! spellId, spellName, spellSchool when the entry has a spell, then the suffix. Filters form a
//! list (`0xadb970`); none admits everything, otherwise any filter that matches admits
//! (`0x74f6c0` → `0x74e1a0`). Labelled: the destination mask test (`0x74d1a0`) is taken to be the
//! source's IsA test; ResetFilter (`0x74fa60`) and ClearEntries (`0x751120`) were not decompiled
//! and are taken to clear the filter list and the entries; retention is held here (default 300 s)
//! rather than in the CVar `0x766940` writes, and entries age by their GetTime arrival.

use std::collections::VecDeque;

use mlua::{Lua, MultiValue, Value};

use super::{Model, ScriptValue};

/// The event table at `0xadb758`, in index order.
pub const WRATH_COMBAT_LOG_EVENTS: [&str; 50] = [
    "ENVIRONMENTAL_DAMAGE",
    "SWING_DAMAGE",
    "SWING_MISSED",
    "RANGE_DAMAGE",
    "RANGE_MISSED",
    "SPELL_CAST_START",
    "SPELL_CAST_SUCCESS",
    "SPELL_CAST_FAILED",
    "SPELL_MISSED",
    "SPELL_DAMAGE",
    "SPELL_HEAL",
    "SPELL_ENERGIZE",
    "SPELL_DRAIN",
    "SPELL_LEECH",
    "SPELL_INSTAKILL",
    "SPELL_SUMMON",
    "SPELL_CREATE",
    "SPELL_INTERRUPT",
    "SPELL_EXTRA_ATTACKS",
    "SPELL_DURABILITY_DAMAGE",
    "SPELL_DURABILITY_DAMAGE_ALL",
    "SPELL_AURA_APPLIED",
    "SPELL_AURA_APPLIED_DOSE",
    "SPELL_AURA_REMOVED_DOSE",
    "SPELL_AURA_REMOVED",
    "SPELL_AURA_REFRESH",
    "SPELL_DISPEL",
    "SPELL_STOLEN",
    "SPELL_AURA_BROKEN",
    "SPELL_AURA_BROKEN_SPELL",
    "DAMAGE_AURA_BROKEN",
    "ENCHANT_APPLIED",
    "ENCHANT_REMOVED",
    "SPELL_PERIODIC_MISSED",
    "SPELL_PERIODIC_DAMAGE",
    "SPELL_PERIODIC_HEAL",
    "SPELL_PERIODIC_ENERGIZE",
    "SPELL_PERIODIC_DRAIN",
    "SPELL_PERIODIC_LEECH",
    "SPELL_DISPEL_FAILED",
    "DAMAGE_SHIELD",
    "DAMAGE_SHIELD_MISSED",
    "DAMAGE_SPLIT",
    "PARTY_KILL",
    "UNIT_DIED",
    "UNIT_DESTROYED",
    "SPELL_RESURRECT",
    "SPELL_BUILDING_DAMAGE",
    "SPELL_BUILDING_HEAL",
    "UNIT_DISSIPATES",
];

/// One side of an entry. `None` on the entry pushes GUID 0, nil and flags 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCombatLogUnit {
    pub guid: u64,
    /// The name as Lua sees it, a cross-realm `-realm` suffix included; `None` or empty is nil.
    pub name: Option<String>,
    pub flags: u32,
}

/// One combat-log entry as App builds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrathCombatLogEntry {
    /// Argument 1 as native computes it (Unix-epoch seconds with milliseconds).
    pub timestamp: f64,
    /// Index into [`WRATH_COMBAT_LOG_EVENTS`].
    pub event: u8,
    pub source: Option<WrathCombatLogUnit>,
    pub dest: Option<WrathCombatLogUnit>,
    /// The spell prefix: id, name, school.
    pub spell: Option<(u32, String, u32)>,
    /// The suffix arguments, ordered and nil-filled by App.
    pub suffix: Vec<ScriptValue>,
}

#[derive(Clone, Debug)]
struct Stored {
    seq: u64,
    arrived: f64,
    entry: WrathCombatLogEntry,
}

#[derive(Clone, Debug, PartialEq)]
enum Side {
    Guid(u64),
    Mask(u32),
}

#[derive(Clone, Debug, PartialEq)]
enum SpellSel {
    Any,
    Id(u32),
    Name(String),
}

#[derive(Clone, Debug, PartialEq)]
struct Filter {
    /// Bit `i` admits event `i`.
    events: u64,
    src: Side,
    dst: Side,
    spell: SpellSel,
}

/// The store, cursor, filters and retention.
#[derive(Clone, Debug)]
pub(crate) struct State {
    entries: VecDeque<Stored>,
    next_seq: u64,
    cursor: Option<u64>,
    filters: Vec<Filter>,
    retention: i32,
}

impl Default for State {
    fn default() -> Self {
        State {
            entries: VecDeque::new(),
            next_seq: 0,
            cursor: None,
            filters: Vec::new(),
            retention: 300,
        }
    }
}

/// `CombatLog_Object_IsA`'s test (`0x74d600`): any special bit, or a bit in each of affiliation,
/// reaction, control and type.
fn is_a(v: u32) -> bool {
    v & 0xffff_0000 != 0 || (v & 0xf != 0 && v & 0xf0 != 0 && v & 0x300 != 0 && v & 0xfc00 != 0)
}

fn unit(u: &Option<WrathCombatLogUnit>) -> (u64, u32) {
    u.as_ref().map_or((0, 0), |u| (u.guid, u.flags))
}

impl Filter {
    /// `0x74e1a0`.
    fn admits(&self, e: &WrathCombatLogEntry) -> bool {
        if e.event >= 64 || self.events & (1 << e.event) == 0 {
            return false;
        }
        let side = |side: &Side, u: &Option<WrathCombatLogUnit>| {
            let (guid, flags) = unit(u);
            match side {
                Side::Guid(g) => guid == *g,
                Side::Mask(m) => is_a(m & flags),
            }
        };
        if !side(&self.src, &e.source) || !side(&self.dst, &e.dest) {
            return false;
        }
        match &self.spell {
            SpellSel::Any => true,
            SpellSel::Id(id) => e.spell.as_ref().map_or(0, |s| s.0) == *id,
            SpellSel::Name(n) => e
                .spell
                .as_ref()
                .is_some_and(|s| s.1.eq_ignore_ascii_case(n)),
        }
    }
}

impl State {
    fn admitted(&self, e: &WrathCombatLogEntry) -> bool {
        self.filters.is_empty() || self.filters.iter().any(|f| f.admits(e))
    }

    /// Drop entries older than the retention (`0x750400`/`0x750390`).
    fn prune(&mut self, now: f64) {
        let keep = f64::from(self.retention.max(0));
        while self.entries.front().is_some_and(|s| now - s.arrived > keep) {
            let gone = self.entries.pop_front().map(|s| s.seq);
            if gone.is_some() && gone == self.cursor {
                self.cursor = None;
            }
        }
    }

    fn visible(&self, ignore: bool) -> impl Iterator<Item = &Stored> + '_ {
        self.entries
            .iter()
            .filter(move |s| ignore || self.admitted(&s.entry))
    }

    fn position(&self, seq: u64) -> Option<usize> {
        self.entries.iter().position(|s| s.seq == seq)
    }
}

/// The pusher's arguments (`0x74e290`).
fn args(e: &WrathCombatLogEntry) -> Vec<ScriptValue> {
    let side = |u: &Option<WrathCombatLogUnit>| {
        let (guid, flags) = unit(u);
        let name = u
            .as_ref()
            .and_then(|u| u.name.clone())
            .filter(|n| !n.is_empty())
            .map_or(ScriptValue::Nil, ScriptValue::Str);
        [
            ScriptValue::Str(format!("0x{guid:016X}")),
            name,
            ScriptValue::Int(i64::from(flags)),
        ]
    };
    let mut out = vec![
        ScriptValue::Number(e.timestamp),
        ScriptValue::Str(
            WRATH_COMBAT_LOG_EVENTS
                .get(usize::from(e.event))
                .copied()
                .unwrap_or("")
                .to_string(),
        ),
    ];
    out.extend(side(&e.source));
    out.extend(side(&e.dest));
    if let Some((id, name, school)) = &e.spell {
        out.push(ScriptValue::Int(i64::from(*id)));
        out.push(ScriptValue::Str(name.clone()));
        out.push(ScriptValue::Int(i64::from(*school)));
    }
    out.extend(e.suffix.iter().cloned());
    out
}

impl super::UiScript {
    /// Store one entry and fire `COMBAT_LOG_EVENT` (when the filters admit it), then
    /// `COMBAT_LOG_EVENT_UNFILTERED`, synchronously, with the same arguments. Push in wire order.
    pub fn push_wrath_combat_log(&mut self, entry: WrathCombatLogEntry) {
        let (admitted, values) = {
            let mut model = self.model_mut();
            let now = model.now;
            let state = &mut model.wrath_combat_log;
            state.prune(now);
            let admitted = state.admitted(&entry);
            let values = args(&entry);
            let seq = state.next_seq;
            state.next_seq += 1;
            state.entries.push_back(Stored {
                seq,
                arrived: now,
                entry,
            });
            (admitted, values)
        };
        if admitted {
            self.fire_event("COMBAT_LOG_EVENT", values.clone());
        }
        self.fire_event("COMBAT_LOG_EVENT_UNFILTERED", values);
    }
}

/// `lua_tointeger`, rounded; 0 for anything else.
fn to_integer(lua: &Lua, v: &Value) -> i64 {
    lua.coerce_number(v.clone())
        .ok()
        .flatten()
        .filter(|n| n.is_finite())
        .map_or(0, |n| n.round_ties_even() as i64)
}

/// A number argument through `fistp` under RC=11, truncated; 0 for anything else.
fn truncated(lua: &Lua, v: &Value) -> i64 {
    lua.coerce_number(v.clone())
        .ok()
        .flatten()
        .filter(|n| n.is_finite())
        .map_or(0, |n| n.trunc() as i64)
}

/// A GUID string as `0x74d120` parses it: an optional `0x`, then up to 16 hex digits.
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

/// A mask argument of CombatLogAddFilter: a number (truncated, `0x75002e`, which must pass the IsA
/// test), a GUID string, or anything else (every flag).
fn side_arg(lua: &Lua, v: &Value, which: &str) -> mlua::Result<Side> {
    match v {
        Value::Integer(_) | Value::Number(_) => {
            let m = truncated(lua, v) as u32;
            if !is_a(m) {
                return Err(mlua::Error::runtime(format!(
                    "CombatLogAddFilter(): incomplete filter for {which}"
                )));
            }
            Ok(Side::Mask(m))
        }
        Value::String(s) => match parse_guid(&s.to_string_lossy()) {
            0 => Ok(Side::Mask(0)),
            guid => Ok(Side::Guid(guid)),
        },
        _ => Ok(Side::Mask(u32::MAX)),
    }
}

/// The admitted entry `n` steps from `start` (`0x74fae0`/`0x74fc20`): walking forward or
/// backward from `start` itself, each admitted entry either is the one (when `n` has reached 0) or
/// counts one step; `None` past the end.
fn walk(state: &State, start: usize, mut n: u64, forward: bool) -> Option<u64> {
    let mut pick = |s: &Stored| {
        if state.admitted(&s.entry) {
            if n == 0 {
                return Some(s.seq);
            }
            n -= 1;
        }
        None
    };
    if forward {
        state.entries.iter().skip(start).find_map(&mut pick)
    } else {
        state
            .entries
            .iter()
            .take(start + 1)
            .rev()
            .find_map(&mut pick)
    }
}

/// [`walk`] with the ignoreFilter flag: every entry counts.
fn walk_any(state: &State, start: usize, n: u64, forward: bool, ignore: bool) -> Option<u64> {
    if !ignore {
        return walk(state, start, n, forward);
    }
    let len = state.entries.len();
    let at = if forward {
        start.checked_add(n as usize).filter(|i| *i < len)
    } else {
        start.checked_sub(n as usize)
    };
    at.and_then(|i| state.entries.get(i)).map(|s| s.seq)
}

/// The model, with the store pruned to the retention first.
fn with(lua: &Lua) -> mlua::AppDataRefMut<'_, Model> {
    let mut model = lua.app_data_mut::<Model>().expect("model app_data");
    let now = model.now;
    model.wrath_combat_log.prune(now);
    model
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();

    // CombatLogGetNumEntries([ignoreFilter]) → the stored entries the filters admit, or all.
    g.set(
        "CombatLogGetNumEntries",
        lua.create_function(|lua, args: MultiValue| {
            let ignore = super::binding_abi::bool_or_default(args.front(), false);
            let model = with(lua);
            Ok(model.wrath_combat_log.visible(ignore).count() as i64)
        })?,
    )?;

    // CombatLogGetCurrentEntry() → the cursor entry's arguments, nothing without a cursor.
    g.set(
        "CombatLogGetCurrentEntry",
        lua.create_function(|lua, ()| {
            let values = {
                let model = with(lua);
                let state = &model.wrath_combat_log;
                state
                    .cursor
                    .and_then(|seq| state.entries.iter().find(|s| s.seq == seq))
                    .map(|s| args(&s.entry))
            };
            let mut out = MultiValue::new();
            for v in values.unwrap_or_default() {
                out.push_back(v.into_lua(lua)?);
            }
            Ok(out)
        })?,
    )?;

    // CombatLogSetCurrentEntry(index [, ignoreFilter]) → 1 or nil: index n >= 1 is the nth
    // admitted entry from the oldest, 0 the newest, -n the nth before the newest.
    g.set(
        "CombatLogSetCurrentEntry",
        lua.create_function(|lua, args: MultiValue| {
            let first = args.front().cloned().unwrap_or(Value::Nil);
            if lua.coerce_number(first.clone())?.is_none() {
                return Err(mlua::Error::runtime(
                    "Usage: CombatLogSetCurrentEntry(index [,ignoreFilter])",
                ));
            }
            let index = to_integer(lua, &first);
            let ignore = super::binding_abi::bool_or_default(args.get(1), false);
            let mut model = with(lua);
            let state = &mut model.wrath_combat_log;
            state.cursor = None;
            let found = if state.entries.is_empty() {
                None
            } else if index >= 1 {
                walk_any(state, 0, (index - 1) as u64, true, ignore)
            } else {
                walk_any(
                    state,
                    state.entries.len() - 1,
                    index.unsigned_abs(),
                    false,
                    ignore,
                )
            };
            state.cursor = found;
            Ok(super::binding_abi::flag(found.is_some()))
        })?,
    )?;

    // CombatLogAdvanceEntry(count [, ignoreFilter]) → 1 or nil: the cursor moves `count` admitted
    // entries (0 leaves it), running off either end clears it.
    g.set(
        "CombatLogAdvanceEntry",
        lua.create_function(|lua, args: MultiValue| {
            let first = args.front().cloned().unwrap_or(Value::Nil);
            if lua.coerce_number(first.clone())?.is_none() {
                return Err(mlua::Error::runtime(
                    "Usage: CombatLogAdvanceEntry(count [,ignoreFilter])",
                ));
            }
            let count = to_integer(lua, &first);
            let ignore = super::binding_abi::bool_or_default(args.get(1), false);
            let mut model = with(lua);
            let state = &mut model.wrath_combat_log;
            if count != 0 {
                state.cursor = state
                    .cursor
                    .and_then(|seq| state.position(seq))
                    .and_then(|at| walk_any(state, at, count.unsigned_abs(), count > 0, ignore));
            }
            Ok(super::binding_abi::flag(state.cursor.is_some()))
        })?,
    )?;

    g.set(
        "CombatLogClearEntries",
        lua.create_function(|lua, _ignored: MultiValue| {
            let mut model = with(lua);
            model.wrath_combat_log.entries.clear();
            model.wrath_combat_log.cursor = None;
            Ok(())
        })?,
    )?;

    g.set(
        "CombatLogResetFilter",
        lua.create_function(|lua, _ignored: MultiValue| {
            with(lua).wrath_combat_log.filters.clear();
            Ok(())
        })?,
    )?;

    // CombatLogAddFilter([events [, srcMask [, dstMask [, spell]]]]) (`0x74ff70`): events are
    // names split on spaces and commas (`" ,"`, `0x9eb7ec`; nil admits all, unknown names are
    // skipped); each mask is a number that must pass the IsA test, a GUID string, or anything else
    // for every flag; spell is an id, truncated, where 0 is any spell (`0x74e155`), or a name.
    g.set(
        "CombatLogAddFilter",
        lua.create_function(|lua, args: MultiValue| {
            let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Nil);
            let events = match lua.coerce_string(arg(0))? {
                None => u64::MAX,
                Some(list) => {
                    let list = list.to_string_lossy();
                    list.split([' ', ','])
                        .filter_map(|t| {
                            WRATH_COMBAT_LOG_EVENTS
                                .iter()
                                .position(|n| n.eq_ignore_ascii_case(t))
                        })
                        .fold(0u64, |m, i| m | 1 << i)
                }
            };
            let src = side_arg(lua, &arg(1), "srcMask")?;
            let dst = side_arg(lua, &arg(2), "dstMask")?;
            let spell = match arg(3) {
                v @ (Value::Integer(_) | Value::Number(_)) => match truncated(lua, &v) as u32 {
                    0 => SpellSel::Any,
                    id => SpellSel::Id(id),
                },
                Value::String(s) => SpellSel::Name(s.to_string_lossy()),
                _ => SpellSel::Any,
            };
            with(lua).wrath_combat_log.filters.push(Filter {
                events,
                src,
                dst,
                spell,
            });
            Ok(())
        })?,
    )?;

    g.set(
        "CombatLogGetRetentionTime",
        lua.create_function(|lua, ()| Ok(i64::from(with(lua).wrath_combat_log.retention)))?,
    )?;
    g.set(
        "CombatLogSetRetentionTime",
        lua.create_function(|lua, args: MultiValue| {
            let first = args.front().cloned().unwrap_or(Value::Nil);
            if lua.coerce_number(first.clone())?.is_none() {
                return Err(mlua::Error::runtime(
                    "Usage: CombatLogSetRetentionTime(seconds)",
                ));
            }
            let seconds = to_integer(lua, &first) as i32;
            with(lua).wrath_combat_log.retention = seconds;
            Ok(())
        })?,
    )?;

    // CombatLog_Object_IsA(flags, mask) → 1 or nil over `flags & mask` (`0x74d600`).
    g.set(
        "CombatLog_Object_IsA",
        lua.create_function(|lua, (flags, mask): (Value, Value)| {
            let v = (to_integer(lua, &flags) as u32) & (to_integer(lua, &mask) as u32);
            Ok(super::binding_abi::flag(is_a(v)))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::UiScript;

    const MINE_PLAYER: u32 = 0x511; // mine, friendly, player-controlled, player
    const HOSTILE_NPC: u32 = 0xa48; // outsider, hostile, npc-controlled, npc

    fn entry(event: u8, src: u32, dst: u32, spell: Option<(u32, &str)>) -> WrathCombatLogEntry {
        WrathCombatLogEntry {
            timestamp: 1_700_000_000.25,
            event,
            source: Some(WrathCombatLogUnit {
                guid: 0x0700_0000_0000_002a,
                name: Some("Me".into()),
                flags: src,
            }),
            dest: Some(WrathCombatLogUnit {
                guid: 0xf130_0000_0000_0001,
                name: None,
                flags: dst,
            }),
            spell: spell.map(|(id, n)| (id, n.to_string(), 4)),
            suffix: vec![ScriptValue::Int(120), ScriptValue::Nil],
        }
    }

    fn vm() -> UiScript {
        let mut s = UiScript::new().unwrap();
        s.run(
            "F, U = {}, {} local f = CreateFrame('Frame') \
             f:RegisterEvent('COMBAT_LOG_EVENT') f:RegisterEvent('COMBAT_LOG_EVENT_UNFILTERED') \
             f:SetScript('OnEvent', function(_, e, ...) \
               if e == 'COMBAT_LOG_EVENT' then F[#F + 1] = {...} else U[#U + 1] = {...} end end)",
        )
        .unwrap();
        s.set_now(10.0);
        s
    }

    #[test]
    fn a_push_fires_filtered_then_unfiltered_with_the_native_arguments() {
        let mut s = vm();
        s.run(
            "ORDER = {} local g = CreateFrame('Frame') \
               g:RegisterEvent('COMBAT_LOG_EVENT') g:RegisterEvent('COMBAT_LOG_EVENT_UNFILTERED') \
               g:SetScript('OnEvent', function(_, e) ORDER[#ORDER + 1] = e end)",
        )
        .unwrap();
        s.push_wrath_combat_log(entry(9, MINE_PLAYER, HOSTILE_NPC, Some((133, "Fireball"))));
        assert_eq!(
            s.eval::<Vec<String>>("return ORDER").unwrap(),
            vec![
                "COMBAT_LOG_EVENT".to_string(),
                "COMBAT_LOG_EVENT_UNFILTERED".to_string()
            ]
        );
        assert_eq!(
            s.eval::<(f64, String, String, String, i64, String, bool, i64)>(
                "local a = U[1] return a[1], a[2], a[3], a[4], a[5], a[6], a[7] == nil, a[8]"
            )
            .unwrap(),
            (
                1_700_000_000.25,
                "SPELL_DAMAGE".into(),
                "0x070000000000002A".into(),
                "Me".into(),
                i64::from(MINE_PLAYER),
                "0xF130000000000001".into(),
                true,
                i64::from(HOSTILE_NPC)
            )
        );
        assert_eq!(
            s.eval::<(i64, String, i64, i64)>("local a = U[1] return a[9], a[10], a[11], a[12]")
                .unwrap(),
            (133, "Fireball".into(), 4, 120)
        );
    }

    #[test]
    fn filters_gate_the_filtered_event_and_the_cursor() {
        let mut s = vm();
        // Only my spells' damage, by source mask.
        s.run(&format!(
            "CombatLogResetFilter() CombatLogAddFilter('SPELL_DAMAGE,SWING_DAMAGE', {MINE_PLAYER}, nil)"
        ))
        .unwrap();
        s.push_wrath_combat_log(entry(9, MINE_PLAYER, HOSTILE_NPC, Some((133, "Fireball"))));
        s.push_wrath_combat_log(entry(9, HOSTILE_NPC, MINE_PLAYER, Some((1, "Bite"))));
        s.push_wrath_combat_log(entry(10, MINE_PLAYER, MINE_PLAYER, Some((2, "Heal"))));
        assert_eq!(s.eval::<i64>("return #F").unwrap(), 1);
        assert_eq!(s.eval::<i64>("return #U").unwrap(), 3);
        assert_eq!(s.eval::<i64>("return CombatLogGetNumEntries()").unwrap(), 1);
        assert_eq!(
            s.eval::<i64>("return CombatLogGetNumEntries(true)")
                .unwrap(),
            3
        );
        assert!(
            s.run("CombatLogAddFilter(nil, 0x1)").is_err(),
            "incomplete srcMask"
        );
        // Cursor over admitted entries; ignoreFilter walks them all.
        assert!(s
            .eval::<Option<i64>>("return CombatLogSetCurrentEntry(1)")
            .unwrap()
            .is_some());
        assert_eq!(
            s.eval::<String>("return select(10, CombatLogGetCurrentEntry())")
                .unwrap(),
            "Fireball"
        );
        assert!(
            s.eval::<Option<i64>>("return CombatLogAdvanceEntry(1)")
                .unwrap()
                .is_none(),
            "past the last admitted entry"
        );
        assert!(s
            .eval::<Option<i64>>("return CombatLogSetCurrentEntry(0, true)")
            .unwrap()
            .is_some());
        assert_eq!(
            s.eval::<String>("return select(10, CombatLogGetCurrentEntry())")
                .unwrap(),
            "Heal"
        );
        assert!(s
            .eval::<Option<i64>>("return CombatLogAdvanceEntry(-1, true)")
            .unwrap()
            .is_some());
        assert_eq!(
            s.eval::<String>("return select(10, CombatLogGetCurrentEntry())")
                .unwrap(),
            "Bite"
        );
        // A GUID filter on the destination and a spell-name filter.
        s.run(
            "CombatLogResetFilter() CombatLogAddFilter(nil, nil, '0xF130000000000001', 'fireball')",
        )
        .unwrap();
        assert_eq!(s.eval::<i64>("return CombatLogGetNumEntries()").unwrap(), 1);
        // Names split on spaces too (`" ,"`), a mask truncates, and spell id 0 is any spell.
        let num = "return CombatLogGetNumEntries()";
        s.run("CombatLogResetFilter() CombatLogAddFilter('SPELL_HEAL  SWING_DAMAGE')")
            .unwrap();
        assert_eq!(s.eval::<i64>(num).unwrap(), 1, "the heal");
        s.run(&format!(
            "CombatLogResetFilter() CombatLogAddFilter(nil, {MINE_PLAYER}.9, nil, 0.5)"
        ))
        .unwrap();
        assert_eq!(s.eval::<i64>(num).unwrap(), 2, "my two, whatever the spell");
    }

    #[test]
    fn retention_prunes_by_arrival_and_clear_empties() {
        let mut s = vm();
        s.run("CombatLogSetRetentionTime(5)").unwrap();
        assert_eq!(
            s.eval::<i64>("return CombatLogGetRetentionTime()").unwrap(),
            5
        );
        s.push_wrath_combat_log(entry(1, MINE_PLAYER, HOSTILE_NPC, None));
        s.set_now(16.0);
        s.push_wrath_combat_log(entry(1, MINE_PLAYER, HOSTILE_NPC, None));
        assert_eq!(s.eval::<i64>("return CombatLogGetNumEntries()").unwrap(), 1);
        s.run("CombatLogClearEntries()").unwrap();
        assert_eq!(s.eval::<i64>("return CombatLogGetNumEntries()").unwrap(), 0);
        assert_eq!(
            s.eval::<i64>("return select('#', CombatLogGetCurrentEntry())")
                .unwrap(),
            0
        );
    }

    #[test]
    fn object_is_a_needs_every_group_or_a_special_bit() {
        let s = vm();
        assert!(s
            .eval::<Option<i64>>(&format!(
                "return CombatLog_Object_IsA({MINE_PLAYER}, 0xFFFF)"
            ))
            .unwrap()
            .is_some());
        assert!(s
            .eval::<Option<i64>>(&format!("return CombatLog_Object_IsA({MINE_PLAYER}, 0x1)"))
            .unwrap()
            .is_none());
        assert!(s
            .eval::<Option<i64>>("return CombatLog_Object_IsA(0x10000, 0x10000)")
            .unwrap()
            .is_some());
    }
}
