//! The player's cooldowns, the client's `SpellHistory` list (`0xd3f5ac`, getter `0x807980`;
//! handlers `0x806dd0`, `0x807060`, `0x804010`). Other `0x6e…` addresses here are 1.12's.
//!
//! - A record holds three timers: the spell's recovery, its category's recovery and the GCD
//!   (`startRecoveryCategory`, `startRecoveryTime`); `on_hold` parks the first two until
//!   `SMSG_COOLDOWN_EVENT` (`SPELL_ATTR_COOLDOWN_ON_EVENT`: Stealth, Feign Death).
//! - The GCD starts at cast send (`0x6e58fb`); the spell's recovery is computed from `Spell.dbc`
//!   when our own `SMSG_SPELL_GO` arrives (`0x6e8498`, `0x6e8566`). vmangos sends no cooldown
//!   packet for a plain cast; `SMSG_SPELL_COOLDOWN` is its override path.
//! - A failed cast clears the GCD armed at send (`0x6e1d83`, `0x6e1630`) and, unless the reason
//!   is 0x3c, removes a cooldown-on-event spell's parked record (`0x6e73cc`).
//!
//! Every mutation the reference flushes bumps [`Cooldowns::generation`], the edge of the flush
//! `0x4b31b0` makes (`ACTIONBAR_UPDATE_COOLDOWN`, `SPELL_UPDATE_COOLDOWN`); a natural expiry does
//! not, since the stock `Cooldown.lua` frame hides itself at the end.

use std::time::{Duration, Instant};
#[cfg(test)]
use wrathplus_protocol::types::ItemUseSpell;

use bevy::prelude::*;

use super::{SpellModifiers, OP_GCD};
use wrathplus_formats::SpellDisplay;

pub(crate) use crate::app_core::cooldowns::{Cooldowns, Timer};

/// `Cooldowns`'s methods that need this module (the type lives in `app_core` for the crate split).
pub trait CooldownsSpell {
    /// A spell's own cooldown from `Spell.dbc` (`StartCooldown 0x6e2c60`), without a GCD.
    ///
    /// `ranged_attack_time_ms` is `UNIT_FIELD_RANGEDATTACKTIME` when
    /// [`SpellDisplay::ranged_speed_cooldown`], else 0, added to the category timer (`0x6e2b60`):
    /// the Throw and wand Shoot sweep. Category 0 (Auto Shot) never surfaces it. `mods` applies
    /// op 11 to both timers, as the self and pet legs do; `None` inserts the DBC values.
    fn start_spell(
        &mut self,
        spell_id: u32,
        spell: &SpellDisplay,
        ranged_attack_time_ms: u32,
        now: Instant,
        mods: Option<&SpellModifiers>,
    );
    /// Arm the GCD at cast send (`StartGlobalCooldown 0x6e2de0`, from `0x6e58fb`) for any cast or
    /// item use, and at a pet-bar spell press (`0x4bd367`, into the pet's list), unless
    /// `startRecoveryCategory` and `startRecoveryTime` are both 0
    /// (`0x6e2e0f`-`0x6e2e21`), then with op 21 applied (`0x6e2e2c`) and nothing armed for a zero
    /// result (`0x6e2e3d`). A negative one arms nothing either, where the reference inserts a
    /// record no reader shows. `on_hold` is Attributes bit 25.
    fn start_gcd(
        &mut self,
        spell_id: u32,
        spell: &SpellDisplay,
        now: Instant,
        mods: &SpellModifiers,
    );
}

