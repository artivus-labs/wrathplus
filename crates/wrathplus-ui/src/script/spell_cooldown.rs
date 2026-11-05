//! `GetSpellCooldown`'s answer for a spell id in neither book: `0x809000` asks the player's
//! cooldown list through `0x807980`, which reads each record's spell, category and GCD legs. The
//! app pushes the list's legs ([`CooldownLeg`]); the catalog carries each spell's keys into them
//! ([`SpellCooldownFacts`]). Labelled: `0x807980`'s fourth leg (`0x807beb`-`0x807cfe`, through
//! `0x5ee7e0`) is not built, a GCD leg starts at its own timer where the reference takes the
//! record's start (`0x807be1`), and the enable is the winning leg's where the reference only
//! clears it (`0x807b15`, `0x807ba6`), so a held leg that led keeps it 0.

use super::UiScript;

/// The key one leg of a cooldown record answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CooldownKey {
    /// The record's spell, cast without an item: a query for that spell.
    Spell(u32),
    /// The record's category: a query whose spell has it.
    Category(u32),
    /// A wildcard category (`SpellCategory` flags `0x2`): any query.
    AnyCategory,
    /// The record's GCD category: a query whose `StartRecoveryCategory` equals it.
    Gcd(u32),
}

/// One leg on the `GetTime` clock. A held record's spell and category legs are not `enabled` and
/// carry their full duration, which ranks them; [`player_cooldown`] answers them from now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CooldownLeg {
    pub key: CooldownKey,
    pub start_ms: i64,
    pub duration_ms: u32,
    pub enabled: bool,
}

/// A spell's `Spell.dbc` keys into the legs: `Category`, `StartRecoveryCategory`, and whether
/// the query reads cold whatever runs (`Effect[0]` ATTACK or TRADE_SKILL, `0x8079cb`, `0x8079d4`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpellCooldownFacts {
    pub category: u32,
    pub start_recovery_category: u32,
    pub excluded: bool,
}

/// The player's cooldown legs, as last pushed.
#[derive(Default)]
struct PlayerCooldowns(Vec<CooldownLeg>);

impl UiScript {
    /// Replace the player's cooldown legs.
    pub fn set_player_cooldown_legs(&mut self, legs: Vec<CooldownLeg>) {
        self.lua.set_app_data(PlayerCooldowns(legs));
    }
}

impl CooldownKey {
    fn reads(self, spell: u32, f: &SpellCooldownFacts) -> bool {
        match self {
            Self::Spell(s) => s == spell,
            Self::Category(c) => c != 0 && c == f.category,
            Self::AnyCategory => true,
            Self::Gcd(g) => g == f.start_recovery_category,
        }
    }
}

/// The leg a query for `spell` reads with the most time left at `now_ms`, as
/// `(start_ms, duration_ms, enabled)`; `None` when none runs. A held leg ranks by its full
/// duration, a spell leg's one longer (`0x807ad4`), and answers now (`0x807b04`, `0x807b95`) and
/// disabled: the spell leg 1 ms long (`0x807af2`), the category leg its full duration.
pub(super) fn player_cooldown(
    lua: &mlua::Lua,
    spell: u32,
    f: &SpellCooldownFacts,
    now_ms: i64,
) -> Option<(i64, u32, bool)> {
    let legs = lua.app_data_ref::<PlayerCooldowns>()?;
    if f.excluded {
        return None;
    }
    let mut best: Option<(u32, CooldownLeg)> = None;
    for &leg in legs.0.iter().filter(|l| l.key.reads(spell, f)) {
        let spell_leg = matches!(leg.key, CooldownKey::Spell(_));
        let left = match leg.enabled {
            true => leg.start_ms + i64::from(leg.duration_ms) - now_ms,
            false => i64::from(leg.duration_ms) + i64::from(spell_leg),
        }
        .clamp(0, i64::from(u32::MAX)) as u32;
        if left > best.map_or(0, |b| b.0) {
            best = Some((left, leg));
        }
    }
    best.map(|(_, l)| match (l.enabled, l.key) {
        (true, _) => (l.start_ms, l.duration_ms, true),
        (false, CooldownKey::Spell(_)) => (now_ms, 1, false),
        (false, _) => (now_ms, l.duration_ms, false),
    })
}
