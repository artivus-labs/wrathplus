//! The player's cooldown list as the legs the VM reads for `GetSpellCooldown` off the books
//! ([`wrathplus_ui::script::CooldownLeg`]), pushed by [`super::spell_use`]'s feed.

use wrathplus_ui::script::{CooldownKey, CooldownLeg};

use crate::app_core::cooldowns::{CooldownInfo, Cooldowns, Timer};
use crate::app_core::ui_flags::UiClock;

/// The player's cooldown records as the legs `Cooldowns::info` reads, on the frame's one clock
/// pair: the spell leg of a record cast without an item, the category leg (any query for a
/// wildcard row), the GCD leg; a held record's spell and category legs disabled at full duration.
pub(super) fn cooldown_legs(cooldowns: &Cooldowns, clock: &UiClock) -> Vec<CooldownLeg> {
    let leg = |key, t: &Timer, enabled| {
        let duration_ms = t.duration.as_millis().min(u128::from(u32::MAX)) as u32;
        let info = CooldownInfo {
            start: t.start,
            remaining_ms: duration_ms,
            duration_ms,
            enabled,
        };
        let (start_ms, duration_ms, enabled) = info.ui_triple(clock.anchor, clock.ui_now)?;
        Some(CooldownLeg {
            key,
            start_ms,
            duration_ms,
            enabled,
        })
    };
    let legs = cooldowns.records.iter().flat_map(|r| {
        let category = match r.category_wildcard {
            true => Some(CooldownKey::AnyCategory),
            false => (r.category != 0).then_some(CooldownKey::Category(r.category)),
        };
        [
            (r.item_id == 0)
                .then(|| leg(CooldownKey::Spell(r.spell_id), &r.recovery, !r.on_hold))
                .flatten(),
            category.and_then(|k| leg(k, &r.category_recovery, !r.on_hold)),
            leg(CooldownKey::Gcd(r.gcd_category), &r.gcd, true),
        ]
    });
    legs.flatten().collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use wrathplus_formats::SpellDisplay;
    use wrathplus_ui::script::{SpellInfoView, UiScript};

    use super::*;
    use crate::ui_action::spell_use::catalog_facts;
    use crate::ui_action::Spells;

    /// The VM's answer off the books equals `Cooldowns::wrath_pet_info`'s (`0x807980`): a spell's
    /// own leg, an item's category leg, a GCD leg, a held record, and a spell nothing reaches.
    #[test]
    fn off_book_cooldowns_read_as_the_cooldown_list_does() {
        let anchor = Instant::now();
        let clock = UiClock {
            anchor,
            ui_now: 100.0,
        };
        let timer = |ago: u64, ms: u64| Timer {
            start: anchor - Duration::from_millis(ago),
            duration: Duration::from_millis(ms),
        };
        let none = Timer::none(anchor);
        let mut cds = Cooldowns::default();
        cds.add(
            anchor,
            100,
            0,
            timer(2_000, 10_000),
            0,
            false,
            none,
            false,
            0,
            none,
        );
        cds.add(
            anchor,
            200,
            6948,
            none,
            7,
            false,
            timer(1_000, 20_000),
            false,
            0,
            none,
        );
        cds.add(
            anchor,
            300,
            0,
            none,
            0,
            false,
            none,
            false,
            133,
            timer(500, 1_500),
        );
        cds.add(
            anchor,
            400,
            0,
            timer(5_000, 60_000),
            0,
            false,
            none,
            true,
            0,
            none,
        );
        let spell = |category, start_recovery_category| SpellDisplay {
            category,
            start_recovery_category,
            ..Default::default()
        };
        let spells = Spells::empty_for_tests();
        let queries = [
            (100, spell(0, 0)),
            (201, spell(7, 0)),
            (500, spell(0, 133)),
            (400, spell(0, 0)),
            (600, spell(8, 0)),
        ];
        let mut s = UiScript::new().unwrap();
        s.set_now(clock.ui_now);
        s.set_spell_catalog(queries.iter().map(|(id, d)| SpellInfoView {
            spell_id: *id,
            ..catalog_facts(d, &spells)
        }));
        s.set_player_cooldown_legs(cooldown_legs(&cds, &clock));
        let cd = |s: &UiScript, id| -> (f64, f64, i64) {
            s.eval(&format!("return GetSpellCooldown({id})")).unwrap()
        };
        for (id, d) in &queries {
            let want = match cds
                .wrath_pet_info(*id, Some(d), anchor)
                .ui_triple(anchor, clock.ui_now)
            {
                Some((start, dur, on)) => (start as f64 / 1000.0, f64::from(dur) / 1000.0, on),
                None => (0.0, 0.0, true),
            };
            assert_eq!(
                cd(&s, *id),
                (want.0, want.1, i64::from(want.2)),
                "spell {id}"
            );
        }
        // Held: the spell leg answers now and 1 ms (`0x807af2`), the category leg now and its
        // full length (`0x807b95`), both disabled; an id with no row reads cold (`0x8079bf`).
        assert_eq!(cd(&s, 400), (100.0, 0.001, 0), "a held spell leg");
        cds.add(
            anchor,
            700,
            0,
            none,
            351,
            true,
            timer(5_000, 30_000),
            true,
            0,
            none,
        );
        s.set_player_cooldown_legs(cooldown_legs(&cds, &clock));
        assert_eq!(
            (cd(&s, 600), cd(&s, 999)),
            ((100.0, 30.0, 0), (0.0, 0.0, 1))
        );
    }
}