impl CooldownsSpell for Cooldowns {
    fn start_spell(
        &mut self,
        spell_id: u32,
        spell: &SpellDisplay,
        ranged_attack_time_ms: u32,
        now: Instant,
        mods: Option<&SpellModifiers>,
    ) {
        let (recovery_ms, category_ms) = mods.map_or_else(
            || {
                (
                    spell.recovery_ms,
                    spell
                        .category_recovery_ms
                        .saturating_add(ranged_attack_time_ms),
                )
            },
            |mods| mods.spell_cooldowns(spell, ranged_attack_time_ms),
        );
        self.add(
            now,
            spell_id,
            0,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(recovery_ms)),
            },
            spell.category,
            spell.category_wildcard,
            Timer {
                start: now,
                duration: Duration::from_millis(u64::from(category_ms)),
            },
            spell.cooldown_on_event(),
            0,
            Timer::none(now),
        );
    }

    fn start_gcd(
        &mut self,
        spell_id: u32,
        spell: &SpellDisplay,
        now: Instant,
        mods: &SpellModifiers,
    ) {
        if spell.start_recovery_category == 0 && spell.start_recovery_ms == 0 {
            return;
        }
        // `StartGlobalCooldown 0x6e2de0`: op 21 before the zero-duration exit (`0x6e2e2c`).
        let duration_ms = mods.apply(spell, OP_GCD, spell.start_recovery_ms as i32);
        if duration_ms <= 0 {
            return;
        }
        if wrathplus_assets::trace::enabled() {
            wrathplus_assets::trace::line(
                "cd",
                &format!(
                    "arm-gcd spell={spell_id} gcdcat={} dur={}ms (cast-send)",
                    spell.start_recovery_category, duration_ms
                ),
            );
        }
        self.add(
            now,
            spell_id,
            0,
            Timer::none(now),
            0,
            false,
            Timer::none(now),
            spell.cooldown_on_event(),
            spell.start_recovery_category,
            Timer {
                start: now,
                duration: Duration::from_millis(duration_ms as u64),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(
        category: u32,
        recovery_ms: u32,
        category_recovery_ms: u32,
        gcd: (u32, u32),
        attributes: u32,
    ) -> SpellDisplay {
        SpellDisplay {
            category,
            recovery_ms,
            category_recovery_ms,
            start_recovery_category: gcd.0,
            start_recovery_ms: gcd.1,
            attributes,
            ..Default::default()
        }
    }

    /// Fireball-shaped: no own cooldown, the ordinary 133/1500 GCD.
    fn fireball() -> SpellDisplay {
        spell(0, 0, 0, (133, 1500), 0x10000)
    }

    #[test]
    fn gcd_uses_op_21_before_arming() {
        let mut d = fireball();
        d.spell_family = 3;
        d.spell_family_flags = 1 << 5;
        let mut mods = SpellModifiers::default();
        mods.set_class_family(3);
        mods.set(false, 5, OP_GCD, -50);
        let now = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_gcd(133, &d, now, &mods);
        assert_eq!(cds.info(133, 0, Some(&d), now).duration_ms, 750);

        d.start_recovery_ms = 0;
        mods.set(true, 5, OP_GCD, 1000);
        let mut cds = Cooldowns::default();
        cds.start_gcd(133, &d, now, &mods);
        assert_eq!(cds.info(133, 0, Some(&d), now).duration_ms, 500);
    }

    #[test]
    fn spell_cooldown_applies_op_11_to_both_timers_after_the_ranged_pad() {
        let mut d = spell(44, 8_000, 5_000, (0, 0), 0);
        d.spell_family = 3;
        d.spell_family_flags = 1 << 5;
        let mut mods = SpellModifiers::default();
        mods.set_class_family(3);
        mods.set(true, 5, super::super::OP_COOLDOWN, -1_500);
        assert_eq!(mods.spell_cooldowns(&d, 0), (6_500, 3_500));
        assert_eq!(mods.spell_cooldowns(&d, 2_000), (6_500, 5_500));

        let now = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_spell(2136, &d, 2_000, now, Some(&mods));
        assert_eq!(cds.info(2136, 0, Some(&d), now).duration_ms, 6_500);
        let sibling = spell(44, 0, 0, (0, 0), 0);
        assert_eq!(cds.info(999, 0, Some(&sibling), now).duration_ms, 5_500);
        let mut cds = Cooldowns::default();
        cds.start_spell(2136, &d, 2_000, now, None);
        assert_eq!(cds.info(2136, 0, Some(&d), now).duration_ms, 8_000);
        assert_eq!(cds.info(999, 0, Some(&sibling), now).duration_ms, 7_000);

        // Endurance 2/2: Sprint's five-minute recovery loses 30%.
        d.recovery_ms = 300_000;
        d.category_recovery_ms = 0;
        mods.set(true, 5, super::super::OP_COOLDOWN, 0);
        mods.set(false, 5, super::super::OP_COOLDOWN, -30);
        assert_eq!(mods.spell_cooldowns(&d, 0), (210_000, 0));
    }

    /// Charge-shaped: category 44, 15 s category cooldown, NO GCD pair.
    fn charge() -> SpellDisplay {
        spell(44, 0, 15_000, (0, 0), 0)
    }

    #[test]
    fn the_gcd_spreads_to_every_spell_sharing_the_start_recovery_category() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_gcd(133, &fireball(), t0, &super::SpellModifiers::default());

        // Any spell with the same startRecoveryCategory reads the GCD; Charge has none.
        let mid = t0 + Duration::from_millis(500);
        let fb = cds.info(133, 0, Some(&fireball()), mid);
        assert_eq!(
            (fb.remaining_ms, fb.duration_ms, fb.enabled),
            (1000, 1500, true)
        );
        let frostbolt = fireball(); // same shape, different id
        let other = cds.info(116, 0, Some(&frostbolt), mid);
        assert_eq!((other.remaining_ms, other.duration_ms), (1000, 1500));
        let ch = cds.info(100, 0, Some(&charge()), mid);
        assert_eq!(ch.remaining_ms, 0, "no startRecoveryCategory — no GCD read");

        // The press gate reads the same getter; `0x6e1690` stays false with nothing parked.
        assert!(cds.not_ready(133, 0, Some(&fireball()), mid));
        assert!(!cds.has_on_hold_record(133, 0, fireball().category));
    }

    /// The SPELL_GO insert (`StartCooldown`, no GCD) is its own node beside the cast-send GCD.
    #[test]
    fn a_go_self_insert_never_wipes_the_running_gcd() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        // Frost-Nova-shaped: own 25 s recovery, category 35, the ordinary 133/1500 GCD pair.
        let frost_nova = spell(35, 25_000, 0, (133, 1500), 0);
        cds.start_gcd(122, &frost_nova, t0, &super::SpellModifiers::default()); // the cast-send arm
        cds.start_spell(122, &frost_nova, 0, t0 + Duration::from_millis(100), None); // the GO insert

        let mid = t0 + Duration::from_millis(200);
        // Every GCD sibling still reads the running GCD.
        let fb = cds.info(133, 0, Some(&fireball()), mid);
        assert_eq!(
            (fb.remaining_ms, fb.duration_ms),
            (1300, 1500),
            "the GO self-insert must not eat the GCD node"
        );
        assert!(
            cds.not_ready(133, 0, Some(&fireball()), mid),
            "the local lock holds too"
        );
        // Frost Nova's own button reads the longer, its own cooldown.
        let own = cds.info(122, 0, Some(&frost_nova), mid);
        assert_eq!((own.remaining_ms, own.duration_ms), (24_900, 25_000));
    }

    #[test]
    fn a_failed_cast_clears_the_gcd_and_the_optimistic_recovery() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let fd = spell(0, 30_000, 0, (133, 1500), 0); // Feign-Death-shaped minus on-event
        cds.start_gcd(5384, &fd, t0, &super::SpellModifiers::default());
        cds.start_spell(5384, &fd, 0, t0, None);
        let mid = t0 + Duration::from_millis(100);
        assert!(cds.not_ready(5384, 0, Some(&fd), mid));

        // The 1.12 fail path (0x6e1a00): GCD cleared (0x6e1630), the record removed (0x6e3050).
        cds.clear_gcd(5384, mid);
        cds.clear_spell(5384);
        assert!(!cds.not_ready(5384, 0, Some(&fd), mid));
        assert_eq!(cds.info(5384, 0, Some(&fd), mid).remaining_ms, 0);
    }

    #[test]
    fn category_cooldowns_reach_category_siblings_but_not_others() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_spell(100, &charge(), 0, t0, None); // Charge: category 44, 15 s

        let mid = t0 + Duration::from_secs(5);
        // Another spell in category 44 reads the shared remainder.
        let sibling = spell(44, 0, 15_000, (0, 0), 0);
        let s = cds.info(999, 0, Some(&sibling), mid);
        assert_eq!((s.remaining_ms, s.duration_ms), (10_000, 15_000));
        assert!(
            cds.not_ready(999, 0, Some(&sibling), mid),
            "category lock is a not-ready"
        );
        // An unrelated spell reads nothing.
        assert_eq!(cds.info(133, 0, Some(&fireball()), mid).remaining_ms, 0);
    }

    #[test]
    fn an_on_event_cooldown_parks_until_the_event_starts_it() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        // Feign Death: 30 s recovery, SPELL_ATTR_COOLDOWN_ON_EVENT (bit 25).
        let fd = spell(0, 30_000, 0, (0, 0), 0x0200_0000);
        cds.start_spell(5384, &fd, 0, t0, None);

        // Parked: full duration, disabled, and still not ready.
        let parked = cds.info(5384, 0, Some(&fd), t0 + Duration::from_secs(60));
        assert_eq!(
            (parked.remaining_ms, parked.enabled),
            (30_000, false),
            "an on-hold record never elapses on its own"
        );
        assert!(cds.not_ready(5384, 0, Some(&fd), t0 + Duration::from_secs(60)));
        assert!(
            cds.has_on_hold_record(5384, 0, fd.category),
            "the corrected 0x6e1690: an on-hold record — the usable walk's grey-while-parked"
        );

        // SMSG_COOLDOWN_EVENT starts the clocks now.
        let event_at = t0 + Duration::from_secs(60);
        cds.cooldown_event(5384, event_at);
        let running = cds.info(5384, 0, Some(&fd), event_at + Duration::from_secs(10));
        assert_eq!(
            (running.remaining_ms, running.duration_ms, running.enabled),
            (20_000, 30_000, true)
        );
    }

    #[test]
    fn wire_cooldowns_take_the_server_duration_or_fall_back_to_the_dbc() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        // A school lockout: nonzero wire ms is the recovery verbatim.
        cds.apply_wire_cooldown(133, 8_000, Some(&fireball()), t0);
        let locked = cds.info(133, 0, Some(&fireball()), t0 + Duration::from_secs(3));
        assert_eq!((locked.remaining_ms, locked.duration_ms), (5_000, 8_000));

        // Zero wire ms: the spell's own Spell.dbc recovery/category pair.
        let mut cds = Cooldowns::default();
        cds.apply_wire_cooldown(100, 0, Some(&charge()), t0);
        let ch = cds.info(100, 0, Some(&charge()), t0 + Duration::from_secs(5));
        assert_eq!((ch.remaining_ms, ch.duration_ms), (10_000, 15_000));
    }

    #[test]
    fn a_session_end_empties_the_list_so_the_next_logins_wire_is_the_whole_truth() {
        use wrathplus_protocol::types::SpellCooldown;
        let wire = |ms| SpellCooldown {
            spell_id: 12975, // Last Stand: 10 minutes, no category
            item_id: 0,
            category: 0,
            spell_cd_ms: ms,
            category_cd_ms: 0,
        };
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();

        // Login one: 600 s still to run.
        cds.seed_initial(&wire(600_000), t0);
        assert_eq!(cds.info(12975, 0, None, t0).duration_ms, 600_000);

        // Relog six seconds later: the server now says 594 s.
        let t1 = t0 + Duration::from_secs(6);
        cds.clear_session();
        assert!(cds.records.is_empty(), "the list dies with the session");
        cds.seed_initial(&wire(594_000), t1);

        let read = cds.info(12975, 0, None, t1);
        assert_eq!(
            (cds.records.len(), read.duration_ms, read.remaining_ms),
            (1, 594_000, 594_000),
            "one record, the second login's — a surviving first-login record would answer 600 s \
             and go on answering it for the whole cooldown"
        );
    }

    /// `0x6e1690` matches the spell and item pair; only the item gate `0x6e2fc0` passes an entry.
    #[test]
    fn an_on_hold_record_is_findable_only_under_the_key_that_armed_it() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        // An on-event on-use spell, category 0 so the category leg cannot answer.
        let use_spell = ItemUseSpell {
            spell_id: 5384,
            cooldown_ms: 30_000,
            category: 0,
            category_cooldown_ms: 0,
        };
        let on_event = spell(0, 30_000, 0, (0, 0), 0x0200_0000);
        cds.start_item(1487, &use_spell, Some(&on_event), t0);

        assert!(
            cds.has_on_hold_record(5384, 1487, 0),
            "the ITEM gate's own query — `0x6e2fc0` passes the entry as itemId"
        );
        assert!(
            !cds.has_on_hold_record(5384, 0, 0),
            "the spell-slot form must not reach an item's record: itemId is part of the match"
        );
        assert!(
            !cds.has_on_hold_record(5384, 999, 0),
            "and not another item's entry either"
        );
    }

    #[test]
    fn item_use_cooldowns_key_on_the_item_and_respect_the_wire_triple() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        // A potion: category 4 for 60 s; the use cooldown -1 falls back to the spell's (none).
        let use_spell = ItemUseSpell {
            spell_id: 439,
            cooldown_ms: -1,
            category: 4,
            category_cooldown_ms: 60_000,
        };
        let potion_spell = spell(4, 0, 60_000, (133, 1500), 0);
        cds.start_item(118, &use_spell, Some(&potion_spell), t0);

        let mid = t0 + Duration::from_secs(15);
        // Spell 439 as cast from item 118 reads the category remainder.
        let info = cds.info(439, 118, Some(&potion_spell), mid);
        assert_eq!((info.remaining_ms, info.duration_ms), (45_000, 60_000));
        // So does any other category-4 potion.
        let other = cds.info(440, 929, Some(&potion_spell), mid);
        assert_eq!(other.remaining_ms, 45_000);
    }

    #[test]
    fn prune_drops_only_fully_elapsed_records() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_gcd(133, &fireball(), t0, &super::SpellModifiers::default());
        cds.start_spell(100, &charge(), 0, t0, None);
        assert_eq!(cds.records.len(), 2);

        // Past the 1.5 s GCD, inside Charge's 15 s.
        cds.prune(t0 + Duration::from_secs(5));
        assert_eq!(cds.records.len(), 1);
        assert_eq!(cds.records[0].spell_id, 100);

        cds.prune(t0 + Duration::from_secs(20));
        assert!(cds.records.is_empty());
    }

    /// The list is bounded where it grows: whatever the inserts' spacing, it holds the records
    /// live at the insert plus the insert itself, never every record ever added.
    #[test]
    fn an_insert_bounds_the_list_by_its_live_records_plus_itself() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let mods = super::SpellModifiers::default();
        // Charge's 15 s category timer, then a 1.5 s GCD every 2 s for a minute.
        cds.start_spell(100, &charge(), 0, t0, None);
        for i in 0..30u64 {
            let now = t0 + Duration::from_secs(2 * i);
            cds.start_gcd(133, &fireball(), now, &mods);
            let charge_live = usize::from(2 * i < 15);
            assert_eq!(
                cds.record_count(),
                1 + charge_live,
                "insert {i}: Charge if it still runs, and the new GCD"
            );
        }
    }

    /// An insert drops the records that have run out and only those: a running timer and a parked
    /// record survive, no read of what stays changes, and the insert is the one mutation.
    #[test]
    fn an_insert_keeps_a_running_and_a_parked_record_and_changes_no_read() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let mods = super::SpellModifiers::default();
        let brief = || spell(7, 2_000, 0, (0, 0), 0);
        // Parked: `Attributes` bit 25, its timers wait for `SMSG_COOLDOWN_EVENT`.
        let parked = || spell(9, 1_000, 0, (0, 0), 1 << 25);
        cds.start_gcd(133, &fireball(), t0, &mods);
        cds.start_spell(100, &brief(), 0, t0, None);
        cds.start_spell(200, &charge(), 0, t0, None);
        cds.start_spell(300, &parked(), 0, t0, None);
        assert_eq!(cds.record_count(), 4);

        let now = t0 + Duration::from_secs(5);
        let asks = [
            (133, fireball()),
            (100, brief()),
            (200, charge()),
            (201, spell(44, 0, 0, (0, 0), 0)),
            (300, parked()),
            (301, spell(9, 0, 0, (0, 0), 0)),
            (999, spell(0, 0, 0, (0, 0), 0)),
        ];
        let reads = |cds: &Cooldowns| {
            asks.iter()
                .map(|(id, d)| cds.info(*id, 0, Some(d), now))
                .collect::<Vec<_>>()
        };
        let before = reads(&cds);
        assert_eq!(before[2].remaining_ms, 10_000, "Charge runs");
        assert!(!before[4].enabled, "and the parked spell reads disabled");
        let generation = cds.generation;

        // A spell of no category the others share.
        cds.start_spell(500, &spell(99, 5_000, 0, (0, 0), 0), 0, now, None);

        assert_eq!(
            cds.record_count(),
            3,
            "Charge, the parked record and the new one"
        );
        assert!(!cds.sweep_pending(now), "nothing that ran out is left");
        assert_eq!(
            reads(&cds),
            before,
            "the records that stay read as they did"
        );
        assert_eq!(
            cds.generation,
            generation + 1,
            "one mutation, the insert's own"
        );
    }

    /// An elapsed record answers no read, so a prune cannot change one: what it keeps, a running
    /// timer and a parked record whose own timers have run down, answers as before.
    #[test]
    fn a_prune_changes_no_read() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let mods = super::SpellModifiers::default();
        let brief = || spell(7, 2_000, 0, (0, 0), 0);
        // Parked: `Attributes` bit 25, its timers wait for `SMSG_COOLDOWN_EVENT`.
        let parked = || spell(9, 1_000, 0, (0, 0), 1 << 25);
        cds.start_gcd(133, &fireball(), t0, &mods);
        cds.start_spell(100, &brief(), 0, t0, None);
        cds.start_spell(200, &charge(), 0, t0, None);
        cds.start_spell(300, &parked(), 0, t0, None);
        assert_eq!(cds.records.len(), 4);

        let now = t0 + Duration::from_secs(5);
        let asks = [
            (133, fireball()),
            (100, brief()),
            (200, charge()),
            // Another spell of Charge's category, and one of the parked spell's.
            (201, spell(44, 0, 0, (0, 0), 0)),
            (300, parked()),
            (301, spell(9, 0, 0, (0, 0), 0)),
            (999, spell(0, 0, 0, (0, 0), 0)),
        ];
        let reads = |cds: &Cooldowns| {
            asks.iter()
                .map(|(id, d)| cds.info(*id, 0, Some(d), now))
                .collect::<Vec<_>>()
        };
        let before = reads(&cds);
        assert!(cds.sweep_pending(now), "the GCD and Brief have run out");
        assert_eq!(before[2].remaining_ms, 10_000, "Charge runs");
        assert!(!before[4].enabled, "and the parked spell reads disabled");

        cds.prune(now);

        assert_eq!(cds.records.len(), 2, "the GCD and Brief are gone");
        assert!(!cds.sweep_pending(now));
        assert_eq!(reads(&cds), before);
    }

    /// A prune that removes nothing must not move the epoch, or the per-frame prune would hold
    /// every gated feed open.
    #[test]
    fn the_feed_epoch_moves_on_expiry_but_an_empty_prune_is_silent() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let e0 = cds.feed_epoch();
        cds.start_gcd(133, &fireball(), t0, &super::SpellModifiers::default());
        let armed = cds.feed_epoch();
        assert_ne!(e0, armed, "a mutation moves the epoch (generation)");

        cds.prune(t0 + Duration::from_millis(100));
        assert_eq!(
            cds.feed_epoch(),
            armed,
            "nothing elapsed — the prune is silent"
        );

        cds.prune(t0 + Duration::from_secs(5));
        let expired = cds.feed_epoch();
        assert_ne!(
            armed, expired,
            "the expiry IS a feed edge (the triple flips to None)"
        );
        assert_eq!(
            cds.generation,
            {
                let mut probe = Cooldowns::default();
                probe.start_gcd(133, &fireball(), t0, &super::SpellModifiers::default());
                probe.generation
            },
            "…but the event-edge generation never saw it (the reference is silent on expiry)"
        );

        cds.prune(t0 + Duration::from_secs(6));
        assert_eq!(cds.feed_epoch(), expired, "an empty prune stays silent");
    }

    /// A fail-clear and re-arm between two feeds must still read as a new start.
    #[test]
    fn the_ui_triple_is_stable_per_arm_and_distinct_across_arms() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_gcd(772, &fireball(), t0, &super::SpellModifiers::default());

        // Two reads of one arm, frames apart, with both clocks in lockstep.
        let read1 = cds
            .info(772, 0, Some(&fireball()), t0 + Duration::from_millis(16))
            .ui_triple(t0 + Duration::from_millis(16), 10.016);
        let read2 = cds
            .info(772, 0, Some(&fireball()), t0 + Duration::from_millis(160))
            .ui_triple(t0 + Duration::from_millis(160), 10.160);
        assert_eq!(read1, Some((10_000, 1500, true)));
        assert_eq!(read1, read2, "one arm reads one start, every frame");

        // The fail clears the GCD and a re-press re-arms 200 ms later, unseen by the feed.
        cds.clear_gcd(772, t0 + Duration::from_millis(200));
        cds.start_gcd(
            772,
            &fireball(),
            t0 + Duration::from_millis(200),
            &super::SpellModifiers::default(),
        );
        let rearmed = cds
            .info(772, 0, Some(&fireball()), t0 + Duration::from_millis(216))
            .ui_triple(t0 + Duration::from_millis(216), 10.216);
        assert_eq!(
            rearmed,
            Some((10_200, 1500, true)),
            "a re-arm never aliases"
        );
    }

    #[test]
    fn a_mid_frame_arm_derives_the_same_start_as_the_next_frames_pair() {
        let anchor0 = Instant::now();
        let mut cds = Cooldowns::default();
        // Armed 4 ms after this frame's anchor sample.
        cds.start_gcd(
            133,
            &fireball(),
            anchor0 + Duration::from_millis(4),
            &super::SpellModifiers::default(),
        );

        // Frame 1 converts through the pre-arm pair, frame 2 through the next, 16 ms on.
        let read1 = cds
            .info(
                133,
                0,
                Some(&fireball()),
                anchor0 + Duration::from_millis(4),
            )
            .ui_triple(anchor0, 10.000);
        let read2 = cds
            .info(
                133,
                0,
                Some(&fireball()),
                anchor0 + Duration::from_millis(20),
            )
            .ui_triple(anchor0 + Duration::from_millis(16), 10.016);
        assert_eq!(read1, Some((10_004, 1500, true)));
        assert_eq!(read1, read2, "the projected start IS the settled start");
    }

    /// The GCD leg (`0x807bba`) ignores the pressed spell's own time: a `{133, 0}` scroll press
    /// is refused, a `{0, 0}` press is not.
    #[test]
    fn a_running_gcd_locks_presses_by_category_equality_alone() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        assert!(
            !cds.not_ready(133, 0, Some(&fireball()), t0),
            "no GCD running — nothing locks"
        );

        cds.start_gcd(772, &fireball(), t0, &super::SpellModifiers::default());
        let mid = t0 + Duration::from_millis(200);
        assert!(
            cds.not_ready(133, 0, Some(&fireball()), mid),
            "the spam press 200 ms later is locked — refused, never sent, the GCD lives"
        );
        // Scroll of Armor's shape.
        let scroll = spell(0, 0, 0, (133, 0), 0x10000);
        assert!(
            cds.not_ready(8091, 0, Some(&scroll), mid),
            "a zero-GCD press in the shared category is locked on the reference"
        );
        assert!(!cds.not_ready(100, 0, Some(&charge()), mid));
        assert!(!cds.not_ready(133, 0, Some(&fireball()), t0 + Duration::from_millis(1_501)));
    }

    /// The getter's exclusion (`0x8079cb`, `0x8079d4`).
    #[test]
    fn attack_and_tradeskill_reads_are_always_cold() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        cds.start_gcd(772, &fireball(), t0, &super::SpellModifiers::default());
        let mid = t0 + Duration::from_millis(200);
        let attack = SpellDisplay {
            effects: [78, 0, 0], // SPELL_EFFECT_ATTACK
            start_recovery_category: 133,
            ..Default::default()
        };
        assert_eq!(cds.info(6603, 0, Some(&attack), mid).remaining_ms, 0);
        assert!(!cds.not_ready(6603, 0, Some(&attack), mid));
    }

    /// The wildcard leg (`0x807b63`): wand Shoot's category 351 sweeps every button.
    #[test]
    fn a_wildcard_category_record_reaches_every_query() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        // Shoot: no DBC recovery; the ranged pad is the category duration.
        let shoot = SpellDisplay {
            category: 351,
            category_wildcard: true,
            attributes: 0x50012,
            ..Default::default()
        };
        cds.start_spell(5019, &shoot, 1500, t0, None);
        let mid = t0 + Duration::from_millis(500);
        // Unrelated Fireball reads the wand swing and is not ready for its duration.
        let fb = cds.info(133, 0, Some(&fireball()), mid);
        assert_eq!((fb.remaining_ms, fb.duration_ms), (1000, 1500));
        assert!(cds.not_ready(133, 0, Some(&fireball()), mid));
        assert!(!cds.not_ready(133, 0, Some(&fireball()), t0 + Duration::from_millis(1_501)));
    }

    #[test]
    fn cheat_wipe_and_clear_bump_the_generation_only_on_change() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let g0 = cds.generation;
        cds.clear_spell(133);
        cds.wipe();
        assert_eq!(cds.generation, g0);

        cds.start_gcd(133, &fireball(), t0, &super::SpellModifiers::default());
        assert_ne!(cds.generation, g0);
        let g1 = cds.generation;
        cds.wipe();
        assert_ne!(cds.generation, g1);
        assert_eq!(cds.info(133, 0, Some(&fireball()), t0).remaining_ms, 0);
    }

    /// The ranged pad (`0x6e2b60`): Throw (category 76, no DBC recovery) sweeps the weapon speed.
    #[test]
    fn throw_sweeps_the_ranged_attack_time_via_its_category() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let throw = spell(76, 0, 0, (0, 0), 0x410012);
        cds.start_spell(2764, &throw, 2200, t0, None);
        let mid = t0 + Duration::from_millis(1000);
        let info = cds.info(2764, 0, Some(&throw), mid);
        assert_eq!(info.duration_ms, 2200, "the sweep is the weapon speed");
        assert_eq!(info.remaining_ms, 1200);
        assert!(info.enabled, "running, not parked");
        assert!(cds.not_ready(2764, 0, Some(&throw), mid));
        assert!(
            !cds.not_ready(2764, 0, Some(&throw), t0 + Duration::from_millis(2300)),
            "free again after the weapon speed elapses"
        );
    }

    /// Auto Shot's category 0 has no `SpellCategory` row, so its padded timer never surfaces.
    #[test]
    fn auto_shot_category_zero_never_surfaces_its_pad() {
        let t0 = Instant::now();
        let mut cds = Cooldowns::default();
        let auto_shot = spell(0, 0, 0, (0, 0), 0x50012);
        cds.start_spell(75, &auto_shot, 3200, t0, None);
        let mid = t0 + Duration::from_millis(100);
        assert_eq!(
            cds.info(75, 0, Some(&auto_shot), mid).remaining_ms,
            0,
            "no sweep for a category-0 record"
        );
        assert!(
            !cds.not_ready(75, 0, Some(&auto_shot), mid),
            "no local refusal either"
        );
    }
}

