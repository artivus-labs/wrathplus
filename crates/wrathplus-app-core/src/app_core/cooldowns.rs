//! The spell cooldown store every layer reads: the action bars, the pet bar and the cast gates.
//! `spell::cooldowns` keeps its systems and re-exports these from its old path. Build 12340's
//! addresses here are `0xd3f5ac`, `0x5d6b90` and the `0x80…` ones; every other is 1.12's.

use std::time::{Duration, Instant};

use wrathplus_formats::SpellDisplay;
use wrathplus_protocol::types::ItemUseSpell;

use bevy::prelude::*;

/// One timer; a zero duration is untracked.
#[derive(Clone, Copy, Debug)]
pub struct Timer {
    pub start: Instant,
    pub duration: Duration,
}

impl Timer {
    pub fn none(now: Instant) -> Self {
        Self {
            start: now,
            duration: Duration::ZERO,
        }
    }

    pub(crate) fn remaining(&self, now: Instant) -> Duration {
        (self.start + self.duration).saturating_duration_since(now)
    }
}

/// One `SpellHistory` node (12340's list `0xd3f5ac`).
#[derive(Clone, Debug)]
pub struct Record {
    pub spell_id: u32,
    /// The cast item's entry, 0 for a spell: records match on the spell and item pair.
    pub item_id: u32,
    pub recovery: Timer,
    pub category: u32,
    /// The category's `SpellCategory` row has flags bit 0x2, so it matches every query
    /// (`0x807b63` tests the row's `0x2`); wand Shoot's 351 is the only one.
    pub category_wildcard: bool,
    pub category_recovery: Timer,
    /// The recovery timers hold their durations but have not started.
    pub on_hold: bool,
    pub gcd_category: u32,
    pub gcd: Timer,
}

/// One cooldown list, the client's `SpellHistory`: the player's is this resource (`0xcecaec`), the
/// pet's is [`crate::ui_pet::PetBar::cooldowns`] (`0xcecb04`), which the pet bar, the pet book and
/// the pet's `SMSG_SPELL_GO` leg read and write. Each list's edge feeds its own events: the
/// player's `feed_action_state`, the pet's `ui_pet::bar::fire_pet_cooldown_events`.
#[derive(Resource, Default)]
pub struct Cooldowns {
    pub records: Vec<Record>,
    /// Bumped on every mutation the reference flushes (`0x4b31b0`), the flush's edge. Only counts
    /// up, across [`Self::clear_silent`] too, so a watcher reads a reset as no mutation.
    pub generation: u64,
    /// Bumped when [`Self::prune`] removes records. Kept apart from [`Self::generation`]: gated
    /// feeds must see an expiry, but the reference never fires the event for one.
    pub(crate) expiry_epoch: u64,
}

impl Cooldowns {
    pub fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// How many records the list holds, elapsed ones included.
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// The counter a gated feed watches: moves on any mutation or pruned expiry.
    pub fn feed_epoch(&self) -> u64 {
        self.generation.wrapping_add(self.expiry_epoch)
    }

    /// An elapsed record awaits its prune. The feeds run before `feed_action_state`'s prune, so
    /// on the frame a timer crosses zero the read changes a frame before [`Self::feed_epoch`].
    pub fn sweep_pending(&self, now: Instant) -> bool {
        self.records.iter().any(|r| {
            !r.on_hold
                && r.recovery.remaining(now).is_zero()
                && r.category_recovery.remaining(now).is_zero()
                && r.gcd.remaining(now).is_zero()
        })
    }

    /// `AddCooldown` (1.12 `0x6e12c0`; 12340 not located): always appends, never matching by id, so
    /// one spell holds separate nodes for its cast-send GCD and its SPELL_GO recovery. Replacing by
    /// id would let the GO insert wipe the running GCD.
    ///
    /// The list only grows here, so this is where it is bounded: the records that have run out by
    /// `now` are dropped first ([`Self::prune`]), and a list holds its live records plus those
    /// that ran out since its last insert. The pet's list has no other pruner.
    pub fn add(
        &mut self,
        now: Instant,
        spell_id: u32,
        item_id: u32,
        recovery: Timer,
        category: u32,
        category_wildcard: bool,
        category_recovery: Timer,
        on_hold: bool,
        gcd_category: u32,
        gcd: Timer,
    ) {
        // The client's early-out when nothing is tracked (1.12 `0x6e12c3`).
        if recovery.duration.is_zero()
            && category_recovery.duration.is_zero()
            && !on_hold
            && gcd.duration.is_zero()
        {
            return;
        }
        self.prune(now);
        self.records.push(Record {
            spell_id,
            item_id,
            recovery,
            category,
            category_wildcard,
            category_recovery,
            on_hold,
            gcd_category,
            gcd,
        });
        self.bump();
    }

    /// Drop fully elapsed records that are not on hold; invisible to reads, it stands in for the
    /// client's event-driven sweeps. Bumps [`Self::expiry_epoch`], not [`Self::generation`], so it
    /// flushes no event. [`Self::add`] runs it at every insert, and the player's feed each frame,
    /// which the gated feeds need to see an expiry with no insert.
    pub fn prune(&mut self, now: Instant) {
        let before = self.records.len();
        self.records.retain(|r| {
            r.on_hold
                || !r.recovery.remaining(now).is_zero()
                || !r.category_recovery.remaining(now).is_zero()
                || !r.gcd.remaining(now).is_zero()
        });
        if self.records.len() != before {
            self.expiry_epoch = self.expiry_epoch.wrapping_add(1);
        }
    }