#[cfg(test)]
mod wrath_pet_tests {
    use super::*;
    use wrathplus_protocol::wrath_pet::Cooldown;

    #[test]
    fn native_infinity_is_held_and_reads_current_time_without_losing_raw_marker() {
        let t0 = Instant::now();
        let raw = Cooldown {
            spell: 0x12abcd,
            category: 7,
            cooldown_ms: 1,
            category_cooldown_ms: 0x80000000,
        };
        let mut cds = Cooldowns::default();
        cds.seed_wrath_pet(&raw, None, t0);
        let later = t0 + Duration::from_secs(30);
        cds.prune(later);
        let info = cds.wrath_pet_info(raw.spell, None, later);
        assert_eq!(info.start, later);
        assert_eq!(
            (info.remaining_ms, info.duration_ms, info.enabled),
            (1, 1, false)
        );
        assert_eq!(raw.category_cooldown_ms, 0x80000000);
        assert_eq!(cds.generation, 0);
    }

    #[test]
    fn any_native_category_high_bit_is_held_and_lower_bits_survive() {
        let now = Instant::now();
        let mut cds = Cooldowns::default();
        cds.seed_wrath_pet(
            &Cooldown {
                spell: 123,
                category: 8,
                cooldown_ms: 0,
                category_cooldown_ms: 0x80000042,
            },
            None,
            now,
        );
        assert_eq!(cds.records[0].category_recovery.duration.as_millis(), 0x42);
        assert!(
            !cds.wrath_pet_info(123, None, now + Duration::from_secs(300))
                .enabled
        );
    }

    #[test]
    fn native_normal_category_keeps_bit27_and_expires_normally() {
        let now = Instant::now();
        let mut cds = Cooldowns::default();
        cds.seed_wrath_pet(
            &Cooldown {
                spell: 0x12abcd,
                category: 0,
                cooldown_ms: 3000,
                category_cooldown_ms: 0,
            },
            None,
            now,
        );
        assert_eq!(cds.wrath_pet_info(0x12abcd, None, now).duration_ms, 3000);
        assert_eq!(
            cds.wrath_pet_info(0x12abcd, None, now + Duration::from_secs(4))
                .remaining_ms,
            0
        );
        cds.seed_wrath_pet(
            &Cooldown {
                spell: 99,
                category: 8,
                cooldown_ms: 0,
                category_cooldown_ms: 0x08000000,
            },
            None,
            now,
        );
        assert_eq!(
            cds.records
                .last()
                .unwrap()
                .category_recovery
                .duration
                .as_millis(),
            0x08000000
        );
        assert!(!cds.records.last().unwrap().on_hold);
    }
}