    /// An item use's cooldown (`StartCooldown 0x6e2c60`): the item's values, a negative one
    /// falling back to the spell's `Spell.dbc` value.
    pub fn start_item(
        &mut self,
        item_entry: u32,
        use_spell: &ItemUseSpell,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) {
        let recovery_ms = if use_spell.cooldown_ms >= 0 {
            use_spell.cooldown_ms as u32
        } else {
            spell.map_or(0, |s| s.recovery_ms)
        };
        let category_ms = if use_spell.category_cooldown_ms >= 0 {
            use_spell.category_cooldown_ms as u32
        } else {
            spell.map_or(0, |s| s.category_recovery_ms)
        };
        self.add(
            now,
            use_spell.spell_id,
            item_entry,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(recovery_ms)),
            },
            use_spell.category,
            // Resolved through the spell only when the categories agree; no item carries 351.
            spell.is_some_and(|s| s.category == use_spell.category && s.category_wildcard),
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(category_ms)),
            },
            spell.is_some_and(|s| s.cooldown_on_event()),
            0,
            Timer::none(now),
        );
    }

    /// Clear the GCD of a spell's records (1.12 `0x6e1630`; 12340 not located), on a failed cast.
    pub fn clear_gcd(&mut self, spell_id: u32, now: Instant) {
        let mut touched = false;
        for r in &mut self.records {
            if r.spell_id == spell_id && !r.gcd.duration.is_zero() {
                // 1.12's `0x6e1630` zeroes both the category and the time.
                r.gcd_category = 0;
                r.gcd = Timer::none(now);
                touched = true;
            }
        }
        if touched {
            if wrathplus_assets::trace::enabled() {
                wrathplus_assets::trace::line(
                    "cd",
                    &format!("clear-gcd spell={spell_id} (cast-fail)"),
                );
            }
            self.prune(now);
            self.bump();
        }
    }

    /// `SMSG_COOLDOWN_EVENT` (handler `0x804010`; 1.12 `0x6e1790`, force 0): an on-hold record's
    /// timers start now.
    pub fn cooldown_event(&mut self, spell_id: u32, now: Instant) {
        let mut touched = false;
        for r in &mut self.records {
            if r.spell_id == spell_id && r.on_hold {
                r.recovery.start = now;
                r.category_recovery.start = now;
                r.on_hold = false;
                touched = true;
            }
        }
        if touched {
            self.bump();
        }
    }

    /// `SMSG_CLEAR_COOLDOWN` (handler `0x804010`) and the cast-fail revert (1.12 `0x6e1790` with
    /// force 1).
    pub fn clear_spell(&mut self, spell_id: u32) {
        let before = self.records.len();
        self.records.retain(|r| r.spell_id != spell_id);
        if self.records.len() != before {
            self.bump();
        }
    }

    /// `SMSG_COOLDOWN_CHEAT` (`0x6e9700`).
    pub fn wipe(&mut self) {
        if !self.records.is_empty() {
            self.records.clear();
            self.bump();
        }
    }

    /// Empty the list with no flush: `SMSG_PET_SPELLS` clears the pet's list (`0x6e9b60` →
    /// `0x6e1880`, 1.12; 12340 not located) before it seeds it again, or for a teardown leaves it
    /// empty, and the packet calls no `0x4b31b0`. The counters keep their place, so a watcher never
    /// reads the reset as a mutation.
    pub fn clear_silent(&mut self) {
        self.records.clear();
    }

    /// Session end ([`crate::net::session::disconnected`]): `SMSG_INITIAL_SPELLS` re-sends every
    /// running cooldown at world entry and [`Self::seed_initial`] appends, so a surviving record
    /// would outlive the fresh one.
    pub fn clear_session(&mut self) {
        self.wipe();
    }

    /// One `SMSG_SPELL_COOLDOWN` entry (handler `0x806dd0`; 1.12 `0x6e9460`): a nonzero duration is
    /// the recovery with no category timer; zero means the spell's own `Spell.dbc` pair. An
    /// on-event spell parks with no GCD; any other carries its GCD.
    pub fn apply_wire_cooldown(
        &mut self,
        spell_id: u32,
        cooldown_ms: u32,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) {
        let on_hold = spell.is_some_and(|s| s.cooldown_on_event());
        let (recovery_ms, category, category_ms) = if cooldown_ms != 0 {
            (cooldown_ms, spell.map_or(0, |s| s.category), 0)
        } else {
            match spell {
                Some(s) => (s.recovery_ms, s.category, s.category_recovery_ms),
                None => (0, 0, 0),
            }
        };
        let (gcd_category, gcd_ms) = if on_hold {
            (0, 0)
        } else {
            match spell {
                Some(s) => (s.start_recovery_category, s.start_recovery_ms),
                None => (0, 0),
            }
        };
        self.add(
            now,
            spell_id,
            0,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(recovery_ms)),
            },
            category,
            spell.is_some_and(|s| s.category == category && s.category_wildcard),
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(category_ms)),
            },
            on_hold,
            gcd_category,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(gcd_ms)),
            },
        );
    }

    /// A 3.3.5 cast of ours went off: its `Spell.dbc` recovery and category recovery start now,
    /// or an on-event spell parks. The GCD is the cast send's ([`Self::start_gcd`]), so none here.
    pub fn start_cast_recovery(&mut self, spell_id: u32, spell: &SpellDisplay, now: Instant) {
        let on_hold = spell.cooldown_on_event();
        self.add(
            now,
            spell_id,
            0,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(spell.recovery_ms)),
            },
            spell.category,
            spell.category_wildcard,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(spell.category_recovery_ms)),
            },
            on_hold,
            0,
            Timer::none(now),
        );
    }

    /// `SMSG_ITEM_COOLDOWN` (`0x807060`): the client's hardcoded 30 s (`0x807183`) on the item's
    /// on-use spell.
    pub fn apply_wire_item_cooldown(&mut self, item_entry: u32, spell_id: u32, now: Instant) {
        self.add(
            now,
            spell_id,
            item_entry,
            Timer {
                start: now,
                duration: Duration::from_millis(30_000),
            },
            0,
            false,
            Timer::none(now),
            false,
            0,
            Timer::none(now),
        );
    }

    /// One `SMSG_INITIAL_SPELLS` cooldown: the wire carries the remainder, so the record starts
    /// now. A permanent cooldown (the category word's top bit) arrives as 1 ms and is kept.
    pub fn seed_initial(&mut self, cd: &wrathplus_protocol::types::SpellCooldown, now: Instant) {
        self.add(
            now,
            cd.spell_id,
            u32::from(cd.item_id),
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(cd.spell_cd_ms)),
            },
            u32::from(cd.category & 0x7FFF),
            // No catalog here; the wildcard category 351 never arrives in this list.
            false,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(cd.category_cd_ms)),
            },
            false,
            0,
            Timer::none(now),
        );
    }

    /// One `SMSG_PET_SPELLS` cooldown: remainders, starting now. The category duration's
    /// [`PET_COOLDOWN_PERMANENT`] marker is stripped, or it would read as a 37-hour sweep.
    ///
    /// The handler seeds through `StartCooldown 0x6e2c60` (`0x4bdabf`), which calls no flush, so
    /// the insert leaves [`Self::generation`] where it was.
    pub fn seed_pet(
        &mut self,
        cd: &wrathplus_protocol::types::PetSpellCooldown,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) {
        use wrathplus_protocol::types::PET_COOLDOWN_PERMANENT;
        let category_ms = cd.category_cd_ms & !PET_COOLDOWN_PERMANENT;
        let generation = self.generation;
        self.add(
            now,
            cd.spell_id,
            0,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(cd.spell_cd_ms)),
            },
            u32::from(cd.category),
            // Off the catalog when present, else not a wildcard.
            spell.is_some_and(|s| s.category_wildcard),
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(category_ms)),
            },
            false,
            0,
            Timer::none(now),
        );
        self.generation = generation;
    }

    /// Build 12340 PET receive 0x5d6b90 -> setter 0x805b10: category bit 31
    /// is the held byte, not Vanilla's permanent marker. Keep the packet unchanged.
    pub fn seed_wrath_pet(
        &mut self,
        cd: &wrathplus_protocol::wrath_pet::Cooldown,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) {
        let generation = self.generation;
        self.add(
            now,
            cd.spell,
            0,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(cd.cooldown_ms)),
            },
            u32::from(cd.category),
            spell.is_some_and(|s| s.category_wildcard),
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(cd.category_cooldown_ms & 0x7fff_ffff)),
            },
            cd.category_cooldown_ms & 0x8000_0000 != 0,
            0,
            Timer::none(now),
        );
        self.generation = generation;
    }

    /// Native getter 0x807980: a held matching spell reads NOW, 1ms, disabled,
    /// independent of its stored duration. This must not change Vanilla reads.
    pub fn wrath_pet_info(
        &self,
        spell_id: u32,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) -> CooldownInfo {
        if self
            .records
            .iter()
            .any(|r| r.spell_id == spell_id && r.item_id == 0 && r.on_hold)
        {
            return CooldownInfo {
                start: now,
                remaining_ms: 1,
                duration_ms: 1,
                enabled: false,
            };
        }
        self.info(spell_id, 0, spell, now)
    }

    /// The client's `IsSpellOnCooldown` (1.12 `0x6e1690`; 12340 not located), which despite its
    /// name is true only for an on-hold record; it feeds the usable check's grey-while-parked leg
    /// (`0x6e3fb1`).
    ///
    /// Keyed on spell and item (1.12 `0x6e173f`): the action bar's item gate `0x6e2fc0` passes the
    /// item entry (`0x6e3037`), every other caller 0. `category` is the item's for an item
    /// (`0x6e16f6` takes it from the item's spell slot), else the spell's.
    pub fn has_on_hold_record(&self, spell_id: u32, item_entry: u32, category: u32) -> bool {
        self.records.iter().any(|r| {
            r.on_hold
                && ((r.spell_id == spell_id && r.item_id == item_entry)
                    || (category != 0
                        && r.category == category
                        && !r.category_recovery.duration.is_zero()))
        })
    }

    /// The cast validator's first check (`0x6094f0` at `0x609565`, via `0x6e2ea0`): refused while
    /// [`Self::info`] reads any remainder, so a press sharing the running GCD's category is
    /// refused whatever its own `startRecoveryTime`. The caller picks the reason (`0x60952b`):
    /// 0x28 for an item, 0x3c for a spell. Refusing locally matters: the server's NOT_READY
    /// failure would clear the running GCD.
    pub fn not_ready(
        &self,
        spell_id: u32,
        item_entry: u32,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) -> bool {
        self.info(spell_id, item_entry, spell, now).remaining_ms > 0
    }

    /// `GetCooldownInfo 0x807980`: the longest remainder over every record's three legs.
    ///
    /// - `Effect[0]` of ATTACK or TRADE_SKILL always reads cold (`0x8079cb`).
    /// - Spell leg: id and item match; a parked record reads its full duration, disabled.
    /// - Category leg: equal category, or any query for a wildcard row; parked likewise.
    /// - GCD leg (`0x807bba`): the node's GCD category equals the query's
    ///   `startRecoveryCategory` and the node's time is nonzero; it ignores on-hold.
    pub fn info(
        &self,
        spell_id: u32,
        item_entry: u32,
        spell: Option<&SpellDisplay>,
        now: Instant,
    ) -> CooldownInfo {
        let mut best = CooldownInfo {
            start: now,
            remaining_ms: 0,
            duration_ms: 0,
            enabled: true,
        };
        if spell.is_some_and(|s| s.cooldown_query_excluded()) {
            return best;
        }
        let category = spell.map_or(0, |s| s.category);
        let start_recovery_category = spell.map_or(0, |s| s.start_recovery_category);
        let mut consider = |timer: &Timer, remaining: Duration, enabled: bool| {
            let remaining_ms = remaining.as_millis().min(u128::from(u32::MAX)) as u32;
            if remaining_ms > best.remaining_ms {
                best = CooldownInfo {
                    start: timer.start,
                    remaining_ms,
                    duration_ms: timer.duration.as_millis().min(u128::from(u32::MAX)) as u32,
                    enabled,
                };
            }
        };
        for r in &self.records {
            if r.spell_id == spell_id && r.item_id == item_entry {
                if r.on_hold {
                    // Parked: full duration, disabled (enable 0 hides the sweep).
                    consider(&r.recovery, r.recovery.duration, false);
                } else {
                    consider(&r.recovery, r.recovery.remaining(now), true);
                }
            }
            if (r.category != 0 && r.category == category) || r.category_wildcard {
                if r.on_hold {
                    consider(&r.category_recovery, r.category_recovery.duration, false);
                } else {
                    consider(
                        &r.category_recovery,
                        r.category_recovery.remaining(now),
                        true,
                    );
                }
            }
            if r.gcd_category == start_recovery_category && !r.gcd.duration.is_zero() {
                consider(&r.gcd, r.gcd.remaining(now), true);
            }
        }
        best
    }
}

/// One action's cooldown read (`GetActionCooldown`): the winning timer's start, duration and
/// remainder, and `enabled == false` for an on-hold record, which `CooldownFrame_SetTimer`
/// hides. The start is what `GetCooldownInfo 0x807980` returns, so two arms never alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CooldownInfo {
    /// For an on-hold record, when it was inserted.
    pub start: Instant,
    pub remaining_ms: u32,
    pub duration_ms: u32,
    pub enabled: bool,
}

impl CooldownInfo {
    /// `(start_ms on the GetTime clock, duration_ms, enabled)`, or `None` when cold. `anchor` and
    /// `ui_now` must be the frame's one clock pair ([`crate::ui_script::UiClock`]), so one arm
    /// derives the same start every frame; a fresh `Instant::now()` here would jitter it.
    pub fn ui_triple(&self, anchor: Instant, ui_now: f64) -> Option<(i64, u32, bool)> {
        (self.remaining_ms > 0).then(|| {
            // Signed both ways: a timer armed after the anchor sample projects forward.
            let start = match self.start.checked_duration_since(anchor) {
                Some(ahead) => ui_now + ahead.as_secs_f64(),
                None => ui_now - anchor.duration_since(self.start).as_secs_f64(),
            };
            #[allow(clippy::cast_possible_truncation)] // session-clock ms fit i64
            (
                (start * 1000.0).round() as i64,
                self.duration_ms,
                self.enabled,
            )
        })
    }
}
